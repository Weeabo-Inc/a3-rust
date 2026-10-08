//! The engine's pixel shader IDs and the shader families we render them with.
//!
//! ODOL models embed each material's `PixelShaderID` as a number; rvmat files name it. The
//! numbering is the engine's enum, read from the executable (see `docs/re/render-materials.md` §1).

/// Names of the engine's pixel shader IDs, indexed by ID (game build 2.22).
const PIXEL_SHADER_NAMES: [&str; 153] = [
    "Normal",
    "NormalDXTA",
    "NormalMap",
    "NormalMapThrough",
    "NormalMapGrass",
    "NormalMapDiffuse",
    "Detail",
    "Interpolation",
    "Water",
    "WaterSimple",
    "White",
    "WhiteAlpha",
    "AlphaShadow",
    "AlphaNoShadow",
    "Dummy0",
    "DetailMacroAS",
    "NormalMapMacroAS",
    "NormalMapDiffuseMacroAS",
    "NormalMapSpecularMap",
    "NormalMapDetailSpecularMap",
    "NormalMapMacroASSpecularMap",
    "NormalMapDetailMacroASSpecularMap",
    "NormalMapSpecularDIMap",
    "NormalMapDetailSpecularDIMap",
    "NormalMapMacroASSpecularDIMap",
    "NormalMapDetailMacroASSpecularDIMap",
    "Terrain1",
    "Terrain2",
    "Terrain3",
    "Terrain4",
    "Terrain5",
    "Terrain6",
    "Terrain7",
    "Terrain8",
    "Terrain9",
    "Terrain10",
    "Terrain11",
    "Terrain12",
    "Terrain13",
    "Terrain14",
    "Terrain15",
    "TerrainSimple1",
    "TerrainSimple2",
    "TerrainSimple3",
    "TerrainSimple4",
    "TerrainSimple5",
    "TerrainSimple6",
    "TerrainSimple7",
    "TerrainSimple8",
    "TerrainSimple9",
    "TerrainSimple10",
    "TerrainSimple11",
    "TerrainSimple12",
    "TerrainSimple13",
    "TerrainSimple14",
    "TerrainSimple15",
    "Glass",
    "NonTL",
    "NormalMapSpecularThrough",
    "Grass",
    "NormalMapThroughSimple",
    "NormalMapSpecularThroughSimple",
    "Road",
    "Shore",
    "ShoreWet",
    "Road2Pass",
    "ShoreFoam",
    "NonTLFlare",
    "NormalMapThroughLowEnd",
    "TerrainGrass1",
    "TerrainGrass2",
    "TerrainGrass3",
    "TerrainGrass4",
    "TerrainGrass5",
    "TerrainGrass6",
    "TerrainGrass7",
    "TerrainGrass8",
    "TerrainGrass9",
    "TerrainGrass10",
    "TerrainGrass11",
    "TerrainGrass12",
    "TerrainGrass13",
    "TerrainGrass14",
    "TerrainGrass15",
    "Crater1",
    "Crater2",
    "Crater3",
    "Crater4",
    "Crater5",
    "Crater6",
    "Crater7",
    "Crater8",
    "Crater9",
    "Crater10",
    "Crater11",
    "Crater12",
    "Crater13",
    "Crater14",
    "Sprite",
    "SpriteSimple",
    "Cloud",
    "Horizon",
    "Super",
    "Multi",
    "TerrainX",
    "TerrainSimpleX",
    "TerrainGrassX",
    "Tree",
    "TreePRT",
    "TreeSimple",
    "Skin",
    "CalmWater",
    "TreeAToC",
    "GrassAToC",
    "TreeAdv",
    "TreeAdvSimple",
    "TreeAdvTrunk",
    "TreeAdvTrunkSimple",
    "TreeAdvAToC",
    "TreeAdvSimpleAToC",
    "TreeSN",
    "SpriteExtTi",
    "TerrainSNX",
    "InterpolationAlpha",
    "VolCloud",
    "VolCloudSimple",
    "UnderwaterOcclusion",
    "SimulWeatherClouds",
    "SimulWeatherCloudsWithLightning",
    "SimulWeatherCloudsCPU",
    "SimulWeatherCloudsWithLightningCPU",
    "SuperExt",
    "SuperHair",
    "SuperHairAtoC",
    "Caustics",
    "Refract",
    "SpriteRefract",
    "SpriteRefractSimple",
    "SuperAToC",
    "NonTLFlareNew",
    "NonTLFlareLight",
    "TerrainNoDetailX",
    "TerrainNoDetailSNX",
    "TerrainSimpleSNX",
    "NormalPiP",
    "NonTLFlareNewNoOcclusion",
    "Empty",
    "Point",
    "TreeAdvTrans",
    "TreeAdvTransAToC",
    "Collimator",
    "LODDiag",
    "DepthOnly",
];

/// A pixel shader ID of the engine (`PixelShaderID` in an rvmat).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelShader(pub u32);

impl PixelShader {
    /// The shader with the engine's numeric ID (as embedded in ODOL materials).
    pub fn from_id(id: u32) -> PixelShader {
        PixelShader(id)
    }

    /// The shader named `name` (as written in rvmat files), ignoring case.
    pub fn from_name(name: &str) -> Option<PixelShader> {
        PIXEL_SHADER_NAMES
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .map(|i| PixelShader(i as u32))
    }

    /// The engine's name of the shader, `"Unknown"` for IDs outside the enum.
    pub fn name(self) -> &'static str {
        PIXEL_SHADER_NAMES
            .get(self.0 as usize)
            .copied()
            .unwrap_or("Unknown")
    }

    /// How we render materials with this shader.
    pub fn family(self) -> ShaderFamily {
        let name = self.name();
        match name {
            "Super" | "SuperExt" | "SuperHair" | "Skin" | "NormalPiP" => ShaderFamily::Super,
            "SuperAToC" | "SuperHairAtoC" => ShaderFamily::SuperAlphaTest,
            "Multi" => ShaderFamily::Multi,
            "Glass" | "Refract" => ShaderFamily::Glass,
            "Normal" | "NormalDXTA" | "Detail" | "Interpolation" | "InterpolationAlpha"
            | "White" | "WhiteAlpha" | "AlphaShadow" | "AlphaNoShadow" | "DetailMacroAS" => {
                ShaderFamily::Basic
            }
            "Grass" | "GrassAToC" | "NormalMapGrass" => ShaderFamily::Tree,
            _ if name.starts_with("Tree") => ShaderFamily::Tree,
            _ if name.starts_with("NormalMap") => ShaderFamily::Super,
            _ => ShaderFamily::Unsupported,
        }
    }
}

/// The shader families our model renderer implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShaderFamily {
    /// Diffuse texture only, lit per vertex normal (`Normal`, `Detail`, `White`...).
    Basic,
    /// Normal, specular (`_smdi`/`_sm`), ambient-shadow, macro and detail maps: `Super` and the
    /// `NormalMap*` shaders.
    Super,
    /// `Super` with alpha-to-coverage in the engine; alpha-tested here.
    SuperAlphaTest,
    /// Up to four masked layers (`Multi`), used by rocks and large terrain objects.
    Multi,
    /// Vegetation (`Tree*`, `Grass`): alpha-tested, two-sided.
    Tree,
    /// Transparent, alpha-blended (`Glass`, `Refract`).
    Glass,
    /// Water, terrain, sky, clouds, particles...: not drawn by the model renderer.
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_the_executables_enum() {
        assert_eq!(PixelShader::from_id(0).name(), "Normal");
        assert_eq!(PixelShader::from_id(102).name(), "Super");
        assert_eq!(PixelShader::from_id(103).name(), "Multi");
        assert_eq!(PixelShader::from_id(114).name(), "TreeAdv");
        assert_eq!(PixelShader::from_id(56).name(), "Glass");
        assert_eq!(PixelShader::from_id(9999).name(), "Unknown");
    }

    #[test]
    fn names_parse_case_insensitively() {
        assert_eq!(
            PixelShader::from_name("super"),
            Some(PixelShader::from_id(102))
        );
        assert_eq!(
            PixelShader::from_name("SuperAtoc"),
            Some(PixelShader::from_id(138))
        );
        assert_eq!(PixelShader::from_name("NoSuchShader"), None);
    }

    #[test]
    fn families_group_the_shaders() {
        let family = |id| PixelShader::from_id(id).family();
        assert_eq!(family(102), ShaderFamily::Super);
        assert_eq!(family(19), ShaderFamily::Super);
        assert_eq!(family(0), ShaderFamily::Basic);
        assert_eq!(family(103), ShaderFamily::Multi);
        assert_eq!(family(114), ShaderFamily::Tree);
        assert_eq!(family(116), ShaderFamily::Tree);
        assert_eq!(family(56), ShaderFamily::Glass);
        assert_eq!(family(111), ShaderFamily::Unsupported);
    }
}
