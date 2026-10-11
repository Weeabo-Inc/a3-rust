//! The volume buses: the master gain and the sound, music and radio gains under it, each
//! settable at once or faded to a target over a duration in engine ticks.
//!
//! A script's gain is the *scripted* gain (`fadeSound`, `fadeMusic`, `fadeRadio`); the player's
//! in-game setting would multiply it (`Final Volume = Client Setting * Scripted Volume`,
//! community wiki), and we have no settings screen yet, so the scripted gain is the gain.
//! `soundVolume` and its siblings answer the bus, which is what the engine answers too.

/// Engine ticks per second: what a fade's duration is measured in. The engine's simulation step
/// is 1/15 s (`a3-world`'s `DEFAULT_SIMULATION_STEP`, the rate `World::simulate` is fed at), and
/// the SQF `time fadeSound volume`'s `time` is in seconds, converted with
/// [`seconds_to_ticks`].
pub const TICKS_PER_SECOND: f32 = 15.0;

/// A duration in seconds as a number of engine ticks.
pub fn seconds_to_ticks(seconds: f32) -> f32 {
    (seconds * TICKS_PER_SECOND).max(0.0)
}

/// A mixer bus a scripted gain drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bus {
    /// `fadeSound` and `soundVolume`: effects and every other ordinary sound.
    Sound,
    /// `fadeMusic` and `musicVolume`: the music `playMusic` starts.
    Music,
    /// `fadeRadio` and `radioVolume`: radio sentences.
    Radio,
}

/// A gain that can ramp to a target over a number of engine ticks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    gain: f32,
    target: f32,
    /// Change per tick; 0 when the gain has reached its target.
    step: f32,
}

impl Fade {
    /// A gain already at `gain`.
    pub fn new(gain: f32) -> Self {
        let gain = clean(gain);
        Self {
            gain,
            target: gain,
            step: 0.0,
        }
    }

    /// The gain now.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// The gain the fade is heading for (the gain itself when none runs).
    pub fn target(&self) -> f32 {
        self.target
    }

    /// Whether a fade is still running.
    pub fn is_fading(&self) -> bool {
        self.step != 0.0
    }

    /// Sets the gain at once.
    pub fn set(&mut self, gain: f32) {
        *self = Self::new(gain);
    }

    /// Fades to `target` over `ticks` engine ticks; a duration of zero (or less) sets it at once.
    pub fn fade(&mut self, target: f32, ticks: f32) {
        let target = clean(target);
        self.target = target;
        if ticks <= 0.0 || target == self.gain {
            self.gain = target;
            self.step = 0.0;
            return;
        }
        self.step = (target - self.gain) / ticks;
    }

    /// Advances by `ticks` ticks and returns the gain, which stops on the target.
    pub fn step(&mut self, ticks: f32) -> f32 {
        if self.step == 0.0 {
            return self.gain;
        }
        self.gain += self.step * ticks.max(0.0);
        // The step's sign says which side of the target the fade came from: once the difference
        // has that sign too, it has arrived (or overshot, when a long tick jumped past it).
        if (self.target - self.gain) * self.step <= 0.0 {
            self.gain = self.target;
            self.step = 0.0;
        }
        self.gain
    }
}

/// NaN and negative gains both mean silence.
fn clean(gain: f32) -> f32 {
    gain.max(0.0)
}

/// The mixer's volume buses: the master gain over the sound, music and radio gains.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeBus {
    master: Fade,
    sound: Fade,
    music: Fade,
    radio: Fade,
}

impl Default for VolumeBus {
    fn default() -> Self {
        Self::new()
    }
}

impl VolumeBus {
    /// Every bus at full gain, nothing fading.
    pub fn new() -> Self {
        Self {
            master: Fade::new(1.0),
            sound: Fade::new(1.0),
            music: Fade::new(1.0),
            radio: Fade::new(1.0),
        }
    }

    /// The gain of the whole mix (`setMasterGain`).
    pub fn master_gain(&self) -> f32 {
        self.master.gain()
    }

    /// The gain of `bus` now.
    pub fn gain(&self, bus: Bus) -> f32 {
        self.fade(bus).gain()
    }

    /// The gain `bus` is fading to.
    pub fn target(&self, bus: Bus) -> f32 {
        self.fade(bus).target()
    }

    /// Whether `bus` is fading.
    pub fn is_fading(&self, bus: Bus) -> bool {
        self.fade(bus).is_fading()
    }

    /// What a voice on `bus` is multiplied by: its bus gain under the master.
    pub fn voice_gain(&self, bus: Bus) -> f32 {
        self.gain(bus) * self.master.gain()
    }

    /// Sets the master gain at once.
    pub fn set_master_gain(&mut self, gain: f32) {
        self.master.set(gain);
    }

    /// Sets a bus's gain at once (`soundVolume`, `musicVolume`, `radioVolume`).
    pub fn set_gain(&mut self, bus: Bus, gain: f32) {
        self.fade_mut(bus).set(gain);
    }

    /// Fades a bus to `gain` over `ticks` engine ticks (`fadeSound`, `fadeMusic`, `fadeRadio`).
    pub fn fade_gain(&mut self, bus: Bus, gain: f32, ticks: f32) {
        self.fade_mut(bus).fade(gain, ticks);
    }

    /// Advances every running fade by `ticks` engine ticks.
    pub fn step(&mut self, ticks: f32) {
        self.master.step(ticks);
        self.sound.step(ticks);
        self.music.step(ticks);
        self.radio.step(ticks);
    }

    fn fade(&self, bus: Bus) -> &Fade {
        match bus {
            Bus::Sound => &self.sound,
            Bus::Music => &self.music,
            Bus::Radio => &self.radio,
        }
    }

    fn fade_mut(&mut self, bus: Bus) -> &mut Fade {
        match bus {
            Bus::Sound => &mut self.sound,
            Bus::Music => &mut self.music,
            Bus::Radio => &mut self.radio,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_interpolates_over_its_ticks_and_stops_on_the_target() {
        let mut fade = Fade::new(1.0);
        fade.fade(0.0, 15.0);
        assert!(fade.is_fading());
        assert_eq!(fade.gain(), 1.0);
        fade.step(5.0);
        assert!((fade.gain() - 2.0 / 3.0).abs() < 1e-6, "{}", fade.gain());
        fade.step(10.0);
        assert_eq!(fade.gain(), 0.0);
        assert_eq!(fade.target(), 0.0);
        assert!(!fade.is_fading());
    }

    #[test]
    fn a_long_tick_does_not_overshoot_the_target() {
        let mut fade = Fade::new(1.0);
        fade.fade(0.25, 2.0);
        assert_eq!(fade.step(30.0), 0.25);
        assert!(!fade.is_fading());
    }

    #[test]
    fn a_zero_length_fade_sets_the_gain_at_once() {
        let mut fade = Fade::new(1.0);
        fade.fade(0.5, 0.0);
        assert_eq!(fade.gain(), 0.5);
        assert!(!fade.is_fading());
    }

    #[test]
    fn seconds_convert_to_engine_ticks() {
        assert_eq!(seconds_to_ticks(0.0), 0.0);
        assert!((seconds_to_ticks(1.0) - TICKS_PER_SECOND).abs() < 1e-6);
        assert_eq!(seconds_to_ticks(-3.0), 0.0);
    }

    #[test]
    fn the_buses_scale_a_voice_independently_of_the_master() {
        let mut bus = VolumeBus::new();
        bus.fade_gain(Bus::Music, 0.0, 15.0);
        bus.set_gain(Bus::Sound, 0.5);
        assert_eq!(bus.voice_gain(Bus::Sound), 0.5);
        assert_eq!(bus.voice_gain(Bus::Music), 1.0);
        // Half the fade's 15 ticks: the music bus is half way to silence.
        bus.step(7.5);
        assert!((bus.voice_gain(Bus::Music) - 0.5).abs() < 1e-6);
        assert!((bus.gain(Bus::Music) - 0.5).abs() < 1e-6);
        bus.set_master_gain(0.5);
        assert!((bus.voice_gain(Bus::Music) - 0.25).abs() < 1e-6);
        assert_eq!(bus.voice_gain(Bus::Radio), 0.5);
    }
}
