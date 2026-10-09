//! The gearbox: the ratios the `complexGearbox` class gives, and the automatic shifting that
//! picks among them. `docs/re/sim-vehicles.md` §2 "Clutch/gears".

use a3_config::ConfigRef;

use crate::engine::EngineData;
use crate::value;

/// What a gear is: a box has its reverse gears, one neutral gear, and the forward gears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GearKind {
    Reverse,
    Neutral,
    Forward,
}

/// One gear of the box, as `GearboxRatios[]` lists it:
/// `{"R1", -4, "N", 0, "D1", 4.5, …}` — one reverse (a negative ratio), one neutral (zero),
/// and the forward gears from the highest ratio to the lowest.
#[derive(Debug, Clone, PartialEq)]
pub struct Gear {
    /// The gear's name (`"D1"`), for diagnostics; the original does not use it either.
    pub name: String,
    /// The gearbox ratio, signed: negative in reverse.
    pub ratio: f64,
    /// Which family the gear belongs to.
    pub kind: GearKind,
}

/// Which way the driver wants to go. The autobox shifts among the gears of one direction and
/// engages the other one only when the driver asks for it — with the neutral window of
/// [`GearboxData::switch_time`] in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveDirection {
    Forward,
    Reverse,
}

impl DriveDirection {
    /// The other direction.
    pub fn opposite(self) -> DriveDirection {
        match self {
            DriveDirection::Forward => DriveDirection::Reverse,
            DriveDirection::Reverse => DriveDirection::Forward,
        }
    }
}

/// How the automatic gearbox picks its gear (`changeGearType`, `"effective"` by default).
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeGear {
    /// `changeGearType = "effective"`: hold the current gear while its *effectivity* is at least
    /// `changeGearMinEffectivity[]` for it; below that, take the gear with the highest
    /// effectivity. Effectivity is our reading of the wiki's "engine torque ratio": the torque
    /// curve's multiplier at the engine speed the gear would turn at the current wheel speed
    /// (`wheelSpeed · ratio · finalRatio`), which is comparable across gears and makes the box
    /// keep the engine near its torque peak. Not confirmed against the binary.
    Effective { min_effectivity: Vec<f64> },
    /// `changeGearType = "rpmratio"`: shift up above and down below the (max, min) pairs of
    /// `changeGearOmegaRatios[]`, per gear, as fractions of `maxOmega`.
    RpmRatio { omega_ratios: Vec<(f64, f64)> },
}

/// The gearbox of a vehicle: `class complexGearbox` and the clutch/gearbox entries beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct GearboxData {
    /// The gears, in config order.
    pub gears: Vec<Gear>,
    /// The drivetrain ratio after the gearbox (`TransmissionRatios[]`, e.g. the final drive).
    /// The total ratio in a gear is `ratio · final_ratio`.
    pub final_ratio: f64,
    /// The gear number an automatic box moves off in (`moveOffGear`, 1 = the first forward
    /// gear, negative = reverse), default 1.
    pub move_off_gear: i32,
    /// How the automatic box picks its gear.
    pub change: ChangeGear,
    /// Seconds a gear change takes, in neutral (`switchTime`, default 0.01).
    pub switch_time: f64,
    /// Seconds that must pass between two gear changes (`latency`, default 2).
    pub latency: f64,
    /// How strongly the clutch couples the engine to the gearbox (`clutchStrength`, default 10).
    pub clutch_strength: f64,
}

impl Default for GearboxData {
    fn default() -> Self {
        Self {
            gears: direct_drive(),
            final_ratio: 1.0,
            move_off_gear: 1,
            change: ChangeGear::Effective {
                min_effectivity: vec![0.95; 3],
            },
            switch_time: 0.01,
            latency: 2.0,
            clutch_strength: 10.0,
        }
    }
}

/// The gearbox of a vehicle that has no `complexGearbox`: reverse, neutral and one forward gear,
/// so it still drives, just without gearing.
fn direct_drive() -> Vec<Gear> {
    vec![
        Gear {
            name: "R1".to_string(),
            ratio: -1.0,
            kind: GearKind::Reverse,
        },
        Gear {
            name: "N".to_string(),
            ratio: 0.0,
            kind: GearKind::Neutral,
        },
        Gear {
            name: "D1".to_string(),
            ratio: 1.0,
            kind: GearKind::Forward,
        },
    ]
}

impl GearboxData {
    /// Reads the gearbox of a vehicle config node: `class complexGearbox` for the ratios, the
    /// entries beside it for the clutch and the automatic box.
    pub fn from_config(config: &ConfigRef<'_>) -> GearboxData {
        let default = GearboxData::default();
        let class = config.get("complexGearbox");
        let class: &ConfigRef<'_> = if class.is_class() { &class } else { config };
        let gears = gears_of(&value::named_numbers(&class.get("GearboxRatios")));
        let named_ratio = |name: &str| {
            let node = context(config, class, name);
            let pairs = value::named_numbers(&node);
            let first = pairs.first().filter(|(_, ratio)| *ratio != 0.0);
            first.map(|(_, ratio)| *ratio)
        };
        let default_ratio = named_ratio("TransmissionRatios");
        let change = match config
            .get("changeGearType")
            .text()
            .to_ascii_lowercase()
            .as_str()
        {
            "rpmratio" => {
                let ratios = value::number_pairs(&config.get("changeGearOmegaRatios"));
                if ratios.is_empty() {
                    ChangeGear::Effective {
                        min_effectivity: effectivity_of(config, gears.len(), &default.change),
                    }
                } else {
                    ChangeGear::RpmRatio {
                        omega_ratios: ratios,
                    }
                }
            }
            _ => ChangeGear::Effective {
                min_effectivity: effectivity_of(config, gears.len(), &default.change),
            },
        };
        GearboxData {
            gears,
            final_ratio: default_ratio.unwrap_or(1.0),
            move_off_gear: value::number_or(&config.get("moveOffGear"), 1.0) as i32,
            change,
            switch_time: value::number_or(&config.get("switchTime"), 0.01),
            latency: value::number_or(&config.get("latency"), 2.0),
            clutch_strength: value::number_or(&config.get("clutchStrength"), 10.0),
        }
    }

    /// The gears, in config order.
    pub fn gears(&self) -> &[Gear] {
        &self.gears
    }

    /// The index of a gear by its *gear number*: 1 is the first forward gear, 2 the second, and
    /// so on; −1 is the first reverse gear; 0 is neutral.
    pub fn index_of_gear_number(&self, number: i32) -> Option<usize> {
        match number.cmp(&0) {
            std::cmp::Ordering::Greater => self
                .of_direction(DriveDirection::Forward)
                .get(number as usize - 1)
                .copied(),
            std::cmp::Ordering::Less => self
                .of_direction(DriveDirection::Reverse)
                .get(number.unsigned_abs() as usize - 1)
                .copied(),
            std::cmp::Ordering::Equal => self
                .gears
                .iter()
                .position(|gear| gear.kind == GearKind::Neutral),
        }
    }

    /// The gear number of a gear index, the inverse of [`Self::index_of_gear_number`].
    pub fn gear_number(&self, index: usize) -> i32 {
        let kind = self.gears.get(index).map(|gear| gear.kind);
        let direction = match kind {
            Some(GearKind::Forward) => DriveDirection::Forward,
            Some(GearKind::Reverse) => DriveDirection::Reverse,
            _ => return 0,
        };
        let ordinal = self
            .of_direction(direction)
            .iter()
            .position(|&i| i == index)
            .unwrap_or(0) as i32
            + 1;
        match direction {
            DriveDirection::Forward => ordinal,
            DriveDirection::Reverse => -ordinal,
        }
    }

    /// The indices of the gears of one direction, in gear-number order.
    pub fn of_direction(&self, direction: DriveDirection) -> Vec<usize> {
        let kind = match direction {
            DriveDirection::Forward => GearKind::Forward,
            DriveDirection::Reverse => GearKind::Reverse,
        };
        self.gears
            .iter()
            .enumerate()
            .filter_map(|(index, gear)| (gear.kind == kind).then_some(index))
            .collect()
    }

    /// The index the automatic box moves off in for a direction: `moveOffGear` of that
    /// direction, or the first gear of the direction when the config's number is out of range.
    pub fn move_off_index(&self, direction: DriveDirection) -> Option<usize> {
        let number = match direction {
            DriveDirection::Forward => self.move_off_gear.abs().max(1),
            DriveDirection::Reverse => -self.move_off_gear.abs().max(1),
        };
        self.index_of_gear_number(number)
            .or_else(|| self.of_direction(direction).first().copied())
    }

    /// The total ratio (gearbox × final drive) of a gear.
    pub fn total_ratio(&self, index: usize) -> f64 {
        self.gears.get(index).map_or(0.0, |gear| gear.ratio) * self.final_ratio
    }

    /// The engine speed a gear turns the engine at when the wheels turn at `wheel_speed`
    /// (rad/s of the driven wheel, signed by the direction of travel).
    pub fn engine_omega(&self, index: usize, wheel_speed: f64) -> f64 {
        wheel_speed * self.total_ratio(index)
    }

    /// A gear's *effectivity* — the engine's torque multiplier at the engine speed the gear
    /// would turn at `wheel_speed`. See [`ChangeGear::Effective`].
    pub fn effectivity(&self, index: usize, wheel_speed: f64, engine: &EngineData) -> f64 {
        engine.torque_factor(self.engine_omega(index, wheel_speed))
    }

    /// The gear of `direction` with the highest effectivity at `wheel_speed`; the first on a
    /// tie. `None` when the direction has no gears.
    pub fn best_gear(
        &self,
        wheel_speed: f64,
        engine: &EngineData,
        direction: DriveDirection,
    ) -> Option<usize> {
        self.of_direction(direction)
            .into_iter()
            .map(|index| (index, self.effectivity(index, wheel_speed, engine)))
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(index, _)| index)
    }

    /// The minimum effectivity to hold a gear, when the box shifts by effectivity.
    pub fn min_effectivity(&self, index: usize) -> Option<f64> {
        match &self.change {
            ChangeGear::Effective { min_effectivity } => {
                Some(min_effectivity.get(index).copied().unwrap_or(0.95))
            }
            ChangeGear::RpmRatio { .. } => None,
        }
    }
}

/// `TransmissionRatios[]` sits in `complexGearbox` in most configs and beside it in some.
fn context<'a>(config: &ConfigRef<'a>, class: &ConfigRef<'a>, name: &str) -> ConfigRef<'a> {
    let in_class = class.get(name);
    if in_class.is_array() {
        return in_class;
    }
    let outside = config.get(name);
    if outside.is_array() {
        outside
    } else {
        in_class
    }
}

/// The gear list of the `(name, ratio)` pairs of `GearboxRatios[]`.
fn gears_of(pairs: &[(String, f64)]) -> Vec<Gear> {
    if pairs.is_empty() {
        return direct_drive();
    }
    pairs
        .iter()
        .map(|(name, ratio)| Gear {
            name: name.clone(),
            ratio: *ratio,
            kind: if *ratio < 0.0 {
                GearKind::Reverse
            } else if *ratio == 0.0 {
                GearKind::Neutral
            } else {
                GearKind::Forward
            },
        })
        .collect()
}

/// `changeGearMinEffectivity[]`, one entry per gear, default 0.95.
fn effectivity_of(config: &ConfigRef<'_>, gears: usize, default: &ChangeGear) -> Vec<f64> {
    let values = value::numbers(&config.get("changeGearMinEffectivity"));
    if values.is_empty() {
        let ChangeGear::Effective { min_effectivity } = default else {
            return vec![0.95; gears];
        };
        return std::iter::repeat_n(min_effectivity[0], gears).collect();
    }
    values
        .into_iter()
        .map(|value| value.clamp(0.0, 1.0))
        .collect()
}

/// The gearbox's state: which gear is engaged, which one is being shifted to, and the timers
/// the automatic box keeps (`switchTime` in neutral, `latency` between changes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transmission {
    gear: usize,
    target: usize,
    shifting: f64,
    latency: f64,
    direction: DriveDirection,
}

impl Transmission {
    /// A box that starts in the gear an automatic box moves off in.
    pub fn new(data: &GearboxData, direction: DriveDirection) -> Transmission {
        let gear = data.move_off_index(direction).unwrap_or(0);
        Transmission {
            gear,
            target: gear,
            shifting: 0.0,
            latency: 0.0,
            direction,
        }
    }

    /// The index of the engaged gear.
    pub fn gear(&self) -> usize {
        self.gear
    }

    /// The index the box is shifting to (the engaged gear when not shifting).
    pub fn target(&self) -> usize {
        self.target
    }

    /// The gear number of the engaged gear.
    pub fn gear_number(&self, data: &GearboxData) -> i32 {
        data.gear_number(self.gear)
    }

    /// Seconds left of a gear change; the box is in neutral while this is positive.
    pub fn shifting(&self) -> f64 {
        self.shifting
    }

    /// The direction the driver has selected.
    pub fn direction(&self) -> DriveDirection {
        self.direction
    }

    /// The total ratio the drivetrain turns at; zero while a gear change has the box in neutral.
    pub fn ratio(&self, data: &GearboxData) -> f64 {
        if self.shifting > 0.0 {
            0.0
        } else {
            data.total_ratio(self.gear)
        }
    }

    /// Turns the automatic box by `dt`. `wheel_speed` is the driven wheel's angular velocity
    /// (rad/s) and `engine_omega` the engine's; `direction` is what the driver asks for, which
    /// the box engages with a gear change of its own.
    pub fn step(
        &mut self,
        data: &GearboxData,
        engine: &EngineData,
        wheel_speed: f64,
        engine_omega: f64,
        direction: DriveDirection,
        dt: f64,
    ) {
        self.step_with(data, engine, wheel_speed, engine_omega, direction, true, dt);
    }

    /// [`Self::step`], with the automatic shifting itself optional: a driver who selects the
    /// gears by hand still has the neutral window and the direction change.
    #[allow(clippy::too_many_arguments)]
    pub fn step_with(
        &mut self,
        data: &GearboxData,
        engine: &EngineData,
        wheel_speed: f64,
        engine_omega: f64,
        direction: DriveDirection,
        automatic: bool,
        dt: f64,
    ) {
        // Finish a gear change that is under way: the box has been in neutral for switchTime.
        if self.shifting > 0.0 {
            self.shifting -= dt;
            if self.shifting <= 0.0 {
                self.shifting = 0.0;
                self.gear = self.target;
            }
            return;
        }
        if self.latency > 0.0 {
            self.latency -= dt;
        }
        if self.latency > 0.0 {
            return;
        }
        // The driver asks for the other direction: engage its move-off gear.
        if direction != self.direction {
            self.direction = direction;
            if let Some(target) = data.move_off_index(direction) {
                self.request(data, target);
            }
            return;
        }
        if !automatic {
            return;
        }
        match &data.change {
            ChangeGear::Effective { .. } => self.shift_by_effectivity(data, engine, wheel_speed),
            ChangeGear::RpmRatio { omega_ratios } => {
                self.shift_by_omega_ratio(data, engine, omega_ratios, engine_omega)
            }
        }
    }

    /// The `"effective"` rule: hold the gear while its effectivity is at least its
    /// `changeGearMinEffectivity`, else take the best gear of the direction.
    fn shift_by_effectivity(&mut self, data: &GearboxData, engine: &EngineData, wheel_speed: f64) {
        let effectivity = data.effectivity(self.gear, wheel_speed, engine);
        let threshold = data.min_effectivity(self.gear).unwrap_or(0.95);
        if effectivity >= threshold {
            return;
        }
        if let Some(best) = data.best_gear(wheel_speed, engine, self.direction) {
            if best != self.gear {
                self.request(data, best);
            }
        }
    }

    /// The `"rpmratio"` rule: upshift above the gear's maximum engine-speed ratio and downshift
    /// below its minimum, one gear at a time.
    fn shift_by_omega_ratio(
        &mut self,
        data: &GearboxData,
        engine: &EngineData,
        omega_ratios: &[(f64, f64)],
        engine_omega: f64,
    ) {
        let Some((max, min)) = omega_ratios.get(self.gear).copied() else {
            return;
        };
        let ratio = if engine.max_omega > 0.0 {
            engine_omega / engine.max_omega
        } else {
            0.0
        };
        let gears = data.of_direction(self.direction);
        let Some(position) = gears.iter().position(|&index| index == self.gear) else {
            return;
        };
        let target = if ratio > max {
            gears.get(position + 1).copied()
        } else if ratio < min {
            // Downshifting, but never out of the direction's lowest gear: the box stays in gear
            // at a standstill instead of dropping into neutral.
            position
                .checked_sub(1)
                .and_then(|previous| gears.get(previous).copied())
        } else {
            None
        };
        if let Some(target) = target {
            if target != self.gear {
                self.request(data, target);
            }
        }
    }

    /// Starts a gear change: the box is in neutral for `switchTime` and may not change again
    /// for `latency`.
    pub fn request(&mut self, data: &GearboxData, target: usize) {
        if target == self.gear || target >= data.gears.len() {
            return;
        }
        self.target = target;
        self.shifting = data.switch_time;
        self.latency = data.latency.max(data.switch_time);
        if self.shifting <= 0.0 {
            self.gear = target;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::TorqueCurve;
    use a3_config::parse_text;

    /// The shipped offroad's gearbox, trimmed to the entries the gearbox reads.
    const OFFROAD: &str = r#"
        class C_Offroad_01_F {
            changeGearMinEffectivity[] = {1, 0.15, 1, 1, 1, 1, 1, 1};
            switchTime = 0.31;
            latency = 1.5;
            clutchStrength = 20.0;
            class complexGearbox {
                GearboxRatios[] = {"R1", -4, "N", 0, "D1", "4.5*(0.58^0)", "D2", "4.5*(0.58^1)",
                                   "D3", "4.5*(0.58^2)", "D4", "4.5*(0.58^3)", "D5", "4.5*(0.59^4)",
                                   "D6", "4.5*(0.6^5)"};
                TransmissionRatios[] = {"High", 7};
                moveOffGear = 1;
                gearBoxMode = "auto";
            };
        }"#;

    /// The shipped tank's gearbox (rpm-ratio method), trimmed the same way.
    const TANK: &str = r#"
        class B_MBT_01_cannon_F {
            latency = 1.5;
            switchTime = 0;
            clutchStrength = 40.0;
            changeGearType = "rpmratio";
            changeGearOmegaRatios[] = {1, 0.424242, 0.454545, 0.333333, 0.939394, 0.424242,
                                       0.909091, 0.636364, 0.848485, 0.666667, 1, 0.666667};
            class complexGearbox {
                GearboxRatios[] = {"R1", -3.5, "N", 0, "D1", 4.7, "D2", 2.9, "D3", 1.9, "D4", 1.05};
                transmissionRatios[] = {"High", 12};
                gearBoxMode = "auto";
                moveOffGear = 1;
            };
        }"#;

    fn vehicle(text: &str) -> a3_config::ConfigTree {
        a3_config::ConfigTree::from_config(&parse_text(text).unwrap())
    }

    /// Config numbers are `f32`; compare what is read back with that much slack.
    fn close(got: f64, want: f64) -> bool {
        (got - want).abs() < 1e-5
    }

    /// An engine whose torque curve peaks in the middle, as the shipped ones do.
    fn engine() -> EngineData {
        EngineData {
            power: 150.0,
            max_omega: 450.0,
            min_omega: 100.0,
            moi: 1.0,
            peak_torque: 425.0,
            torque_curve: TorqueCurve::new(
                vec![
                    (0.0, 0.0),
                    (0.143, 0.471),
                    (0.429, 0.953),
                    (0.571, 1.0),
                    (0.857, 0.706),
                ],
                TorqueCurve::default(),
            ),
            ..EngineData::default()
        }
    }

    #[test]
    fn reads_the_offroad_gearbox() {
        let tree = vehicle(OFFROAD);
        let data = GearboxData::from_config(&tree.root().get("C_Offroad_01_F"));
        assert_eq!(data.gears().len(), 8);
        assert_eq!(data.gears()[0].ratio, -4.0);
        assert_eq!(data.gears()[1].kind, GearKind::Neutral);
        // Expressions are evaluated: D2 = 4.5 * 0.58.
        assert!(
            close(data.gears()[3].ratio, 2.61),
            "{}",
            data.gears()[3].ratio
        );
        assert_eq!(data.final_ratio, 7.0);
        assert_eq!(data.move_off_gear, 1);
        assert!(close(data.switch_time, 0.31));
        assert_eq!(data.latency, 1.5);
        assert_eq!(data.clutch_strength, 20.0);
        let ChangeGear::Effective { min_effectivity } = &data.change else {
            panic!("no changeGearType, so the default: {:?}", data.change);
        };
        assert_eq!(min_effectivity.len(), 8);
        assert_eq!(min_effectivity[0], 1.0, "reverse never holds");
        assert!(close(min_effectivity[1], 0.15), "{}", min_effectivity[1]);
        assert_eq!(min_effectivity[2], 1.0);
        assert_eq!(min_effectivity[7], 1.0, "the last gear never holds");
        // Gear numbers: 1 = D1, 7 = the reverse of gear 0, 0 = neutral.
        assert_eq!(data.index_of_gear_number(1), Some(2));
        assert_eq!(data.index_of_gear_number(6), Some(7));
        assert_eq!(data.index_of_gear_number(-1), Some(0));
        assert_eq!(data.index_of_gear_number(0), Some(1));
        assert_eq!(data.gear_number(7), 6);
        assert_eq!(data.gear_number(0), -1);
        assert_eq!(data.gear_number(1), 0);
    }

    #[test]
    fn reads_the_tank_gearbox() {
        let tree = vehicle(TANK);
        let data = GearboxData::from_config(&tree.root().get("B_MBT_01_cannon_F"));
        assert_eq!(data.gears().len(), 6);
        assert_eq!(data.final_ratio, 12.0, "lowercase transmissionRatios");
        assert_eq!(data.switch_time, 0.0);
        let ChangeGear::RpmRatio { omega_ratios } = &data.change else {
            panic!("changeGearType = rpmratio, got {:?}", data.change);
        };
        assert_eq!(omega_ratios.len(), 6);
        assert!(
            close(omega_ratios[2].0, 0.939394) && close(omega_ratios[2].1, 0.424242),
            "D1: {:?}",
            omega_ratios[2]
        );
        assert_eq!(data.of_direction(DriveDirection::Forward), vec![2, 3, 4, 5]);
        assert_eq!(data.of_direction(DriveDirection::Reverse), vec![0]);
        assert_eq!(data.move_off_index(DriveDirection::Forward), Some(2));
        assert_eq!(data.move_off_index(DriveDirection::Reverse), Some(0));
        assert!(
            close(data.total_ratio(2), 4.7 * 12.0),
            "{}",
            data.total_ratio(2)
        );
    }

    #[test]
    fn a_vehicle_without_a_complex_gearbox_still_drives() {
        let tree = vehicle("class V { enginePower = 100; };");
        let data = GearboxData::from_config(&tree.root().get("V"));
        assert_eq!(data.gears().len(), 3);
        assert_eq!(data.final_ratio, 1.0);
        assert_eq!(data.move_off_index(DriveDirection::Forward), Some(2));
        assert_eq!(data.total_ratio(2), 1.0, "one forward gear, no gearing");
        assert_eq!(data.total_ratio(0), -1.0);
    }

    #[test]
    fn effectivity_is_the_torque_ratio_the_gear_turns_at() {
        let tree = vehicle(OFFROAD);
        let data = GearboxData::from_config(&tree.root().get("C_Offroad_01_F"));
        let engine = engine();
        // D1's total ratio is 4.5 * 7 = 31.5: at 2 rad/s of wheel the engine turns at 63 rad/s,
        // 63/450 = 0.14 of its speed, where the curve makes almost nothing.
        assert_eq!(data.engine_omega(2, 2.0), 63.0);
        assert!((data.effectivity(2, 2.0, &engine) - 0.471).abs() < 0.02);
        // At 20 rad/s of wheel, D3's engine speed (212 rad/s, 0.471 of maximum) sits nearest
        // the peak of the curve, so D3 (index 4) has the most torque ratio.
        assert_eq!(
            data.best_gear(20.0, &engine, DriveDirection::Forward),
            Some(4)
        );
        assert!((data.effectivity(4, 20.0, &engine) - 0.967).abs() < 0.01);
        // Only the gears of the asked-for direction are candidates, however slowly they turn:
        // the reverse gear turns backwards while rolling forward and cannot win.
        assert_eq!(
            data.best_gear(-20.0, &engine, DriveDirection::Reverse),
            Some(0)
        );
        assert_eq!(
            data.best_gear(20.0, &engine, DriveDirection::Reverse),
            Some(0)
        );
    }

    #[test]
    fn the_effective_box_keeps_the_engine_near_its_torque_peak() {
        let tree = vehicle(OFFROAD);
        let data = GearboxData::from_config(&tree.root().get("C_Offroad_01_F"));
        let engine = engine();
        let mut box_of_gears = Transmission::new(&data, DriveDirection::Forward);
        assert_eq!(box_of_gears.gear_number(&data), 1, "moves off in gear 1");

        // Rolling forward at 20 rad/s, the box leaves D1 for D3 (the best gear there), taking
        // switchTime = 0.31 s of neutral: one step to ask for the change, 13 of them to shift.
        let mut shifted = 0;
        for _ in 0..20 {
            box_of_gears.step(&data, &engine, 20.0, 200.0, DriveDirection::Forward, 0.025);
            shifted += 1;
            if box_of_gears.gear() == 4 {
                break;
            }
        }
        assert_eq!(box_of_gears.gear(), 4, "D3 is the best gear at 20 rad/s");
        assert_eq!(shifted, 14);
        // And it holds there: nothing better is on offer at that wheel speed.
        for _ in 0..400 {
            box_of_gears.step(&data, &engine, 20.0, 200.0, DriveDirection::Forward, 0.025);
        }
        assert_eq!(box_of_gears.gear(), 4);
        assert_eq!(box_of_gears.target(), 4);
        assert_eq!(box_of_gears.shifting(), 0.0);
    }

    #[test]
    fn the_rpm_ratio_box_shifts_on_engine_speed() {
        // A tank-scale engine, so the shipped ratios mean what they mean.
        let engine = EngineData {
            max_omega: 345.6,
            min_omega: 146.6,
            peak_torque: 5000.0,
            ..engine()
        };
        let tree = vehicle(TANK);
        let data = GearboxData::from_config(&tree.root().get("B_MBT_01_cannon_F"));
        let mut transmission = Transmission::new(&data, DriveDirection::Forward);
        assert_eq!(transmission.gear_number(&data), 1);
        // Above D1's maximum (0.939 * 345.6 = 324.6 rad/s) it upshifts, at once: this box's
        // switchTime is 0.
        transmission.step(&data, &engine, 10.0, 330.0, DriveDirection::Forward, 0.025);
        assert_eq!(transmission.gear(), 3, "D2");
        assert_eq!(transmission.shifting(), 0.0);
        // 330 rad/s is 0.955 of the maximum, above D2's maximum of 0.909: up again — after the
        // latency of 1.5 s, 60 steps.
        for _ in 0..100 {
            transmission.step(&data, &engine, 10.0, 330.0, DriveDirection::Forward, 0.025);
        }
        assert_eq!(transmission.gear(), 4, "D3");
        // Down: 228 rad/s is 0.66 of the maximum, below D3's minimum of 0.667 but above D2's
        // 0.636, so it drops one gear and holds there.
        for _ in 0..100 {
            transmission.step(&data, &engine, 10.0, 228.0, DriveDirection::Forward, 0.025);
        }
        assert_eq!(transmission.gear(), 3, "back to D2");
        // Never below the lowest forward gear, however slowly the engine turns: 140 rad/s is
        // below D1's minimum of 0.424 too.
        for _ in 0..200 {
            transmission.step(&data, &engine, 1.0, 140.0, DriveDirection::Forward, 0.025);
        }
        assert_eq!(transmission.gear(), 2, "D1");
    }

    #[test]
    fn a_gear_change_is_a_neutral_window_and_the_latency_holds_the_next_one() {
        let tree = vehicle(OFFROAD);
        let data = GearboxData::from_config(&tree.root().get("C_Offroad_01_F"));
        let mut transmission = Transmission::new(&data, DriveDirection::Forward);
        let ratio = data.total_ratio(2);
        transmission.request(&data, 4);
        assert_eq!(transmission.ratio(&data), 0.0, "in neutral while shifting");
        // switchTime = 0.31: 12 steps of 0.025 is 0.3, one more completes the change.
        for _ in 0..12 {
            transmission.step(
                &data,
                &engine(),
                20.0,
                200.0,
                DriveDirection::Forward,
                0.025,
            );
        }
        assert_eq!(transmission.ratio(&data), 0.0);
        transmission.step(
            &data,
            &engine(),
            20.0,
            200.0,
            DriveDirection::Forward,
            0.025,
        );
        assert_eq!(transmission.gear(), 4);
        assert_eq!(transmission.ratio(&data), data.total_ratio(4));
        assert!(transmission.ratio(&data) != ratio);
        // latency = 1.5: a request right after the change is ignored.
        let before = transmission.gear();
        transmission.step(
            &data,
            &engine(),
            20.0,
            200.0,
            DriveDirection::Forward,
            0.025,
        );
        transmission.step(
            &data,
            &engine(),
            20.0,
            200.0,
            DriveDirection::Forward,
            0.025,
        );
        assert_eq!(transmission.gear(), before);
    }

    #[test]
    fn the_driver_asking_for_reverse_engages_it_through_a_gear_change() {
        let tree = vehicle(OFFROAD);
        let data = GearboxData::from_config(&tree.root().get("C_Offroad_01_F"));
        let mut transmission = Transmission::new(&data, DriveDirection::Forward);
        transmission.step(&data, &engine(), 0.0, 200.0, DriveDirection::Reverse, 0.025);
        assert_eq!(transmission.direction(), DriveDirection::Reverse);
        assert_eq!(transmission.target(), 0, "the reverse gear is on its way");
        assert_eq!(transmission.gear_number(&data), 1, "still in D1");
        assert_eq!(transmission.ratio(&data), 0.0, "through neutral");
        for _ in 0..40 {
            transmission.step(&data, &engine(), 0.0, 200.0, DriveDirection::Reverse, 0.025);
        }
        assert_eq!(transmission.gear_number(&data), -1);
        assert!(transmission.ratio(&data) < 0.0, "reverse drives backwards");
    }
}
