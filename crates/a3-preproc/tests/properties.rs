//! Property tests over arbitrary input.

use a3_preproc::{MemoryResolver, Options, Preprocessor};
use proptest::prelude::*;

fn run(source: &str) -> Result<a3_preproc::Output, a3_preproc::PreprocessError> {
    let files = MemoryResolver::new().with_file("\\inc.hpp", "#define INC 1\nINC\n");
    Preprocessor::new(&files)
        .with_options(Options::config())
        .preprocess_str("\\main.sqf", source)
}

/// Source-like text built from fragments that exercise every feature.
fn source() -> impl Strategy<Value = String> {
    let fragment = prop_oneof![
        Just("#define A 1\n".to_owned()),
        Just("#define F(x,y) [x,#y]\n".to_owned()),
        Just("#define G(x) F(x,x)##_\n".to_owned()),
        Just("#ifdef A\n".to_owned()),
        Just("#ifndef A\n".to_owned()),
        Just("#if A == 1\n".to_owned()),
        Just("#else\n".to_owned()),
        Just("#endif\n".to_owned()),
        Just("#undef A\n".to_owned()),
        Just("#include \"\\inc.hpp\"\n".to_owned()),
        Just("F(a,b) ".to_owned()),
        Just("G((1,2)) ".to_owned()),
        Just("__LINE__ ".to_owned()),
        Just("\"str A\" ".to_owned()),
        Just("'A' ".to_owned()),
        Just("/* c\n */".to_owned()),
        Just("// c\n".to_owned()),
        Just("\\\n".to_owned()),
        Just("\n".to_owned()),
        "[ \ta-zA-Z0-9_;=(),\\[\\]{}]{0,12}",
    ];
    prop::collection::vec(fragment, 0..40).prop_map(|parts| parts.concat())
}

proptest! {
    #[test]
    fn never_panics_and_maps_every_line(src in source()) {
        if let Ok(out) = run(&src) {
            prop_assert_eq!(out.source_map.len(), out.text.lines().count());
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic(src in "\\PC{0,200}") {
        let _ = run(&src);
    }

    #[test]
    fn plain_code_is_kept_except_indentation(lines in prop::collection::vec("[ \t]{0,3}[a-z0-9_;=\\[\\],{}+ ]{0,20}", 0..10)) {
        let src = lines.join("\n");
        let expected: Vec<&str> = lines.iter().map(|l| l.trim_start_matches([' ', '\t'])).collect();
        prop_assert_eq!(run(&src).unwrap().text, expected.join("\n"));
    }

    #[test]
    fn preprocessing_twice_changes_nothing_more(src in source()) {
        // Indentation after the end of a multi-line comment survives the first pass (the line
        // started inside the comment) but not the second, so compare without indentation.
        let unindent = |text: &str| -> Vec<String> {
            text.split('\n').map(|l| l.trim_start_matches([' ', '\t']).to_owned()).collect()
        };
        if let Ok(once) = run(&src) {
            if !once.text.lines().any(|l| l.trim_start().starts_with('#')) {
                let twice = run(&once.text).unwrap();
                prop_assert_eq!(unindent(&twice.text), unindent(&once.text));
            }
        }
    }
}
