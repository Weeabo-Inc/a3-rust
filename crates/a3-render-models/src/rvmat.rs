//! Loose rvmat files ([`a3_p3d::RvMat`]) in the shape ODOL embeds ([`a3_p3d::Material`]), so
//! MLOD models and materials referenced by path render like binarised ones.

use a3_p3d::{Material, MaterialStage, RvMat, TexGen};

use crate::shader::PixelShader;

/// The embedded-material form of an rvmat named `name`. Stage `N` keeps index `N` (missing
/// stages, usually stage 0, stay empty) and gets its own tex gen from `uvSource` and
/// `uvTransform`.
pub fn material_from_rvmat(rvmat: &RvMat, name: &str) -> Material {
    let mut material = Material {
        name: name.to_owned(),
        version: 11,
        ambient: rvmat.ambient,
        diffuse: rvmat.diffuse,
        forced_diffuse: rvmat.forced_diffuse,
        emissive: rvmat.emissive,
        specular: rvmat.specular,
        specular_power: rvmat.specular_power,
        pixel_shader: PixelShader::from_name(&rvmat.pixel_shader).map_or(0, |p| p.0),
        surface: rvmat.surface.clone(),
        ..Material::default()
    };
    let count = rvmat.stages.iter().map(|s| s.index + 1).max().unwrap_or(0);
    material.stages = vec![MaterialStage::default(); count as usize];
    for stage in &rvmat.stages {
        // The engine's uvSource enum, in value order (read from the executable).
        const UV_SOURCES: [&str; 11] = [
            "none",
            "tex",
            "texwateranim",
            "pos",
            "norm",
            "tex1",
            "worldpos",
            "worldnorm",
            "texshoreanim",
            "texcollimator",
            "texcollimatorinv",
        ];
        let uv_source = UV_SOURCES
            .iter()
            .position(|s| s.eq_ignore_ascii_case(stage.uv_source.trim()))
            .map_or(1, |i| i as u32);
        let transform = stage.uv_transform.unwrap_or([
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0; 3],
        ]);
        material.tex_gens.push(TexGen {
            uv_source,
            transform,
        });
        material.stages[stage.index as usize] = MaterialStage {
            texture: stage.texture.clone(),
            tex_gen: material.tex_gens.len() as u32 - 1,
            ..MaterialStage::default()
        };
    }
    material
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUPER: &str = r##"
        ambient[]={1,1,1,1};
        diffuse[]={0.5,0.5,0.5,1};
        emmisive[]={0,0,0,1};
        specular[]={0.2,0.2,0.2,1};
        specularPower=60;
        PixelShaderID="Super";
        VertexShaderID="Super";
        class Stage1
        {
            texture="a3\data_f\wall_nohq.paa";
            uvSource="tex";
            class uvTransform
            {
                aside[]={1,0,0};
                up[]={0,1,0};
                dir[]={0,0,0};
                pos[]={0,0,0};
            };
        };
        class Stage2
        {
            texture="#(argb,8,8,3)color(0.5,0.5,0.5,1,DT)";
            uvSource="tex";
            class uvTransform
            {
                aside[]={10,0,0};
                up[]={0,10,0};
                dir[]={0,0,0};
                pos[]={0,0,0};
            };
        };
        class Stage4
        {
            texture="a3\data_f\wall_as.paa";
            uvSource="tex1";
        };
    "##;

    #[test]
    fn super_rvmat_becomes_an_embedded_style_material() {
        let rvmat = RvMat::from_bytes(SUPER.as_bytes()).unwrap();
        let m = material_from_rvmat(&rvmat, r"a3\data_f\wall.rvmat");
        assert_eq!(m.name, r"a3\data_f\wall.rvmat");
        assert_eq!(m.pixel_shader, 102);
        assert_eq!(m.diffuse, glam::Vec4::new(0.5, 0.5, 0.5, 1.0));
        assert_eq!(m.specular_power, 60.0);
        assert_eq!(m.stages.len(), 5);
        assert_eq!(m.stages[0].texture, "");
        assert_eq!(m.stages[1].texture, r"a3\data_f\wall_nohq.paa");
        let detail = &m.tex_gens[m.stages[2].tex_gen as usize];
        assert_eq!(detail.uv_source, 1);
        assert_eq!(detail.transform[0], [10.0, 0.0, 0.0]);
        let ambient_shadow = &m.tex_gens[m.stages[4].tex_gen as usize];
        assert_eq!(ambient_shadow.uv_source, crate::material::UV_SOURCE_TEX1);
        assert_eq!(ambient_shadow.transform[1], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn the_material_maps_to_super_slots() {
        let rvmat = RvMat::from_bytes(SUPER.as_bytes()).unwrap();
        let m = material_from_rvmat(&rvmat, "wall.rvmat");
        let desc = crate::MaterialDesc::new(Some(&m), Some("wall_co.paa"));
        assert_eq!(desc.family, crate::ShaderFamily::Super);
        let detail = desc.texture(crate::Slot::Detail).unwrap();
        assert_eq!(detail.uv.rows[0], [10.0, 0.0, 0.0]);
        assert_eq!(
            desc.texture(crate::Slot::AmbientShadow).unwrap().uv.uv_set,
            1
        );
    }
}
