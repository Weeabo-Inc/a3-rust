//! Surface materials (the engine's `SurfaceInfo`): what a face is made of, for sounds, impact
//! effects, friction and bullet penetration.
//!
//! A surface name is either a `.bisurf` file path (the `surfaceInfo` of an rvmat, so the
//! `surface` of an ODOL material) or `#Class`, a `CfgSurfaces` class. Faces without a material
//! (Roadway LODs, the terrain) select a `CfgSurfaces` class by their texture file name.
//! See `docs/re/physics-collision.md`.

use std::collections::HashMap;
use std::sync::Arc;

use a3_config::{ConfigRef, ConfigTree};

use crate::files::{FileSource, normalize};

/// Index of a surface in its [`SurfaceBank`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub u32);

/// One surface material.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceInfo {
    /// The name it was loaded by: a `.bisurf` path, or `#Class` for a `CfgSurfaces` class.
    pub name: String,
    /// `false` when the file or class could not be read ("Cannot load surface info"); the
    /// values are then the engine's defaults.
    pub loaded: bool,
    /// `files` (`CfgSurfaces` only): the texture name pattern that selects it.
    pub files: String,
    /// `rough`: bumpiness for vehicles.
    pub rough: f32,
    /// `dust`: dust raised by vehicles and impacts.
    pub dust: f32,
    /// `lucidity` (default 1).
    pub lucidity: f32,
    /// `grassCover` (default 0).
    pub grass_cover: f32,
    /// `maxSpeedCoef` (default 1).
    pub max_speed_coef: f32,
    /// `surfaceFriction` (default 2).
    pub surface_friction: f32,
    /// `tracksAlpha` (default 1).
    pub tracks_alpha: f32,
    /// `transparency` for view and fire queries; -1 (opaque) by default, at most 1.
    pub transparency: f32,
    /// `isWater`.
    pub is_water: bool,
    /// `soundEnviron`: footstep and movement sound environment.
    pub sound_environ: String,
    /// `soundHit`: the hit sound category (`concrete`, `metal_plate`, `soft_ground`, ...).
    pub sound_hit: String,
    /// `impact`: the CfgAmmo impact effect class.
    pub impact: String,
    /// `1e6 / bulletPenetrability` (or `bulletPenetrabilityWithThickness`), as the engine keeps
    /// it; 0 when neither is given. Higher stops bullets sooner.
    pub penetration_resistance: f32,
    /// `thickness` in metres (the file gives millimetres): plate materials whose penetration
    /// does not depend on the geometry's depth.
    pub thickness: Option<f32>,
    /// `deflection` (default 1): how readily bullets ricochet.
    pub deflection: f32,
    /// `character` (`CfgSurfaces` only): the clutter class.
    pub character: String,
    /// `Density` in kg/m³ (bisurf only; not read by the engine's `SurfaceInfo`, used for mass).
    pub density: f32,
    /// `friction` (not read by the engine's `SurfaceInfo`; used as the contact friction).
    pub friction: f32,
    /// `restitution` (as `friction`).
    pub restitution: f32,
}

impl SurfaceInfo {
    /// The values the engine uses when a surface cannot be loaded.
    pub fn unloaded(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            loaded: false,
            files: String::new(),
            rough: 0.0,
            dust: 0.0,
            lucidity: 1.0,
            grass_cover: 0.0,
            max_speed_coef: 1.0,
            surface_friction: 2.0,
            tracks_alpha: 1.0,
            transparency: -1.0,
            is_water: false,
            sound_environ: String::new(),
            sound_hit: String::new(),
            impact: String::new(),
            penetration_resistance: 0.0,
            thickness: None,
            deflection: 1.0,
            character: String::new(),
            density: 0.0,
            friction: 1.0,
            restitution: 0.0,
        }
    }

    /// Reads a surface from a config class or a parsed `.bisurf` root.
    pub fn from_config(name: &str, c: &ConfigRef<'_>) -> Self {
        let num = |key: &str, default: f32| number(&c.get(key)).unwrap_or(default);
        let text = |key: &str| {
            let e = c.get(key);
            if e.is_null() { String::new() } else { e.text() }
        };
        let penetrability = number(&c.get("bulletPenetrabilityWithThickness"))
            .or_else(|| number(&c.get("bulletPenetrability")))
            .filter(|&p| p != 0.0);
        Self {
            name: name.to_owned(),
            loaded: true,
            files: text("files"),
            rough: num("rough", 0.0),
            dust: num("dust", 0.0),
            lucidity: num("lucidity", 1.0),
            grass_cover: num("grassCover", 0.0),
            max_speed_coef: num("maxSpeedCoef", 1.0),
            surface_friction: num("surfaceFriction", 2.0),
            tracks_alpha: num("tracksAlpha", 1.0),
            transparency: num("transparency", -1.0).min(1.0),
            is_water: num("isWater", 0.0) != 0.0,
            sound_environ: text("soundEnviron"),
            sound_hit: text("soundHit"),
            impact: text("impact"),
            penetration_resistance: penetrability.map_or(0.0, |p| 1e6 / p),
            thickness: number(&c.get("thickness")).map(|mm| mm * 0.001),
            deflection: num("deflection", 1.0),
            character: text("character"),
            density: num("Density", 0.0),
            friction: num("friction", 1.0),
            restitution: num("restitution", 0.0),
        }
    }

    /// Parses a `.bisurf` file: config text (often with unquoted words) or rapified.
    pub fn parse_bisurf(name: &str, bytes: &[u8]) -> Option<Self> {
        let config = if a3_config::is_rap(bytes) {
            a3_config::read_rap(bytes).ok()?
        } else {
            a3_config::parse_text(&String::from_utf8_lossy(bytes)).ok()?
        };
        let tree = ConfigTree::from_config(&config);
        Some(Self::from_config(name, &tree.root()))
    }
}

/// A number entry, also accepting `true`/`false` and numeric text; `None` if absent.
fn number(e: &ConfigRef<'_>) -> Option<f32> {
    if e.is_null() {
        None
    } else if e.is_number() {
        Some(e.number())
    } else if e.is_text() {
        let t = e.text();
        match t.trim().to_ascii_lowercase().as_str() {
            "true" => Some(1.0),
            "false" => Some(0.0),
            s => s.parse().ok(),
        }
    } else {
        None
    }
}

/// Every surface the engine has met, by name (case-insensitive), loaded on first use and kept.
pub struct SurfaceBank {
    files: Arc<dyn FileSource>,
    infos: Vec<SurfaceInfo>,
    by_name: HashMap<String, SurfaceId>,
    /// `CfgSurfaces` classes in config order, with their lower-case `files` pattern.
    patterns: Vec<(String, SurfaceId)>,
    cfg_surfaces: HashMap<String, SurfaceInfo>,
}

impl std::fmt::Debug for SurfaceBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceBank")
            .field("surfaces", &self.infos.len())
            .finish()
    }
}

impl SurfaceBank {
    /// A bank reading `.bisurf` files from `files`. With a config, every `CfgSurfaces` class is
    /// loaded up front as `#Class` (the engine does the same) for texture matching.
    pub fn new(files: Arc<dyn FileSource>, config: Option<&ConfigTree>) -> Self {
        let mut bank = Self {
            files,
            infos: Vec::new(),
            by_name: HashMap::new(),
            patterns: Vec::new(),
            cfg_surfaces: HashMap::new(),
        };
        if let Some(config) = config {
            for class in config.root().get("CfgSurfaces").entries() {
                if !class.is_class() {
                    continue;
                }
                let name = format!("#{}", class.name());
                let info = SurfaceInfo::from_config(&name, &class);
                bank.cfg_surfaces
                    .insert(class.name().to_ascii_lowercase(), info.clone());
                let pattern = info.files.to_ascii_lowercase();
                let id = bank.push(info);
                if !pattern.is_empty() {
                    bank.patterns.push((pattern, id));
                }
            }
        }
        bank
    }

    fn push(&mut self, info: SurfaceInfo) -> SurfaceId {
        let id = SurfaceId(self.infos.len() as u32);
        self.by_name.insert(normalize(&info.name), id);
        self.infos.push(info);
        id
    }

    /// The surface called `name` (a `.bisurf` path or `#Class`), loading it on first use. A name
    /// that cannot be loaded gets the engine's defaults ([`SurfaceInfo::unloaded`]).
    pub fn surface(&mut self, name: &str) -> SurfaceId {
        if let Some(&id) = self.by_name.get(&normalize(name)) {
            return id;
        }
        let info = if let Some(class) = name.strip_prefix('#') {
            self.cfg_surfaces
                .get(&class.to_ascii_lowercase())
                .cloned()
                .unwrap_or_else(|| SurfaceInfo::unloaded(name))
        } else {
            self.files
                .read(name)
                .and_then(|bytes| SurfaceInfo::parse_bisurf(name, &bytes))
                .unwrap_or_else(|| SurfaceInfo::unloaded(name))
        };
        self.push(info)
    }

    /// The `CfgSurfaces` class whose `files` pattern matches the file name of `texture`
    /// (without its extension); the first in config order wins.
    pub fn for_texture(&self, texture: &str) -> Option<SurfaceId> {
        let file = texture.rsplit(['\\', '/']).next()?.to_ascii_lowercase();
        let stem = file.rsplit_once('.').map_or(file.as_str(), |(s, _)| s);
        self.patterns
            .iter()
            .find(|(p, _)| wildcard_match(p, stem) || wildcard_match(p, &file))
            .map(|&(_, id)| id)
    }

    pub fn get(&self, id: SurfaceId) -> &SurfaceInfo {
        &self.infos[id.0 as usize]
    }

    /// Number of surfaces known.
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }
}

/// `*` matches any run of characters, `?` one character.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            pi += 1;
            mark = ti;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}
