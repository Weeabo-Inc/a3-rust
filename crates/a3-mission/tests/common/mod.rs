//! Shared fixtures for the a3-mission tests: a small config, a World over flat 12 m terrain,
//! and a mission folder on disk mounted into a VFS.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_vfs::{Vfs, VfsPath};
use a3_world::{ClientId, TypeBank, World};
use a3_wrp::TerrainBuilder;

/// The classes the fixtures spawn. `O_Missing_F` is deliberately absent.
pub const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; };
    class AllVehicles: All {};
    class Land: AllVehicles {};
    class Man: Land { simulation = "soldier"; };
    class B_Soldier_F: Man { scope = 2; };
    class B_soldier_AR_F: Man { scope = 2; };
    class O_Soldier_F: Man { scope = 2; };
    class LandVehicle: Land {};
    class Car: LandVehicle { simulation = "carx"; };
    class Thing: All { simulation = "thing"; };
    class Land_Cargo10_F: Thing { scope = 2; };
};
"#;

/// A World whose terrain is 12 m high everywhere, and the types of [`CONFIG`].
pub fn world_and_types() -> (World, TypeBank) {
    world_and_types_at(12.0)
}

/// Like [`world_and_types`], with the terrain at `height` everywhere (0 or below is sea).
pub fn world_and_types_at(height: f32) -> (World, TypeBank) {
    let terrain = Arc::new(
        TerrainBuilder::new(4, 8, 50.0)
            .heights(|_, _| height)
            .build(),
    );
    let mut world = World::new(ClientId::SERVER);
    world.load_terrain(terrain).unwrap();
    let config = parse_text(CONFIG).unwrap();
    let types = TypeBank::new(Arc::new(ConfigTree::from_config(&config)));
    (world, types)
}

/// Writes `files` (virtual path, text) under a fresh temporary directory and mounts it at
/// `prefix`. Keep the returned directory alive while the VFS is used.
pub fn mount(files: &[(&str, &str)], prefix: &str) -> (tempfile::TempDir, Vfs) {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        write(&dir.path().join(path.replace('\\', "/")), text);
    }
    let vfs = mount_dir(&dir, prefix);
    (dir, vfs)
}

/// Like [`mount`], for files that are not text (a rapified `mission.sqm`).
pub fn mount_bytes(files: &[(&str, &[u8])], prefix: &str) -> (tempfile::TempDir, Vfs) {
    let dir = tempfile::tempdir().unwrap();
    for (path, bytes) in files {
        let file = dir.path().join(path.replace('\\', "/"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, bytes).unwrap();
    }
    let vfs = mount_dir(&dir, prefix);
    (dir, vfs)
}

/// The [`Vfs`] that a mounted directory's [`tempfile::TempDir`] wraps; the `prefix` is a virtual
/// path ([`VfsPath::new`] takes either separator).
fn mount_dir(dir: &tempfile::TempDir, prefix: &str) -> Vfs {
    let vfs = Vfs::new();
    vfs.mount_dir(dir.path(), VfsPath::new(prefix)).unwrap();
    vfs
}

/// Creates the file's parents and writes `text` (as the game ships missions: CRLF endings).
pub fn write(file: &Path, text: &str) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text.replace('\n', "\r\n")).unwrap();
}
