//! Texture suffixes: what a texture is for, from its file name.
//!
//! TexConvert (the Arma 3 Tools texture converter) picks pixel format, swizzle, mipmap filter
//! and gamma handling from the suffix after the last `_` of the file stem; the engine relies on
//! that layout (a `_nohq` must be a swizzled normal map, a `_smdi` has its specular in green).
//! Binarize records the resulting [`TextureType`] in `texHeaders.bin`. Notes on each kind are
//! in `docs/re/paa.md`; entries marked _(uncertain)_ are inferred from shipped data, not from
//! the executable.

/// The engine's texture type enum, stored per texture in `texHeaders.bin`
/// ([`crate::TexHeader::texture_type`]). Values 4, 5, 6 and 10 belong to procedural
/// generators and never appear in shipped headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureType {
    /// 0: colour map, sampled with sRGB decoding _(uncertain: gamma handling)_.
    Diffuse,
    /// 1: colour map kept linear (sky textures, `sky_*_lco`).
    DiffuseLinear,
    /// 2: tiled detail map (`_dt`, `_cdt`, `_mco`).
    Detail,
    /// 3: normal map.
    Normal,
    /// 4: specular irradiance lookup (procedural `irradiance`).
    Irradiance,
    /// 5: random test pattern (procedural).
    RandomTest,
    /// 6: tree crown lighting lookup (procedural `treeCrown`).
    TreeCrown,
    /// 7: macro map (`_mc`).
    Macro,
    /// 8: ambient shadow (`_as`).
    AmbientShadow,
    /// 9: specular map (`_smdi`, `_sm`).
    Specular,
    /// 10: dither pattern (procedural).
    Dither,
    /// 11: detail specular map (`_dtsmdi`).
    DetailSpecular,
    /// 12: terrain surface mask (`_mask`).
    Mask,
    /// 13: thermal imaging texture.
    Thermal,
}

impl TextureType {
    const ALL: [TextureType; 14] = [
        Self::Diffuse,
        Self::DiffuseLinear,
        Self::Detail,
        Self::Normal,
        Self::Irradiance,
        Self::RandomTest,
        Self::TreeCrown,
        Self::Macro,
        Self::AmbientShadow,
        Self::Specular,
        Self::Dither,
        Self::DetailSpecular,
        Self::Mask,
        Self::Thermal,
    ];

    /// The type for a stored enum value.
    pub fn from_index(index: u32) -> Option<Self> {
        Self::ALL.get(index as usize).copied()
    }

    /// The stored enum value.
    pub fn index(self) -> u32 {
        Self::ALL
            .iter()
            .position(|&t| t == self)
            .expect("every variant is listed") as u32
    }
}

/// The role of a texture, decided by its file-name suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureKind {
    /// `_co`: colour (diffuse) map, opaque.
    Color,
    /// `_ca`: colour map with alpha (alpha-tested or blended).
    ColorAlpha,
    /// `_cdt`: colour detail map, tiled over a surface.
    ColorDetail,
    /// `_lco`: terrain satellite segment (`layers\s_*_lco`); sky `sky_*_lco` files are
    /// recorded as linear diffuse instead.
    LayerColor,
    /// `_lca`: terrain surface-mask segment (`layers\m_*_lca`).
    LayerColorAlpha,
    /// `_mc`: macro map, a large-scale colour overlay with alpha as blend factor.
    Macro,
    /// `_mca`: macro map with alpha, recorded as a plain diffuse map.
    MacroAlpha,
    /// `_mco`: terrain middle-distance detail colour (`l_middle_mco`), recorded as detail.
    MacroColor,
    /// `_dt` (older `_detail`): detail map, a tiled overlay.
    Detail,
    /// `_no` (older `_ns`): normal map, normal quality (DXT1).
    Normal,
    /// `_nohq`: normal map, high quality: DXT5 with X moved to alpha (swizzle `05 04 02 03`).
    NormalHq,
    /// `_nof`: normal map whose mipmaps fade to flat.
    NormalFaded,
    /// `_nofhq`: high-quality faded normal map.
    NormalFadedHq,
    /// `_non`: normal map variant _(uncertain)_.
    NormalNoFade,
    /// `_nopx`: normal map with a parallax height map in alpha.
    NormalParallax,
    /// `_smdi`: specular map: green = specular intensity, blue = specular power (gloss),
    /// red unused (white).
    SpecularMetalDetail,
    /// `_dtsmdi`: detail `_smdi` for tiled surfaces.
    DetailSpecular,
    /// `_sm`: older specular map.
    Specular,
    /// `_as`: ambient shadow (baked ambient occlusion in green; swizzle `08 08 02 08`).
    AmbientShadow,
    /// `_ads`: terrain/multimaterial map, recorded as diffuse _(uncertain: channel use)_.
    AmbientDiffuseSpecular,
    /// `_adshq`: high-quality `_ads` _(uncertain)_.
    AmbientDiffuseSpecularHq,
    /// `_mask`: terrain surface mask (each colour selects a surface layer).
    Mask,
    /// `_sky`: sky dome texture, swizzled (green moved to alpha, inverted).
    Sky,
    /// `_ti_ca`: thermal imaging texture (`_ti_co` and `_ti` are recorded as plain diffuse).
    Thermal,
    /// `_gs`: grey-scale image with alpha (AI88), mostly UI icons.
    GreyScale,
    /// No known suffix.
    Unknown,
}

const SUFFIXES: [(&str, TextureKind); 26] = [
    ("co", TextureKind::Color),
    ("ca", TextureKind::ColorAlpha),
    ("cdt", TextureKind::ColorDetail),
    ("lco", TextureKind::LayerColor),
    ("lca", TextureKind::LayerColorAlpha),
    ("mc", TextureKind::Macro),
    ("mca", TextureKind::MacroAlpha),
    ("mco", TextureKind::MacroColor),
    ("dt", TextureKind::Detail),
    ("no", TextureKind::Normal),
    ("nohq", TextureKind::NormalHq),
    ("nof", TextureKind::NormalFaded),
    ("nofhq", TextureKind::NormalFadedHq),
    ("non", TextureKind::NormalNoFade),
    ("nopx", TextureKind::NormalParallax),
    ("smdi", TextureKind::SpecularMetalDetail),
    ("dtsmdi", TextureKind::DetailSpecular),
    ("sm", TextureKind::Specular),
    ("as", TextureKind::AmbientShadow),
    ("ads", TextureKind::AmbientDiffuseSpecular),
    ("adshq", TextureKind::AmbientDiffuseSpecularHq),
    ("mask", TextureKind::Mask),
    ("sky", TextureKind::Sky),
    ("gs", TextureKind::GreyScale),
    // Older spellings, listed after the canonical suffix of the same kind.
    ("detail", TextureKind::Detail),
    ("ns", TextureKind::Normal),
];

impl TextureKind {
    /// The kind for a texture path (OS or VFS, any case, any separator).
    pub fn from_path(path: &str) -> Self {
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
        let stem = stem.to_ascii_lowercase();
        // Thermal textures are `<name>_ti_ca`; `_ti_co` and bare `_ti` are plain diffuse maps.
        if stem.ends_with("_ti_ca") {
            return Self::Thermal;
        }
        let Some((_, suffix)) = stem.rsplit_once('_') else {
            return Self::Unknown;
        };
        SUFFIXES
            .iter()
            .find(|(s, _)| *s == suffix)
            .map_or(Self::Unknown, |&(_, kind)| kind)
    }

    /// The file-name suffix (without `_`), `None` for [`TextureKind::Unknown`].
    pub fn suffix(self) -> Option<&'static str> {
        SUFFIXES.iter().find(|(_, k)| *k == self).map(|&(s, _)| s)
    }

    /// The texture type Binarize records for this kind (the most common one where shipped
    /// data varies, see `docs/re/paa.md`).
    pub fn texture_type(self) -> TextureType {
        match self {
            Self::Sky => TextureType::DiffuseLinear,
            Self::Detail | Self::ColorDetail | Self::MacroColor => TextureType::Detail,
            Self::Normal
            | Self::NormalHq
            | Self::NormalFaded
            | Self::NormalFadedHq
            | Self::NormalNoFade
            | Self::NormalParallax => TextureType::Normal,
            Self::Macro => TextureType::Macro,
            Self::AmbientShadow => TextureType::AmbientShadow,
            Self::SpecularMetalDetail | Self::Specular => TextureType::Specular,
            Self::DetailSpecular => TextureType::DetailSpecular,
            Self::Mask => TextureType::Mask,
            Self::Thermal => TextureType::Thermal,
            Self::Color
            | Self::ColorAlpha
            | Self::LayerColor
            | Self::LayerColorAlpha
            | Self::MacroAlpha
            | Self::AmbientDiffuseSpecular
            | Self::AmbientDiffuseSpecularHq
            | Self::GreyScale
            | Self::Unknown => TextureType::Diffuse,
        }
    }

    /// `true` for the normal map kinds.
    pub fn is_normal_map(self) -> bool {
        self.texture_type() == TextureType::Normal
    }

    /// `true` for colour images meant to be seen.
    pub fn is_color(self) -> bool {
        matches!(
            self,
            Self::Color
                | Self::ColorAlpha
                | Self::ColorDetail
                | Self::LayerColor
                | Self::LayerColorAlpha
                | Self::Macro
                | Self::MacroAlpha
                | Self::MacroColor
                | Self::Sky
                | Self::GreyScale
        )
    }
}
