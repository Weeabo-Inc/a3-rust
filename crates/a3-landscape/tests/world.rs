//! World config, surfaces and per-cell lookups on small hand-written configs.

use a3_config::{ConfigTree, parse_text};
use a3_landscape::{Error, Surfaces, TerrainLayers, WorldConfig, world_classes};
use a3_wrp::TerrainBuilder;
use glam::{Vec2, Vec3};

const CONFIG: &str = r##"
class CfgWorlds {
    class DefaultWorld { soundMapSizeCoef = 1; outsideHeight = -10;
        class DefaultClutter { scaleMin = 0.5; scaleMax = 1.0; swLighting = 1; relativeColor[] = {1, 1, 1, 1}; };
    };
    class CAWorld: DefaultWorld { clutterGrid = 1.2; };
    class Testland: CAWorld {
        worldName = "\A3\Map_Test\Testland.wrp";
        description = "$STR_TESTLAND";
        mapSize = 2048;
        centerPosition[] = {1000, 1024, 100};
        soundMapSizeCoef = 4;
        newRoadsShape = "\A3\Map_Test\data\roads\roads.shp";
        skyObject = "A3\Map_Test\data\obloha.p3d";
        class Lighting { access = 3; };
        class DayLightingBrightAlmost {};
        class EnvMaps { class EnvMap1 { texture = "env1_ca.paa"; overcast = 0.3; }; };
        class Sea { seaTexture = "sea_co.paa"; MaxWave = 0.25; WaterGrid = 50; SeaWaveXScale = "2.0/50"; SeaWaveZDuration = 8000; };
        class WaterExPars { fogDensity = 0.07; fogGradientCoefs[] = {0.35, 1.0, 1.7}; refractionMaxDist = 5.1; };
        class Underwater { waterColor[] = {0.04, 0.16, 0.22}; };
        class Grid { offsetX = 0; offsetY = 2048;
            class Zoom1 { zoomMax = 0.15; format = "XY"; formatX = "000"; formatY = "000"; stepX = 100; stepY = -100; };
        };
        class OutsideTerrain { satellite = "s_satout_co.paa"; enableTerrainSynth = 0; colorOutside[] = {0.2, 0.3, 0.4, 1};
            class Layers { class Layer0 { nopx = "gdt_seabed_nopx.paa"; texture = "gdt_seabed_co.paa"; }; };
        };
        class clutter {
            class TestGrass { model = "grass.p3d"; affectedByWind = 0.6; scaleMax = 1.4; };
        };
        class AmbientA3 { class Radius440_500 { areaSpawnRadius = 70;
            class Species { class Seagull { maxCircleCount = "sea"; maxWorldCount = 40; cost = 3; }; };
        }; };
        class Names { class Town { name = "Townsville"; type = "NameCity"; position[] = {512, 640}; radiusA = 200; radiusB = 150; angle = 15; }; };
    };
};
class CfgSurfaces {
    class Default { files = "default"; rough = 0.075; character = "Empty"; };
    class GdtSeabed: Default { files = "gdt_seabed_*"; isWater = 0; soundEnviron = "sand"; character = "SeabedClutter"; };
    class GdtGrass: Default { files = "gdt_grass_*"; character = "GrassClutter"; };
};
class CfgSurfaceCharacters {
    class Empty { probability[] = {}; names[] = {}; };
    class GrassClutter { probability[] = {0.8, 0.1}; names[] = {"TestGrass", "TestFlower"}; };
};
"##;

fn tree() -> ConfigTree {
    ConfigTree::from_config(&parse_text(CONFIG).unwrap())
}

#[test]
fn lists_world_classes_with_a_wrp() {
    assert_eq!(world_classes(&tree()), ["Testland"]);
}

#[test]
fn reads_world_settings_with_inheritance() {
    let tree = tree();
    let w = WorldConfig::load(&tree, "testland").unwrap();
    assert_eq!(w.class, "Testland");
    assert_eq!(w.wrp.as_str(), "a3\\map_test\\testland.wrp");
    assert_eq!(w.map_size, 2048.0);
    assert_eq!(w.center_position, Vec3::new(1000.0, 1024.0, 100.0));
    assert_eq!(w.sound_map_size_coef, 4);
    assert_eq!(w.outside_height, -10.0, "inherited from DefaultWorld");
    assert_eq!(w.clutter_grid, 1.2, "inherited from CAWorld");
    assert_eq!(
        w.roads_shape.as_ref().unwrap().as_str(),
        "a3\\map_test\\data\\roads\\roads.shp"
    );
    assert_eq!(w.grid.offset_y, 2048.0);
    assert_eq!(w.grid.zooms[0].step_y, -100.0);
    assert_eq!(w.outside_terrain.layers[0].texture, "gdt_seabed_co.paa");
    assert_eq!(w.outside_terrain.color, [0.2, 0.3, 0.4, 1.0]);
    assert_eq!(w.sky.sky_object, "A3\\Map_Test\\data\\obloha.p3d");
    assert_eq!(w.sky.env_maps[0].overcast, 0.3);
    assert_eq!(
        w.sky.lighting_classes,
        ["Lighting", "DayLightingBrightAlmost"]
    );
    assert_eq!(w.sea.max_wave, 0.25);
    assert_eq!(w.sea.water_color, Vec3::new(0.04, 0.16, 0.22));
    assert_eq!(w.ambient[0].species[0].class, "Seagull");
    assert_eq!(w.locations[0].position, Vec2::new(512.0, 640.0));
    assert_eq!(w.locations[0].kind, "NameCity");
}

#[test]
fn clutter_models_fill_gaps_from_default_clutter() {
    let w = WorldConfig::load(&tree(), "Testland").unwrap();
    let grass = w.clutter_model("testgrass").unwrap();
    assert_eq!(grass.affected_by_wind, 0.6);
    assert_eq!(grass.scale_max, 1.4);
    assert_eq!(grass.scale_min, 0.5);
    assert!(grass.sw_lighting);
}

#[test]
fn unknown_world_is_an_error() {
    assert!(matches!(
        WorldConfig::load(&tree(), "Nowhere"),
        Err(Error::NoWorld(_))
    ));
}

#[test]
fn surfaces_are_found_by_layer_texture() {
    let s = Surfaces::load(&tree());
    assert_eq!(s.surfaces.len(), 3);
    let seabed = s.for_texture("a3\\map_data\\GDT_Seabed_co.paa").unwrap();
    assert_eq!(seabed.class, "GdtSeabed");
    assert_eq!(seabed.rough, 0.075, "inherited");
    assert_eq!(seabed.sound_environ, "sand");
    assert!(s.for_texture("a3\\map_data\\gdt_rock_co.paa").is_none());
    let grass = s
        .character(&s.surface("gdtgrass").unwrap().character)
        .unwrap();
    assert_eq!(
        grass.clutter,
        [
            ("TestGrass".to_owned(), 0.8),
            ("TestFlower".to_owned(), 0.1)
        ]
    );
}

const CELL_RVMAT: &str = r##"
PixelShaderID = "TerrainSNX";
class Stage0 { texture = "s_000_001_lco.paa"; texGen = 3; };
class Stage1 { texture = "m_000_001_lca.paa"; texGen = 3; };
class TexGen3 { uvSource = "worldPos"; class uvTransform { aside[] = {0.01, 0, 0}; up[] = {0, 0, 0.01}; dir[] = {0, -0.01, 0}; pos[] = {0, 1, 0}; }; };
class Stage3 { texture = "gdt_grass_nopx.paa"; texGen = 1; };
class Stage4 { texture = "gdt_grass_co.paa"; texGen = 2; };
"##;

#[test]
fn each_cell_resolves_to_its_tile_material() {
    let terrain = TerrainBuilder::new(4, 8, 25.0)
        .material("a3\\map_test\\data\\layers\\p_000-001_l00.rvmat")
        .material("a3\\map_test\\data\\layers\\p_001-001_missing.rvmat")
        .edit(|t| {
            *t.material_indices.get_mut(1, 0).unwrap() = 1;
            *t.material_indices.get_mut(2, 0).unwrap() = 2;
        })
        .build();
    let layers = TerrainLayers::load_with(&terrain, |path| {
        if path.ends_with("p_000-001_l00.rvmat") {
            Ok(parse_text(CELL_RVMAT).unwrap())
        } else {
            Err(Error::Parse {
                path: path.to_owned(),
                detail: "missing".into(),
            })
        }
    });
    assert_eq!(layers.errors.len(), 1);

    let cell = layers.cell(&terrain, 1, 0).unwrap();
    assert_eq!(cell.material_index, 1);
    assert_eq!(cell.satellite().texture, "s_000_001_lco.paa");
    assert_eq!(cell.mask().texture, "m_000_001_lca.paa");
    assert_eq!(cell.layers()[0].color.texture, "gdt_grass_co.paa");
    assert_eq!(cell.material.tile, Some((0, 1)));
    // World (30, 10) lies in cell (1, 0).
    let at = layers.at_world(&terrain, 30.0, 10.0).unwrap();
    assert_eq!(at.cell, (1, 0));
    // Cell (0, 0) uses the empty material 0; cell (2, 0) a material that failed to load.
    assert!(layers.cell(&terrain, 0, 0).is_none());
    assert!(layers.cell(&terrain, 2, 0).is_none());
    assert!(layers.cell(&terrain, 4, 0).is_none());
}

#[test]
fn sea_waves_evaluate_expressions_and_default_like_the_engine() {
    let w = WorldConfig::load(&tree(), "Testland").unwrap();
    let waves = w.sea.waves;
    assert_eq!(waves.x_scale, 2.0 / 50.0, "\"2.0/50\" is evaluated");
    assert_eq!(waves.z_duration_ms, 8000);
    assert_eq!(waves.water_grid, 50.0);
    // Missing entries take the engine's defaults.
    assert_eq!(waves.z_scale, 0.02);
    assert_eq!(waves.x_duration_ms, 5000);
    assert_eq!(waves.max_tide, 1.5);
    let ex = w.sea.water_ex;
    assert_eq!(ex.fog_density, Some(0.07));
    assert_eq!(ex.fog_gradient_coefs, Some(Vec3::new(0.35, 1.0, 1.7)));
    assert_eq!(ex.refraction_max_dist, Some(5.1));
    assert_eq!(ex.surface_opacity, None, "unset entries stay unset");
}
