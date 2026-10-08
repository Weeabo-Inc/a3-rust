//! `#include`, resolvers and the source map.

mod common;
use a3_preproc::{
    ErrorKind, FsResolver, IncludeError, MemoryResolver, Options, Preprocessor, SourceLocation,
};
use std::sync::Arc;

fn loc(file: &str, line: u32) -> SourceLocation {
    SourceLocation {
        file: Arc::from(file),
        line,
    }
}

#[test]
fn include_relative_to_current_file() {
    let files = MemoryResolver::new().with_file(
        "\\tag\\main\\script_macros.hpp",
        "#define GVAR(x) tag_main_##x",
    );
    let out = Preprocessor::new(&files)
        .preprocess_str(
            "\\tag\\main\\fn_a.sqf",
            "#include \"script_macros.hpp\"\nGVAR(a) = 1;",
        )
        .unwrap();
    assert_eq!(out.text, "\ntag_main_a = 1;");
}

#[test]
fn include_with_angle_brackets_and_parent_dir() {
    let files = MemoryResolver::new().with_file("\\a\\common.hpp", "X");
    let out = Preprocessor::new(&files)
        .preprocess_str("\\a\\b\\c.sqf", "#include <..\\common.hpp>")
        .unwrap();
    assert_eq!(out.text, "X");
}

#[test]
fn include_absolute_and_case_insensitive() {
    let files =
        MemoryResolver::new().with_file("\\A3\\UI_F\\hpp\\defineCommon.inc", "#define CT_STATIC 0");
    let out = Preprocessor::new(&files)
        .preprocess_str(
            "\\x\\y.sqf",
            "#include \"\\a3\\ui_f\\hpp\\definecommon.inc\"\nCT_STATIC",
        )
        .unwrap();
    assert_eq!(out.text, "\n0");
}

#[test]
fn nested_includes_resolve_relative_to_their_own_file() {
    let files = MemoryResolver::new()
        .with_file("\\m\\a\\one.hpp", "#include \"sub\\two.hpp\"\n1")
        .with_file("\\m\\a\\sub\\two.hpp", "2\n");
    let out = Preprocessor::new(&files)
        .preprocess_str("\\m\\a\\main.sqf", "#include \"one.hpp\"\nmain")
        .unwrap();
    assert_eq!(out.text, "2\n\n1\nmain");
}

#[test]
fn defines_from_included_file_persist() {
    let files = MemoryResolver::new().with_file("\\inc.hpp", "#define A 1\n");
    let out = Preprocessor::new(&files)
        .preprocess_str("\\main.sqf", "#include \"\\inc.hpp\"\nA")
        .unwrap();
    assert_eq!(out.text, "\n\n1");
}

#[test]
fn missing_include_is_an_error() {
    let files = MemoryResolver::new();
    let err = Preprocessor::new(&files)
        .preprocess_str("\\a\\main.sqf", "\n#include \"nope.hpp\"")
        .unwrap_err();
    assert_eq!(err.line, 2);
    assert_eq!(&*err.file, "\\a\\main.sqf");
    assert_eq!(
        err.kind,
        ErrorKind::Include {
            path: "nope.hpp".into(),
            source: IncludeError::NotFound("\\a\\nope.hpp".into())
        }
    );
}

#[test]
fn include_without_quotes_is_an_error() {
    // wiki: macros cannot name include paths; engine error 2
    let files = MemoryResolver::new();
    let err = Preprocessor::new(&files)
        .preprocess_str("\\main.sqf", "#define path \"x.txt\"\n#include path")
        .unwrap_err();
    assert!(matches!(
        err.kind,
        ErrorKind::MalformedDirective {
            directive: "include",
            ..
        }
    ));
}

#[test]
fn recursive_include_hits_depth_limit() {
    let files = MemoryResolver::new().with_file("\\loop.hpp", "#include \"loop.hpp\"\n");
    let err = Preprocessor::new(&files)
        .with_options(Options {
            max_include_depth: 8,
            ..Options::default()
        })
        .preprocess_file("\\loop.hpp")
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::IncludeDepth(8));
}

#[test]
fn preprocess_file_loads_through_resolver() {
    let files = MemoryResolver::new().with_file("/Mission/Init.sqf", "hint \"hi\";");
    let out = Preprocessor::new(&files)
        .preprocess_file("\\mission\\init.sqf")
        .unwrap();
    assert_eq!(out.text, "hint \"hi\";");
    assert_eq!(
        out.source_map.location(1),
        Some(&loc("\\Mission\\Init.sqf", 1))
    );
}

#[test]
fn fs_resolver_reads_from_disk_case_insensitively() {
    let dir = std::env::temp_dir().join(format!("a3-preproc-fs-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Addon").join("Inc")).unwrap();
    std::fs::write(
        dir.join("Addon").join("Inc").join("Macros.hpp"),
        "#define V 7\n",
    )
    .unwrap();
    let files = FsResolver::new(&dir);
    let mut pp = Preprocessor::new(&files);
    let out = pp
        .preprocess_str("\\addon\\script.sqf", "#include \"inc\\macros.hpp\"\nV")
        .unwrap();
    assert_eq!(out.text, "\n\n7");
    assert!(a3_preproc::IncludeResolver::exists(
        &files,
        "",
        "\\addon\\inc\\MACROS.HPP"
    ));
    assert!(!a3_preproc::IncludeResolver::exists(
        &files,
        "",
        "\\addon\\nope.hpp"
    ));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn source_map_follows_includes_and_skipped_lines() {
    let files = MemoryResolver::new().with_file("\\m\\inc.hpp", "i1\ni2\n");
    let src = "a\n#include \"inc.hpp\"\n#ifdef NO\nskipped\n#endif\nb";
    let out = Preprocessor::new(&files)
        .preprocess_str("\\m\\main.sqf", src)
        .unwrap();
    // `#endif` returning to active code keeps its line break.
    assert_eq!(out.text, "a\ni1\ni2\n\n\nb");
    let map: Vec<_> = out.source_map.iter().cloned().collect();
    assert_eq!(
        map,
        [
            loc("\\m\\main.sqf", 1),
            loc("\\m\\inc.hpp", 1),
            loc("\\m\\inc.hpp", 2),
            loc("\\m\\main.sqf", 2),
            loc("\\m\\main.sqf", 5),
            loc("\\m\\main.sqf", 6),
        ]
    );
}

#[test]
fn line_directives_mark_discontinuities() {
    // preprocessFileLineNumbers
    let files = MemoryResolver::new().with_file("\\m\\inc.hpp", "i1\n");
    let src = "a\n#include \"inc.hpp\"\n#ifdef NO\nskipped\n#endif\nb\nc";
    let out = Preprocessor::new(&files)
        .preprocess_str("\\m\\main.sqf", src)
        .unwrap();
    assert_eq!(
        out.with_line_directives(),
        "#line 1 \"\\m\\main.sqf\"\na\n#line 1 \"\\m\\inc.hpp\"\ni1\n#line 2 \"\\m\\main.sqf\"\n\n#line 5 \"\\m\\main.sqf\"\n\nb\nc"
    );
}

#[test]
fn line_directives_always_start_with_the_root_file() {
    // wiki: preprocessFileLineNumbers adds `#line 1 "aFilename"` at the beginning.
    let files = MemoryResolver::new().with_file("\\m\\inc.hpp", "i1\n");
    let out = Preprocessor::new(&files)
        .preprocess_str("m\\main.sqf", "#include \"\\m\\inc.hpp\"\nx")
        .unwrap();
    assert_eq!(
        out.with_line_directives(),
        "#line 1 \"m\\main.sqf\"\n#line 1 \"\\m\\inc.hpp\"\ni1\n#line 1 \"m\\main.sqf\"\n\nx"
    );
    let empty = Preprocessor::new(&files)
        .preprocess_str("e.sqf", "")
        .unwrap();
    assert_eq!(empty.with_line_directives(), "#line 1 \"e.sqf\"\n");
}

#[test]
fn line_directive_renumbers() {
    let files = MemoryResolver::new();
    let out = Preprocessor::new(&files)
        .preprocess_str("\\m\\a.sqf", "x\n#line 100 \"\\orig\\b.sqf\"\n__LINE__\ny")
        .unwrap();
    assert_eq!(out.text, "x\n\n100\ny");
    assert_eq!(out.source_map.location(3), Some(&loc("\\orig\\b.sqf", 100)));
    assert_eq!(out.source_map.location(4), Some(&loc("\\orig\\b.sqf", 101)));
}

#[test]
fn source_map_has_one_entry_per_output_line() {
    let files = MemoryResolver::new();
    let src = "#define F(a) a\nx\n/*\n*/\nF(1,\n";
    let err = Preprocessor::new(&files)
        .preprocess_str("\\a.sqf", src)
        .unwrap_err();
    assert_eq!(err.line, 5);
    let out = Preprocessor::new(&files)
        .preprocess_str("\\a.sqf", "x\nF(1)\n#define F(a,b) a\nF(1)")
        .unwrap();
    assert_eq!(out.warnings[0].line, 4);
    let out = Preprocessor::new(&files)
        .preprocess_str("\\a.sqf", "a\n\nb\n")
        .unwrap();
    assert_eq!(out.source_map.len(), out.text.lines().count());
}
