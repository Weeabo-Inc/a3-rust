//! Materials: what an ODOL section's embedded rvmat means for our shaders.
//!
//! A section draws with its texture (stage 0) and its material. Which stage holds what follows
//! `docs/re/render-materials.md` (the engine's shader caches): `Super`, `Glass` and `Multi` have
//! fixed stage layouts; the other shaders' stages are recognised by their Texture suffix
//! (`_nohq` normal, `_smdi` specular, `_as` ambient shadow, `_mc` macro, `_dt` detail). Each
//! stage's UV set and transform come from the tex gen it names.

use std::hash::{Hash, Hasher};

use a3_paa::{AlphaFlags, Procedural, ProceduralFunction, TextureKind};

use crate::shader::{PixelShader, ShaderFamily};

/// What a texture is for in our model shaders.
///
/// Like the engine, which binds stage `k` to `t<k>`, a material binds [`Slot::COUNT`] textures;
/// slots that no shader family uses together share a binding ([`Slot::binding`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// The colour map (`_co`/`_ca`, stage 0).
    Diffuse,
    /// Tangent-space normal map (`_nohq`).
    Normal,
    /// `_smdi`/`_sm`: green specular intensity, blue glossiness.
    Specular,
    /// `_as`: ambient occlusion for ambient and sun light.
    AmbientShadow,
    /// `_mc`: large-scale colour blended by its alpha; for trees the `_mca` crown map.
    Macro,
    /// `_dt`: tiled detail, applied as `colour * 2 * detail`.
    Detail,
    /// `Super`/`Glass`: Fresnel reflectance table (`fresnel(n,k)`, read in alpha at N.V).
    Fresnel,
    /// `Super`/`Glass`: environment map (`_sky`/`_env`/`_co`), alpha = HDR exponent.
    Environment,
    /// `Multi`: the layer mask (red, green, blue weight layers 1, 2, 3).
    Mask,
    /// `Multi`: colour map of layer 0..=3 (layer 0 is [`Slot::Diffuse`]).
    Layer(u8),
    /// `Multi`: normal map of layer 0..=3 (layer 0 is [`Slot::Normal`]).
    LayerNormal(u8),
    /// `Multi`: `_dtsmdi` of layer 0..=3: red detail, green specular, blue gloss.
    LayerSpecular(u8),
}

impl Slot {
    /// Number of texture bindings of a material.
    pub const COUNT: usize = 15;

    /// The binding index, `0..COUNT`. `Multi` uses every binding; the other families use
    /// 0..=7, which `Multi` shares for its layers.
    pub fn binding(self) -> usize {
        match self {
            Slot::Diffuse | Slot::Layer(0) => 0,
            Slot::Normal | Slot::LayerNormal(0) => 1,
            Slot::Specular | Slot::LayerSpecular(0) => 2,
            Slot::AmbientShadow => 3,
            Slot::Macro => 4,
            Slot::Detail | Slot::Mask => 5,
            Slot::Fresnel | Slot::Layer(1) => 6,
            Slot::Environment | Slot::Layer(2) => 7,
            Slot::Layer(_) => 8,
            Slot::LayerNormal(n) => 8 + usize::from(n.min(3)),
            Slot::LayerSpecular(n) => 11 + usize::from(n.min(3)),
        }
    }

    /// The slot at `binding` for materials of `family`.
    pub fn at(binding: usize, family: ShaderFamily) -> Slot {
        let multi = family == ShaderFamily::Multi;
        match binding {
            0 => Slot::Diffuse,
            1 => Slot::Normal,
            2 if multi => Slot::LayerSpecular(0),
            2 => Slot::Specular,
            3 => Slot::AmbientShadow,
            4 => Slot::Macro,
            5 if multi => Slot::Mask,
            5 => Slot::Detail,
            6 if multi => Slot::Layer(1),
            6 => Slot::Fresnel,
            7 if multi => Slot::Layer(2),
            7 => Slot::Environment,
            8 => Slot::Layer(3),
            9..=11 => Slot::LayerNormal((binding - 8) as u8),
            _ => Slot::LayerSpecular((binding - 11).min(3) as u8),
        }
    }

    /// What the shader samples when a material of `family` leaves this slot empty (RGBA8).
    pub fn default_texel(self, family: ShaderFamily) -> [u8; 4] {
        match self {
            // A flat normal in the `_nohq` layout: x = 2 (r - a) + 1, y = 2 g - 1, z = 2 b - 1.
            Slot::Normal | Slot::LayerNormal(_) => [0, 128, 255, 128],
            // No specular; `_dtsmdi` detail red at 0.5 leaves the colour unchanged.
            Slot::Specular => [255, 0, 0, 255],
            Slot::LayerSpecular(_) => [128, 0, 0, 255],
            // Tree `_mca`: neutral sRGB grey (scaled by 2^2.2 to 1), no occlusion.
            Slot::Macro if family == ShaderFamily::Tree => [128, 128, 128, 255],
            // Black, transparent: the macro map's alpha blends nothing.
            Slot::Macro => [0, 0, 0, 0],
            // 0.5 grey: `colour * 2 * detail` leaves the colour unchanged.
            Slot::Detail => [128, 128, 128, 255],
            Slot::Mask => [0, 0, 0, 0],
            // No reflection.
            Slot::Environment => [0, 0, 0, 255],
            _ => [255; 4],
        }
    }

    /// Whether the slot holds colour (sampled sRGB-decoded) rather than data (linear).
    pub fn is_color(self) -> bool {
        matches!(
            self,
            Slot::Diffuse | Slot::Layer(_) | Slot::Macro | Slot::Environment
        )
    }
}

/// Where a stage samples: a UV set and a 2x3 affine transform of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvTransform {
    /// 0 for the model's first UV set, 1 for the second.
    pub uv_set: u32,
    /// `u' = rows[0] . (u, v, 1)`, `v' = rows[1] . (u, v, 1)`.
    pub rows: [[f32; 3]; 2],
}

impl UvTransform {
    /// UV set `uv_set`, untransformed.
    pub fn set(uv_set: u32) -> UvTransform {
        UvTransform {
            uv_set,
            rows: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        }
    }

    fn from_tex_gen(tex_gen: Option<&a3_p3d::TexGen>) -> UvTransform {
        let Some(g) = tex_gen else {
            return UvTransform::set(0);
        };
        // uvSource enum (executable): 0 None, 1 Tex, 2 TexWaterAnim, 3 Pos, 4 Norm, 5 Tex1,
        // 6 WorldPos, 7 WorldNorm, 8 TexShoreAnim, 9 TexCollimator, 10 TexCollimatorInv.
        // Only Tex1 samples the second UV set; the others fall back to the first.
        let uv_set = u32::from(g.uv_source == UV_SOURCE_TEX1);
        let [aside, up, _dir, pos] = g.transform;
        UvTransform {
            uv_set,
            rows: [[aside[0], up[0], pos[0]], [aside[1], up[1], pos[1]]],
        }
    }
}

/// The `uvSource` value of `Tex1`, the model's second UV set.
pub const UV_SOURCE_TEX1: u32 = 5;

/// One texture a material samples.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureRef {
    /// VFS path or procedural texture string.
    pub path: String,
    /// Where it samples.
    pub uv: UvTransform,
}

/// How a section's pixels combine with what is behind them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AlphaMode {
    /// Depth-written, alpha ignored.
    Opaque,
    /// Depth-written, pixels with alpha below one half discarded; drawn two-sided.
    Test,
    /// Blended back to front after all opaque geometry, without depth writes.
    Blend,
}

impl AlphaMode {
    /// The mode for a shader family and the `FLAG` of the section's colour map.
    pub fn decide(family: ShaderFamily, diffuse_alpha: Option<AlphaFlags>) -> AlphaMode {
        match family {
            ShaderFamily::Glass => AlphaMode::Blend,
            ShaderFamily::Tree | ShaderFamily::SuperAlphaTest => AlphaMode::Test,
            _ => match diffuse_alpha {
                Some(flags) if flags.is_interpolated() => AlphaMode::Blend,
                Some(flags) if flags.is_binary() => AlphaMode::Test,
                _ => AlphaMode::Opaque,
            },
        }
    }
}

/// Everything our shaders need to know about a section's material.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialDesc {
    /// The rvmat path (empty without material).
    pub name: String,
    /// The engine's pixel shader.
    pub pixel_shader: PixelShader,
    /// How we render it.
    pub family: ShaderFamily,
    /// Blending; [`MaterialDesc::new`] assumes an opaque colour map.
    pub alpha: AlphaMode,
    /// Material colours, linear RGBA.
    pub ambient: [f32; 4],
    pub diffuse: [f32; 4],
    pub emissive: [f32; 4],
    pub specular: [f32; 4],
    pub specular_power: f32,
    textures: [Option<TextureRef>; Slot::COUNT],
}

impl MaterialDesc {
    /// The material of a section with texture `section_texture` and embedded `material`.
    pub fn new(material: Option<&a3_p3d::Material>, section_texture: Option<&str>) -> Self {
        let section_texture = section_texture.filter(|t| !t.is_empty());
        let Some(mat) = material else {
            let mut desc = MaterialDesc::plain(PixelShader::from_id(0));
            if let Some(t) = section_texture {
                desc.set(Slot::Diffuse, t, UvTransform::set(0));
            }
            return desc;
        };
        let pixel_shader = PixelShader::from_id(mat.pixel_shader);
        let mut desc = MaterialDesc {
            name: mat.name.clone(),
            ambient: mat.ambient.to_array(),
            diffuse: mat.diffuse.to_array(),
            emissive: mat.emissive.to_array(),
            specular: mat.specular.to_array(),
            specular_power: mat.specular_power,
            ..MaterialDesc::plain(pixel_shader)
        };
        let uv_of = |stage: &a3_p3d::MaterialStage| {
            UvTransform::from_tex_gen(mat.tex_gens.get(stage.tex_gen as usize))
        };
        let stage0 = mat.stages.first();
        let diffuse_uv = stage0.map_or(UvTransform::set(0), uv_of);
        if let Some(t) = section_texture.or(stage0.map(|s| s.texture.as_str())) {
            desc.set(Slot::Diffuse, t, diffuse_uv);
        }
        match desc.family {
            ShaderFamily::Multi => {
                for (i, stage) in mat.stages.iter().enumerate() {
                    let slot = match i {
                        0..=3 => Slot::Layer(i as u8),
                        4 => Slot::Mask,
                        5..=8 => Slot::LayerSpecular((i - 5) as u8),
                        9 => Slot::Macro,
                        10 => Slot::AmbientShadow,
                        11..=14 => Slot::LayerNormal((i - 11) as u8),
                        _ => continue,
                    };
                    // Stage 8 samples with stage 3's tex gen.
                    let uv_stage = if i == 8 { &mat.stages[3] } else { stage };
                    desc.set(slot, &stage.texture, uv_of(uv_stage));
                }
            }
            _ if fixed_layout(pixel_shader).is_some() => {
                let layout = fixed_layout(pixel_shader).unwrap_or(&[]);
                for (i, stage) in mat.stages.iter().enumerate() {
                    if let Some(Some(slot)) = layout.get(i) {
                        desc.set(*slot, &stage.texture, uv_of(stage));
                    }
                }
            }
            _ => {
                for stage in mat.stages.iter().skip(1) {
                    let slot = match stage_kind(&stage.texture) {
                        k if k.is_normal_map() => Slot::Normal,
                        TextureKind::SpecularMetalDetail | TextureKind::Specular => Slot::Specular,
                        TextureKind::AmbientShadow => Slot::AmbientShadow,
                        TextureKind::Macro | TextureKind::MacroAlpha => Slot::Macro,
                        TextureKind::Detail | TextureKind::ColorDetail => Slot::Detail,
                        TextureKind::Sky => Slot::Environment,
                        _ if is_fresnel(&stage.texture) => Slot::Fresnel,
                        _ => continue,
                    };
                    if desc.texture(slot).is_none() {
                        desc.set(slot, &stage.texture, uv_of(stage));
                    }
                }
            }
        }
        desc
    }

    fn plain(pixel_shader: PixelShader) -> Self {
        let family = pixel_shader.family();
        MaterialDesc {
            name: String::new(),
            pixel_shader,
            family,
            alpha: AlphaMode::decide(family, None),
            ambient: [1.0; 4],
            diffuse: [1.0; 4],
            emissive: [0.0; 4],
            specular: [0.0; 4],
            specular_power: 0.0,
            textures: Default::default(),
        }
    }

    fn set(&mut self, slot: Slot, path: &str, uv: UvTransform) {
        if path.is_empty() {
            return;
        }
        self.textures[slot.binding()] = Some(TextureRef {
            path: path.to_owned(),
            uv,
        });
    }

    /// The texture bound to `slot`, if any.
    pub fn texture(&self, slot: Slot) -> Option<&TextureRef> {
        self.textures[slot.binding()].as_ref()
    }

    /// Every bound texture with its slot.
    pub fn textures(&self) -> impl Iterator<Item = (Slot, &TextureRef)> {
        self.textures
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.as_ref().map(|t| (Slot::at(i, self.family), t)))
    }
}

impl Eq for MaterialDesc {}

impl Hash for MaterialDesc {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.pixel_shader.hash(state);
        self.alpha.hash(state);
        let colours = [self.ambient, self.diffuse, self.emissive, self.specular];
        for v in colours.iter().flatten().chain([&self.specular_power]) {
            v.to_bits().hash(state);
        }
        for t in &self.textures {
            if let Some(t) = t {
                t.path.hash(state);
                t.uv.uv_set.hash(state);
                for v in t.uv.rows.iter().flatten() {
                    v.to_bits().hash(state);
                }
            } else {
                0u8.hash(state);
            }
        }
    }
}

/// The stage layout of shaders whose stages have fixed meanings (`render-materials.md` §4), by
/// stage index. Other shaders' stages are recognised by suffix.
fn fixed_layout(shader: PixelShader) -> Option<&'static [Option<Slot>]> {
    use Slot::*;
    const SUPER: &[Option<Slot>] = &[
        None,
        Some(Normal),
        Some(Detail),
        Some(Macro),
        Some(AmbientShadow),
        Some(Specular),
        Some(Fresnel),
        Some(Environment),
    ];
    // Stage 3 is a second colour map we do not use yet.
    const SKIN: &[Option<Slot>] = &[
        None,
        Some(Normal),
        Some(Macro),
        None,
        Some(AmbientShadow),
        Some(Specular),
        Some(Fresnel),
    ];
    const GLASS: &[Option<Slot>] = &[None, Some(Fresnel), Some(Environment)];
    match shader.name() {
        "Super" | "SuperExt" | "SuperAToC" | "SuperHair" | "SuperHairAtoC" => Some(SUPER),
        "Skin" => Some(SKIN),
        "Glass" => Some(GLASS),
        _ => None,
    }
}

/// Whether a stage texture is the engine's Fresnel table (`fresnel(n,k)`, `fresnelGlass(n)`).
fn is_fresnel(texture: &str) -> bool {
    Procedural::parse(texture).is_ok_and(|p| {
        matches!(
            p.function,
            ProceduralFunction::Fresnel { .. } | ProceduralFunction::FresnelGlass { .. }
        )
    })
}

/// The Texture kind of a stage texture; for procedural colours, the kind their tag names.
fn stage_kind(texture: &str) -> TextureKind {
    if !Procedural::is_procedural(texture) {
        return TextureKind::from_path(texture);
    }
    match Procedural::parse(texture).map(|p| p.function) {
        Ok(ProceduralFunction::Color { tag: Some(tag), .. }) => {
            TextureKind::from_path(&format!("procedural_{tag}"))
        }
        _ => TextureKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_p3d::{Material, MaterialStage, TexGen};

    fn stage(texture: &str, tex_gen: u32) -> MaterialStage {
        MaterialStage {
            texture: texture.to_owned(),
            tex_gen,
            ..MaterialStage::default()
        }
    }

    fn identity_gen(uv_source: u32) -> TexGen {
        TexGen {
            uv_source,
            transform: [
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
            ],
        }
    }

    /// The stage layout of most shipped `Super` materials.
    fn super_material() -> Material {
        let mut tiled = identity_gen(1);
        tiled.transform[0] = [10.0, 0.0, 0.0];
        tiled.transform[1] = [0.0, 10.0, 0.0];
        tiled.transform[3] = [0.5, 0.25, 0.0];
        Material {
            name: r"a3\structures_f\house\data\wall.rvmat".into(),
            pixel_shader: 102,
            ambient: glam::Vec4::new(1.0, 1.0, 1.0, 1.0),
            diffuse: glam::Vec4::new(0.8, 0.8, 0.8, 1.0),
            specular: glam::Vec4::new(0.2, 0.2, 0.2, 1.0),
            specular_power: 60.0,
            stages: vec![
                stage("", 0),
                stage(r"a3\structures_f\house\data\wall_nohq.paa", 1),
                stage("#(argb,8,8,3)color(0.5,0.5,0.5,1,DT)", 2),
                stage(r"a3\structures_f\house\data\wall_mc.paa", 0),
                stage(r"a3\structures_f\house\data\wall_as.paa", 3),
                stage(r"a3\structures_f\house\data\wall_smdi.paa", 1),
                stage("#(ai,64,64,1)fresnel(1.3,7)", 0),
                stage(r"a3\data_f\env_land_co.paa", 0),
            ],
            tex_gens: vec![
                identity_gen(1),
                identity_gen(1),
                tiled,
                identity_gen(UV_SOURCE_TEX1),
            ],
            ..Material::default()
        }
    }

    #[test]
    fn super_stages_are_assigned_by_texture_suffix() {
        let m = MaterialDesc::new(
            Some(&super_material()),
            Some(r"a3\structures_f\house\data\wall_co.paa"),
        );
        assert_eq!(m.family, ShaderFamily::Super);
        let path = |slot: Slot| m.texture(slot).map(|t| t.path.as_str());
        assert_eq!(
            path(Slot::Diffuse),
            Some(r"a3\structures_f\house\data\wall_co.paa")
        );
        assert_eq!(
            path(Slot::Normal),
            Some(r"a3\structures_f\house\data\wall_nohq.paa")
        );
        assert_eq!(
            path(Slot::Specular),
            Some(r"a3\structures_f\house\data\wall_smdi.paa")
        );
        assert_eq!(
            path(Slot::AmbientShadow),
            Some(r"a3\structures_f\house\data\wall_as.paa")
        );
        assert_eq!(
            path(Slot::Macro),
            Some(r"a3\structures_f\house\data\wall_mc.paa")
        );
        assert_eq!(
            path(Slot::Detail),
            Some("#(argb,8,8,3)color(0.5,0.5,0.5,1,DT)")
        );
        assert_eq!(path(Slot::Fresnel), Some("#(ai,64,64,1)fresnel(1.3,7)"));
        assert_eq!(path(Slot::Environment), Some(r"a3\data_f\env_land_co.paa"));
        assert_eq!(m.specular_power, 60.0);
        assert_eq!(m.diffuse, [0.8, 0.8, 0.8, 1.0]);
    }

    #[test]
    fn stage_uv_transforms_come_from_their_tex_gen() {
        let m = MaterialDesc::new(Some(&super_material()), Some("wall_co.paa"));
        let detail = m.texture(Slot::Detail).unwrap();
        assert_eq!(detail.uv.uv_set, 0);
        assert_eq!(detail.uv.rows, [[10.0, 0.0, 0.5], [0.0, 10.0, 0.25]]);
        let ambient_shadow = m.texture(Slot::AmbientShadow).unwrap();
        assert_eq!(ambient_shadow.uv.uv_set, 1);
        assert_eq!(ambient_shadow.uv, UvTransform::set(1));
    }

    #[test]
    fn sections_without_material_draw_their_texture_with_the_basic_shader() {
        let m = MaterialDesc::new(None, Some(r"a3\data_f\metal_co.paa"));
        assert_eq!(m.family, ShaderFamily::Basic);
        assert_eq!(
            m.texture(Slot::Diffuse).map(|t| t.path.as_str()),
            Some(r"a3\data_f\metal_co.paa")
        );
        assert!(m.texture(Slot::Normal).is_none());
        assert_eq!(m.diffuse, [1.0; 4]);
    }

    #[test]
    fn stage_zero_is_the_diffuse_map_when_the_section_has_no_texture() {
        let mut mat = super_material();
        mat.stages[0].texture = r"a3\plants_f\leaf_ca.paa".into();
        let m = MaterialDesc::new(Some(&mat), None);
        assert_eq!(
            m.texture(Slot::Diffuse).map(|t| t.path.as_str()),
            Some(r"a3\plants_f\leaf_ca.paa")
        );
    }

    #[test]
    fn multi_material_layers_follow_the_fixed_stage_layout() {
        let paths = [
            "l0_co",
            "l1_co",
            "l2_co",
            "l3_co",
            "mask",
            "d0_dtsmdi",
            "d1_dtsmdi",
            "d2_dtsmdi",
            "d3_dtsmdi",
            "m_mc",
            "m_as",
            "n0_nohq",
            "n1_nohq",
            "n2_nohq",
            "n3_nohq",
        ];
        let mat = Material {
            pixel_shader: 103,
            stages: paths
                .iter()
                .map(|p| stage(&format!("{p}.paa"), 0))
                .collect(),
            tex_gens: vec![identity_gen(1)],
            ..Material::default()
        };
        let m = MaterialDesc::new(Some(&mat), None);
        assert_eq!(m.family, ShaderFamily::Multi);
        let path = |slot: Slot| m.texture(slot).map(|t| t.path.clone());
        assert_eq!(path(Slot::Layer(2)).as_deref(), Some("l2_co.paa"));
        assert_eq!(path(Slot::LayerNormal(3)).as_deref(), Some("n3_nohq.paa"));
        assert_eq!(path(Slot::Mask).as_deref(), Some("mask.paa"));
        assert_eq!(path(Slot::Macro).as_deref(), Some("m_mc.paa"));
        assert_eq!(path(Slot::AmbientShadow).as_deref(), Some("m_as.paa"));
        assert_eq!(
            path(Slot::LayerSpecular(3)).as_deref(),
            Some("d3_dtsmdi.paa")
        );
    }

    #[test]
    fn normal_map_shaders_are_assigned_by_suffix_not_super_positions() {
        // NormalMapSpecularDIMap: stage 2 is the smdi map, not a detail map.
        let mat = Material {
            pixel_shader: 22,
            stages: vec![
                stage("", 0),
                stage("#(argb,8,8,3)color(0.5,0.5,1,1,nohq)", 0),
                stage("#(argb,8,8,3)color(1,0.005,1,1,smdi)", 0),
            ],
            tex_gens: vec![identity_gen(1)],
            ..Material::default()
        };
        let m = MaterialDesc::new(Some(&mat), Some("house_co.paa"));
        assert_eq!(m.family, ShaderFamily::Super);
        assert!(m.texture(Slot::Detail).is_none());
        assert_eq!(
            m.texture(Slot::Specular).unwrap().path,
            "#(argb,8,8,3)color(1,0.005,1,1,smdi)"
        );
    }

    #[test]
    fn skin_stages_follow_its_own_layout() {
        let paths = [
            "",
            "s_nohq",
            "s_mc",
            "s2_co",
            "s_as",
            "s_smdi",
            "#(ai,64,64,1)fresnel(1.3,7)",
        ];
        let mat = Material {
            pixel_shader: 110,
            stages: paths.iter().map(|p| stage(p, 0)).collect(),
            tex_gens: vec![identity_gen(1)],
            ..Material::default()
        };
        let m = MaterialDesc::new(Some(&mat), Some("s_co"));
        let path = |slot: Slot| m.texture(slot).map(|t| t.path.as_str());
        assert_eq!(path(Slot::Macro), Some("s_mc"));
        assert_eq!(path(Slot::AmbientShadow), Some("s_as"));
        assert_eq!(path(Slot::Specular), Some("s_smdi"));
        assert_eq!(path(Slot::Fresnel), Some("#(ai,64,64,1)fresnel(1.3,7)"));
        assert_eq!(path(Slot::Diffuse), Some("s_co"));
    }

    #[test]
    fn multi_stage_8_samples_with_stage_3s_tex_gen() {
        let mut mat = Material {
            pixel_shader: 103,
            stages: (0..15).map(|i| stage(&format!("s{i}_co.paa"), 0)).collect(),
            tex_gens: vec![identity_gen(1), identity_gen(UV_SOURCE_TEX1)],
            ..Material::default()
        };
        mat.stages[3].tex_gen = 1;
        let m = MaterialDesc::new(Some(&mat), None);
        assert_eq!(m.texture(Slot::LayerSpecular(3)).unwrap().uv.uv_set, 1);
        assert_eq!(m.texture(Slot::LayerSpecular(2)).unwrap().uv.uv_set, 0);
    }

    #[test]
    fn slots_a_family_uses_have_distinct_bindings() {
        for family in [ShaderFamily::Super, ShaderFamily::Multi, ShaderFamily::Tree] {
            let mut seen = std::collections::HashSet::new();
            for binding in 0..Slot::COUNT {
                let slot = Slot::at(binding, family);
                assert_eq!(slot.binding(), binding, "{slot:?} in {family:?}");
                assert!(seen.insert(slot), "{slot:?} bound twice");
            }
        }
    }

    #[test]
    fn tree_mca_is_the_crown_map_in_the_macro_slot() {
        let mat = Material {
            pixel_shader: 114,
            stages: vec![
                stage("", 0),
                stage("leaf_nohq.paa", 0),
                stage("leaf_mca.paa", 0),
            ],
            tex_gens: vec![identity_gen(1)],
            ..Material::default()
        };
        let m = MaterialDesc::new(Some(&mat), Some("leaf_ca.paa"));
        assert_eq!(m.family, ShaderFamily::Tree);
        assert!(m.texture(Slot::Normal).is_some());
        assert_eq!(m.texture(Slot::Macro).unwrap().path, "leaf_mca.paa");
    }

    #[test]
    fn alpha_mode_follows_shader_family_and_texture_alpha() {
        use a3_paa::AlphaFlags;
        let blend = Some(AlphaFlags(AlphaFlags::INTERPOLATED));
        let binary = Some(AlphaFlags(AlphaFlags::BINARY));
        assert_eq!(
            AlphaMode::decide(ShaderFamily::Glass, None),
            AlphaMode::Blend
        );
        assert_eq!(
            AlphaMode::decide(ShaderFamily::Tree, blend),
            AlphaMode::Test
        );
        assert_eq!(AlphaMode::decide(ShaderFamily::Tree, None), AlphaMode::Test);
        assert_eq!(
            AlphaMode::decide(ShaderFamily::Super, blend),
            AlphaMode::Blend
        );
        assert_eq!(
            AlphaMode::decide(ShaderFamily::Super, binary),
            AlphaMode::Test
        );
        assert_eq!(
            AlphaMode::decide(ShaderFamily::Super, None),
            AlphaMode::Opaque
        );
        assert_eq!(
            AlphaMode::decide(ShaderFamily::SuperAlphaTest, None),
            AlphaMode::Test
        );
    }
}
