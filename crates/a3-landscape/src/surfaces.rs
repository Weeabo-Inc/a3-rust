//! Surface types (`CfgSurfaces`) and their clutter (`CfgSurfaceCharacters`).

use a3_config::{ConfigRef, ConfigTree};

use crate::cfg::{classes, number_or, numbers, text_or_empty, texts};

/// One surface type: how the ground under a layer behaves (sounds, friction, dust, clutter).
#[derive(Debug, Clone, PartialEq)]
pub struct Surface {
    /// The class name, e.g. `GdtSeabed`.
    pub class: String,
    /// The texture file name pattern that selects this surface (`*` wildcard), e.g.
    /// `gdt_seabed_*`.
    pub files: String,
    /// `CfgSurfaceCharacters` class: the clutter grown on it.
    pub character: String,
    /// Sound environment name.
    pub sound_environ: String,
    /// Sound of hits on it.
    pub sound_hit: String,
    /// Impact effect class.
    pub impact: String,
    /// `rough`: vehicle bumpiness.
    pub rough: f32,
    /// `dust`: dust raised by vehicles.
    pub dust: f32,
    /// `lucidity`.
    pub lucidity: f32,
    /// `grassCover`: how much clutter hides prone soldiers.
    pub grass_cover: f32,
    /// `isWater`.
    pub is_water: bool,
    /// `maxSpeedCoef`.
    pub max_speed_coef: f32,
    /// `friction`.
    pub friction: f32,
    /// `surfaceFriction`.
    pub surface_friction: f32,
    /// `restitution`.
    pub restitution: f32,
    /// `tracksAlpha`.
    pub tracks_alpha: f32,
    /// `transparency`.
    pub transparency: f32,
    /// `AIAvoidStance`.
    pub ai_avoid_stance: f32,
}

/// A clutter mix: which clutter classes grow on a surface and how often.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceCharacter {
    /// The class name.
    pub class: String,
    /// `(clutter class in CfgWorlds >> <world> >> clutter, probability)`.
    pub clutter: Vec<(String, f32)>,
}

/// Every surface type and clutter character of the merged config.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Surfaces {
    /// `CfgSurfaces` classes in config order.
    pub surfaces: Vec<Surface>,
    /// `CfgSurfaceCharacters` classes in config order.
    pub characters: Vec<SurfaceCharacter>,
}

impl Surfaces {
    /// Reads `CfgSurfaces` and `CfgSurfaceCharacters`.
    pub fn load(config: &ConfigTree) -> Self {
        let root = config.root();
        Self {
            surfaces: classes(&root.get("CfgSurfaces"))
                .iter()
                .map(read_surface)
                .collect(),
            characters: classes(&root.get("CfgSurfaceCharacters"))
                .iter()
                .map(read_character)
                .collect(),
        }
    }

    /// The surface whose `files` pattern matches the file name of `texture` (a layer's detail
    /// colour texture, e.g. `a3\map_data\gdt_seabed_co.paa`). The first match in config order
    /// wins _(uncertain: the engine's tie-break)_.
    pub fn for_texture(&self, texture: &str) -> Option<&Surface> {
        let name = texture.rsplit(['\\', '/']).next()?.to_ascii_lowercase();
        self.surfaces
            .iter()
            .find(|s| !s.files.is_empty() && wildcard_match(&s.files.to_ascii_lowercase(), &name))
    }

    /// The surface class named `class` (case-insensitive).
    pub fn surface(&self, class: &str) -> Option<&Surface> {
        self.surfaces
            .iter()
            .find(|s| s.class.eq_ignore_ascii_case(class))
    }

    /// The character class named `class` (case-insensitive).
    pub fn character(&self, class: &str) -> Option<&SurfaceCharacter> {
        self.characters
            .iter()
            .find(|c| c.class.eq_ignore_ascii_case(class))
    }
}

fn read_surface(c: &ConfigRef<'_>) -> Surface {
    Surface {
        class: c.name().to_owned(),
        files: text_or_empty(c, "files"),
        character: text_or_empty(c, "character"),
        sound_environ: text_or_empty(c, "soundEnviron"),
        sound_hit: text_or_empty(c, "soundHit"),
        impact: text_or_empty(c, "impact"),
        rough: number_or(c, "rough", 0.0),
        dust: number_or(c, "dust", 0.0),
        lucidity: number_or(c, "lucidity", 1.0),
        grass_cover: number_or(c, "grassCover", 0.0),
        is_water: number_or(c, "isWater", 0.0) != 0.0,
        max_speed_coef: number_or(c, "maxSpeedCoef", 1.0),
        friction: number_or(c, "friction", 0.9),
        surface_friction: number_or(c, "surfaceFriction", 2.0),
        restitution: number_or(c, "restitution", 0.0),
        tracks_alpha: number_or(c, "tracksAlpha", 1.0),
        transparency: number_or(c, "transparency", -1.0),
        ai_avoid_stance: number_or(c, "AIAvoidStance", 0.0),
    }
}

fn read_character(c: &ConfigRef<'_>) -> SurfaceCharacter {
    let names = texts(c, "names");
    let probabilities = numbers(c, "probability");
    SurfaceCharacter {
        class: c.name().to_owned(),
        clutter: names
            .into_iter()
            .zip(probabilities.into_iter().chain(std::iter::repeat(0.0)))
            .collect(),
    }
}

/// `*` matches any run of characters, `?` one character.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::wildcard_match;

    #[test]
    fn wildcards_match_like_file_globs() {
        assert!(wildcard_match("gdt_seabed_*", "gdt_seabed_co.paa"));
        assert!(wildcard_match("more_anim*", "more_anim.01.paa"));
        assert!(!wildcard_match("gdt_seabed_*", "gdt_seabedexp_co.paa"));
        assert!(wildcard_match("default", "default"));
        assert!(wildcard_match("a?c*", "abcdef"));
    }
}
