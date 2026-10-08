//! Behaviour of the VFS over synthetic PBOs and loose folders.

use std::path::Path;

use a3_core::VfsPath;
use a3_pbo::{Pbo, PboWriter};
use a3_vfs::{Error, Vfs};

fn pbo(prefix: &str, files: &[(&str, &str)]) -> Pbo {
    let mut writer = PboWriter::new().property("prefix", prefix);
    for (name, data) in files {
        writer = writer.file(*name, data.as_bytes().to_vec());
    }
    Pbo::from_bytes(writer.to_bytes()).unwrap()
}

fn write_pbo(path: &Path, prefix: Option<&str>, files: &[(&str, &str)]) {
    let mut writer = PboWriter::new();
    if let Some(prefix) = prefix {
        writer = writer.property("prefix", prefix);
    }
    for (name, data) in files {
        writer = writer.file(*name, data.as_bytes().to_vec());
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, writer.to_bytes()).unwrap();
}

fn text(vfs: &Vfs, path: &str) -> String {
    String::from_utf8(vfs.open(path).unwrap().to_vec()).unwrap()
}

#[test]
fn opens_files_of_a_mounted_pbo_under_its_prefix() {
    let vfs = Vfs::new();
    vfs.mount_pbo(
        pbo(
            r"a3\data_f",
            &[("config.bin", "cfg"), (r"Images\X.paa", "img")],
        ),
        None,
    );

    assert_eq!(text(&vfs, r"a3\data_f\config.bin"), "cfg");
    assert_eq!(text(&vfs, "/A3/Data_F/images/x.PAA"), "img");
    assert!(vfs.exists(r"\a3\data_f\config.bin"));
    assert!(!vfs.exists(r"a3\data_f\nope.bin"));
    assert!(matches!(
        vfs.open("a3\\data_f\\nope.bin"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn opens_compressed_pbo_entries_unpacked() {
    let config = "class CfgPatches { class A {}; };\n".repeat(30);
    let bytes = PboWriter::new()
        .property("prefix", r"x\packed")
        .compressed_file("config.cpp", config.as_bytes().to_vec())
        .to_bytes();
    let vfs = Vfs::new();
    vfs.mount_pbo(Pbo::from_bytes(bytes).unwrap(), None);

    assert_eq!(text(&vfs, r"x\packed\config.cpp"), config);
    assert_eq!(
        vfs.stat(r"x\packed\config.cpp").unwrap().size,
        config.len() as u64
    );
}

#[test]
fn later_mounts_override_earlier_ones_per_file() {
    let vfs = Vfs::new();
    vfs.mount_pbo(pbo("a3\\x", &[("a.txt", "old"), ("b.txt", "kept")]), None);
    vfs.mount_pbo(pbo("a3\\x", &[("a.txt", "new")]), None);

    assert_eq!(text(&vfs, r"a3\x\a.txt"), "new");
    assert_eq!(text(&vfs, r"a3\x\b.txt"), "kept");
}

#[test]
fn lists_immediate_children_of_a_directory() {
    let vfs = Vfs::new();
    vfs.mount_pbo(
        pbo(
            r"a3\data_f",
            &[
                ("config.bin", ""),
                (r"images\a.paa", ""),
                (r"images\b.paa", ""),
            ],
        ),
        None,
    );
    vfs.mount_pbo(pbo(r"a3\ui_f", &[("config.bin", "")]), None);

    let names = |dir: &str| -> Vec<String> {
        vfs.list_dir(dir)
            .into_iter()
            .map(|e| format!("{}{}", e.name, if e.is_dir { "\\" } else { "" }))
            .collect()
    };
    assert_eq!(names(""), [r"a3\"]);
    assert_eq!(names("A3"), [r"data_f\", r"ui_f\"]);
    assert_eq!(names(r"a3\data_f"), ["config.bin", r"images\"]);
    assert!(names(r"a3\data").is_empty());
    assert!(vfs.is_dir(r"a3\data_f\images"));
    assert!(!vfs.is_dir(r"a3\data_f\config.bin"));
}

#[test]
fn list_dir_sorts_by_name_even_when_separators_sort_after_other_characters() {
    let vfs = Vfs::new();
    vfs.mount_pbo(
        pbo("p", &[(r"b\x.txt", ""), ("b0.txt", ""), (r"b\y.txt", "")]),
        None,
    );
    let names: Vec<_> = vfs.list_dir("p").into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["b", "b0.txt"]);
}

#[test]
fn walk_and_glob_find_files_below_a_directory() {
    let vfs = Vfs::new();
    vfs.mount_pbo(
        pbo(
            r"a3\data_f",
            &[
                ("config.bin", ""),
                (r"images\a.paa", ""),
                (r"images\sub\b.paa", ""),
            ],
        ),
        None,
    );
    let all: Vec<_> = vfs
        .walk(r"a3\data_f\images")
        .into_iter()
        .map(VfsPath::into_string)
        .collect();
    assert_eq!(
        all,
        [r"a3\data_f\images\a.paa", r"a3\data_f\images\sub\b.paa"]
    );

    let globbed: Vec<_> = vfs
        .glob(r"a3\*\images\*.PAA")
        .into_iter()
        .map(VfsPath::into_string)
        .collect();
    assert_eq!(globbed, [r"a3\data_f\images\a.paa"]);
    let deep: Vec<_> = vfs
        .glob(r"a3\**\*.paa")
        .into_iter()
        .map(VfsPath::into_string)
        .collect();
    assert_eq!(
        deep,
        [r"a3\data_f\images\a.paa", r"a3\data_f\images\sub\b.paa"]
    );
}

#[test]
fn stat_reports_size_and_source() {
    let vfs = Vfs::new();
    vfs.mount_pbo(pbo("p", &[("a.txt", "12345")]), None);
    let info = vfs.stat(r"p\a.txt").unwrap();
    assert_eq!(info.size, 5);
    assert!(vfs.stat(r"p\b.txt").is_none());
}

#[test]
fn mounting_a_pbo_file_uses_its_prefix_or_falls_back_to_the_file_stem() {
    let dir = tempfile::tempdir().unwrap();
    let with = dir.path().join("whatever.pbo");
    let without = dir.path().join("My_Addon.pbo");
    write_pbo(&with, Some(r"x\with_prefix"), &[("a.txt", "a")]);
    write_pbo(&without, None, &[("b.txt", "b")]);

    let vfs = Vfs::new();
    vfs.mount_pbo_file(&with).unwrap();
    vfs.mount_pbo_file(&without).unwrap();

    assert_eq!(text(&vfs, r"x\with_prefix\a.txt"), "a");
    assert_eq!(text(&vfs, r"my_addon\b.txt"), "b");
}

#[test]
fn mounts_a_loose_directory_at_a_virtual_prefix() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Scripts")).unwrap();
    std::fs::write(dir.path().join("mission.sqm"), "sqm").unwrap();
    std::fs::write(dir.path().join("Scripts").join("Init.SQF"), "hint 1").unwrap();

    let vfs = Vfs::new();
    vfs.mount_dir(dir.path(), VfsPath::new(r"missions\test.altis"))
        .unwrap();

    assert_eq!(text(&vfs, r"missions\test.altis\mission.sqm"), "sqm");
    assert_eq!(
        text(&vfs, r"missions\test.altis\scripts\init.sqf"),
        "hint 1"
    );
    assert_eq!(
        vfs.stat(r"missions\test.altis\scripts\init.sqf")
            .unwrap()
            .size,
        6
    );
}

#[test]
fn mounts_a_mod_folder_in_file_name_order_and_skips_encrypted_archives() {
    let dir = tempfile::tempdir().unwrap();
    let addons = dir.path().join("@my_mod").join("Addons");
    write_pbo(&addons.join("b.pbo"), Some("m"), &[("x.txt", "from b")]);
    write_pbo(&addons.join("A.pbo"), Some("m"), &[("x.txt", "from a")]);
    std::fs::write(
        addons.join("c.ebo"),
        PboWriter::new().property("prefix", "enc").to_bytes(),
    )
    .unwrap();
    std::fs::write(addons.join("b.pbo.my.bisign"), "sig").unwrap();

    let vfs = Vfs::new();
    let report = vfs.mount_mod(&dir.path().join("@my_mod"));

    assert_eq!(report.pbos, 2);
    assert_eq!(report.encrypted.len(), 1);
    assert!(report.failed.is_empty());
    assert_eq!(text(&vfs, r"m\x.txt"), "from b");
}

#[test]
fn a_broken_pbo_in_a_mod_is_reported_and_the_rest_still_mounts() {
    let dir = tempfile::tempdir().unwrap();
    let addons = dir.path().join("addons");
    write_pbo(&addons.join("good.pbo"), Some("good"), &[("x.txt", "x")]);
    std::fs::write(addons.join("bad.pbo"), b"not a pbo").unwrap();

    let vfs = Vfs::new();
    let report = vfs.mount_mod(dir.path());

    assert_eq!(report.pbos, 1);
    assert_eq!(report.failed.len(), 1);
    assert!(vfs.exists(r"good\x.txt"));
}

#[test]
fn mounts_a_game_install_core_first_then_official_folders_in_load_order() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    write_pbo(
        &r.join("Dta").join("core.pbo"),
        Some("core"),
        &[("x.txt", "core")],
    );
    write_pbo(
        &r.join("Addons").join("a.pbo"),
        Some(r"a3\a"),
        &[("x.txt", "vanilla")],
    );
    write_pbo(
        &r.join("Expansion").join("Addons").join("e.pbo"),
        Some(r"a3\a"),
        &[("x.txt", "apex")],
    );
    write_pbo(
        &r.join("Heli").join("Addons").join("h.pbo"),
        Some(r"a3\a"),
        &[("x.txt", "heli")],
    );
    write_pbo(
        &r.join("@mod").join("addons").join("m.pbo"),
        Some(r"a3\a"),
        &[("y.txt", "mod")],
    );

    let vfs = Vfs::new();
    let report = vfs.mount_game(r, &[r.join("@mod")]);

    assert_eq!(report.pbos, 5);
    assert_eq!(text(&vfs, r"core\x.txt"), "core");
    // Expansion loads after Heli, so its copy wins.
    assert_eq!(text(&vfs, r"a3\a\x.txt"), "apex");
    assert_eq!(text(&vfs, r"a3\a\y.txt"), "mod");
}

#[test]
fn optional_mod_folders_are_discovered_but_not_official_ones() {
    let root = tempfile::tempdir().unwrap();
    let r = root.path();
    for dir in [
        "Addons",
        "Heli/Addons",
        "GM/addons",
        "@cba/addons",
        "Keys",
        "MPMissions",
    ] {
        std::fs::create_dir_all(r.join(dir)).unwrap();
    }
    let found: Vec<_> = a3_vfs::optional_mod_dirs(r)
        .into_iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(found, ["@cba", "GM"]);
}

#[test]
fn a_vfs_is_shared_across_threads_and_clones() {
    let vfs = Vfs::new();
    let clone = vfs.clone();
    std::thread::spawn(move || clone.mount_pbo(pbo("t", &[("a.txt", "a")]), None))
        .join()
        .unwrap();
    assert_eq!(text(&vfs, r"t\a.txt"), "a");
}

#[test]
fn lists_mounted_archives_in_mount_order_with_prefix_and_source() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("My_Addon.pbo");
    write_pbo(&file, None, &[("config.bin", "b")]);

    let vfs = Vfs::new();
    vfs.mount_pbo(pbo(r"a3\first", &[("config.bin", "a")]), None);
    vfs.mount_dir(dir.path(), VfsPath::new("loose")).unwrap();
    vfs.mount_pbo_file(&file).unwrap();

    let archives = vfs.archives();
    let prefixes: Vec<&str> = archives.iter().map(|a| a.prefix.as_str()).collect();
    assert_eq!(prefixes, [r"a3\first", "my_addon"]);
    assert_eq!(archives[0].source, None);
    assert_eq!(archives[1].source.as_deref(), Some(file.as_path()));
    assert_eq!(archives[1].pbo.entries()[0].name(), "config.bin");
}
