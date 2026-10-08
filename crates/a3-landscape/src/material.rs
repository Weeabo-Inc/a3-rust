//! Terrain layer materials: the `p_XXX-YYY_*.rvmat` each land cell uses.

use a3_config::{Config, ConfigRef, ConfigTree};
use glam::Vec3;

use crate::Error;
use crate::cfg::{text, text_or_empty, vec3};

/// Where a texture coordinate generator takes its input from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UvSource {
    /// The mesh's own texture coordinates (`uvSource = "tex"`).
    Tex,
    /// The world position of the vertex (`uvSource = "worldPos"`).
    WorldPos,
    /// No generator for the stage (`uvSource = "none"` or no `TexGen` class).
    None,
    /// Any other source, as written.
    Other(String),
}

/// A texture coordinate transform (`class TexGenN` of an rvmat).
///
/// `uv = aside * p.x + up * p.y + dir * p.z + pos`, where `p` is the source coordinate
/// (`(u, v, 0)` for [`UvSource::Tex`], the world position `(x, y, z)` for
/// [`UvSource::WorldPos`]); the result's `x` and `y` are the texture `u` and `v`.
#[derive(Debug, Clone, PartialEq)]
pub struct UvTransform {
    /// The input.
    pub source: UvSource,
    /// Column multiplying the input's x.
    pub aside: Vec3,
    /// Column multiplying the input's y.
    pub up: Vec3,
    /// Column multiplying the input's z.
    pub dir: Vec3,
    /// Offset.
    pub pos: Vec3,
}

impl UvTransform {
    /// The identity transform of `source`.
    pub fn identity(source: UvSource) -> Self {
        Self {
            source,
            aside: Vec3::X,
            up: Vec3::Y,
            dir: Vec3::Z,
            pos: Vec3::ZERO,
        }
    }

    /// Transforms one source coordinate.
    pub fn apply(&self, p: Vec3) -> Vec3 {
        self.aside * p.x + self.up * p.y + self.dir * p.z + self.pos
    }
}

/// One texture stage of a material.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureStage {
    /// The texture as written: a VFS path (`a3\...\x_co.paa`) or a procedural texture
    /// (`#(rgb,1,1,1)color(...)`). Empty when the stage is unused.
    pub texture: String,
    /// The coordinate generator the stage uses (`texGen = N`).
    pub uv: UvTransform,
}

/// One surface layer of a cell: a tiled detail colour texture and its normal/parallax map.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceLayer {
    /// The layer slot, 0-based: slot `k` uses stages `3 + 2k` (normal) and `4 + 2k` (colour)
    /// and is the `k`-th name after `p_XXX-YYY_` in the rvmat file name.
    pub slot: usize,
    /// The `_nopx` normal/parallax map.
    pub normal: TextureStage,
    /// The `_co` detail colour texture; its file name selects the CfgSurfaces class.
    pub color: TextureStage,
}

/// The material of one satellite tile: what the terrain renderer draws on the land cells that
/// use it.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerMaterial {
    /// The rvmat path.
    pub path: String,
    /// `PixelShaderID` (`TerrainSNX` and its variants in shipped terrains).
    pub pixel_shader: String,
    /// `VertexShaderID`.
    pub vertex_shader: String,
    /// The satellite colour tile (`s_XXX_YYY_lco`), stage 0.
    pub satellite: TextureStage,
    /// The layer mask tile (`m_XXX_YYY_lca`), stage 1.
    pub mask: TextureStage,
    /// The constant / colour-modifier texture of stage 2, if any.
    pub constant: Option<TextureStage>,
    /// The satellite normal map tile (`n_XXX_YYY_nohq`), stage 14, if any.
    pub satellite_normal: Option<TextureStage>,
    /// Layers whose colour texture is set, by slot.
    pub layers: Vec<SurfaceLayer>,
    /// The tile `(column from west, row from north)` parsed from `p_XXX-YYY` in the path.
    pub tile: Option<(u32, u32)>,
}

/// The stage of the satellite normal map.
const SATELLITE_NORMAL_STAGE: usize = 14;
/// The first layer stage; layer slot `k` uses stages `3 + 2k` and `4 + 2k`.
const FIRST_LAYER_STAGE: usize = 3;

impl LayerMaterial {
    /// Reads a layer material from its parsed rvmat.
    pub fn from_config(path: &str, config: &Config) -> Result<Self, Error> {
        let tree = ConfigTree::from_config(config);
        let root = tree.root();
        let stage = |n: usize| read_stage(&root, n);
        let required = |n: usize| {
            stage(n).ok_or_else(|| Error::Material {
                path: path.to_owned(),
                detail: format!("no Stage{n}"),
            })
        };
        let mut layers = Vec::new();
        let mut slot = 0;
        while FIRST_LAYER_STAGE + 2 * slot + 1 < SATELLITE_NORMAL_STAGE {
            let normal = stage(FIRST_LAYER_STAGE + 2 * slot);
            let color = stage(FIRST_LAYER_STAGE + 2 * slot + 1);
            match (normal, color) {
                (Some(normal), Some(color)) if !color.texture.is_empty() => {
                    layers.push(SurfaceLayer {
                        slot,
                        normal,
                        color,
                    })
                }
                (None, None) => break,
                _ => {}
            }
            slot += 1;
        }
        Ok(Self {
            path: path.to_owned(),
            pixel_shader: text_or_empty(&root, "PixelShaderID"),
            vertex_shader: text_or_empty(&root, "VertexShaderID"),
            satellite: required(0)?,
            mask: required(1)?,
            constant: stage(2),
            satellite_normal: stage(SATELLITE_NORMAL_STAGE),
            layers,
            tile: parse_tile(path),
        })
    }
}

fn read_stage(root: &ConfigRef<'_>, n: usize) -> Option<TextureStage> {
    let stage = root.get(&format!("Stage{n}"));
    if !stage.is_class() {
        return None;
    }
    let texture = text_or_empty(&stage, "texture");
    let uv = match crate::cfg::number(&stage, "texGen") {
        Some(g) => read_texgen(root, g as i32),
        // Stages without texGen use their own uvSource / uvTransform (old-style rvmats).
        None => read_uv(&stage),
    };
    Some(TextureStage { texture, uv })
}

fn read_texgen(root: &ConfigRef<'_>, n: i32) -> UvTransform {
    let texgen = root.get(&format!("TexGen{n}"));
    if texgen.is_class() {
        read_uv(&texgen)
    } else {
        UvTransform::identity(UvSource::None)
    }
}

fn read_uv(c: &ConfigRef<'_>) -> UvTransform {
    let source = match text(c, "uvSource").map(|s| s.to_ascii_lowercase()) {
        None => UvSource::None,
        Some(s) if s == "tex" => UvSource::Tex,
        Some(s) if s == "worldpos" => UvSource::WorldPos,
        Some(s) if s == "none" => UvSource::None,
        Some(s) => UvSource::Other(s),
    };
    let t = c.get("uvTransform");
    if !t.is_class() {
        return UvTransform::identity(source);
    }
    UvTransform {
        source,
        aside: vec3(&t, "aside"),
        up: vec3(&t, "up"),
        dir: vec3(&t, "dir"),
        pos: vec3(&t, "pos"),
    }
}

/// `(XXX, YYY)` from a path whose file name starts `p_XXX-YYY`.
pub(crate) fn parse_tile(path: &str) -> Option<(u32, u32)> {
    let name = path.rsplit(['\\', '/']).next()?;
    let rest = name
        .strip_prefix("p_")
        .or_else(|| name.strip_prefix("P_"))?;
    let (x, rest) = rest.split_once('-')?;
    let y = rest.split(['_', '.']).next()?;
    Some((x.parse().ok()?, y.parse().ok()?))
}
