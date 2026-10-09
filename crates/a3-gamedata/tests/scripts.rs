//! Text configs (config.cpp) and scripts loaded through the preprocessor from the VFS.

use std::path::Path;

use a3_config::{parse_text, write_rap};
use a3_gamedata::{GameData, LoadOptions, VfsHost, VfsResolver};
use a3_pbo::PboWriter;
use a3_preproc::{IncludeResolver, Preprocessor};
use a3_sqf::Vm;

fn write_pbo(path: &Path, prefix: &str, files: &[(&str, Vec<u8>)]) {
    let mut writer = PboWriter::new().property("prefix", prefix);
    for (name, bytes) in files {
        writer = writer.file(*name, bytes.clone());
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, writer.to_bytes()).unwrap();
}

fn text(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

fn rap(src: &str) -> Vec<u8> {
    write_rap(&parse_text(src).unwrap())
}

/// A game whose `my_addon.pbo` ships an unbinarised config.cpp with includes, macros and
/// `__EVAL`/`__EXEC`, plus a folder that has both config.bin and config.cpp.
fn game(root: &Path) {
    write_pbo(
        &root.join("Addons/base.pbo"),
        r"a3\base",
        &[(
            "config.bin",
            rap(
                "class CfgPatches { class Base { requiredAddons[] = {}; }; }; class CfgX { base = 1; };",
            ),
        )],
    );
    write_pbo(
        &root.join("Addons/my_addon.pbo"),
        r"x\my\addon",
        &[
            (
                "config.cpp",
                text(
                    "#include \"script_component.hpp\"\n\
                     class CfgPatches { class ADDON { requiredAddons[] = {\"Base\"}; }; };\n\
                     __EXEC(_sizes = [10, 20, 30])\n\
                     class CfgX {\n\
                     \tGVAR(size) = __EVAL(_sizes select 2);\n\
                     \tGVAR(name) = QUOTE(ADDON);\n\
                     \ttotal = __EVAL((_sizes select 0) + (_sizes select 1) + (_sizes select 2));\n\
                     };\n",
                ),
            ),
            (
                "script_component.hpp",
                text(
                    "#define ADDON my_addon\n\
                     #define QUOTE(x) #x\n\
                     #define GVAR(x) my_addon_##x\n\
                     #include \"\\a3\\base\\shared.hpp\"\n",
                ),
            ),
            (
                r"both\config.bin",
                rap(
                    "class CfgPatches { class Both { requiredAddons[] = {}; }; }; class CfgY { from = \"bin\"; };",
                ),
            ),
            (
                r"both\config.cpp",
                text(
                    "class CfgPatches { class Both { requiredAddons[] = {}; }; }; class CfgY { from = \"cpp\"; };",
                ),
            ),
            (
                r"functions\fn_init.sqf",
                text("#include \"..\\script_component.hpp\"\nGVAR(ready) = SHARED;"),
            ),
        ],
    );
    write_pbo(
        &root.join("Addons/shared.pbo"),
        r"a3\base",
        &[("shared.hpp", text("#define SHARED 42\n"))],
    );
}

#[test]
fn config_cpp_is_preprocessed_evaluated_and_merged() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());

    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();

    assert!(
        data.report.config_errors.is_empty(),
        "{:?}",
        data.report.config_errors
    );
    let cfg = data.config.root() >> "CfgX";
    assert_eq!((&cfg >> "base").number(), 1.0);
    assert_eq!((&cfg >> "my_addon_size").number(), 30.0);
    assert_eq!((&cfg >> "my_addon_name").text(), "my_addon");
    assert_eq!((&cfg >> "total").number(), 60.0);
    let paths: Vec<&str> = data.addons.iter().map(|a| a.path.as_str()).collect();
    assert!(paths.contains(&r"x\my\addon\config.cpp"), "{paths:?}");
    // config.cpp loads after its requirement.
    assert_eq!(paths[0], r"a3\base\config.bin");
}

#[test]
fn config_bin_wins_over_config_cpp_in_the_same_folder() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());

    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();

    assert_eq!((data.config.root() >> "CfgY" >> "from").text(), "bin");
    let skipped: Vec<&str> = data
        .report
        .skipped_config_cpp
        .iter()
        .map(|p| p.as_str())
        .collect();
    assert_eq!(skipped, [r"x\my\addon\both\config.cpp"]);
}

#[test]
fn broken_config_cpp_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    write_pbo(
        &dir.path().join("Addons/z_bad.pbo"),
        r"z\bad",
        &[(
            "config.cpp",
            text("#include \"missing.hpp\"\nclass CfgZ {};"),
        )],
    );

    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();

    assert_eq!(data.report.config_errors.len(), 1);
    let (path, message) = &data.report.config_errors[0];
    assert_eq!(path.as_str(), r"z\bad\config.cpp");
    assert!(message.contains("missing.hpp"), "{message}");
}

#[test]
fn vfs_resolver_finds_files_across_pbos() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let resolver = VfsResolver::new(&data.vfs);

    assert!(resolver.exists("\\x\\my\\addon\\config.cpp", "\\A3\\Base\\Shared.hpp"));
    assert!(!resolver.exists("\\x\\my\\addon\\config.cpp", "nope.hpp"));
    let out = Preprocessor::new(&resolver)
        .preprocess_file("\\x\\my\\addon\\functions\\fn_init.sqf")
        .unwrap();
    assert_eq!(out.text.trim(), "my_addon_ready = 42;");
}

#[test]
fn text_decoding_handles_boms_and_utf16() {
    use a3_gamedata::decode_text;
    assert_eq!(decode_text(b"\xEF\xBB\xBFx = 1;"), "x = 1;");
    assert_eq!(decode_text(b"\xFF\xFEx\0=\x001\0"), "x=1");
    assert_eq!(decode_text(b"\xFE\xFF\0x\0=\x001"), "x=1");
    assert_eq!(decode_text(b"caf\xC3\xA9"), "caf\u{e9}");
}

#[test]
fn vfs_host_runs_preprocessed_scripts() {
    let dir = tempfile::tempdir().unwrap();
    game(dir.path());
    let data = GameData::load(&LoadOptions::new(dir.path())).unwrap();
    let mut vm = Vm::new(VfsHost::new(data.vfs.clone()));

    vm.eval("call compileScript [\"\\x\\my\\addon\\functions\\fn_init.sqf\"]")
        .unwrap();
    assert_eq!(vm.get_global("my_addon_ready").to_sqf_string(), "42");
    let text = vm
        .eval("preprocessFileLineNumbers \"x\\my\\addon\\functions\\fn_init.sqf\"")
        .unwrap();
    assert!(
        text.to_sqf_string()
            .starts_with("\"#line 1 \"\"x\\my\\addon\\functions\\fn_init.sqf\"\""),
        "{text}"
    );
    assert!(vm.host.errors.is_empty(), "{:?}", vm.host.errors);
}

#[test]
fn errors_in_log_puts_script_errors_between_the_diag_log_lines() {
    let mut host = VfsHost::new(a3_vfs::Vfs::new());
    host.errors_in_log = true;
    let mut vm = Vm::new(host);
    vm.eval("diag_log text \"before\"").unwrap();
    assert!(vm.eval("1 + \"a\"").is_err());
    vm.eval("diag_log text \"after\"").unwrap();
    let log = &vm.host.log;
    assert_eq!(log.first().map(String::as_str), Some("before"));
    assert_eq!(log.last().map(String::as_str), Some("after"));
    assert!(
        log[1].starts_with("Error in expression"),
        "the error sits between: {log:?}"
    );
    assert_eq!(vm.host.errors.len(), 1, "errors are still collected");
}
