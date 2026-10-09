//! Wheels: one `class Wheels` subclass each, the geometry, the suspension and the tire.
//! `docs/re/sim-vehicles.md` §2 "Wheels" — the `PxVehicleWheelData`,
//! `PxVehicleSuspensionData` and `PxVehicleTireData` fields.

use crate::value;
use a3_config::ConfigRef;
use glam::DVec3;

/// Standard gravity, m/s² (the original's `G_CONST`, `docs/re/sim-vehicles.md`).
pub const GRAVITY: f64 = 9.8066;

/// Which side of the vehicle a wheel sits on, from `side`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelSide {
    Left,
    Right,
}

/// The suspension of one wheel: a spring and a damper along `suspTravelDirection`, preloaded at
/// the design position.
#[derive(Debug, Clone, PartialEq)]
pub struct Suspension {
    /// `suspTravelDirection`, the direction the suspension compresses in. Default (0, −1, 0).
    pub travel_direction: DVec3,
    /// `maxCompression`, m.
    pub max_compression: f64,
    /// `maxDroop`, m.
    pub max_droop: f64,
    /// `sprungMass`, kg: the share of the vehicle the wheel carries. Negative when the config
    /// leaves it to the model (tanks ship `-1`).
    pub sprung_mass: f64,
    /// `springStrength`, N/m.
    pub spring_strength: f64,
    /// `springDamperRate`, N·s/m.
    pub spring_damper_rate: f64,
}

impl Default for Suspension {
    fn default() -> Self {
        Suspension {
            travel_direction: DVec3::new(0.0, -1.0, 0.0),
            max_compression: 0.0,
            max_droop: 0.0,
            sprung_mass: -1.0,
            spring_strength: 0.0,
            spring_damper_rate: 0.0,
        }
    }
}

impl Suspension {
    /// The compression the spring holds at the design position when it carries `sprung_mass` kg:
    /// the deflection the spring is preloaded with, so a wheel at the design position holds
    /// exactly that weight (PhysX preloads the spring, `docs/re/sim-vehicles.md` §2).
    pub fn static_deflection_with(&self, sprung_mass: f64) -> f64 {
        if self.spring_strength <= 0.0 {
            return 0.0;
        }
        (sprung_mass.max(0.0) * GRAVITY / self.spring_strength).max(0.0)
    }

    /// [`Self::static_deflection_with`] for the config's own `sprungMass`.
    pub fn static_deflection(&self) -> f64 {
        self.static_deflection_with(self.sprung_mass)
    }

    /// The suspension force at `compression` (m, positive when compressed past the design
    /// position) and `compression_speed` (m/s, positive when compressing), N along
    /// `-travel_direction`. Never pulls the wheel down.
    pub fn force_with(&self, sprung_mass: f64, compression: f64, compression_speed: f64) -> f64 {
        let spring =
            self.spring_strength * (self.static_deflection_with(sprung_mass) + compression);
        let damper = self.spring_damper_rate * compression_speed;
        (spring + damper).max(0.0)
    }

    /// [`Self::force_with`] for the config's own `sprungMass`.
    pub fn force(&self, compression: f64, compression_speed: f64) -> f64 {
        self.force_with(self.sprung_mass, compression, compression_speed)
    }

    /// Reads a wheel's suspension entries.
    fn from_config(config: &ConfigRef<'_>) -> Suspension {
        let default = Suspension::default();
        Suspension {
            travel_direction: value::vec3(&config.get("suspTravelDirection"))
                .unwrap_or(default.travel_direction)
                .normalize_or_zero(),
            max_compression: value::number_or(&config.get("maxCompression"), 0.0),
            max_droop: value::number_or(&config.get("maxDroop"), 0.0),
            sprung_mass: value::number_or(&config.get("sprungMass"), -1.0),
            spring_strength: value::number_or(&config.get("springStrength"), 0.0),
            spring_damper_rate: value::number_or(&config.get("springDamperRate"), 0.0),
        }
    }
}

/// The tire: its stiffness and its friction curve. `PxVehicleTireData`.
#[derive(Debug, Clone, PartialEq)]
pub struct Tire {
    /// `longitudinalStiffnessPerUnitGravity`: the longitudinal stiffness per unit of load,
    /// divided by gravity. Default 1000.
    pub longitudinal_stiffness_per_unit_gravity: f64,
    /// `latStiffX`: the load (in units of gravity) at which the lateral stiffness peaks.
    /// Default 25.
    pub lat_stiff_x: f64,
    /// `latStiffY`: the lateral stiffness at that load. Default 180.
    pub lat_stiff_y: f64,
    /// `frictionVsSlipGraph[]`, sampled at the slip and held at the ends. Default a flat 1.
    pub friction_vs_slip_graph: Vec<(f64, f64)>,
}

impl Default for Tire {
    fn default() -> Self {
        Tire {
            longitudinal_stiffness_per_unit_gravity: 1000.0,
            lat_stiff_x: 25.0,
            lat_stiff_y: 180.0,
            friction_vs_slip_graph: vec![(0.0, 1.0)],
        }
    }
}

impl Tire {
    /// The longitudinal stiffness at `load` (N), N per unit of slip.
    pub fn longitudinal_stiffness(&self, load: f64) -> f64 {
        (self.longitudinal_stiffness_per_unit_gravity * load / GRAVITY).max(0.0)
    }

    /// The lateral stiffness at `load` (N), N per unit of slip.
    pub fn lateral_stiffness(&self, load: f64) -> f64 {
        if self.lat_stiff_x <= 0.0 {
            return 0.0;
        }
        (self.lat_stiff_y * load / self.lat_stiff_x).max(0.0)
    }

    /// The friction multiplier at `slip` (a normalised slip, 0 rolling, 1 sliding): the graph,
    /// held at the ends.
    pub fn friction_at(&self, slip: f64) -> f64 {
        let points = &self.friction_vs_slip_graph;
        let Some(first) = points.first() else {
            return 1.0;
        };
        if slip <= first.0 {
            return first.1;
        }
        for pair in points.windows(2) {
            let (x0, y0) = pair[0];
            let (x1, y1) = pair[1];
            if slip <= x1 {
                let t = if x1 > x0 {
                    (slip - x0) / (x1 - x0)
                } else {
                    0.0
                };
                return y0 + (y1 - y0) * t;
            }
        }
        points.last().map_or(1.0, |last| last.1)
    }

    fn from_config(config: &ConfigRef<'_>) -> Tire {
        let default = Tire::default();
        let graph = value::points(&config.get("frictionVsSlipGraph"));
        Tire {
            longitudinal_stiffness_per_unit_gravity: value::number_or(
                &config.get("longitudinalStiffnessPerUnitGravity"),
                default.longitudinal_stiffness_per_unit_gravity,
            ),
            lat_stiff_x: value::number_or(&config.get("latStiffX"), default.lat_stiff_x),
            lat_stiff_y: value::number_or(&config.get("latStiffY"), default.lat_stiff_y),
            friction_vs_slip_graph: if graph.is_empty() {
                default.friction_vs_slip_graph
            } else {
                graph
            },
        }
    }
}

/// One wheel: its geometry in the model, its rotating mass, its brakes, and the suspension and
/// tire above it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wheel {
    /// The subclass name in `class Wheels`, e.g. `LF`.
    pub name: String,
    /// `side`.
    pub side: Option<WheelSide>,
    /// `center`: the memory point of the wheel's axis, from the model.
    pub center_point: String,
    /// `boundary`: the memory point of the wheel's outer edge.
    pub boundary_point: String,
    /// `steering`: the wheel turns with the steering.
    pub steering: bool,
    /// `width`, m.
    pub width: f64,
    /// `mass`, kg.
    pub mass: f64,
    /// `MOI`, kg·m² — the wheel's rotational inertia.
    pub moi: f64,
    /// `dampingRate`: the torque per rad/s that resists the free-rolling wheel.
    pub damping_rate: f64,
    /// `dampingRateInAir`.
    pub damping_rate_in_air: f64,
    /// `maxBrakeTorque`, N·m.
    pub max_brake_torque: f64,
    /// `maxHandBrakeTorque`, N·m.
    pub max_hand_brake_torque: f64,
    /// The suspension.
    pub suspension: Suspension,
    /// The tire.
    pub tire: Tire,
    /// The resolved position of the wheel's axis in vehicle space (X right, Y up, Z forward).
    /// Zero until [`Wheel::position`] is resolved from the model's memory points.
    pub position: DVec3,
    /// The resolved wheel radius, m. Zero until resolved.
    pub radius: f64,
}

impl Wheel {
    /// Reads one wheel subclass. `width` is often text (`"0.3"`), which [`value::number`] reads.
    pub fn from_config(name: &str, config: &ConfigRef<'_>) -> Wheel {
        Wheel {
            name: name.to_string(),
            side: match config.get("side").text().to_ascii_lowercase().as_str() {
                "left" => Some(WheelSide::Left),
                "right" => Some(WheelSide::Right),
                _ => None,
            },
            center_point: config.get("center").text(),
            boundary_point: config.get("boundary").text(),
            steering: value::number_or(&config.get("steering"), 0.0) > 0.0,
            width: value::number_or(&config.get("width"), 0.0),
            mass: value::number_or(&config.get("mass"), 0.0),
            moi: value::number_or(&config.get("MOI"), 0.0),
            damping_rate: value::number_or(&config.get("dampingRate"), 0.0),
            damping_rate_in_air: value::number_or(&config.get("dampingRateInAir"), 0.0),
            max_brake_torque: value::number_or(&config.get("maxBrakeTorque"), 0.0),
            max_hand_brake_torque: value::number_or(&config.get("maxHandBrakeTorque"), 0.0),
            suspension: Suspension::from_config(config),
            tire: Tire::from_config(config),
            position: DVec3::ZERO,
            radius: 0.0,
        }
    }

    /// Reads every subclass of a `class Wheels` node, in config order.
    pub fn all_from_config(wheels: &ConfigRef<'_>) -> Vec<Wheel> {
        wheels
            .entries()
            .into_iter()
            .filter(|entry| entry.is_class())
            .map(|entry| {
                let name = entry.name().to_string();
                Wheel::from_config(&name, &entry)
            })
            .collect()
    }

    /// Whether the wheel has geometry to work with.
    pub fn is_placed(&self) -> bool {
        self.radius > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::parse_text;

    /// The shipped offroad's left-front wheel, trimmed to what the wheel reads.
    const LF: &str = r#"
        class LF {
            side = "left";
            suspTravelDirection[] = {-0.125, -1, 0};
            boneName = "wheel_1_1_damper";
            steering = 1;
            center = "wheel_1_1_axis";
            boundary = "wheel_1_1_bound";
            width = "0.3";
            mass = 30;
            MOI = 6;
            dampingRate = 1;
            dampingRateInAir = 0.8;
            maxBrakeTorque = 2000;
            maxHandBrakeTorque = 0;
            maxCompression = 0.05;
            maxDroop = 0.1;
            sprungMass = 400;
            springStrength = 14400;
            springDamperRate = "1920*2";
            longitudinalStiffnessPerUnitGravity = 10000;
            latStiffX = 2.5;
            latStiffY = 18;
            frictionVsSlipGraph[] = {{0, 1.75}, {0.5, 1.35}, {1, 1.2}};
        };"#;

    fn wheel(text: &str) -> Wheel {
        let tree = a3_config::ConfigTree::from_config(&parse_text(text).unwrap());
        let root = tree.root();
        let class = root.get("LF");
        Wheel::from_config("LF", &class)
    }

    #[test]
    fn reads_the_offroad_wheel() {
        let w = wheel(LF);
        assert_eq!(w.name, "LF");
        assert_eq!(w.side, Some(WheelSide::Left));
        assert!(w.steering);
        assert_eq!(w.center_point, "wheel_1_1_axis");
        assert_eq!(w.boundary_point, "wheel_1_1_bound");
        assert!((w.width - 0.3).abs() < 1e-6);
        assert_eq!(w.mass, 30.0);
        assert_eq!(w.moi, 6.0);
        assert_eq!(w.damping_rate, 1.0);
        assert!((w.damping_rate_in_air - 0.8).abs() < 1e-6);
        assert_eq!(w.max_brake_torque, 2000.0);
        assert_eq!(w.max_hand_brake_torque, 0.0);
        assert_eq!(w.suspension.sprung_mass, 400.0);
        assert_eq!(w.suspension.spring_strength, 14400.0);
        assert_eq!(w.suspension.spring_damper_rate, 3840.0, "1920*2 evaluated");
        assert!((w.suspension.max_compression - 0.05).abs() < 1e-6);
        assert!((w.suspension.max_droop - 0.1).abs() < 1e-6);
        // The travel direction is the config's, normalised.
        assert!((w.suspension.travel_direction.y + 0.99228).abs() < 1e-4);
        assert!(w.suspension.travel_direction.x < 0.0);
        assert_eq!(w.tire.longitudinal_stiffness_per_unit_gravity, 10000.0);
        assert!((w.tire.lat_stiff_x - 2.5).abs() < 1e-6);
        assert!((w.tire.lat_stiff_y - 18.0).abs() < 1e-6);
        assert_eq!(w.tire.friction_vs_slip_graph.len(), 3);
        assert!(!w.is_placed(), "geometry comes from the model");
    }

    #[test]
    fn a_bare_wheel_takes_the_defaults() {
        let tree = a3_config::ConfigTree::from_config(&parse_text("class LF {};").unwrap());
        let class = tree.root().get("LF");
        let w = Wheel::from_config("LF", &class);
        assert_eq!(w.side, None);
        assert!(!w.steering);
        assert_eq!(w.suspension.travel_direction, DVec3::new(0.0, -1.0, 0.0));
        assert_eq!(w.tire.lat_stiff_x, 25.0);
        assert_eq!(w.tire.lat_stiff_y, 180.0);
        assert_eq!(w.tire.longitudinal_stiffness_per_unit_gravity, 1000.0);
        assert_eq!(w.suspension.sprung_mass, -1.0, "unset: the model decides");
    }

    #[test]
    fn the_graph_is_sampled_and_held_at_the_ends() {
        let w = wheel(LF);
        assert!((w.tire.friction_at(0.0) - 1.75).abs() < 1e-6);
        assert!((w.tire.friction_at(0.25) - 1.55).abs() < 1e-5);
        assert!((w.tire.friction_at(1.0) - 1.2).abs() < 1e-6);
        assert!(
            (w.tire.friction_at(2.0) - 1.2).abs() < 1e-6,
            "held past the end"
        );
        assert!(
            (w.tire.friction_at(-1.0) - 1.75).abs() < 1e-6,
            "and before the start"
        );
        assert_eq!(Tire::default().friction_at(0.5), 1.0);
    }

    #[test]
    fn the_spring_is_preloaded_at_the_design_position() {
        let w = wheel(LF);
        // 400 kg on 14400 N/m: 0.272 m of preload — the spring is wound up at the design
        // position, which is where the wheel model places the axle (0.272 m is more than
        // `maxCompression`, so the config's travel range is smaller than the preload; that is
        // the config authors' choice, and it only means the suspension bottoms out early).
        let deflection = w.suspension.static_deflection();
        assert!((deflection - 0.2723).abs() < 1e-3, "{deflection}");
        assert!(deflection > w.suspension.max_compression);
        // At rest the spring alone carries the sprung mass, also for a mass the config does not
        // give the wheel (the tanks' `sprungMass = -1`).
        let force = w.suspension.force(0.0, 0.0);
        assert!((force - 400.0 * GRAVITY).abs() < 0.5, "{force}");
        assert!((w.suspension.force_with(200.0, 0.0, 0.0) - 200.0 * GRAVITY).abs() < 0.5);
        assert_eq!(w.suspension.static_deflection_with(0.0), 0.0);
        // Compressing adds force, extending takes it away, and it never pulls.
        assert!(w.suspension.force(0.05, 0.0) > force);
        assert!(w.suspension.force(-0.2, 0.0) < force);
        assert_eq!(w.suspension.force(-1.0, 0.0), 0.0);
        // The damper opposes the direction of travel: compressing adds, rebounding subtracts.
        assert!(w.suspension.force(0.0, 1.0) > force);
        assert!(w.suspension.force(0.0, -1.0) < force);
    }

    #[test]
    fn stiffnesses_scale_with_the_load() {
        let w = wheel(LF);
        let load = 400.0 * GRAVITY;
        assert!((w.tire.longitudinal_stiffness(load) - 10000.0 * 400.0).abs() < 1.0);
        assert!((w.tire.lateral_stiffness(load) - 18.0 * load / 2.5).abs() < 1.0);
        assert_eq!(w.tire.longitudinal_stiffness(0.0), 0.0);
    }
}
