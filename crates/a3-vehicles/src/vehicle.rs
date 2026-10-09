//! A whole vehicle: which model runs it, and the aggregate of the engine, the gearbox, the
//! handling entries and the wheels. Plus [`VehicleInput`], what the driver is asking for.

use crate::engine::EngineData;
use crate::gearbox::GearboxData;
use crate::handling::Handling;
use crate::value;
use crate::wheel::Wheel;
use a3_config::ConfigRef;
use glam::DVec3;
use std::collections::BTreeMap;

/// Which physics model runs the vehicle, from `simulation` (`docs/re/sim-vehicles.md` §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VehicleKind {
    /// `carx`: the PhysX 4W drive — engine, gearbox, differential, raycast wheels.
    Car,
    /// `tankx`: the same drive, with a track differential instead of steering wheels.
    Tank,
    /// `shipx`, `submarinex`, `hovercraftx`: a rigid body in water with engine forces.
    Ship,
}

impl VehicleKind {
    /// The kind `simulation` names. The legacy non-PhysX names (`car`, `tank`) and the air
    /// kinds are not ours; `None` for them.
    pub fn from_simulation(simulation: &str) -> Option<VehicleKind> {
        Some(match simulation.to_ascii_lowercase().as_str() {
            "carx" => VehicleKind::Car,
            "tankx" => VehicleKind::Tank,
            "shipx" | "submarinex" | "hovercraftx" => VehicleKind::Ship,
            _ => return None,
        })
    }

    /// Whether the kind drives on wheels (cars and tanks).
    pub fn is_ground(&self) -> bool {
        matches!(self, VehicleKind::Car | VehicleKind::Tank)
    }

    /// The mass used when nothing else gives one, kg. The model in the ODOL is the real source
    /// (`docs/re/odol.md`); these are the documented stand-ins.
    pub fn placeholder_mass(&self) -> f64 {
        match self {
            VehicleKind::Car => 1500.0,
            VehicleKind::Tank => 40000.0,
            VehicleKind::Ship => 1000.0,
        }
    }
}

/// Where a vehicle's mass came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MassSource {
    /// The class's own `mass` entry.
    Config,
    /// The sum of the wheels' `sprungMass` (cars ship this; tanks ship `-1`).
    Wheels,
    /// The per-kind stand-in, because neither was there.
    Placeholder,
}

/// The model's memory points, by name, in vehicle space (X right, Y up, Z forward).
pub type MemoryPoints = BTreeMap<String, DVec3>;

/// How much of the steering the front wheels take at full lock _(ours: the original's limit is
/// not decoded; the config marks the steerable wheels with `steering = 1` and leaves the angle
/// to the physics — `docs/re/sim-vehicles.md` §5)_.
pub const DEFAULT_MAX_STEER_ANGLE: f64 = 0.52;

/// Everything a `CfgVehicles` class says about how the vehicle drives.
#[derive(Debug, Clone, PartialEq)]
pub struct VehicleData {
    /// Which model runs it.
    pub kind: VehicleKind,
    /// The resolved mass, kg.
    pub mass: f64,
    /// Where that mass came from.
    pub mass_source: MassSource,
    /// The engine.
    pub engine: EngineData,
    /// The gearbox.
    pub gearbox: GearboxData,
    /// The handling entries.
    pub handling: Handling,
    /// The wheels, in config order. Empty for ships.
    pub wheels: Vec<Wheel>,
    /// `wheelCircumference`, m. The fallback radius is `circumference / 2π`.
    pub wheel_circumference: f64,
    /// The steering angle at full lock, rad.
    pub max_steer_angle: f64,
}

impl VehicleData {
    /// Reads a vehicle class. `kind` comes from the `simulation` entry, which the caller has
    /// already looked at (`Simulation` classes may inherit it).
    pub fn from_config(config: &ConfigRef<'_>, kind: VehicleKind) -> VehicleData {
        let wheels = Wheel::all_from_config(&config.get("Wheels"));
        let handling = Handling::from_config(config);
        let (mass, mass_source) = resolve_mass(kind, &handling, &wheels);
        VehicleData {
            kind,
            mass,
            mass_source,
            engine: EngineData::from_config(config),
            gearbox: GearboxData::from_config(config),
            handling,
            wheels,
            wheel_circumference: value::number_or(&config.get("wheelCircumference"), 0.0),
            max_steer_angle: DEFAULT_MAX_STEER_ANGLE,
        }
    }

    /// Reads the `simulation` entry and, if it is ours, the rest of the class. `None` when the
    /// vehicle runs another model (a helicopter, a plane, the legacy non-PhysX classes).
    pub fn from_class(config: &ConfigRef<'_>) -> Option<VehicleData> {
        let kind = VehicleKind::from_simulation(&config.get("simulation").text())?;
        Some(VehicleData::from_config(config, kind))
    }

    /// Fills in the wheel geometry: the positions and radii from the model's memory points,
    /// with a synthetic layout for wheels the model does not place. `size` is the model's
    /// bounding-box size, the scale of that layout.
    pub fn resolve_geometry(&mut self, points: &MemoryPoints, size: DVec3) {
        let fallback_radius = self.fallback_radius();
        let plan = fallback_layout(&self.wheels, size);
        for (wheel, position) in self.wheels.iter_mut().zip(plan) {
            let center = points.get(&wheel.center_point);
            let boundary = points.get(&wheel.boundary_point);
            match (center, boundary) {
                (Some(center), Some(boundary)) => {
                    wheel.position = *center;
                    wheel.radius = (*boundary - *center).length().max(0.05);
                }
                _ => {
                    wheel.radius = fallback_radius;
                    wheel.position = position;
                }
            }
        }
    }

    /// The radius used where the model places no wheel: `wheelCircumference / 2π`, or 0.35 m.
    pub fn fallback_radius(&self) -> f64 {
        if self.wheel_circumference > 0.0 {
            self.wheel_circumference / (2.0 * std::f64::consts::PI)
        } else {
            0.35
        }
    }

    /// The wheels that turn with the steering.
    pub fn steering_wheels(&self) -> impl Iterator<Item = usize> + '_ {
        self.wheels
            .iter()
            .enumerate()
            .filter(|(_, wheel)| wheel.steering)
            .map(|(index, _)| index)
    }

    /// The wheels of the front half (Z forward) and of the rear half — how the differential
    /// and the brakes tell the axles apart.
    pub fn front_wheels(&self) -> Vec<usize> {
        self.wheels
            .iter()
            .enumerate()
            .filter(|(_, wheel)| wheel.position.z >= 0.0)
            .map(|(index, _)| index)
            .collect()
    }

    /// The wheels of the rear half.
    pub fn rear_wheels(&self) -> Vec<usize> {
        self.wheels
            .iter()
            .enumerate()
            .filter(|(_, wheel)| wheel.position.z < 0.0)
            .map(|(index, _)| index)
            .collect()
    }

    /// The height of the chassis origin above the ground at rest: the model places the memory
    /// point of a wheel's axis one radius above the ground, so the origin sits that much below
    /// the deepest wheel's axle. A vehicle placed at this height stands on its springs, each
    /// wheel carrying its own sprung mass ([`crate::wheeled::WheeledVehicle`]).
    pub fn rest_height(&self) -> f64 {
        self.wheels
            .iter()
            .filter(|wheel| wheel.is_placed())
            .map(|wheel| wheel.radius - wheel.position.y)
            .fold(0.0, f64::max)
    }

    /// The body's inertia about its axes, kg·m², as a solid box of `size` — the stand-in until
    /// the model's own tensor is read (`docs/re/odol.md`).
    pub fn box_inertia(&self, size: DVec3) -> DVec3 {
        let mass = self.mass;
        let (x, y, z) = (size.x.max(0.1), size.y.max(0.1), size.z.max(0.1));
        DVec3::new(
            mass * (y * y + z * z) / 12.0,
            mass * (x * x + z * z) / 12.0,
            mass * (x * x + y * y) / 12.0,
        )
    }
}

/// Config `mass`, else the wheels' `sprungMass`, else the kind's placeholder.
fn resolve_mass(kind: VehicleKind, handling: &Handling, wheels: &[Wheel]) -> (f64, MassSource) {
    if let Some(mass) = handling.mass {
        return (mass, MassSource::Config);
    }
    let sprung: f64 = wheels.iter().map(|w| w.suspension.sprung_mass).sum();
    if sprung > 0.0 {
        return (sprung, MassSource::Wheels);
    }
    (kind.placeholder_mass(), MassSource::Placeholder)
}

/// Where the wheels land when the model places none: the memory point names carry the layout
/// (`wheel_<side>_<row>_axis`, side 1 = left, row 1 = front), so the wheels spread along the
/// wheelbase in row order within each side.
fn fallback_layout(wheels: &[Wheel], size: DVec3) -> Vec<DVec3> {
    let half_track = size.x.max(0.1) * 0.4;
    let half_wheelbase = size.z.max(0.1) * 0.3;
    let side_of = |index: usize| match wheels[index].side {
        Some(crate::wheel::WheelSide::Left) => 1,
        Some(crate::wheel::WheelSide::Right) => 2,
        // Without `side`, the shipped configs order the wheels left, right, left, right.
        None => (index % 2) as u32 + 1,
    };
    let row_of = |index: usize| {
        parse_point_name(&wheels[index].center_point).map_or(index as u32, |(_, row)| row)
    };
    let mut out = vec![DVec3::ZERO; wheels.len()];
    for side in 1..=2u32 {
        let mut members: Vec<usize> = (0..wheels.len()).filter(|i| side_of(*i) == side).collect();
        members.sort_by_key(|index| row_of(*index));
        let count = members.len();
        for (rank, index) in members.iter().enumerate() {
            let z = if count > 1 {
                half_wheelbase * (1.0 - 2.0 * rank as f64 / (count - 1) as f64)
            } else {
                0.0
            };
            let x = if side == 1 { half_track } else { -half_track };
            out[*index] = DVec3::new(x, 0.0, z);
        }
    }
    out
}

/// `wheel_1_1_axis` → (1, 1): the side and the row.
fn parse_point_name(name: &str) -> Option<(u32, u32)> {
    let rest = name.strip_prefix("wheel_")?;
    let mut parts = rest.split('_');
    let side = parts.next()?.parse().ok()?;
    let row = parts.next()?.parse().ok()?;
    Some((side, row))
}

/// What the driver is asking for — the analogue of the Man input, filled by the player
/// controller or, later, by AI.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VehicleInput {
    /// The forward drive demand, 0..=1.
    pub throttle: f64,
    /// The brake demand, 0..=1.
    pub brake: f64,
    /// The steering demand, −1 (full left) ..= 1 (full right), positive to the right.
    pub steer: f64,
    /// The handbrake demand, 0..=1: it locks the rear wheels only.
    pub handbrake: f64,
    /// The driver asking for the reverse gear (held below `brakeIdleSpeed`, as the engine's own
    /// input scheme does).
    pub reverse: bool,
}

impl VehicleInput {
    /// Throttle and steering, nothing else.
    pub fn drive(throttle: f64, steer: f64) -> VehicleInput {
        VehicleInput {
            throttle: throttle.clamp(0.0, 1.0),
            steer: steer.clamp(-1.0, 1.0),
            ..VehicleInput::default()
        }
    }

    /// Full brake, straight.
    pub fn braking(brake: f64) -> VehicleInput {
        VehicleInput {
            brake: brake.clamp(0.0, 1.0),
            ..VehicleInput::default()
        }
    }

    /// Nothing pressed.
    pub fn idle() -> VehicleInput {
        VehicleInput::default()
    }

    /// The same input, in reverse.
    pub fn reversing(mut self) -> VehicleInput {
        self.reverse = true;
        self
    }

    /// Normalised: the fields clamped to their ranges.
    pub fn clamped(mut self) -> VehicleInput {
        self.throttle = self.throttle.clamp(0.0, 1.0);
        self.brake = self.brake.clamp(0.0, 1.0);
        self.steer = self.steer.clamp(-1.0, 1.0);
        self.handbrake = self.handbrake.clamp(0.0, 1.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_config::parse_text;

    const OFFROAD: &str = r#"
        class C_Offroad_01_F {
            simulation = "carx";
            maxSpeed = 200;
            wheelCircumference = 2.805;
            enginePower = 150;
            maxOmega = 450;
            class complexGearbox {
                GearboxRatios[] = {"R1", -4, "N", 0, "D1", 4.5};
                TransmissionRatios[] = {"High", 7};
            };
            class Wheels {
                class LF { side = "left"; steering = 1; center = "wheel_1_1_axis";
                           boundary = "wheel_1_1_bound"; sprungMass = 400;
                           springStrength = 14400; maxBrakeTorque = 2000; };
                class LR { side = "left"; steering = 0; center = "wheel_1_2_axis";
                           boundary = "wheel_1_2_bound"; sprungMass = 400;
                           springStrength = 14400; maxBrakeTorque = 2000;
                           maxHandBrakeTorque = 3000; };
                class RF { side = "right"; steering = 1; center = "wheel_2_1_axis";
                           boundary = "wheel_2_1_bound"; sprungMass = 400;
                           springStrength = 14400; maxBrakeTorque = 2000; };
                class RR { side = "right"; steering = 0; center = "wheel_2_2_axis";
                           boundary = "wheel_2_2_bound"; sprungMass = 400;
                           springStrength = 14400; maxBrakeTorque = 2000;
                           maxHandBrakeTorque = 3000; };
            };
        };"#;

    fn offroad() -> (a3_config::ConfigTree, VehicleData) {
        let tree = a3_config::ConfigTree::from_config(&parse_text(OFFROAD).unwrap());
        let data = VehicleData::from_class(&tree.root().get("C_Offroad_01_F")).unwrap();
        (tree, data)
    }

    #[test]
    fn reads_the_kind_from_simulation() {
        assert_eq!(VehicleKind::from_simulation("carX"), Some(VehicleKind::Car));
        assert_eq!(
            VehicleKind::from_simulation("tankX"),
            Some(VehicleKind::Tank)
        );
        assert_eq!(
            VehicleKind::from_simulation("shipX"),
            Some(VehicleKind::Ship)
        );
        assert_eq!(
            VehicleKind::from_simulation("submarinex"),
            Some(VehicleKind::Ship)
        );
        assert_eq!(VehicleKind::from_simulation("helicopterrtd"), None);
        assert_eq!(VehicleKind::from_simulation("car"), None, "legacy");
        assert!(VehicleKind::Car.is_ground());
        assert!(!VehicleKind::Ship.is_ground());
    }

    #[test]
    fn mass_comes_from_the_config_then_the_wheels_then_the_kind() {
        let (_tree, data) = offroad();
        assert_eq!(data.mass, 1600.0, "4 x sprungMass");
        assert_eq!(data.mass_source, MassSource::Wheels);

        // A `mass` entry wins.
        let text = "class V { simulation = \"carx\"; mass = 2100; };";
        let tree2 = a3_config::ConfigTree::from_config(&parse_text(text).unwrap());
        let data = VehicleData::from_class(&tree2.root().get("V")).unwrap();
        assert_eq!(data.mass, 2100.0);
        assert_eq!(data.mass_source, MassSource::Config);

        // A tank's wheels say -1: the placeholder stands in.
        let text =
            "class T { simulation = \"tankx\"; class Wheels { class L { sprungMass = -1; }; }; };";
        let tree3 = a3_config::ConfigTree::from_config(&parse_text(text).unwrap());
        let data = VehicleData::from_class(&tree3.root().get("T")).unwrap();
        assert_eq!(data.mass, 40000.0);
        assert_eq!(data.mass_source, MassSource::Placeholder);
    }

    #[test]
    fn a_class_of_another_model_is_not_ours() {
        let tree = a3_config::ConfigTree::from_config(
            &parse_text("class H { simulation = \"helicopterrtd\"; };").unwrap(),
        );
        assert!(VehicleData::from_class(&tree.root().get("H")).is_none());
    }

    #[test]
    fn geometry_comes_from_the_memory_points() {
        let (_tree, mut data) = offroad();
        let mut points = MemoryPoints::new();
        points.insert("wheel_1_1_axis".into(), DVec3::new(0.8, 0.3, 1.4));
        points.insert("wheel_1_1_bound".into(), DVec3::new(0.8, 0.3, 1.4 + 0.45));
        points.insert("wheel_1_2_axis".into(), DVec3::new(0.8, 0.3, -1.4));
        points.insert("wheel_1_2_bound".into(), DVec3::new(0.8, 0.3, -1.4 - 0.45));
        data.resolve_geometry(&points, DVec3::new(2.2, 1.9, 4.3));
        let front = &data.wheels[0];
        assert_eq!(front.position, DVec3::new(0.8, 0.3, 1.4));
        assert!((front.radius - 0.45).abs() < 1e-9);
        // The wheels the model does not place land on the fallback layout: left front, left
        // rear, right front, right rear, spread over the wheelbase. The fallback sits at the
        // chassis origin's height (0), the memory points at the axle height (0.3).
        let laid_out: Vec<usize> = data
            .wheels
            .iter()
            .enumerate()
            .filter(|(_, w)| w.position.y == 0.0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(laid_out, vec![2, 3]);
        assert!(data.wheels[2].radius > 0.4 && data.wheels[2].radius < 0.5);
        assert!(data.wheels[2].position.x < 0.0, "right side");
        assert!(data.wheels[2].position.z > 0.0, "front");
        assert!(data.wheels[3].position.z < 0.0, "rear");
        // 2.805 m of circumference: 0.4465 m of radius.
        assert!((data.fallback_radius() - 0.4465).abs() < 1e-3);
    }

    #[test]
    fn the_axles_and_the_steering_wheels_are_found_by_geometry() {
        let (_tree, mut data) = offroad();
        let mut points = MemoryPoints::new();
        for (name, x) in [("wheel_1_1", 0.8), ("wheel_2_1", -0.8)] {
            points.insert(format!("{name}_axis"), DVec3::new(x, 0.3, 1.4));
            points.insert(format!("{name}_bound"), DVec3::new(x, 0.3, 1.85));
        }
        for (name, x) in [("wheel_1_2", 0.8), ("wheel_2_2", -0.8)] {
            points.insert(format!("{name}_axis"), DVec3::new(x, 0.3, -1.4));
            points.insert(format!("{name}_bound"), DVec3::new(x, 0.3, -1.85));
        }
        data.resolve_geometry(&points, DVec3::new(2.2, 1.9, 4.3));
        assert_eq!(data.front_wheels(), vec![0, 2]);
        assert_eq!(data.rear_wheels(), vec![1, 3]);
        assert_eq!(data.steering_wheels().collect::<Vec<_>>(), vec![0, 2]);
        // The rest height is the deepest wheel's radius less the axle height the model gives it:
        // 0.45 - 0.3 = 0.15 m of chassis origin above the ground.
        assert!(
            (data.rest_height() - 0.15).abs() < 1e-3,
            "{}",
            data.rest_height()
        );
    }

    #[test]
    fn the_input_is_clamped_and_has_reverse() {
        let input = VehicleInput::drive(2.0, -3.0);
        assert_eq!(input.throttle, 1.0);
        assert_eq!(input.steer, -1.0);
        assert_eq!(input.brake, 0.0);
        assert!(!input.reverse);
        let input = VehicleInput::idle().reversing();
        assert!(input.reverse);
        let loose = VehicleInput {
            handbrake: 7.0,
            ..VehicleInput::default()
        };
        assert_eq!(loose.clamped().handbrake, 1.0);
    }
}
