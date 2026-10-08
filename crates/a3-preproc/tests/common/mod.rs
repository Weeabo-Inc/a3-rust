//! Helpers shared by the integration tests.

#![allow(dead_code)]

use a3_preproc::{MemoryResolver, Options, Output, PreprocessError, Preprocessor};

/// Preprocesses `source` as the file `\test\main.sqf` with no other files available.
pub fn pp(source: &str) -> String {
    try_pp(source).expect("preprocessing failed").text
}

/// Like [`pp`] but returns the full result.
pub fn try_pp(source: &str) -> Result<Output, PreprocessError> {
    let resolver = MemoryResolver::new();
    Preprocessor::new(&resolver).preprocess_str("\\test\\main.sqf", source)
}

/// Preprocesses `source` with the given options.
pub fn pp_with(options: Options, source: &str) -> Result<Output, PreprocessError> {
    let resolver = MemoryResolver::new();
    Preprocessor::new(&resolver)
        .with_options(options)
        .preprocess_str("\\test\\main.sqf", source)
}
