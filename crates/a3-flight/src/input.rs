//! The pilot's user actions: what the original reads from the input each step for a helicopter
//! (`0x140da2c50`) or a plane (`0x140d1c5f0`), by `CfgDefaultKeysPresets` action name
//! (`docs/re/sim-air.md` §2.3, §3.2).
//!
//! Each value is the action's analogue state, 0..1 (keys give 0 or 1, mouse and joystick axes
//! anything between). The mouse is bound through the same actions (`heliCyclicForward` takes
//! the mouse moving down in the default preset).

/// The flight actions of one step, 0..1 each.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FlightInput {
    /// `heliUp` (UA 221): plane thrust up.
    pub heli_up: f64,
    /// `heliDown` (222): plane thrust down.
    pub heli_down: f64,
    /// `heliLeft` (223): helicopter yaw (low speed) or bank (fast); plane rudder and aileron.
    pub heli_left: f64,
    /// `heliRight` (224).
    pub heli_right: f64,
    /// `airBankLeft` (225): plane aileron.
    pub air_bank_left: f64,
    /// `airBankRight` (226).
    pub air_bank_right: f64,
    /// `heliRudderLeft` (227): pedal.
    pub heli_rudder_left: f64,
    /// `heliRudderRight` (228).
    pub heli_rudder_right: f64,
    /// `heliForward` (229): plane elevator, nose down.
    pub heli_forward: f64,
    /// `heliBack` (230): plane elevator, nose up.
    pub heli_back: f64,
    /// `heliFastForward` (231): plane elevator, nose down.
    pub heli_fast_forward: f64,
    /// `heliThrottlePos` (236): plane throttle axis.
    pub heli_throttle_pos: f64,
    /// `heliThrottleNeg` (237): plane brake axis.
    pub heli_throttle_neg: f64,
    /// `airPlaneBrake` (239): wheel brake and airbrake.
    pub air_plane_brake: f64,
    /// `heliCyclicForward` (240).
    pub heli_cyclic_forward: f64,
    /// `heliCyclicBack` (241).
    pub heli_cyclic_back: f64,
    /// `heliCyclicLeft` (242).
    pub heli_cyclic_left: f64,
    /// `heliCyclicRight` (243).
    pub heli_cyclic_right: f64,
    /// `heliCollectiveRaise` (244): climb (the digital collective).
    pub heli_collective_raise: f64,
    /// `heliCollectiveLower` (245): descend.
    pub heli_collective_lower: f64,
    /// `heliCollectiveRaiseCont` (246): the collective axis.
    pub heli_collective_raise_cont: f64,
    /// `heliCollectiveLowerCont` (247).
    pub heli_collective_lower_cont: f64,
}

impl FlightInput {
    /// Every action name this input reads, as `CfgDefaultKeysPresets` spells it.
    pub const ACTIONS: [&'static str; 22] = [
        "heliUp",
        "heliDown",
        "heliLeft",
        "heliRight",
        "airBankLeft",
        "airBankRight",
        "heliRudderLeft",
        "heliRudderRight",
        "heliForward",
        "heliBack",
        "heliFastForward",
        "heliThrottlePos",
        "heliThrottleNeg",
        "airPlaneBrake",
        "heliCyclicForward",
        "heliCyclicBack",
        "heliCyclicLeft",
        "heliCyclicRight",
        "heliCollectiveRaise",
        "heliCollectiveLower",
        "heliCollectiveRaiseCont",
        "heliCollectiveLowerCont",
    ];

    /// Builds the input from a lookup of action values by name (case as in [`Self::ACTIONS`]).
    pub fn from_actions(value: impl Fn(&str) -> f64) -> FlightInput {
        FlightInput {
            heli_up: value("heliUp"),
            heli_down: value("heliDown"),
            heli_left: value("heliLeft"),
            heli_right: value("heliRight"),
            air_bank_left: value("airBankLeft"),
            air_bank_right: value("airBankRight"),
            heli_rudder_left: value("heliRudderLeft"),
            heli_rudder_right: value("heliRudderRight"),
            heli_forward: value("heliForward"),
            heli_back: value("heliBack"),
            heli_fast_forward: value("heliFastForward"),
            heli_throttle_pos: value("heliThrottlePos"),
            heli_throttle_neg: value("heliThrottleNeg"),
            air_plane_brake: value("airPlaneBrake"),
            heli_cyclic_forward: value("heliCyclicForward"),
            heli_cyclic_back: value("heliCyclicBack"),
            heli_cyclic_left: value("heliCyclicLeft"),
            heli_cyclic_right: value("heliCyclicRight"),
            heli_collective_raise: value("heliCollectiveRaise"),
            heli_collective_lower: value("heliCollectiveLower"),
            heli_collective_raise_cont: value("heliCollectiveRaiseCont"),
            heli_collective_lower_cont: value("heliCollectiveLowerCont"),
        }
    }
}

/// Which of two inputs for one control is in charge: the one that changed by more than 0.1 most
/// recently (the original's two timestamps, `+0x1510`/`+0x1514` for the helicopter collective,
/// `+0x16f8`/`+0x16f4` for the plane throttle). The digital keys win a tie.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arbiter {
    digital: f64,
    analog: f64,
    /// Whether the digital keys are in charge.
    pub digital_in_charge: bool,
}

impl Default for Arbiter {
    fn default() -> Self {
        Arbiter {
            digital: f64::MAX,
            analog: f64::MAX,
            digital_in_charge: true,
        }
    }
}

impl Arbiter {
    /// Records this step's two values. Returns the analogue value's change when it counted
    /// (0 otherwise), which the helicopter uses to start the engine.
    pub fn update(&mut self, digital: f64, analog: f64) -> f64 {
        let mut analog_change = 0.0;
        let d = analog - self.analog;
        if d.abs() > 0.1 {
            let first = self.analog == f64::MAX;
            self.analog = analog;
            if !first {
                self.digital_in_charge = false;
                analog_change = d;
            }
        }
        if (digital - self.digital).abs() > 0.1 {
            self.digital = digital;
            self.digital_in_charge = true;
        }
        analog_change
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_in_charge_until_the_axis_moves() {
        let mut a = Arbiter::default();
        a.update(0.0, 0.0);
        assert!(a.digital_in_charge);
        a.update(0.0, 0.5);
        assert!(!a.digital_in_charge);
        a.update(1.0, 0.5);
        assert!(a.digital_in_charge);
    }

    #[test]
    fn every_action_has_a_field() {
        let input = FlightInput::from_actions(|name| {
            FlightInput::ACTIONS
                .iter()
                .position(|a| *a == name)
                .unwrap() as f64
        });
        assert_eq!(input.heli_collective_lower_cont, 21.0);
        assert_eq!(input.heli_up, 0.0);
    }
}
