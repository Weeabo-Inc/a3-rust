//! The handling entries of a vehicle class that are neither engine nor wheel:
//! [`Handling`], the [`Differential`], and the per-kind extras (tank turn force, ship water
//! coefficients). `docs/re/sim-vehicles.md` §2.

use crate::value;
use a3_config::ConfigRef;

/// Which axles a differential drives, and whether it limits slip — the
/// `PxVehicleDifferential4WData::mType` values, spelled the config's way
/// (`docs/re/sim-vehicles.md` §2 "Differential").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifferentialType {
    /// `all_open`: all wheels driven, no limited slip.
    AllOpen,
    /// `all_limited`.
    AllLimited,
    /// `front_open`, `front_limited`: front-wheel drive.
    FrontOpen,
    FrontLimited,
    /// `rear_open`, `rear_limited`: rear-wheel drive.
    RearOpen,
    RearLimited,
}

impl DifferentialType {
    /// The type named `name`, or `None`.
    pub fn from_name(name: &str) -> Option<DifferentialType> {
        Some(match name.to_ascii_lowercase().as_str() {
            "all_open" => DifferentialType::AllOpen,
            "all_limited" => DifferentialType::AllLimited,
            "front_open" => DifferentialType::FrontOpen,
            "front_limited" => DifferentialType::FrontLimited,
            "rear_open" => DifferentialType::RearOpen,
            "rear_limited" => DifferentialType::RearLimited,
            _ => return None,
        })
    }

    /// Whether the front wheels are driven.
    pub fn drives_front(&self) -> bool {
        matches!(
            self,
            DifferentialType::AllOpen
                | DifferentialType::AllLimited
                | DifferentialType::FrontOpen
                | DifferentialType::FrontLimited
        )
    }

    /// Whether the rear wheels are driven.
    pub fn drives_rear(&self) -> bool {
        matches!(
            self,
            DifferentialType::AllOpen
                | DifferentialType::AllLimited
                | DifferentialType::RearOpen
                | DifferentialType::RearLimited
        )
    }

    /// Whether the differential limits slip between the two sides of an axle.
    pub fn limited_slip(&self) -> bool {
        matches!(
            self,
            DifferentialType::AllLimited
                | DifferentialType::FrontLimited
                | DifferentialType::RearLimited
        )
    }
}

/// The differential between the driven wheels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Differential {
    /// `differentialType`. Default all-wheel drive with an open differential.
    pub kind: DifferentialType,
    /// `frontRearSplit`: the share of the torque the front axle takes. Default 0.5.
    pub front_rear_split: f64,
    /// `frontBias`, `rearBias`, `centreBias`: the torque ratio a limited-slip differential may
    /// put between its two sides. Default 1.
    pub front_bias: f64,
    pub rear_bias: f64,
    pub centre_bias: f64,
}

impl Default for Differential {
    fn default() -> Self {
        Differential {
            kind: DifferentialType::AllOpen,
            front_rear_split: 0.5,
            front_bias: 1.0,
            rear_bias: 1.0,
            centre_bias: 1.0,
        }
    }
}

impl Differential {
    fn from_config(config: &ConfigRef<'_>) -> Differential {
        let default = Differential::default();
        Differential {
            kind: DifferentialType::from_name(&config.get("differentialType").text())
                .unwrap_or(default.kind),
            front_rear_split: value::number_or(
                &config.get("frontRearSplit"),
                default.front_rear_split,
            )
            .clamp(0.0, 1.0),
            front_bias: value::number_or(&config.get("frontBias"), default.front_bias),
            rear_bias: value::number_or(&config.get("rearBias"), default.rear_bias),
            centre_bias: value::number_or(&config.get("centreBias"), default.centre_bias),
        }
    }
}

/// The handling entries: how fast the vehicle may go, how hard it brakes and turns, and the
/// extras each kind adds.
#[derive(Debug, Clone, PartialEq)]
pub struct Handling {
    /// `mass`, kg. The shipped vehicle classes mostly leave it to the model
    /// (`docs/re/odol.md`), so the loader falls back to the wheels' `sprungMass` and then to a
    /// per-kind placeholder; this is only the config's own entry.
    pub mass: Option<f64>,
    /// `maxSpeed`, km/h — the top speed. The engine's governor holds the vehicle to it.
    pub max_speed_kmh: f64,
    /// `brakeIdleSpeed`, m/s: below this speed the throttle is cut and the vehicle brakes to a
    /// stop instead of creeping (`docs/re/sim-vehicles.md`).
    pub brake_idle_speed: f64,
    /// `thrustDelay`, s: how long the drive takes to come up.
    pub thrust_delay: f64,
    /// `engineBrakeCoef`, `overSpeedBrakeCoef`: engine braking, and the extra braking above
    /// `maxSpeed`. The cars' configs often omit them; the physics carries engine braking anyway.
    pub engine_brake_coef: f64,
    pub over_speed_brake_coef: f64,
    /// `antiRollbarForceCoef`, `antiRollbarForceLimit`, `antiRollbarSpeedMin`,
    /// `antiRollbarSpeedMax`: the bar between the two sides of an axle.
    pub anti_rollbar_force_coef: f64,
    pub anti_rollbar_force_limit: f64,
    pub anti_rollbar_speed_min: f64,
    pub anti_rollbar_speed_max: f64,
    /// The differential.
    pub differential: Differential,
    /// `tankTurnForce`: the yaw moment a tank's track differential may apply, N·m.
    pub tank_turn_force: f64,
    /// `tankTurnForceAngMinSpd`, `tankTurnForceAngSpd`: the speed below which the full turn
    /// force is available, and the normalising speed of its falloff.
    pub tank_turn_force_ang_min_speed: f64,
    pub tank_turn_force_ang_speed: f64,
    /// The water model, for ships (`ShipHandling`).
    pub water: WaterHandling,
}

impl Default for Handling {
    fn default() -> Self {
        Handling {
            mass: None,
            max_speed_kmh: 0.0,
            brake_idle_speed: 0.0,
            thrust_delay: 0.0,
            engine_brake_coef: 0.0,
            over_speed_brake_coef: 0.0,
            anti_rollbar_force_coef: 0.0,
            anti_rollbar_force_limit: 0.0,
            anti_rollbar_speed_min: 0.0,
            anti_rollbar_speed_max: 0.0,
            differential: Differential::default(),
            tank_turn_force: 0.0,
            tank_turn_force_ang_min_speed: 0.0,
            tank_turn_force_ang_speed: 0.0,
            water: WaterHandling::default(),
        }
    }
}

impl Handling {
    /// Reads the handling entries of a vehicle class.
    pub fn from_config(config: &ConfigRef<'_>) -> Handling {
        Handling {
            mass: value::number(&config.get("mass")).filter(|mass| *mass > 0.0),
            max_speed_kmh: value::number_or(&config.get("maxSpeed"), 0.0),
            brake_idle_speed: value::number_or(&config.get("brakeIdleSpeed"), 0.0),
            thrust_delay: value::number_or(&config.get("thrustDelay"), 0.0),
            engine_brake_coef: value::number_or(&config.get("engineBrakeCoef"), 0.0),
            over_speed_brake_coef: value::number_or(&config.get("overSpeedBrakeCoef"), 0.0),
            anti_rollbar_force_coef: value::number_or(&config.get("antiRollbarForceCoef"), 0.0),
            anti_rollbar_force_limit: value::number_or(&config.get("antiRollbarForceLimit"), 0.0),
            anti_rollbar_speed_min: value::number_or(&config.get("antiRollbarSpeedMin"), 0.0),
            anti_rollbar_speed_max: value::number_or(&config.get("antiRollbarSpeedMax"), 0.0),
            differential: Differential::from_config(config),
            tank_turn_force: value::number_or(&config.get("tankTurnForce"), 0.0),
            tank_turn_force_ang_min_speed: value::number_or(
                &config.get("tankTurnForceAngMinSpd"),
                0.0,
            ),
            tank_turn_force_ang_speed: value::number_or(&config.get("tankTurnForceAngSpd"), 0.0),
            water: WaterHandling::from_config(config),
        }
    }

    /// `maxSpeed` in m/s.
    pub fn max_speed(&self) -> f64 {
        self.max_speed_kmh / 3.6
    }

    /// The anti-rollbar force between two wheels of one axle at `speed` (m/s) and
    /// `displacement` (m, the difference in compression), N. Zero without the config entry and
    /// outside the speed window.
    pub fn anti_rollbar_force(&self, speed: f64, displacement: f64) -> f64 {
        if self.anti_rollbar_force_coef <= 0.0 || speed < self.anti_rollbar_speed_min {
            return 0.0;
        }
        let mut force = self.anti_rollbar_force_coef * displacement;
        if self.anti_rollbar_speed_max > self.anti_rollbar_speed_min
            && speed > self.anti_rollbar_speed_min
        {
            let t = ((speed - self.anti_rollbar_speed_min)
                / (self.anti_rollbar_speed_max - self.anti_rollbar_speed_min))
                .min(1.0);
            force *= 1.0 - t;
        }
        match self.anti_rollbar_force_limit {
            limit if limit > 0.0 => force.clamp(-limit, limit),
            _ => force,
        }
    }
}

/// The water model of a ship (`shipx`), from the entries `docs/re/sim-vehicles.md` §5 lists as
/// open: `waterLinearDampingCoefY`/`X`, `waterAngularDampingCoef`, `waterResistanceCoef`,
/// `rudderForceCoef`, `rudderForceCoefAtMaxSpeed`, `turnCoef`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterHandling {
    /// `waterLinearDampingCoefY`: damping of the vertical motion (buoyancy's own damping).
    pub linear_damping_y: f64,
    /// `waterLinearDampingCoefX`: damping of the horizontal motion.
    pub linear_damping_x: f64,
    /// `waterAngularDampingCoef`: damping of rotation, about every axis.
    pub angular_damping: f64,
    /// `waterResistanceCoef`: the resistance that grows with speed.
    pub resistance: f64,
    /// `rudderForceCoef`: the yaw moment per unit of water speed.
    pub rudder_force: f64,
    /// `rudderForceCoefAtMaxSpeed`: the reduced rudder authority at top speed.
    pub rudder_force_at_max_speed: f64,
    /// `turnCoef`: the steering gain.
    pub turn_coef: f64,
    /// `waterLeakiness`: how fast the hull takes water when damaged. Recorded, not modelled.
    pub leakiness: f64,
}

impl Default for WaterHandling {
    fn default() -> Self {
        WaterHandling {
            linear_damping_y: 2.0,
            linear_damping_x: 2.0,
            angular_damping: 1.2,
            resistance: 0.012,
            rudder_force: 0.1,
            rudder_force_at_max_speed: 0.003,
            turn_coef: 1.0,
            leakiness: 0.0,
        }
    }
}

impl WaterHandling {
    fn from_config(config: &ConfigRef<'_>) -> WaterHandling {
        let default = WaterHandling::default();
        WaterHandling {
            linear_damping_y: value::number_or(
                &config.get("waterLinearDampingCoefY"),
                default.linear_damping_y,
            ),
            linear_damping_x: value::number_or(
                &config.get("waterLinearDampingCoefX"),
                default.linear_damping_x,
            ),
            angular_damping: value::number_or(
                &config.get("waterAngularDampingCoef"),
                default.angular_damping,
            ),
            resistance: value::number_or(&config.get("waterResistanceCoef"), default.resistance),
            rudder_force: value::number_or(&config.get("rudderForceCoef"), default.rudder_force),
            rudder_force_at_max_speed: value::number_or(
                &config.get("rudderForceCoefAtMaxSpeed"),
                default.rudder_force_at_max_speed,
            ),
            turn_coef: value::number_or(&config.get("turnCoef"), default.turn_coef),
            leakiness: value::number_or(&config.get("waterLeakiness"), 0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::parse_text;

    const OFFROAD: &str = r#"
        class C_Offroad_01_F {
            thrustDelay = 0.5;
            brakeIdleSpeed = 1.78;
            maxSpeed = 200;
            antiRollbarForceCoef = 1.9;
            antiRollbarForceLimit = 5;
            antiRollbarSpeedMin = 10;
            antiRollbarSpeedMax = 150;
            differentialType = "all_limited";
            frontRearSplit = 0.5;
            frontBias = 1.5;
            rearBias = 1.5;
            centreBias = 1.3;
        };"#;

    const TANK: &str = r#"
        class B_MBT_01_cannon_F {
            brakeIdleSpeed = 0.1;
            maxSpeed = 60;
            tankTurnForce = 1280000.0;
            tankTurnForceAngMinSpd = 0.6;
            tankTurnForceAngSpd = 0.91;
        };"#;

    const SHIP: &str = r#"
        class C_Boat_Civil_01_F {
            maxSpeed = 80;
            overSpeedBrakeCoef = 0.8;
            turnCoef = 0.25;
            thrustDelay = 2;
            waterLinearDampingCoefY = 2;
            waterLinearDampingCoefX = 2.0;
            waterAngularDampingCoef = 1.2;
            waterResistanceCoef = 0.012;
            rudderForceCoef = 0.1;
            rudderForceCoefAtMaxSpeed = 0.003;
        };"#;

    /// Config numbers are `f32`; compare what is read back with that much slack.
    fn close(got: f64, want: f64) -> bool {
        (got - want).abs() < 1e-6
    }

    fn read(text: &str, class: &str) -> Handling {
        let tree = a3_config::ConfigTree::from_config(&parse_text(text).unwrap());
        let root = tree.root();
        Handling::from_config(&root.get(class))
    }

    #[test]
    fn reads_the_offroad_handling() {
        let h = read(OFFROAD, "C_Offroad_01_F");
        assert_eq!(h.mass, None, "the model carries the mass");
        assert_eq!(h.max_speed_kmh, 200.0);
        assert!((h.max_speed() - 55.555).abs() < 1e-2);
        assert!((h.brake_idle_speed - 1.78).abs() < 1e-6);
        assert_eq!(h.thrust_delay, 0.5);
        assert!(matches!(h.differential.kind, DifferentialType::AllLimited));
        assert!(h.differential.kind.drives_front() && h.differential.kind.drives_rear());
        assert!(h.differential.kind.limited_slip());
        assert_eq!(h.differential.front_rear_split, 0.5);
        assert_eq!(h.differential.front_bias, 1.5);
        assert!(close(h.differential.centre_bias, 1.3));
    }

    #[test]
    fn reads_the_tank_and_ship_extras() {
        let tank = read(TANK, "B_MBT_01_cannon_F");
        assert_eq!(tank.max_speed_kmh, 60.0);
        assert_eq!(tank.tank_turn_force, 1280000.0);
        assert!(close(tank.tank_turn_force_ang_min_speed, 0.6));
        assert!(close(tank.tank_turn_force_ang_speed, 0.91));
        // No differentialType: all-wheel drive, open.
        assert!(matches!(tank.differential.kind, DifferentialType::AllOpen));

        let ship = read(SHIP, "C_Boat_Civil_01_F");
        assert!((ship.over_speed_brake_coef - 0.8).abs() < 1e-6);
        assert!((ship.water.linear_damping_y - 2.0).abs() < 1e-6);
        assert!((ship.water.linear_damping_x - 2.0).abs() < 1e-6);
        assert!((ship.water.angular_damping - 1.2).abs() < 1e-6);
        assert!((ship.water.resistance - 0.012).abs() < 1e-6);
        assert!((ship.water.rudder_force - 0.1).abs() < 1e-6);
        assert!((ship.water.rudder_force_at_max_speed - 0.003).abs() < 1e-6);
        assert!((ship.water.turn_coef - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_differential_names_parse() {
        assert_eq!(
            DifferentialType::from_name("front_open"),
            Some(DifferentialType::FrontOpen)
        );
        assert_eq!(
            DifferentialType::from_name("Rear_Limited"),
            Some(DifferentialType::RearLimited)
        );
        assert!(!DifferentialType::FrontOpen.drives_rear());
        assert!(DifferentialType::FrontOpen.drives_front());
        assert_eq!(DifferentialType::from_name("all_wheel"), None);
    }

    #[test]
    fn the_anti_rollbar_fades_in_and_out_with_speed() {
        let h = read(OFFROAD, "C_Offroad_01_F");
        assert_eq!(h.anti_rollbar_force(5.0, 0.1), 0.0, "below the window");
        assert!((h.anti_rollbar_force(10.0, 0.1) - 0.19).abs() < 1e-6);
        assert!((h.anti_rollbar_force(80.0, 0.1) - 0.19 * 0.5).abs() < 1e-6);
        assert_eq!(h.anti_rollbar_force(200.0, 0.1), 0.0, "above the window");
        // The force limit clamps it.
        assert_eq!(h.anti_rollbar_force(10.0, 100.0), 5.0);
        // Symmetric in the sign of the displacement.
        assert_eq!(h.anti_rollbar_force(10.0, -100.0), -5.0);
        // Nothing configured, no force.
        assert_eq!(Handling::default().anti_rollbar_force(50.0, 1.0), 0.0);
    }
}
