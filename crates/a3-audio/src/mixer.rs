//! The software mixer: voices, priorities, resampling, panning and filtering into an interleaved
//! stereo `f32` buffer. Runs on the audio thread; deterministic, so tests drive it directly.

use crate::filter::LowPass;
use crate::spatial::{Emitter, Listener, equal_power_pan, spatialize};
use crate::stream::StreamReader;
use crate::{Clip, Stream};

/// Frames mixed with one set of voice parameters; gains ramp across each block.
const BLOCK: usize = 256;
/// Below this summed gain a voice is inaudible and never takes a voice slot.
const SILENT: f32 = 1e-5;
/// Resonance of the distance and occlusion low-pass.
const FILTER_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Identifies a voice; allocated by the caller (see [`crate::AudioEngine`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VoiceId(pub u64);

/// What a voice plays.
#[derive(Debug)]
pub enum Source {
    /// A decoded sound in memory.
    Clip(Clip),
    /// A sound decoded while it plays.
    Stream(Stream),
}

/// How a voice plays.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayParams {
    /// Linear gain.
    pub gain: f32,
    /// Playback rate factor (1 = native pitch).
    pub pitch: f32,
    /// Restart a clip at its end (streams loop when opened looping).
    pub looping: bool,
    /// Higher priorities win voice slots; equal priorities go by loudness.
    pub priority: i32,
    /// Placement in the world; `None` plays the sound "2D" with `pan`.
    pub emitter: Option<Emitter>,
    /// Stereo position of a 2D sound, -1 left to 1 right.
    pub pan: f32,
    /// Fade-in time in seconds.
    pub fade_in: f32,
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pitch: 1.0,
            looping: false,
            priority: 0,
            emitter: None,
            pan: 0.0,
            fade_in: 0.0,
        }
    }
}

/// A change to the mixer, sent from the game thread.
#[derive(Debug)]
pub enum Command {
    /// Start a voice.
    Play {
        /// Its identifier.
        id: VoiceId,
        /// What to play.
        source: Source,
        /// How to play it.
        params: PlayParams,
    },
    /// Fade a voice out over `fade` seconds and remove it.
    Stop {
        /// The voice.
        id: VoiceId,
        /// Fade-out time in seconds (0 = immediately, with a short de-click ramp).
        fade: f32,
    },
    /// Set a voice's gain.
    SetGain(VoiceId, f32),
    /// Set a voice's pitch factor.
    SetPitch(VoiceId, f32),
    /// Set a voice's stereo position (2D voices).
    SetPan(VoiceId, f32),
    /// Replace a voice's emitter (position, velocity, attenuation...).
    SetEmitter(VoiceId, Box<Emitter>),
    /// Move the listener.
    SetListener(Listener),
    /// Set the gain applied to the whole mix.
    SetMasterGain(f32),
    /// Stop every voice.
    StopAll,
}

enum Reader {
    Clip { clip: Clip, looping: bool },
    Stream(StreamReader),
}

impl Reader {
    fn sample_rate(&self) -> u32 {
        match self {
            Reader::Clip { clip, .. } => clip.sample_rate(),
            Reader::Stream(s) => s.sample_rate,
        }
    }

    fn frame(&mut self, index: u64) -> Option<[f32; 2]> {
        match self {
            Reader::Clip { clip, looping } => {
                let frames = clip.frames();
                if frames == 0 {
                    return None;
                }
                if *looping {
                    clip.frame(index % frames)
                } else {
                    clip.frame(index)
                }
            }
            Reader::Stream(s) => s.frame(index),
        }
    }

    fn release(&mut self, index: u64) {
        if let Reader::Stream(s) = self {
            s.release(index);
        }
    }
}

struct Voice {
    id: VoiceId,
    reader: Reader,
    params: PlayParams,
    /// Read position in source frames.
    position: f64,
    /// Fade envelope, 0..1, and its change per output frame.
    envelope: f32,
    envelope_step: f32,
    stopping: bool,
    finished: bool,
    /// Channel gains used at the end of the last block (start of the next ramp).
    gains: [f32; 2],
    filters: [LowPass; 2],
}

/// The per-block result of evaluating a voice.
#[derive(Clone, Copy)]
struct Target {
    gains: [f32; 2],
    step: f64,
    cutoff: Option<f32>,
}

/// Counters of the mixer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MixerStats {
    /// Voices playing (audible or not).
    pub voices: usize,
    /// Voices mixed in the last block.
    pub audible: usize,
    /// Output frames rendered so far.
    pub frames: u64,
}

/// Mixes voices into stereo output at a fixed sample rate.
pub struct Mixer {
    sample_rate: u32,
    max_voices: usize,
    voices: Vec<Voice>,
    listener: Listener,
    master_gain: f32,
    stats: MixerStats,
}

impl Mixer {
    /// A mixer producing `sample_rate` frames per second, mixing at most `max_voices` voices.
    pub fn new(sample_rate: u32, max_voices: usize) -> Self {
        Self {
            sample_rate: sample_rate.max(1),
            max_voices,
            voices: Vec::new(),
            listener: Listener::default(),
            master_gain: 1.0,
            stats: MixerStats::default(),
        }
    }

    /// Output frames per second.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Counters.
    pub fn stats(&self) -> MixerStats {
        MixerStats {
            voices: self.voices.len(),
            ..self.stats
        }
    }

    /// Whether a voice is still playing.
    pub fn is_playing(&self, id: VoiceId) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    /// Applies a command.
    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Play { id, source, params } => self.play(id, source, params),
            Command::Stop { id, fade } => self.stop(id, fade),
            Command::SetGain(id, gain) => self.with_voice(id, |v| v.params.gain = gain),
            Command::SetPitch(id, pitch) => self.with_voice(id, |v| v.params.pitch = pitch),
            Command::SetPan(id, pan) => self.with_voice(id, |v| v.params.pan = pan),
            Command::SetEmitter(id, emitter) => {
                self.with_voice(id, |v| v.params.emitter = Some(*emitter));
            }
            Command::SetListener(listener) => self.listener = listener,
            Command::SetMasterGain(gain) => self.master_gain = gain,
            Command::StopAll => {
                for v in &mut self.voices {
                    v.fade_out(0.0, self.sample_rate);
                }
            }
        }
    }

    fn with_voice(&mut self, id: VoiceId, f: impl FnOnce(&mut Voice)) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
            f(v);
        }
    }

    /// Starts a voice.
    pub fn play(&mut self, id: VoiceId, source: Source, params: PlayParams) {
        let reader = match source {
            Source::Clip(clip) => Reader::Clip {
                clip,
                looping: params.looping,
            },
            Source::Stream(stream) => Reader::Stream(stream.into_reader()),
        };
        let (envelope, envelope_step) = if params.fade_in > 0.0 {
            (0.0, 1.0 / (params.fade_in * self.sample_rate as f32))
        } else {
            (1.0, 0.0)
        };
        self.voices.push(Voice {
            id,
            reader,
            params,
            position: 0.0,
            envelope,
            envelope_step,
            stopping: false,
            finished: false,
            gains: [0.0; 2],
            filters: [LowPass::default(); 2],
        });
    }

    /// Fades a voice out over `fade` seconds, then removes it.
    pub fn stop(&mut self, id: VoiceId, fade: f32) {
        let rate = self.sample_rate;
        self.with_voice(id, |v| v.fade_out(fade, rate));
    }

    /// Mixes the next `out.len() / 2` frames into `out` (interleaved stereo, overwritten).
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for block in out.chunks_mut(BLOCK * 2) {
            self.render_block(block);
        }
        self.voices.retain(|v| !v.finished);
    }

    fn target(&self, v: &Voice) -> Target {
        let gain = v.params.gain.max(0.0);
        let rate = v.reader.sample_rate() as f64 / self.sample_rate as f64;
        let pitch = f64::from(v.params.pitch.max(0.0));
        match &v.params.emitter {
            None => Target {
                gains: equal_power_pan(v.params.pan).map(|g| g * gain),
                step: rate * pitch,
                cutoff: None,
            },
            Some(emitter) => {
                let s = spatialize(&self.listener, emitter);
                Target {
                    gains: s.gains.map(|g| g * gain),
                    step: rate * pitch * f64::from(s.pitch),
                    cutoff: s.cutoff_hz,
                }
            }
        }
    }

    fn render_block(&mut self, out: &mut [f32]) {
        let frames = out.len() / 2;
        let targets: Vec<Target> = self.voices.iter().map(|v| self.target(v)).collect();

        // Pick the voices to mix: audible ones by priority, then by loudness.
        let mut order: Vec<usize> = (0..self.voices.len())
            .filter(|&i| loudness(&targets[i], &self.voices[i]) > SILENT)
            .collect();
        order.sort_by(|&a, &b| {
            let (va, vb) = (&self.voices[a], &self.voices[b]);
            vb.params
                .priority
                .cmp(&va.params.priority)
                .then(loudness(&targets[b], vb).total_cmp(&loudness(&targets[a], va)))
        });
        let mut mixed = vec![false; self.voices.len()];
        for &i in order.iter().take(self.max_voices) {
            mixed[i] = true;
        }

        let rate = self.sample_rate as f32;
        let master = self.master_gain;
        for (i, voice) in self.voices.iter_mut().enumerate() {
            let target = targets[i];
            if mixed[i] {
                voice.mix(out, frames, target, master, rate);
            } else {
                voice.skip(frames, target);
            }
        }
        self.stats.audible = mixed.iter().filter(|&&m| m).count();
        self.stats.frames += frames as u64;
    }
}

fn loudness(target: &Target, voice: &Voice) -> f32 {
    let envelope = if voice.envelope_step > 0.0 && !voice.stopping {
        1.0 // fading in: judge by where it is heading
    } else {
        voice.envelope
    };
    (target.gains[0] + target.gains[1]) * envelope
}

impl Voice {
    fn fade_out(&mut self, seconds: f32, sample_rate: u32) {
        // At least a 5 ms ramp, to avoid a click.
        let frames = (seconds.max(0.005) * sample_rate as f32).max(1.0);
        self.stopping = true;
        self.envelope_step = -self.envelope.max(1e-3) / frames;
    }

    fn advance_envelope(&mut self) {
        if self.envelope_step == 0.0 {
            return;
        }
        self.envelope += self.envelope_step;
        if self.envelope >= 1.0 {
            self.envelope = 1.0;
            self.envelope_step = 0.0;
        } else if self.envelope <= 0.0 {
            self.envelope = 0.0;
            self.envelope_step = 0.0;
            if self.stopping {
                self.finished = true;
            }
        }
    }

    /// Mixes `frames` frames into `out`, ramping the gains from the last block to `target`.
    fn mix(&mut self, out: &mut [f32], frames: usize, target: Target, master: f32, rate: f32) {
        for filter in &mut self.filters {
            match target.cutoff {
                Some(cutoff) => filter.set(cutoff, FILTER_Q, rate),
                None if filter.is_active() => *filter = LowPass::default(),
                None => {}
            }
        }
        let start = self.gains;
        for n in 0..frames {
            if self.finished {
                break;
            }
            let Some(sample) = self.read() else {
                self.finished = true;
                break;
            };
            let t = (n + 1) as f32 / frames as f32;
            let env = self.envelope * master;
            for c in 0..2 {
                let gain = start[c] + (target.gains[c] - start[c]) * t;
                out[2 * n + c] += self.filters[c].process(sample[c]) * gain * env;
            }
            self.position += target.step;
            self.advance_envelope();
        }
        self.gains = target.gains;
        self.reader.release(self.position as u64);
    }

    /// Advances without mixing (a virtual voice).
    fn skip(&mut self, frames: usize, target: Target) {
        let end = self.position + target.step * frames as f64;
        // A one-shot that runs past its end finishes even while virtual.
        if self.reader.frame(end as u64).is_none() {
            self.finished = true;
        }
        self.position = end;
        for _ in 0..frames {
            self.advance_envelope();
        }
        self.gains = [0.0; 2];
        self.reader.release(self.position as u64);
    }

    /// The sample at the current (fractional) position, linearly interpolated.
    fn read(&mut self) -> Option<[f32; 2]> {
        let index = self.position as u64;
        let frac = (self.position - index as f64) as f32;
        let a = self.reader.frame(index)?;
        if frac == 0.0 {
            return Some(a);
        }
        let b = self.reader.frame(index + 1).unwrap_or(a);
        Some([a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac])
    }
}
