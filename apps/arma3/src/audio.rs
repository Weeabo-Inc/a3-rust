//! The game's sound output: the real [`AudioEngine`], the config sound bank the script commands
//! name sounds in, and the scripted volume buses they drive.
//!
//! This is the app's [`AudioHost`]: a world script host that has a `GameAudio` answers it from
//! `WorldHost::audio`, and then `5 fadeSound 0`, `playSound "Alarm"` or `playMusic "Wasteland"`
//! change what is heard. The engine starts with the World (its audio device, or the silent null
//! backend on a machine without one), and every frame moves the listener with the camera so 3D
//! sounds are heard from where the view is.

use a3_audio::config::{SoundBank, SoundSet};
use a3_audio::player::{Rng, choose_sample, start_levels};
use a3_audio::{
    AudioEngine, Bus, EngineConfig, Listener, PlayParams, SoundLoader, VoiceId, seconds_to_ticks,
};
use a3_config::ConfigTree;
use a3_vfs::Vfs;
use a3_world::script::AudioHost;

use glam::{DVec3, Vec3};

/// The sounds the game plays: the output engine, the sound files and the `CfgSounds`/`CfgMusic`
/// classes of the loaded config.
pub struct GameAudio {
    engine: AudioEngine,
    loader: SoundLoader,
    bank: SoundBank,
    /// The music playing now (`playMusic`), so the next call can stop it.
    music: Option<VoiceId>,
    /// The engine's sample choice (`Rng`): `choose_sample`'s draws.
    rng: Rng,
}

impl GameAudio {
    /// Starts the output and loads the sound bank of `config` over the game files in `vfs`.
    pub fn new(vfs: Vfs, config: &ConfigTree) -> Self {
        let bank = SoundBank::load(config);
        let loader = SoundLoader::new(vfs);
        let engine = AudioEngine::start_or_null(EngineConfig::default());
        let [
            shaders,
            sets,
            curves,
            filters,
            processors,
            sounds,
            music,
            sfx,
        ] = bank.counts();
        log::info!(
            "audio: {:?} output at {} Hz; {sounds} CfgSounds, {music} CfgMusic, {sets} sound sets \
             ({shaders} shaders, {curves} curves, {filters} filters, {processors} processors, \
             {sfx} SFX)",
            engine.backend(),
            engine.sample_rate()
        );
        for warning in &bank.warnings {
            log::warn!("audio config: {warning}");
        }
        Self {
            engine,
            loader,
            bank,
            music: None,
            rng: Rng::new(0x1234_5678),
        }
    }

    /// Moves the listener to the camera's frame, so 3D sounds are placed relative to the view.
    pub fn set_listener(&self, position: DVec3, forward: Vec3) {
        self.engine.set_listener(Listener {
            position,
            forward,
            ..Listener::default()
        });
    }

    /// Plays the samples of a `CfgSoundSets` set `name` once, each shader on the bus the command
    /// asked for: the same gains the engine's sound-set player starts a set with
    /// (`a3_audio::player`).
    fn play_set(&mut self, set: &SoundSet, bus: Bus) -> bool {
        let (volume, frequency) = start_levels(set, &mut self.rng);
        let mut started = false;
        for shader_name in &set.shaders {
            let Some(shader) = self.bank.shader(shader_name) else {
                continue;
            };
            let weights: Vec<f32> = shader.samples.iter().map(|s| s.probability).collect();
            let Some(sample) = choose_sample(&weights, None, &mut self.rng) else {
                continue;
            };
            let Some(source) = self.loader.source(&shader.samples[sample].path, false) else {
                continue;
            };
            self.engine.play(
                source,
                PlayParams {
                    gain: volume,
                    pitch: frequency,
                    bus,
                    ..PlayParams::default()
                },
            );
            started = true;
        }
        started
    }
}

impl AudioHost for GameAudio {
    fn set_sound_volume(&mut self, volume: f32) {
        self.engine.set_sound_gain(volume);
    }

    fn fade_sound(&mut self, seconds: f32, volume: f32) {
        self.engine.fade_sound(volume, seconds_to_ticks(seconds));
    }

    fn sound_volume(&self) -> f32 {
        self.engine.sound_gain()
    }

    fn set_music_volume(&mut self, volume: f32) {
        self.engine.set_music_gain(volume);
    }

    fn fade_music(&mut self, seconds: f32, volume: f32) {
        self.engine.fade_music(volume, seconds_to_ticks(seconds));
    }

    fn music_volume(&self) -> f32 {
        self.engine.music_gain()
    }

    fn fade_radio(&mut self, seconds: f32, volume: f32) {
        self.engine.fade_radio(volume, seconds_to_ticks(seconds));
    }

    fn radio_volume(&self) -> f32 {
        self.engine.radio_gain()
    }

    /// A `CfgSounds` class (its `sound[]` path, volume and pitch) or a `CfgSoundSets` set, played
    /// once as a 2D sound on the sound bus.
    fn play_sound(&mut self, name: &str) -> bool {
        if let Some(sound) = self.bank.sound(name) {
            let Some(source) = self.loader.source(&sound.path, false) else {
                return false;
            };
            self.engine.play(
                source,
                PlayParams {
                    gain: sound.volume,
                    pitch: sound.pitch,
                    bus: Bus::Sound,
                    ..PlayParams::default()
                },
            );
            return true;
        }
        let Some(set) = self.bank.set(name).cloned() else {
            log::warn!("Sound {name} not found");
            return false;
        };
        self.play_set(&set, Bus::Sound)
    }

    /// A `CfgMusic` class, looping, on the music bus; the empty name stops the music playing.
    fn play_music(&mut self, name: &str) -> bool {
        if let Some(playing) = self.music.take() {
            self.engine.stop(playing, 0.5);
        }
        if name.is_empty() {
            return true;
        }
        let Some(music) = self.bank.music(name) else {
            log::warn!("Music {name} not found");
            return false;
        };
        let Some(source) = self.loader.source(&music.path, true) else {
            return false;
        };
        self.music = Some(self.engine.play(
            source,
            PlayParams {
                gain: music.volume,
                pitch: music.pitch,
                looping: true,
                bus: Bus::Music,
                ..PlayParams::default()
            },
        ));
        true
    }
}
