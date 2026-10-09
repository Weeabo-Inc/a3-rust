//! Environment (ambient) sound: the 22 environment values at the listener, computed from the
//! terrain's sound map and the weather, and the `soundSetEnvironment[]` loops they drive
//! (`docs/re/audio.md` section 4).

use glam::DVec3;

use crate::config::SoundBank;
use crate::player::{Rng, SoundSetPlayer};
use crate::{AudioEngine, SoundLoader};

/// The variables of environment sound expressions, in the engine's order (lower case; the
/// lookup ignores case). An expression naming anything else does not compile, and its sound set
/// does not play.
pub const ENVIRONMENT_VARIABLES: [&str; 22] = [
    "rain",
    "night",
    "meadow",
    "trees",
    "hills",
    "houses",
    "windy",
    "deadbody",
    "sea",
    "forest",
    "waterdepth",
    "camdepth",
    "anomaly",
    "coast",
    "altitudeground",
    "altitudesea",
    "daytime",
    "shooting",
    "fog",
    "yeartime",
    "ambienttemp",
    "snow",
];

/// The four weights packed in a sound map byte (2 bits each, read as `field / 3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundLayer {
    /// Bits 0-1.
    Sea = 0,
    /// Bits 2-3.
    Trees = 1,
    /// Bits 4-5.
    Meadow = 2,
    /// Bits 6-7.
    Houses = 3,
}

/// The terrain data the environment values need: the sound map, water classes of the
/// geography grid, and the heightmap.
#[derive(Debug, Clone)]
pub struct EnvironmentMap {
    sound: Vec<u8>,
    sound_size: (u32, u32),
    sound_cell: f64,
    water: Vec<u8>,
    land_size: (u32, u32),
    land_cell: f64,
    heights: Vec<f32>,
    height_size: (u32, u32),
    height_cell: f64,
    /// `minHillsALtitude` of the world (default 0).
    pub min_hills: f32,
    /// `maxHillsALtitude` of the world (default 500).
    pub max_hills: f32,
}

impl EnvironmentMap {
    /// Builds the map from a WRP terrain. The sound cell size follows from the sound map's
    /// resolution (land cell / `soundMapSizeCoef`).
    pub fn from_terrain(terrain: &a3_wrp::Terrain) -> Self {
        let world = f64::from(terrain.world_size());
        let sm = &terrain.sound_map;
        let land = &terrain.geography;
        let hm = &terrain.heightmap;
        Self {
            sound: sm.as_slice().to_vec(),
            sound_size: (sm.width(), sm.height()),
            sound_cell: world / f64::from(sm.width().max(1)),
            water: land
                .as_slice()
                .iter()
                .map(|g| g.min_water_depth() + g.max_water_depth())
                .collect(),
            land_size: (land.width(), land.height()),
            land_cell: f64::from(terrain.land_cell_size),
            heights: hm.as_slice().to_vec(),
            height_size: (hm.width(), hm.height()),
            height_cell: f64::from(terrain.terrain_cell_size()),
            min_hills: 0.0,
            max_hills: 500.0,
        }
    }

    /// A map from raw grids (row-major, `z` rows of `x` cells): sound bytes over cells of
    /// `sound_cell` metres, water classes (0..=6) per land cell of `land_cell` metres, and
    /// heights per grid point every `height_cell` metres.
    pub fn from_grids(
        sound: (Vec<u8>, u32, f64),
        water: (Vec<u8>, u32, f64),
        heights: (Vec<f32>, u32, f64),
    ) -> Self {
        let rows = |len: usize, w: u32| (len as u32).checked_div(w.max(1)).unwrap_or(0);
        Self {
            sound_size: (sound.1, rows(sound.0.len(), sound.1)),
            sound: sound.0,
            sound_cell: sound.2,
            land_size: (water.1, rows(water.0.len(), water.1)),
            water: water.0,
            land_cell: water.2,
            height_size: (heights.1, rows(heights.0.len(), heights.1)),
            heights: heights.0,
            height_cell: heights.2,
            min_hills: 0.0,
            max_hills: 500.0,
        }
    }

    /// The sound map byte of cell `(x, z)`. Outside the map the indices clamp and only the sea
    /// and meadow weights survive (as in the engine).
    fn sound_cell(&self, x: i64, z: i64) -> u8 {
        let (w, h) = (i64::from(self.sound_size.0), i64::from(self.sound_size.1));
        if w == 0 || h == 0 {
            return 0;
        }
        let inside = (0..w).contains(&x) && (0..h).contains(&z);
        let (cx, cz) = (x.clamp(0, w - 1), z.clamp(0, h - 1));
        let byte = self.sound[(cz * w + cx) as usize];
        if inside { byte } else { byte & 0x33 }
    }

    /// The weight of `layer` at world `(x, z)`, interpolated bilinearly between cell centres.
    pub fn layer(&self, x: f64, z: f64, layer: SoundLayer) -> f32 {
        let inv = 1.0 / self.sound_cell;
        let (fx, fz) = (x * inv - 0.5, z * inv - 0.5);
        let (ix, iz) = (fx.floor() as i64, fz.floor() as i64);
        let (tx, tz) = ((fx - ix as f64) as f32, (fz - iz as f64) as f32);
        let shift = 2 * layer as u32;
        let v = |i: i64, j: i64| f32::from((self.sound_cell(i, j) >> shift) & 3) / 3.0;
        v(ix, iz) * (1.0 - tx) * (1.0 - tz)
            + v(ix + 1, iz) * tx * (1.0 - tz)
            + v(ix, iz + 1) * (1.0 - tx) * tz
            + v(ix + 1, iz + 1) * tx * tz
    }

    fn height_at(&self, i: i64, j: i64) -> f32 {
        let (w, h) = (i64::from(self.height_size.0), i64::from(self.height_size.1));
        if w == 0 || h == 0 {
            return 0.0;
        }
        self.heights[(j.clamp(0, h - 1) * w + i.clamp(0, w - 1)) as usize]
    }

    /// Terrain height at world `(x, z)`, interpolated over the heightmap's triangles.
    pub fn terrain_height(&self, x: f64, z: f64) -> f32 {
        let inv = 1.0 / self.height_cell;
        let (gx, gz) = (x * inv, z * inv);
        let (i, j) = (gx.floor() as i64, gz.floor() as i64);
        let (fx, fz) = ((gx - i as f64) as f32, (gz - j as f64) as f32);
        let h00 = self.height_at(i, j);
        let h10 = self.height_at(i + 1, j);
        let h01 = self.height_at(i, j + 1);
        let h11 = self.height_at(i + 1, j + 1);
        if fx + fz <= 1.0 {
            h00 + (h10 - h00) * fx + (h01 - h00) * fz
        } else {
            (h01 + h10 - h11) + (h11 - h01) * fx + (h11 - h10) * fz
        }
    }

    fn water_class(&self, i: i64, j: i64) -> f32 {
        let (w, h) = (i64::from(self.land_size.0), i64::from(self.land_size.1));
        if w == 0 || h == 0 {
            return 0.0;
        }
        f32::from(self.water[(j.clamp(0, h - 1) * w + i.clamp(0, w - 1)) as usize])
    }

    /// The water fraction `c` around `(x, z)` from the geography grid: corner values are 2x2
    /// sums of the cells' water classes (0..=6) over 24, interpolated bilinearly
    /// _(medium: corner placement)_.
    pub fn water_fraction(&self, x: f64, z: f64) -> f32 {
        let inv = 1.0 / self.land_cell;
        let (gx, gz) = (x * inv, z * inv);
        let (i, j) = (gx.floor() as i64, gz.floor() as i64);
        let (tx, tz) = ((gx - i as f64) as f32, (gz - j as f64) as f32);
        let corner = |i: i64, j: i64| {
            (self.water_class(i - 1, j - 1)
                + self.water_class(i, j - 1)
                + self.water_class(i - 1, j)
                + self.water_class(i, j))
                / 24.0
        };
        corner(i, j) * (1.0 - tx) * (1.0 - tz)
            + corner(i + 1, j) * tx * (1.0 - tz)
            + corner(i, j + 1) * (1.0 - tx) * tz
            + corner(i + 1, j + 1) * tx * tz
    }
}

/// Weather and scene state the environment values read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conditions {
    /// Rain intensity, 0..1.
    pub rain: f32,
    /// Night factor, 0 (day) to 1 (night).
    pub night: f32,
    /// Wind speed in m/s.
    pub wind_speed: f32,
    /// Time of day as a fraction of the day (0.5 = noon).
    pub daytime: f32,
    /// Recent gunfire, 0..1.
    pub shooting: f32,
    /// Fog density at the listener, 0..1.
    pub fog: f32,
    /// Fraction of the year.
    pub year_time: f32,
    /// Ambient temperature, °C.
    pub ambient_temp: f32,
    /// Height of the water surface (sea level 0; tides move it).
    pub water_level: f32,
    /// The world has snow instead of rain.
    pub snow: bool,
}

impl Default for Conditions {
    /// A calm, dry noon.
    fn default() -> Self {
        Self {
            rain: 0.0,
            night: 0.0,
            wind_speed: 3.0,
            daytime: 0.5,
            shooting: 0.0,
            fog: 0.0,
            year_time: 0.5,
            ambient_temp: 20.0,
            water_level: 0.0,
            snow: false,
        }
    }
}

/// Offset of the four extra trees samples of `forest`, metres.
const FOREST_PROBE: f64 = 20.0;

/// The 22 environment values at `listener` (indexed like [`ENVIRONMENT_VARIABLES`]).
pub fn environment_values(map: &EnvironmentMap, listener: DVec3, c: &Conditions) -> [f32; 22] {
    let (x, z) = (listener.x, listener.z);
    let y = listener.y as f32;
    let ground = map.terrain_height(x, z);
    let surface = ground.max(c.water_level);
    let above_water = ((y - surface + 0.1) / 1.1).clamp(0.0, 1.0);
    let trees = map.layer(x, z, SoundLayer::Trees);
    let forest_sum: f32 = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)]
        .iter()
        .map(|(dx, dz)| map.layer(x + dx * FOREST_PROBE, z + dz * FOREST_PROBE, SoundLayer::Trees))
        .sum();
    let hills_span = (map.max_hills - map.min_hills).max(1e-3);
    [
        if c.snow { 0.0 } else { c.rain },
        c.night,
        map.layer(x, z, SoundLayer::Meadow),
        trees,
        ((ground - map.min_hills) / hills_span).clamp(0.0, 1.0),
        map.layer(x, z, SoundLayer::Houses),
        (c.wind_speed.abs() / 10.0).clamp(0.0, 1.0) * above_water,
        0.0, // deadbody: no bodies yet
        map.layer(x, z, SoundLayer::Sea),
        (trees + forest_sum - 4.0).max(0.0),
        (c.water_level - ground).max(0.0),
        (c.water_level - y).max(0.0),
        0.0, // anomaly
        (std::f32::consts::PI * map.water_fraction(x, z).clamp(0.0, 1.0)).sin() * above_water,
        (y - ground).max(0.0),
        y.max(0.0),
        c.daytime,
        c.shooting,
        c.fog,
        c.year_time,
        c.ambient_temp,
        if c.snow { 1.0 } else { 0.0 },
    ]
}

/// A lookup for expressions over the environment values.
pub fn lookup(values: &[f32; 22]) -> impl Fn(&str) -> Option<f32> + '_ {
    move |name: &str| {
        ENVIRONMENT_VARIABLES
            .iter()
            .position(|v| v.eq_ignore_ascii_case(name))
            .map(|i| values[i])
    }
}

/// The `soundSetEnvironment[]` loops of CfgEnvSounds, playing non-spatially with volumes from
/// the environment values.
pub struct EnvironmentSounds {
    players: Vec<SoundSetPlayer>,
    /// Sets that do not play, with the reason (unknown set, expression that does not compile in
    /// the environment context).
    pub skipped: Vec<(String, String)>,
}

impl EnvironmentSounds {
    /// Starts every environment set whose expressions compile against
    /// [`ENVIRONMENT_VARIABLES`], silent until the first [`EnvironmentSounds::update`].
    pub fn start(bank: &SoundBank, loader: &SoundLoader, engine: &AudioEngine, seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        let mut players = Vec::new();
        let mut skipped = Vec::new();
        for name in &bank.environment_sets {
            if let Err(reason) = check_set(bank, name) {
                skipped.push((name.clone(), reason));
                continue;
            }
            match SoundSetPlayer::start_loop(bank, loader, engine, name, &mut rng) {
                Some(player) => players.push(player),
                None => skipped.push((name.clone(), "unknown sound set".into())),
            }
        }
        for (name, reason) in &skipped {
            log::info!("environment sound set {name} does not play: {reason}");
        }
        Self { players, skipped }
    }

    /// Number of sets playing.
    pub fn len(&self) -> usize {
        self.players.len()
    }

    /// Whether no set plays.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// Re-evaluates every set with `values` and sets the voice levels (times `gain`). Returns
    /// `(set name, loudest voice gain)` per set.
    pub fn update(&self, engine: &AudioEngine, values: &[f32; 22], gain: f32) -> Vec<(&str, f32)> {
        let vars = lookup(values);
        self.players
            .iter()
            .map(|p| (p.name(), p.update(engine, &vars, gain)))
            .collect()
    }

    /// Fades every set out.
    pub fn stop(&self, engine: &AudioEngine, fade: f32) {
        for p in &self.players {
            p.stop(engine, fade);
        }
    }
}

/// Whether every expression of the set compiles in the environment context.
fn check_set(bank: &SoundBank, name: &str) -> Result<(), String> {
    let set = bank.set(name).ok_or("unknown sound set")?;
    set.volume
        .check(&ENVIRONMENT_VARIABLES)
        .map_err(|e| e.to_string())?;
    for shader in set.shaders.iter().filter_map(|s| bank.shader(s)) {
        for expr in [&shader.volume, &shader.frequency] {
            expr.check(&ENVIRONMENT_VARIABLES)
                .map_err(|e| format!("{}: {e}", shader.name))?;
        }
    }
    Ok(())
}
