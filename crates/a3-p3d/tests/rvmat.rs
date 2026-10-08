//! `.rvmat` material files.

use a3_p3d::{Error, RvMat};
use glam::Vec4;

const TEXT: &str = r##"
ambient[] = {1, 1, 1, 1};
diffuse[] = {0.5, 0.5, 0.5, 1};
forcedDiffuse[] = {0, 0, 0, 0};
emmisive[] = {0, 0, 0, 1};
specular[] = {0.3, 0.3, 0.3, 1};
specularPower = 70;
PixelShaderID = "Super";
VertexShaderID = "Super";
surfaceInfo = "a3\data_f\metal.bisurf";
class Stage2
{
    texture = "#(argb,8,8,3)color(0.5,0.5,0.5,1,DT)";
    uvSource = "tex";
    class uvTransform
    {
        aside[] = {0, 9, 0};
        up[] = {4.5, 0, 0};
        dir[] = {0, 0, 0};
        pos[] = {0, 0, 0};
    };
};
class Stage1
{
    texture = "a3\weapons_f\data\gun_nohq.paa";
    uvSource = "tex";
};
class StageTI
{
    texture = "a3\data_f\default_ti_ca.paa";
};
"##;

fn check(mat: &RvMat) {
    assert_eq!(mat.pixel_shader, "Super");
    assert_eq!(mat.vertex_shader, "Super");
    assert_eq!(mat.ambient, Vec4::ONE);
    assert_eq!(mat.diffuse, Vec4::new(0.5, 0.5, 0.5, 1.0));
    assert_eq!(mat.emissive, Vec4::new(0.0, 0.0, 0.0, 1.0));
    assert_eq!(mat.specular_power, 70.0);
    assert_eq!(mat.surface, r"a3\data_f\metal.bisurf");
    let stages: Vec<_> = mat
        .stages
        .iter()
        .map(|s| (s.index, s.texture.as_str()))
        .collect();
    assert_eq!(
        stages,
        [
            (1, r"a3\weapons_f\data\gun_nohq.paa"),
            (2, "#(argb,8,8,3)color(0.5,0.5,0.5,1,DT)")
        ],
        "stages in index order"
    );
    assert_eq!(mat.stages[1].uv_source, "tex");
    assert_eq!(
        mat.stages[1].uv_transform,
        Some([[0.0, 9.0, 0.0], [4.5, 0.0, 0.0], [0.0; 3], [0.0; 3]])
    );
    assert_eq!(mat.stages[0].uv_transform, None);
    assert_eq!(
        mat.ti_stage.as_ref().unwrap().texture,
        r"a3\data_f\default_ti_ca.paa"
    );
}

#[test]
fn reads_a_text_rvmat() {
    check(&RvMat::from_bytes(TEXT.as_bytes()).unwrap());
}

#[test]
fn reads_a_rapified_rvmat() {
    let config = a3_config::parse_text(TEXT).unwrap();
    let bytes = a3_config::write_rap(&config);
    check(&RvMat::from_bytes(&bytes).unwrap());
}

#[test]
fn reports_syntax_errors() {
    let err = RvMat::from_bytes(b"class Stage1 {").unwrap_err();
    assert!(matches!(err, Error::Rvmat(_)), "{err}");
}
