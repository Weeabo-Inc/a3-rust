//! The game-side handle: owns the output thread and sends commands to the mixer.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::volume::Bus;
use crate::{Command, Emitter, Error, Listener, Mixer, PlayParams, Result, Source, VoiceId};

/// Where the mix goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The system's default output device, through cpal.
    Device,
    /// Nowhere: the mixer runs in real time on a thread and its output is discarded. For CI,
    /// machines without audio, and `--audio off`.
    Null,
}

/// How to start the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    /// Output backend.
    pub backend: Backend,
    /// Voices mixed at once; quieter and lower-priority voices play silently beyond this.
    pub max_voices: usize,
    /// Sample rate of the null backend.
    pub null_sample_rate: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            backend: Backend::Device,
            max_voices: 64,
            null_sample_rate: 48_000,
        }
    }
}

/// Counters published by the audio thread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// Voices playing.
    pub voices: usize,
    /// Voices mixed in the last block.
    pub audible: usize,
    /// Frames rendered since start.
    pub frames: u64,
}

/// State published by the audio thread for the game thread to read.
struct Shared {
    voices: AtomicUsize,
    audible: AtomicUsize,
    frames: AtomicU64,
    /// The current bus gains, as `f32` bits (`soundVolume`, `musicVolume`, `radioVolume`).
    sound_gain: AtomicU32,
    music_gain: AtomicU32,
    radio_gain: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            voices: AtomicUsize::new(0),
            audible: AtomicUsize::new(0),
            frames: AtomicU64::new(0),
            // Every bus starts at full gain, before the first block is published.
            sound_gain: AtomicU32::new(1.0f32.to_bits()),
            music_gain: AtomicU32::new(1.0f32.to_bits()),
            radio_gain: AtomicU32::new(1.0f32.to_bits()),
        }
    }
}

impl Shared {
    fn publish(&self, mixer: &Mixer) {
        let s = mixer.stats();
        self.voices.store(s.voices, Ordering::Relaxed);
        self.audible.store(s.audible, Ordering::Relaxed);
        self.frames.store(s.frames, Ordering::Relaxed);
        let bus = mixer.buses();
        self.sound_gain
            .store(bus.gain(Bus::Sound).to_bits(), Ordering::Relaxed);
        self.music_gain
            .store(bus.gain(Bus::Music).to_bits(), Ordering::Relaxed);
        self.radio_gain
            .store(bus.gain(Bus::Radio).to_bits(), Ordering::Relaxed);
    }
}

/// The running audio engine. Dropping it stops the output.
pub struct AudioEngine {
    commands: Sender<Command>,
    next_id: AtomicU64,
    shared: Arc<Shared>,
    sample_rate: u32,
    backend: Backend,
    shutdown: Option<Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioEngine {
    /// Starts the engine on the configured backend.
    pub fn start(config: EngineConfig) -> Result<Self> {
        let (commands, rx) = channel();
        let (shutdown, shutdown_rx) = channel();
        let (ready_tx, ready_rx) = channel();
        let shared = Arc::new(Shared::default());
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || match config.backend {
                Backend::Null => {
                    let mixer = Mixer::new(config.null_sample_rate, config.max_voices);
                    let _ = ready_tx.send(Ok(config.null_sample_rate));
                    run_null(mixer, rx, shutdown_rx, &thread_shared);
                }
                Backend::Device => match open_device(config.max_voices, rx, thread_shared) {
                    Ok((stream, rate)) => {
                        let _ = ready_tx.send(Ok(rate));
                        // Keep the stream alive until the engine is dropped.
                        let _ = shutdown_rx.recv();
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                    }
                },
            })
            .map_err(|e| Error::Device(e.to_string()))?;
        let sample_rate = ready_rx
            .recv()
            .map_err(|_| Error::Device("audio thread exited during start".into()))??;
        log::info!("audio: {:?} output at {sample_rate} Hz", config.backend);
        Ok(Self {
            commands,
            next_id: AtomicU64::new(1),
            shared,
            sample_rate,
            backend: config.backend,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    /// Starts on the configured backend, falling back to [`Backend::Null`] if the device fails.
    pub fn start_or_null(config: EngineConfig) -> Self {
        match Self::start(config) {
            Ok(engine) => engine,
            Err(e) => {
                log::warn!("audio: {e}; continuing without sound output");
                Self::start(EngineConfig {
                    backend: Backend::Null,
                    ..config
                })
                .expect("the null backend always starts")
            }
        }
    }

    /// The backend in use.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Output frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Counters from the audio thread.
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            voices: self.shared.voices.load(Ordering::Relaxed),
            audible: self.shared.audible.load(Ordering::Relaxed),
            frames: self.shared.frames.load(Ordering::Relaxed),
        }
    }

    fn send(&self, command: Command) {
        // The audio thread only goes away on drop.
        let _ = self.commands.send(command);
    }

    /// Starts a voice.
    pub fn play(&self, source: Source, params: PlayParams) -> VoiceId {
        let id = VoiceId(self.next_id.fetch_add(1, Ordering::Relaxed));
        self.send(Command::Play { id, source, params });
        id
    }

    /// Fades a voice out over `fade` seconds.
    pub fn stop(&self, id: VoiceId, fade: f32) {
        self.send(Command::Stop { id, fade });
    }

    /// Sets a voice's linear gain.
    pub fn set_gain(&self, id: VoiceId, gain: f32) {
        self.send(Command::SetGain(id, gain));
    }

    /// Sets a voice's pitch factor.
    pub fn set_pitch(&self, id: VoiceId, pitch: f32) {
        self.send(Command::SetPitch(id, pitch));
    }

    /// Sets a 2D voice's stereo position.
    pub fn set_pan(&self, id: VoiceId, pan: f32) {
        self.send(Command::SetPan(id, pan));
    }

    /// Replaces a 3D voice's emitter.
    pub fn set_emitter(&self, id: VoiceId, emitter: Emitter) {
        self.send(Command::SetEmitter(id, Box::new(emitter)));
    }

    /// Moves the listener.
    pub fn set_listener(&self, listener: Listener) {
        self.send(Command::SetListener(listener));
    }

    /// Sets the gain of the whole mix.
    pub fn set_master_gain(&self, gain: f32) {
        self.send(Command::SetMasterGain(gain));
    }

    /// The sound bus gain now (`soundVolume`).
    pub fn sound_gain(&self) -> f32 {
        f32::from_bits(self.shared.sound_gain.load(Ordering::Relaxed))
    }

    /// Sets the sound bus gain at once (`soundVolume`).
    pub fn set_sound_gain(&self, gain: f32) {
        self.send(Command::SetBusGain(Bus::Sound, gain));
    }

    /// Fades the sound bus to `gain` over `ticks` engine ticks (`time fadeSound volume`; the
    /// SQF time is seconds, converted with [`crate::seconds_to_ticks`]).
    pub fn fade_sound(&self, gain: f32, ticks: f32) {
        self.send(Command::FadeBus {
            bus: Bus::Sound,
            target: gain,
            ticks,
        });
    }

    /// The music bus gain now (`musicVolume`).
    pub fn music_gain(&self) -> f32 {
        f32::from_bits(self.shared.music_gain.load(Ordering::Relaxed))
    }

    /// Sets the music bus gain at once (`musicVolume`).
    pub fn set_music_gain(&self, gain: f32) {
        self.send(Command::SetBusGain(Bus::Music, gain));
    }

    /// Fades the music bus to `gain` over `ticks` engine ticks (`time fadeMusic volume`).
    pub fn fade_music(&self, gain: f32, ticks: f32) {
        self.send(Command::FadeBus {
            bus: Bus::Music,
            target: gain,
            ticks,
        });
    }

    /// The radio bus gain now (`radioVolume`).
    pub fn radio_gain(&self) -> f32 {
        f32::from_bits(self.shared.radio_gain.load(Ordering::Relaxed))
    }

    /// Sets the radio bus gain at once (`radioVolume`).
    pub fn set_radio_gain(&self, gain: f32) {
        self.send(Command::SetBusGain(Bus::Radio, gain));
    }

    /// Fades the radio bus to `gain` over `ticks` engine ticks (`time fadeRadio volume`).
    pub fn fade_radio(&self, gain: f32, ticks: f32) {
        self.send(Command::FadeBus {
            bus: Bus::Radio,
            target: gain,
            ticks,
        });
    }

    /// Stops every voice.
    pub fn stop_all(&self) {
        self.send(Command::StopAll);
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn drain(mixer: &mut Mixer, commands: &Receiver<Command>) {
    while let Ok(command) = commands.try_recv() {
        mixer.apply(command);
    }
}

/// The null backend: mix in real time in 10 ms blocks and discard the result.
fn run_null(
    mut mixer: Mixer,
    commands: Receiver<Command>,
    shutdown: Receiver<()>,
    shared: &Shared,
) {
    const BLOCK: Duration = Duration::from_millis(10);
    let frames = (mixer.sample_rate() as usize / 100).max(1);
    let mut buffer = vec![0.0f32; frames * 2];
    let mut next = Instant::now();
    loop {
        match shutdown.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        drain(&mut mixer, &commands);
        mixer.render(&mut buffer);
        shared.publish(&mixer);
        next += BLOCK;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        } else {
            next = Instant::now();
        }
    }
}

fn open_device(
    max_voices: usize,
    commands: Receiver<Command>,
    shared: Arc<Shared>,
) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| Error::Device("no default output device".into()))?;
    let supported = device
        .default_output_config()
        .map_err(|e| Error::Device(e.to_string()))?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate.0;
    let mixer = Mixer::new(rate, max_voices);
    log::info!(
        "audio: device {:?}, {} channels, {format:?}",
        device.name().unwrap_or_default(),
        config.channels
    );
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, mixer, commands, shared),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, mixer, commands, shared),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, mixer, commands, shared),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, mixer, commands, shared),
        other => Err(Error::Device(format!(
            "unsupported sample format {other:?}"
        ))),
    }?;
    stream.play().map_err(|e| Error::Device(e.to_string()))?;
    Ok((stream, rate))
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut mixer: Mixer,
    commands: Receiver<Command>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels.max(1));
    let mut stereo = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                drain(&mut mixer, &commands);
                let frames = data.len() / channels;
                stereo.resize(frames * 2, 0.0);
                mixer.render(&mut stereo);
                write_interleaved(data, channels, &stereo);
                shared.publish(&mixer);
            },
            |e| log::warn!("audio output error: {e}"),
            None,
        )
        .map_err(|e| Error::Device(e.to_string()))
}

/// Writes stereo frames to a device buffer with `channels` channels: left and right go to the
/// first two channels (mono devices get their average), any others are silent.
fn write_interleaved<T: cpal::Sample + cpal::FromSample<f32>>(
    data: &mut [T],
    channels: usize,
    stereo: &[f32],
) {
    for (frame, lr) in data.chunks_mut(channels).zip(stereo.chunks_exact(2)) {
        if channels == 1 {
            frame[0] = T::from_sample(0.5 * (lr[0] + lr[1]));
            continue;
        }
        for (c, sample) in frame.iter_mut().enumerate() {
            *sample = T::from_sample(if c < 2 { lr[c] } else { 0.0 });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scripted bus gains are published by the audio thread, so the game thread can answer
    /// `soundVolume` while the mixer interpolates a fade.
    #[test]
    fn the_engine_publishes_the_scripted_bus_gains() {
        let engine = AudioEngine::start(EngineConfig {
            backend: Backend::Null,
            ..EngineConfig::default()
        })
        .expect("the null backend always starts");
        assert_eq!(engine.sound_gain(), 1.0);
        assert_eq!(engine.music_gain(), 1.0);
        engine.set_sound_gain(0.25);
        // The audio thread publishes after each of its 10 ms blocks.
        let set = wait_for(&engine, |gain| (gain - 0.25).abs() < 1e-6);
        assert_eq!(set, Some(0.25), "the sound bus gain was never published");
        // A 15-tick fade (1 s at 15 ticks per second) from 0.25 towards silence: the published
        // gain moves below 0.25 instead of jumping to the target or staying put.
        engine.fade_sound(0.0, 15.0);
        let faded = wait_for(&engine, |gain| gain < 0.25);
        assert!(
            faded.is_some_and(|gain| (0.0..0.25).contains(&gain)),
            "{:?}",
            engine.sound_gain()
        );
    }

    /// Waits up to five seconds for the audio thread to publish a bus gain `want` accepts,
    /// returning the first gain it accepted.
    fn wait_for(engine: &AudioEngine, want: impl Fn(f32) -> bool) -> Option<f32> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let gain = engine.sound_gain();
            if want(gain) {
                return Some(gain);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn stereo_is_spread_over_device_channels() {
        let stereo = [0.5, -0.5, 1.0, 0.0];
        let mut quad = [9.0f32; 8];
        write_interleaved(&mut quad, 4, &stereo);
        assert_eq!(quad, [0.5, -0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        let mut mono = [9.0f32; 2];
        write_interleaved(&mut mono, 1, &stereo);
        assert_eq!(mono, [0.0, 0.5]);
        let mut ints = [0i16; 2];
        write_interleaved(&mut ints, 2, &stereo[..2]);
        assert_eq!(ints, [16384, -16384]);
    }
}
