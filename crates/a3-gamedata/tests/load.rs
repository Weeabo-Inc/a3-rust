//! Loading a synthetic game install: VFS mount plus merged config.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use a3_config::{parse_text, write_rap};
use a3_gamedata::{Error, GameData, LoadOptions, Localizer};
use a3_pbo::PboWriter;

/// Writes a PBO whose files are config.cpp texts, stored rapified as `<dir>config.bin`.
fn write_addon(path: &Path, prefix: &str, configs: &[(&str, &str)]) {
    let mut writer = PboWriter::new().property("prefix", prefix);
    for (dir, src) in configs {
        let config = parse_text(src).unwrap();
        writer = writer.file(format!("{dir}config.bin"), write_rap(&config));
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, writer.to_bytes()).unwrap();
}

fn patches(name: &str, required: &[&str]) -> String {
    let required: Vec<String> = required.iter().map(|r| format!("\"{r}\"")).collect();
    format!(
        "class CfgPatches {{ class {name} {{ requiredAddons[] = {{{}}}; }}; }};",
        required.join(", ")
    )
}

/// `Addons/a_main.pbo` requires `B`, which `Addons/b_dep.pbo` provides, so b loads first even
/// though a sorts first. b also has a second config in a subfolder.
fn game(root: &Path) {
    write_addon(
        &root.join("Addons/a_main.pbo"),
        r"a3\a",
        &[(
            "",
            &format!(
                "{} class CfgX {{ v = 1; class Thing: Base {{}}; }};",
                patches("A", &["B"])
            ),
        )],
    );
    write_addon(
        &root.join("Addons/b_dep.pbo"),
        r"a3\b",
        &[
            (
                "",
                &format!(
                    "{} class CfgX {{ v = 2; class Base {{ w = 3; }}; }};",
                    patches("B", &[])
                ),
            ),
            (
                r"sub\",
                &format!(
                    "{} class CfgY {{ s = \"$STR_Hello\"; }};",
                    patches("B_Sub", &["B"])
                ),
            ),
        ],
    );
}

#[test]
fn loads_configs_in_required_addons_order() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());

    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();

    let order: Vec<&str> = data.addons.iter().map(|a| a.path.as_str()).collect();
    // Once B is loaded, A (discovered first) and B_Sub are both ready; discovery order decides.
    assert_eq!(
        order,
        [
            r"a3\b\config.bin",
            r"a3\a\config.bin",
            r"a3\b\sub\config.bin"
        ]
    );
    assert_eq!(data.addons[1].patches, ["A"]);
    assert_eq!(data.addons[1].required, ["B"]);

    let cfg = data.config.root();
    assert_eq!((&cfg >> "CfgX" >> "v").number(), 1.0, "A patched last");
    assert_eq!((&cfg >> "CfgX" >> "Thing" >> "w").number(), 3.0);
    assert!((&cfg >> "CfgY").is_class());
    assert!(data.vfs.exists(r"a3\b\sub\config.bin"));
    assert!(data.report.config_errors.is_empty());
    assert_eq!(data.report.mount.pbos, 2);
}

#[test]
fn mods_load_after_the_game_and_patch_it() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let mod_dir = dir.path().join("@mymod");
    write_addon(
        &mod_dir.join("addons/m.pbo"),
        r"my\mod",
        &[(
            "",
            &format!("{} class CfgX {{ v = 9; }};", patches("M", &[])),
        )],
    );

    let data = GameData::load(&LoadOptions::new(dir.path()).with_mods([&mod_dir])).unwrap();

    assert_eq!(
        data.addons.last().unwrap().path.as_str(),
        r"my\mod\config.bin"
    );
    assert_eq!((data.config.root() >> "CfgX" >> "v").number(), 9.0);
}

#[test]
fn a_broken_config_is_reported_and_the_rest_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let pbo = PboWriter::new()
        .property("prefix", "broken")
        .file("config.bin", b"\0raP\0\0".to_vec());
    std::fs::write(dir.path().join("Addons/c_broken.pbo"), pbo.to_bytes()).unwrap();

    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();

    assert_eq!(data.report.config_errors.len(), 1);
    assert_eq!(
        data.report.config_errors[0].0.as_str(),
        r"broken\config.bin"
    );
    assert_eq!(data.addons.len(), 3);
}

#[test]
fn missing_requirements_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    write_addon(
        &dir.path().join("Addons/x.pbo"),
        "x",
        &[("", &patches("X", &["Nowhere"]))],
    );
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    assert_eq!(
        data.report.missing_requirements,
        [(
            a3_core::VfsPath::new(r"x\config.bin"),
            "Nowhere".to_string()
        )]
    );
}

#[test]
fn a_folder_without_archives_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        GameData::load(&LoadOptions::new(dir.path())),
        Err(Error::NothingMounted(_))
    ));
    assert!(matches!(
        GameData::load(&LoadOptions::new(dir.path().join("missing"))),
        Err(Error::GameDirNotFound(_))
    ));
}

struct Table(HashMap<String, String>);

impl Localizer for Table {
    fn localize(&self, key: &str) -> Option<String> {
        self.0.get(&key.to_ascii_lowercase()).cloned()
    }
}

#[test]
fn config_text_is_localized_through_a_pluggable_localizer() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let mut data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let text = (data.config.root() >> "CfgY" >> "s").text();

    assert_eq!(
        data.localize(&text),
        "$STR_Hello",
        "no localizer: unchanged"
    );
    data.localizer = Some(Arc::new(Table(HashMap::from([(
        "str_hello".to_string(),
        "Hello".to_string(),
    )]))));
    assert_eq!(data.localize(&text), "Hello");
    assert_eq!(data.localize("$STR_Missing"), "$STR_Missing");
    assert_eq!(data.localize("plain"), "plain");
}

/// The stringtables of the mounted game are attached at load time, so `localize`,
/// `isLocalized`, `$STR_` config text and `language` work (oracle probes `loc.*`, issue #278).
#[test]
fn the_stringtables_of_the_loaded_game_answer_localize() {
    let dir = tempfile::tempdir().unwrap();
    let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<Project name="Test">
  <Package name="P">
    <Container name="C">
      <Key ID="STR_Hello">
        <English>Hello</English>
        <German>Hallo</German>
      </Key>
    </Container>
  </Package>
</Project>"#;
    let mut writer = PboWriter::new().property("prefix", r"a3\x");
    writer = writer.file(
        "config.bin",
        write_rap(&parse_text(&patches("A", &[])).unwrap()),
    );
    writer = writer.file("stringtable.xml", xml.as_bytes().to_vec());
    std::fs::create_dir_all(dir.path().join("Addons")).unwrap();
    std::fs::write(dir.path().join("Addons/a_main.pbo"), writer.to_bytes()).unwrap();

    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    assert_eq!(data.language(), "English");
    assert_eq!(data.localize("$STR_Hello"), "Hello");
    assert_eq!(data.localize("$STR_Missing"), "$STR_Missing");

    let mut vm = a3_gamedata::script_vm(&data);
    let eval = |vm: &mut a3_sqf::Vm<_>, code: &str| match vm.eval(code) {
        Ok(v) => v.to_sqf_string(),
        Err(e) => panic!("{code}: {}", e.report),
    };
    assert_eq!(eval(&mut vm, "localize \"STR_Hello\""), "\"Hello\"");
    assert_eq!(eval(&mut vm, "localize \"$STR_Hello\""), "\"Hello\"");
    assert_eq!(eval(&mut vm, "localize \"str_hello\""), "\"Hello\"");
    assert_eq!(eval(&mut vm, "localize \"STR_Nope\""), "\"\"");
    assert_eq!(eval(&mut vm, "isLocalized \"STR_Hello\""), "true");
    assert_eq!(eval(&mut vm, "isLocalized \"STR_Nope\""), "false");
    assert_eq!(eval(&mut vm, "language"), "\"English\"");
}
