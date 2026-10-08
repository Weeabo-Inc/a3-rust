//! Surface materials: `.bisurf` files, `#Class` references to CfgSurfaces and texture matching.

use std::sync::Arc;

use a3_physics::{MemoryFiles, SurfaceBank};

const CONCRETE: &str = "Density=2400;\r\nrough=0.1;\r\ndust=0;\r\nbulletPenetrability=80;\r\n\
soundEnviron=Empty;\r\nisWater=false;\r\nfriction=0.9;\r\nrestitution=0;\r\n\
impact = hitConcrete;\r\nsoundHit = Concrete;\r\ndeflection = 0.8;\r\n";

fn files() -> Arc<MemoryFiles> {
    let mut f = MemoryFiles::default();
    f.insert(
        "a3\\data_f\\penetration\\concrete.bisurf",
        CONCRETE.as_bytes(),
    );
    f.insert(
        "a3\\data_f\\penetration\\plate.bisurf",
        b"rough=0;dust=0;bulletPenetrabilityWithThickness=40;thickness=12;soundEnviron=\"Empty\";\
isWater=0;impact=\"hitMetalPlate\";soundHit=\"metal_plate\";transparency=0.5;",
    );
    Arc::new(f)
}

const CFG_SURFACES: &str = r#"
class CfgSurfaces {
    class Default {
        files = "default";
        rough = 0.08; dust = 0.1; soundEnviron = "dirt"; impact = "hitGroundSoft";
        soundHit = "soft_ground"; isWater = 0; friction = 0.9; restitution = 0;
        surfaceFriction = 1.7; maxSpeedCoef = 1; character = "Empty";
    };
    class Betonout: Default { files = "betonout"; rough = 0.05; soundEnviron = "concrete"; };
    class GdtGrass: Default { files = "gdt_grass_green_*"; soundEnviron = "grass"; };
};
"#;

fn bank() -> SurfaceBank {
    let config = a3_config::parse_text(CFG_SURFACES).unwrap();
    let tree = a3_config::ConfigTree::from_config(&config);
    SurfaceBank::new(files(), Some(&tree))
}

#[test]
fn a_bisurf_file_gives_its_surface_properties() {
    let mut bank = bank();
    let id = bank.surface("A3\\Data_F\\Penetration\\Concrete.bisurf");
    let s = bank.get(id);
    assert_eq!(s.rough, 0.1);
    assert_eq!(s.dust, 0.0);
    assert!(!s.is_water);
    assert_eq!(s.sound_environ, "Empty");
    assert_eq!(s.impact, "hitConcrete");
    assert_eq!(s.sound_hit, "Concrete");
    assert_eq!(s.friction, 0.9);
    assert_eq!(s.restitution, 0.0);
    assert_eq!(s.density, 2400.0);
    assert_eq!(s.deflection, 0.8);
    // The engine keeps 1e6 / bulletPenetrability.
    assert_eq!(s.penetration_resistance, 1e6 / 80.0);
    assert_eq!(s.thickness, None);
}

#[test]
fn the_same_name_in_any_case_is_one_surface() {
    let mut bank = bank();
    let a = bank.surface("a3\\data_f\\penetration\\concrete.bisurf");
    let b = bank.surface("A3\\DATA_F\\PENETRATION\\CONCRETE.BISURF");
    assert_eq!(a, b);
}

#[test]
fn plate_surfaces_read_thickness_in_millimetres() {
    let mut bank = bank();
    let id = bank.surface("a3\\data_f\\penetration\\plate.bisurf");
    let s = bank.get(id);
    assert_eq!(s.penetration_resistance, 1e6 / 40.0);
    assert_eq!(s.thickness, Some(0.012));
    assert_eq!(s.transparency, 0.5);
    assert_eq!(s.sound_hit, "metal_plate");
}

#[test]
fn a_missing_file_gives_the_engine_defaults() {
    let mut bank = bank();
    let id = bank.surface("a3\\nothing.bisurf");
    let s = bank.get(id).clone();
    assert!(!s.loaded);
    assert_eq!(s.rough, 0.0);
    assert_eq!(s.surface_friction, 2.0);
    assert_eq!(s.tracks_alpha, 1.0);
    assert_eq!(s.transparency, -1.0);
}

#[test]
fn a_hash_name_refers_to_a_cfg_surfaces_class() {
    let mut bank = bank();
    let id = bank.surface("#Betonout");
    let s = bank.get(id);
    assert!(s.loaded);
    assert_eq!(s.rough, 0.05);
    assert_eq!(s.sound_environ, "concrete");
    // Inherited from Default.
    assert_eq!(s.surface_friction, 1.7);
}

#[test]
fn a_texture_selects_the_cfg_surfaces_class_whose_files_pattern_matches_its_name() {
    let bank = bank();
    let betonout = bank
        .for_texture("a3\\data_f\\surfaces\\betonout.paa")
        .unwrap();
    assert_eq!(bank.get(betonout).name, "#Betonout");
    let grass = bank
        .for_texture("a3\\map_data\\gdt_grass_green_co.paa")
        .unwrap();
    assert_eq!(bank.get(grass).name, "#GdtGrass");
    assert_eq!(bank.for_texture("a3\\data_f\\surfaces\\tasky.paa"), None);
}
