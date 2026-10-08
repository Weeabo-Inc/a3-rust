//! Terrain layer materials (the per-cell `p_XXX-YYY_*.rvmat`) parsed from config text.

use a3_config::parse_text;
use a3_landscape::{LayerMaterial, UvSource};
use glam::Vec3;

/// A trimmed copy of a Tanoa layer rvmat: satellite, mask, constant colour, one surface layer in
/// slot 2 and the satellite normal map.
const TANOA_CELL: &str = r##"
PixelShaderID = "TerrainSNX";
VertexShaderID = "Terrain";
class Stage0 { texture = "a3\map_tanoabuka\data\layers\00_00\s_012_028_lco.paa"; texGen = 3; };
class Stage1 { texture = "a3\map_tanoabuka\data\layers\00_00\m_012_028_lca.paa"; texGen = 4; };
class TexGen3 {
    uvSource = "worldPos";
    class uvTransform {
        aside[] = {0.001953125, 0, 0};
        up[] = {0, 0, 0.001953125};
        dir[] = {0, -0.001953125, 0};
        pos[] = {-11.21875, 3.78125, 0};
    };
};
class TexGen4 {
    uvSource = "worldPos";
    class uvTransform {
        aside[] = {0.001953125, 0, 0};
        up[] = {0, 0, 0.001953125};
        dir[] = {0, -0.001953125, 0};
        pos[] = {-11.21875, 3.78125, 0};
    };
};
class TexGen0 { uvSource = "tex"; class uvTransform { aside[] = {1,0,0}; up[] = {0,1,0}; dir[] = {0,0,1}; pos[] = {0,0,0}; }; };
class TexGen1 { uvSource = "tex"; class uvTransform { aside[] = {5,0,0}; up[] = {0,5,0}; dir[] = {0,0,0}; pos[] = {0,0,0}; }; };
class TexGen2 { uvSource = "tex"; class uvTransform { aside[] = {5,0,0}; up[] = {0,5,0}; dir[] = {0,0,0}; pos[] = {0,0,0}; }; };
class Stage2 { texture = "#(rgb,1,1,1)color(0.5,0.5,0.5,1,cdt)"; texGen = 0; };
class Stage3 { texture = ""; texGen = 1; };
class Stage4 { texture = ""; texGen = 2; };
class Stage5 { texture = ""; texGen = 1; };
class Stage6 { texture = ""; texGen = 2; };
class Stage7 { texture = "a3\map_data_exp\gdt_seabedexp_nopx.paa"; texGen = 1; };
class Stage8 { texture = "a3\map_data_exp\gdt_seabedexp_co.paa"; texGen = 2; };
class Stage14 { texture = "a3\map_tanoabuka\data\layers\00_00\n_012_028_nohq.paa"; texGen = 3; };
"##;

fn tanoa_cell() -> LayerMaterial {
    let config = parse_text(TANOA_CELL).unwrap();
    LayerMaterial::from_config(
        "a3\\map_tanoabuka\\data\\layers\\p_012-028_n_n_l05_n_n.rvmat",
        &config,
    )
    .unwrap()
}

#[test]
fn satellite_mask_and_normal_tiles_come_from_their_stages() {
    let m = tanoa_cell();
    assert_eq!(m.pixel_shader, "TerrainSNX");
    assert_eq!(
        m.satellite.texture,
        "a3\\map_tanoabuka\\data\\layers\\00_00\\s_012_028_lco.paa"
    );
    assert_eq!(
        m.mask.texture,
        "a3\\map_tanoabuka\\data\\layers\\00_00\\m_012_028_lca.paa"
    );
    assert_eq!(
        m.satellite_normal.as_ref().unwrap().texture,
        "a3\\map_tanoabuka\\data\\layers\\00_00\\n_012_028_nohq.paa"
    );
    assert_eq!(m.tile, Some((12, 28)));
}

#[test]
fn only_filled_layer_slots_are_listed() {
    let m = tanoa_cell();
    assert_eq!(m.layers.len(), 1);
    let layer = &m.layers[0];
    assert_eq!(layer.slot, 2);
    assert_eq!(
        layer.normal.texture,
        "a3\\map_data_exp\\gdt_seabedexp_nopx.paa"
    );
    assert_eq!(
        layer.color.texture,
        "a3\\map_data_exp\\gdt_seabedexp_co.paa"
    );
    assert_eq!(layer.color.uv.source, UvSource::Tex);
    assert_eq!(layer.color.uv.aside, Vec3::new(5.0, 0.0, 0.0));
}

#[test]
fn satellite_uv_maps_world_position_into_the_tile() {
    let m = tanoa_cell();
    let uv = &m.satellite.uv;
    assert_eq!(uv.source, UvSource::WorldPos);
    // Tile 12 starts 11.21875 * 512 = 5744 m east; v counts down from 3.78125 * 512 = 1936 m.
    let at = |x: f32, z: f32| uv.apply(Vec3::new(x, 123.0, z));
    let a = at(5744.0, 1936.0);
    assert!(a.x.abs() < 1e-4 && a.y.abs() < 1e-4, "{a:?}");
    let b = at(5744.0 + 512.0, 1936.0 - 512.0);
    assert!(
        (b.x - 1.0).abs() < 1e-4 && (b.y - 1.0).abs() < 1e-4,
        "{b:?}"
    );
}
