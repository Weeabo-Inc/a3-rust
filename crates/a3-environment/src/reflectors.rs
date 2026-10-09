//! The lights a Static object carries: `CfgVehicles >> <class> >> Reflectors`.
//!
//! Every placed lamp is an ordinary object whose class holds one `Reflectors` child per light,
//! and those fields are what the engine's `PSC_LPSData` wants
//! (`docs/re/render-materials.md` §3.4): `color`, `ambient`, `intensity` and the cone angles
//! into `L1`-`L3`, `Attenuation` into `L4`/`L5`, and `position`/`direction`, which name memory
//! points of the object's p3d, resolved separately by the caller.
//!
//! Nothing here knows when a lamp is on: `Lamps_base_F` has no day/night entry, so the engine
//! switches lamps with the sun's elevation. This module only finds and decodes them.
//!
//! The WRP lists a model *path* per placed object, not a class, so [`lamp_classes`] indexes the
//! classes that carry `Reflectors` by their `model` and [`placed_lights`] matches the terrain
//! against that index.

use std::collections::HashMap;

use a3_config::ConfigRef;
use glam::{DAffine3, DVec3, Vec3};

use crate::cfg::{classes, number_or, numbers, text_or_empty};

/// The `Attenuation` of one light.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Attenuation {
    /// Distance at which falloff starts (`PSC_LPSData` `L4.x`).
    pub start: f32,
    /// `L4.y`, the constant term.
    pub constant: f32,
    /// `L4.z`, the linear term.
    pub linear: f32,
    /// `L4.w`, the quadratic term.
    pub quadratic: f32,
    /// Distance at which the light is faded out (`L5.x`).
    pub hard_limit_start: f32,
    /// Distance at which it is gone; `L5.y` is `1 / (hard_limit_end - hard_limit_start)`.
    pub hard_limit_end: f32,
}

impl Attenuation {
    /// `1 / (hardLimitEnd - hardLimitStart)`, the `L5.y` the shader multiplies by.
    pub fn fade_scale(&self) -> f32 {
        let span = self.hard_limit_end - self.hard_limit_start;
        if span > 0.0 { 1.0 / span } else { 0.0 }
    }

    /// The `L4` and `L5` vectors (`L5.x` fade start, `L5.y` its scale).
    pub fn vectors(&self) -> ([f32; 4], [f32; 2]) {
        (
            [self.start, self.constant, self.linear, self.quadratic],
            [self.hard_limit_start, self.fade_scale()],
        )
    }
}

/// The flare sprite a light may draw (`useFlare`, `flareSize`, `flareMaxDistance`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flare {
    /// Angular size of the sprite.
    pub size: f32,
    /// Distance at which it stops being drawn, in metres.
    pub max_distance: f32,
}

/// One entry of a class's `Reflectors`: what the engine needs to light with, minus the place.
#[derive(Debug, Clone, PartialEq)]
pub struct Reflector {
    /// `color[]`: the light's own colour, in the engine's absolute units.
    pub colour: Vec3,
    /// `ambient[]`: the flat term it adds to the ambient.
    pub ambient: Vec3,
    /// `intensity`.
    pub intensity: f32,
    /// `size`.
    pub size: f32,
    /// `innerAngle`, in degrees.
    pub inner_angle: f32,
    /// `outerAngle`, in degrees; 180 is a hemisphere, i.e. effectively a point light.
    pub outer_angle: f32,
    /// `coneFadeCoef`, the spot falloff exponent.
    pub cone_fade: f32,
    /// `position`: the name of a memory point.
    pub position: String,
    /// `direction`: the name of a memory point pair.
    pub direction: String,
    /// `selection`: the lamp head the engine hides when the light is off.
    pub selection: String,
    /// `class Attenuation`.
    pub attenuation: Attenuation,
    /// `useFlare` with its size and distance.
    pub flare: Option<Flare>,
    /// The class this came from, for logs and tests.
    pub class: String,
}

impl Reflector {
    /// `cos(outerAngle)`, the `L1.w` the shader compares against.
    pub fn outer_cos(&self) -> f32 {
        self.outer_angle.to_radians().cos()
    }

    /// `1 / (1 - cos(outerAngle))`, the `L2.w` the shader scales the cone by.
    pub fn cone_scale(&self) -> f32 {
        let d = 1.0 - self.outer_cos();
        if d.abs() > 1e-6 { 1.0 / d } else { 0.0 }
    }

    /// Whether the light is a cone rather than a hemisphere: the engine treats `outerAngle` 180
    /// (a full hemisphere, `cos = -1`) as a point light.
    pub fn is_spot(&self) -> bool {
        self.outer_angle < 179.0
    }

    /// Reads one `Reflectors` child class.
    pub fn from_config(c: &ConfigRef<'_>, class: &str) -> Self {
        let colour = |name: &str| -> Vec3 {
            let v = numbers(c, name);
            if v.len() >= 3 {
                Vec3::new(v[0], v[1], v[2])
            } else {
                Vec3::ZERO
            }
        };
        let light = c.get("Attenuation");
        let attenuation = Attenuation {
            start: number_or(&light, "start", 0.0),
            constant: number_or(&light, "constant", 0.0),
            linear: number_or(&light, "linear", 0.0),
            quadratic: number_or(&light, "quadratic", 0.0),
            hard_limit_start: number_or(&light, "hardLimitStart", 0.0),
            hard_limit_end: number_or(&light, "hardLimitEnd", 0.0),
        };
        let flare = if number_or(c, "useFlare", 0.0) > 0.0 {
            Some(Flare {
                size: number_or(c, "flareSize", 0.0),
                max_distance: number_or(c, "flareMaxDistance", 0.0),
            })
        } else {
            None
        };
        Reflector {
            colour: colour("color"),
            ambient: colour("ambient"),
            intensity: number_or(c, "intensity", 0.0),
            size: number_or(c, "size", 0.0),
            inner_angle: number_or(c, "innerAngle", 0.0),
            outer_angle: number_or(c, "outerAngle", 180.0),
            cone_fade: number_or(c, "coneFadeCoef", 1.0),
            position: text_or_empty(c, "position"),
            direction: text_or_empty(c, "direction"),
            selection: text_or_empty(c, "selection"),
            attenuation,
            flare,
            class: class.to_owned(),
        }
    }

    /// `color` scaled by `intensity`: the `L2.rgb` the shader uses.
    pub fn scaled_colour(&self) -> Vec3 {
        self.colour * self.intensity
    }
}

/// A `CfgVehicles` class that carries lights, with the model path the WRP would name.
#[derive(Debug, Clone, PartialEq)]
pub struct LampClass {
    /// The `CfgVehicles` class name.
    pub class: String,
    /// Its `model` entry, already normalised for matching ([`normalise_path`]).
    pub model: String,
    /// Its `Reflectors`, in config order.
    pub reflectors: Vec<Reflector>,
}

/// Normalises a VFS path for matching: lower case, forward slashes, no leading separator.
///
/// The WRP and the config disagree on separators and case (`a3\structures_f\...\lampstreet_f.p3d`
/// against `\A3\Structures_F\...\LampStreet_F.p3d`), which is why this exists.
pub fn normalise_path(path: &str) -> String {
    path.trim_start_matches(['\\', '/'])
        .replace('\\', "/")
        .to_ascii_lowercase()
}

/// Every `CfgVehicles` class that carries `Reflectors`, keyed by its model path.
pub fn lamp_classes(cfg: &ConfigRef<'_>) -> Vec<LampClass> {
    let mut out = Vec::new();
    for class in classes(&cfg.get("CfgVehicles")) {
        let reflectors = reflectors_of(&class);
        if reflectors.is_empty() {
            continue;
        }
        let model = text_or_empty(&class, "model");
        if model.is_empty() {
            continue;
        }
        out.push(LampClass {
            class: class.name().to_owned(),
            model: normalise_path(&model),
            reflectors,
        });
    }
    out
}

/// The `Reflectors` children of one class, in config order.
pub fn reflectors_of(class: &ConfigRef<'_>) -> Vec<Reflector> {
    let reflectors = class.get("Reflectors");
    if reflectors.is_null() {
        return Vec::new();
    }
    // The engine loops the children in the config's own order (Light_1, Light_2, ...), which is
    // the order `classes` returns them in.
    classes(&reflectors)
        .iter()
        .map(|c| Reflector::from_config(c, c.name()))
        .collect()
}

/// A light placed in the world: a [`Reflector`] at a position and, when it is a cone, a
/// direction.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedLight {
    /// The class the light came from.
    pub class: String,
    /// The reflector's own entry.
    pub reflector: Reflector,
    /// World position of the light.
    pub position: Vec3,
    /// Unit direction of the cone; `Vec3::Z` for the point lights the shipped street lamps are.
    pub direction: Vec3,
    /// `true` when this came from a light that should cast a cone.
    pub spot: bool,
}

/// The lights of the objects of one terrain.
///
/// `models` is the terrain's model list and `placed` its `(model index, transform)` pairs, as
/// `WorldObjects` holds them. `locate(model, point)` resolves a *memory point* of a model, by
/// name, in model space — the caller owns the p3d loading, so this stays testable without game
/// data. Lights whose class has no matching model, or whose memory points do not resolve, are
/// skipped.
///
/// `direction` names a second memory point, not a vector: the shipped `LampStreet_F` has
/// `light_1_pos` (0, 6.20, 1.46) and `light_1_dir` (0, 5.77, 1.57), so the direction is
/// `normalise(dir - pos)`, straight down out of the head. Names match ignoring case, because the
/// model stores them lower case and the config writes `Light_1_dir`.
pub fn placed_lights(
    classes: &[LampClass],
    models: &[String],
    placed: &[(u32, DAffine3)],
    locate: impl Fn(&str, &str) -> Option<Vec3>,
) -> Vec<PlacedLight> {
    let by_model: HashMap<&str, &LampClass> =
        classes.iter().map(|c| (c.model.as_str(), c)).collect();
    let mut out = Vec::new();
    for (index, transform) in placed {
        let Some(model) = models.get(*index as usize) else {
            continue;
        };
        let Some(lamp) = by_model.get(normalise_path(model).as_str()) else {
            continue;
        };
        for reflector in &lamp.reflectors {
            let Some(local) = locate(&lamp.model, &reflector.position) else {
                continue;
            };
            // The memory points are in model space; the direction is a difference of two of
            // them, so it takes the rotation only. The transforms are `f64` (world space), the
            // light list is not.
            let world = transform.transform_point3(DVec3::new(
                f64::from(local.x),
                f64::from(local.y),
                f64::from(local.z),
            ));
            let position = Vec3::new(world.x as f32, world.y as f32, world.z as f32);
            let direction = locate(&lamp.model, &reflector.direction)
                .map(|d| (d - local).normalize_or_zero())
                .filter(|d| d.length_squared() > 0.5)
                .and_then(|d| {
                    let rotated = transform.matrix3
                        * DVec3::new(f64::from(d.x), f64::from(d.y), f64::from(d.z));
                    let rotated = rotated.normalize_or_zero();
                    let v = Vec3::new(rotated.x as f32, rotated.y as f32, rotated.z as f32);
                    (v.length_squared() > 1e-6).then_some(v)
                })
                .unwrap_or(-Vec3::Y);
            out.push(PlacedLight {
                class: lamp.class.clone(),
                reflector: reflector.clone(),
                position,
                direction,
                spot: reflector.is_spot(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use a3_config::{ConfigTree, parse_text};

    use super::*;

    /// A synthetic `CfgVehicles`: one street lamp, one harbour lamp, one object without lights.
    const CONFIG: &str = r#"
class CfgVehicles {
    class All {};
    class Static: All {};
    class Lamps_base_F: Static {
        scope = 0;
        model = "\A3\Weapons_F\empty.p3d";
    };
    class Land_LampStreet_F: Lamps_base_F {
        scope = 2;
        model = "\A3\Structures_F\Civ\Lamps\LampStreet_F.p3d";
        class Reflectors {
            class Light_1 {
                color[] = {1200, 600, 300};
                ambient[] = {12, 6, 3};
                intensity = 7;
                size = 1;
                innerAngle = 100;
                outerAngle = 180;
                coneFadeCoef = 2;
                position = "Light_1_pos";
                direction = "Light_1_dir";
                hitpoint = "Light_1_hitpoint";
                selection = "Light_1_hide";
                useFlare = 1;
                flareSize = 2;
                flareMaxDistance = 220;
                class Attenuation {
                    start = 0;
                    constant = 0;
                    linear = 0;
                    quadratic = 0.3;
                    hardLimitStart = 40;
                    hardLimitEnd = 60;
                };
            };
        };
    };
    class Land_LampHarbour_F: Lamps_base_F {
        model = "A3\Structures_F\Civ\Lamps\LampHarbour_F.p3d";
        class Reflectors {
            class Light_1 {
                color[] = {800, 900, 1200};
                ambient[] = {8, 9, 12};
                intensity = 20;
                innerAngle = 60;
                outerAngle = 120;
                coneFadeCoef = 1.5;
                position = "Light_1_pos";
                direction = "Light_1_dir";
                class Attenuation { quadratic = 0.1; hardLimitStart = 30; hardLimitEnd = 50; };
            };
        };
    };
    class Land_House_F: Static {
        model = "\A3\Structures_F\House_F.p3d";
    };
};

class CfgWorlds { class Altis { latitude = -35.152; }; };
"#;

    fn tree() -> ConfigTree {
        ConfigTree::from_config(&parse_text(CONFIG).unwrap())
    }

    fn altis(tree: &ConfigTree) -> ConfigRef<'_> {
        tree.root().get("CfgWorlds").get("Altis")
    }

    #[test]
    fn reads_the_shipped_street_lamp() {
        let tree = tree();
        let vehicles = tree.root().get("CfgVehicles");
        let lamp = vehicles
            .get("Land_LampStreet_F")
            .get("Reflectors")
            .get("Light_1");
        let r = Reflector::from_config(&lamp, "Light_1");
        assert_eq!(r.colour, Vec3::new(1200.0, 600.0, 300.0));
        assert_eq!(r.ambient, Vec3::new(12.0, 6.0, 3.0));
        assert_eq!(r.intensity, 7.0);
        assert_eq!(r.scaled_colour(), Vec3::new(8400.0, 4200.0, 2100.0));
        assert_eq!((r.inner_angle, r.outer_angle), (100.0, 180.0));
        assert!(!r.is_spot(), "a hemisphere is a point light");
        assert!((r.outer_cos() + 1.0).abs() < 1e-6);
        assert_eq!(
            (r.position.as_str(), r.direction.as_str()),
            ("Light_1_pos", "Light_1_dir")
        );
        assert_eq!(r.selection, "Light_1_hide");
        assert_eq!(r.attenuation.start, 0.0);
        assert_eq!(r.attenuation.quadratic, 0.3);
        assert_eq!(r.attenuation.vectors().0, [0.0, 0.0, 0.0, 0.3]);
        assert_eq!(r.attenuation.vectors().1, [40.0, 1.0 / 20.0]);
        assert_eq!(
            r.flare,
            Some(Flare {
                size: 2.0,
                max_distance: 220.0
            })
        );
        // Freshwater: no attenuation defaults are invented.
        let empty =
            Reflector::from_config(&tree.root().get("CfgVehicles").get("Land_House_F"), "X");
        assert_eq!(empty.attenuation, Attenuation::default());
        assert_eq!(empty.flare, None);
        assert_eq!(empty.colour, Vec3::ZERO);
    }

    #[test]
    fn indexes_only_the_classes_with_lights_by_model() {
        let tree = tree();
        let classes = lamp_classes(&tree.root());
        assert_eq!(classes.len(), 2, "{classes:#?}");
        let street = classes
            .iter()
            .find(|c| c.class == "Land_LampStreet_F")
            .unwrap();
        assert_eq!(street.model, "a3/structures_f/civ/lamps/lampstreet_f.p3d");
        assert_eq!(street.reflectors.len(), 1);
        let harbour = classes
            .iter()
            .find(|c| c.class == "Land_LampHarbour_F")
            .unwrap();
        assert!(harbour.reflectors[0].is_spot(), "120 degrees is a cone");
        assert!(
            (harbour.reflectors[0].cone_scale() - 1.0 / (1.0 - 120f32.to_radians().cos())).abs()
                < 1e-4
        );
        // A house has no Reflectors and must not be indexed.
        assert!(classes.iter().all(|c| c.class != "Land_House_F"));
    }

    #[test]
    fn matches_the_terrain_by_path_whatever_the_separators() {
        let tree = tree();
        let classes = lamp_classes(&tree.root());
        // The WRP writes the path the other way round: lower case, forward slashes, no leading
        // separator, and a different case.
        let models = vec![
            "a3\\structures_f\\civ\\lamps\\lampstreet_f.p3d".to_owned(),
            "a3\\structures_f\\house_f.p3d".to_owned(),
        ];
        let placed = vec![
            (
                0u32,
                DAffine3::from_translation(glam::DVec3::new(10.0, 20.0, 30.0)),
            ),
            (
                1,
                DAffine3::from_translation(glam::DVec3::new(99.0, 99.0, 99.0)),
            ),
        ];
        // A locator that puts the light 6 m above the object's origin and points it east.
        let locate = |_model: &str, point: &str| match point {
            "Light_1_pos" => Some(Vec3::new(0.0, 6.0, 0.0)),
            "Light_1_dir" => Some(Vec3::new(0.0, 5.0, 0.0)),
            _ => None,
        };
        let lights = placed_lights(&classes, &models, &placed, locate);
        assert_eq!(lights.len(), 1, "{lights:#?}");
        let light = &lights[0];
        assert_eq!(light.class, "Land_LampStreet_F");
        assert_eq!(light.position, Vec3::new(10.0, 26.0, 30.0));
        assert!(!light.spot);
        assert_eq!(
            light.direction,
            -Vec3::Y,
            "the short vertical point pair is down"
        );
        assert_eq!(light.reflector.attenuation.hard_limit_end, 60.0);
    }

    #[test]
    fn a_spot_light_takes_its_direction_through_the_objects_rotation() {
        let tree = tree();
        let classes = lamp_classes(&tree.root());
        let models = vec!["A3/Structures_F/Civ/Lamps/LampHarbour_F.p3d".to_owned()];
        // The object is turned a quarter turn about Y, so its local east points north.
        let placed = vec![(0u32, DAffine3::from_rotation_y(std::f64::consts::FRAC_PI_2))];
        let locate = |_model: &str, point: &str| match point {
            "Light_1_pos" => Some(Vec3::new(0.0, 4.0, 0.0)),
            "Light_1_dir" => Some(Vec3::new(1.0, 4.0, 0.0)),
            _ => None,
        };
        let lights = placed_lights(&classes, &models, &placed, locate);
        assert_eq!(lights.len(), 1);
        assert!(lights[0].spot);
        assert_eq!(lights[0].position, Vec3::new(0.0, 4.0, 0.0));
        // Rotating (1,0,0) by +90 degrees about Y gives (0,0,-1).
        assert!(
            (lights[0].direction - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5,
            "{:?}",
            lights[0].direction
        );
    }

    #[test]
    fn a_light_whose_memory_point_is_missing_is_skipped() {
        let tree = tree();
        let classes = lamp_classes(&tree.root());
        let models = vec!["a3/structures_f/civ/lamps/lampstreet_f.p3d".to_owned()];
        let placed = vec![(0u32, DAffine3::IDENTITY)];
        assert!(placed_lights(&classes, &models, &placed, |_, _| None).is_empty());
        // An index past the model list is skipped too, not a panic.
        assert!(
            placed_lights(&classes, &models, &[(7, DAffine3::IDENTITY)], |_, _| Some(
                Vec3::Y
            ))
            .is_empty()
        );
    }

    #[test]
    fn the_altis_world_config_alone_has_no_lamps() {
        // CfgVehicles is what carries the lights; a config without it finds none.
        let tree = tree();
        assert!(lamp_classes(&altis(&tree)).is_empty());
    }
}
