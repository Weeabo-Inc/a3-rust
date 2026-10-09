//! The sound commands: the scripted volumes (`fadeSound`, `soundVolume`, `fadeMusic`,
//! `musicVolume`, `fadeRadio`, `radioVolume`) and playback (`playSound`, `playMusic`).
//!
//! The scripted volumes are the engine's audio buses, reached through [`AudioHost`] — the host
//! answers one from [`WorldHost::audio`](super::WorldHost::audio). Every method of the trait is
//! a no-op by default, so a host with no audio engine (a headless tool, a test) keeps compiling;
//! its scripts set nothing and `soundVolume` answers the engine's full volume. `apps/arma3`
//! implements the trait over its real `a3_audio::AudioEngine`.
//!
//! Contracts and handler RVAs: `docs/re/sqf-commands.tsv`, which has the nullary getters
//! (`soundVolume` 0x8aaf30, `musicVolume` 0x8a9c40, `radioVolume` 0x8aaa50) and the binary fades
//! (`fadeSound` 0x557200, `fadeMusic` 0x553190, `fadeRadio` 0x556460, each `time` and `volume`
//! scalars, returning Nothing). The scalar and array setters of `soundVolume` are ours: the
//! engine's table declares only the getter, and a script that assigns the volume gets the bus
//! set rather than a script error.
//!
//! All of these are argument global / effect local (`AG EL`, community wiki): they set the
//! volume of *this* machine's audio and send nothing over the network.

use a3_sqf::vm::Ctx;
use a3_sqf::{Registry, Value};

use super::{ARR, NOTHING, NUM, OBJ, STR, WorldHost, null_object};

/// The engine's audio buses, as the SQF sound commands drive them. A host that has an audio
/// engine implements this over it; the defaults are no-ops, so a host without one answers the
/// engine's defaults instead.
pub trait AudioHost {
    /// The sound bus gain at once (`volume soundVolume`).
    fn set_sound_volume(&mut self, _volume: f32) {}

    /// `time fadeSound volume`: fade the sound bus to `volume` over `time` seconds.
    fn fade_sound(&mut self, _seconds: f32, _volume: f32) {}

    /// The sound bus gain now (`soundVolume`).
    fn sound_volume(&self) -> f32 {
        1.0
    }

    /// The music bus gain at once; the engine sets music through `fadeMusic` alone, so this is
    /// the direct form of the same thing.
    fn set_music_volume(&mut self, _volume: f32) {}

    /// `time fadeMusic volume`: fade the music bus to `volume` over `time` seconds.
    fn fade_music(&mut self, _seconds: f32, _volume: f32) {}

    /// The music bus gain now (`musicVolume`).
    fn music_volume(&self) -> f32 {
        1.0
    }

    /// `time fadeRadio volume`: fade the radio bus to `volume` over `time` seconds.
    fn fade_radio(&mut self, _seconds: f32, _volume: f32) {}

    /// The radio bus gain now (`radioVolume`).
    fn radio_volume(&self) -> f32 {
        1.0
    }

    /// `playSound name`: start the `CfgSounds` class (or `CfgSoundSets` set) `name` as a sound
    /// on the sound bus. `false` when the host has nothing to play it with, or the name is
    /// unknown.
    fn play_sound(&mut self, _name: &str) -> bool {
        false
    }

    /// `playMusic name`: start the `CfgMusic` class `name`, replacing the music playing; the
    /// empty name stops it. `false` when the host plays no music, or the name is unknown.
    fn play_music(&mut self, _name: &str) -> bool {
        false
    }
}

pub(super) fn register<H: WorldHost>(r: &mut Registry<H>) {
    // 0x557200: `time fadeSound volume`. `time` is the fade interval in seconds and `volume` is
    // 0..1 (community wiki, "fadeSound"); the bus interpolates over the interval.
    r.binary("fadeSound", NUM, NUM, NOTHING, |ctx, a, b| {
        let seconds = number(&a);
        let volume = b.as_number().unwrap_or(1.0);
        if let Some(audio) = ctx.host.audio() {
            audio.fade_sound(seconds, volume);
        }
        Ok(Value::Nothing)
    });

    // 0x8aaf30: `soundVolume`, the sound bus gain that `fadeSound` set. The engine's table has
    // the getter alone; the two setters below are ours (see the module doc).
    r.nular("soundVolume", NUM, |ctx| {
        Ok(Value::Number(
            ctx.host.audio().map_or(1.0, |audio| audio.sound_volume()),
        ))
    });
    r.unary("soundVolume", NUM, NOTHING, |ctx, a| {
        set_sound_volume(ctx, a.as_number().unwrap_or(1.0));
        Ok(Value::Nothing)
    });
    r.unary("soundVolume", ARR, NOTHING, |ctx, a| {
        set_sound_volume(ctx, first_number(&a).unwrap_or(1.0));
        Ok(Value::Nothing)
    });

    // 0x553190: `time fadeMusic volume`. The World keeps the scripted music volume as well
    // (`World::music_volume`), which is what `musicVolume` answers without an audio engine.
    r.binary("fadeMusic", NUM, NUM, NOTHING, |ctx, a, b| {
        let seconds = number(&a);
        let volume = b.as_number().unwrap_or(1.0);
        ctx.host.world_mut().set_music_volume(volume);
        if let Some(audio) = ctx.host.audio() {
            audio.fade_music(seconds, volume);
        }
        Ok(Value::Nothing)
    });

    // 0x8a9c40: `musicVolume`, the music bus gain. The engine's range is 0..5 (community wiki),
    // because the player's music setting multiplies it; we have no settings screen, so it is
    // the bus gain under the master.
    r.nular("musicVolume", NUM, |ctx| {
        if let Some(audio) = ctx.host.audio() {
            return Ok(Value::Number(audio.music_volume()));
        }
        Ok(Value::Number(ctx.host.world().music_volume()))
    });

    // 0x556460: `time fadeRadio volume`, `volume` 0..2 (community wiki).
    r.binary("fadeRadio", NUM, NUM, NOTHING, |ctx, a, b| {
        let seconds = number(&a);
        let volume = b.as_number().unwrap_or(1.0);
        if let Some(audio) = ctx.host.audio() {
            audio.fade_radio(seconds, volume);
        }
        Ok(Value::Nothing)
    });

    // 0x8aaa50: `radioVolume`, the radio bus gain that `fadeRadio` set.
    r.nular("radioVolume", NUM, |ctx| {
        Ok(Value::Number(
            ctx.host.audio().map_or(1.0, |audio| audio.radio_volume()),
        ))
    });

    // 0x5458c0: `playSound name`, and its array form `[name, isSpeech, offset]`. The engine
    // answers the speaker Object; a 2D sound from `CfgSounds` has none, so the value is
    // `objNull`. `isSpeech` and `offset` need a speech bus and a playback offset the mixer does
    // not have yet, so they are accepted and ignored.
    r.unary("playSound", STR, OBJ, |ctx, a| {
        play_sound(ctx, a.as_str().unwrap_or_default());
        Ok(null_object())
    });
    r.unary("playSound", ARR, OBJ, |ctx, a| {
        let name = first_string(&a).unwrap_or_default();
        play_sound(ctx, &name);
        Ok(null_object())
    });

    // 0x544c00: `playMusic name`, and `playMusic [name, start]`, where `start` is the position
    // in seconds to start at. The mixer plays a clip from its beginning, so `start` is accepted
    // and ignored. An unknown name logs the engine's "Music <name> not found" (in the host).
    r.unary("playMusic", STR, NOTHING, |ctx, a| {
        play_music(ctx, a.as_str().unwrap_or_default());
        Ok(Value::Nothing)
    });
    r.unary("playMusic", ARR, NOTHING, |ctx, a| {
        let name = first_string(&a).unwrap_or_default();
        play_music(ctx, &name);
        Ok(Value::Nothing)
    });
}

/// Sets the sound bus gain on a host that has one.
fn set_sound_volume<H: WorldHost>(ctx: &mut Ctx<'_, H>, volume: f32) {
    if let Some(audio) = ctx.host.audio() {
        audio.set_sound_volume(volume);
    }
}

/// Starts `name` on a host that has an audio engine.
fn play_sound<H: WorldHost>(ctx: &mut Ctx<'_, H>, name: &str) {
    if let Some(audio) = ctx.host.audio() {
        audio.play_sound(name);
    }
}

/// Starts (or, for the empty name, stops) the music on a host that has an audio engine.
fn play_music<H: WorldHost>(ctx: &mut Ctx<'_, H>, name: &str) {
    if let Some(audio) = ctx.host.audio() {
        audio.play_music(name);
    }
}

/// A scalar argument: `nil` (which the VM never passes) reads as 0.
fn number(value: &Value) -> f32 {
    value.as_number().unwrap_or(0.0)
}

/// The first number of an array value.
fn first_number(value: &Value) -> Option<f32> {
    let array = value.as_array()?;
    let items = array.borrow();
    items.first()?.as_number()
}

/// The first string of an array value.
fn first_string(value: &Value) -> Option<String> {
    let array = value.as_array()?;
    let items = array.borrow();
    Some(items.first()?.as_str()?.to_owned())
}
