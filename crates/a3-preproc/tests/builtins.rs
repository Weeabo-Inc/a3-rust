//! Built-in macros, with an injected clock and random source.

mod common;
use a3_preproc::{
    DateTime, FixedClock, GameVersion, MemoryResolver, Now, Options, Preprocessor, SplitMix64,
};
use common::pp;

fn now() -> Now {
    // wiki example instant: local 2020-10-28 15:17:42 (UTC+1), UTC 14:17:42.
    Now {
        local: DateTime {
            year: 2020,
            month: 10,
            day: 28,
            hour: 15,
            minute: 17,
            second: 42,
        },
        utc: DateTime {
            year: 2020,
            month: 10,
            day: 28,
            hour: 14,
            minute: 17,
            second: 42,
        },
        unix: 1_603_894_662,
    }
}

fn run(source: &str) -> String {
    let files = MemoryResolver::new();
    Preprocessor::new(&files)
        .with_clock(FixedClock(now()))
        .with_random(SplitMix64::new(1))
        .with_options(Options {
            game_version: GameVersion {
                major: 2,
                minor: 0,
                build: 146_790,
            },
            ..Options::default()
        })
        .preprocess_str("\\userconfig\\file1.sqf", source)
        .unwrap()
        .text
}

#[test]
fn date_and_time_macros_match_wiki_formats() {
    assert_eq!(run("__DATE_ARR__"), "2020,10,28,15,17,42");
    assert_eq!(run("__DATE_STR__"), "\"2020/10/28, 15:17:42\"");
    assert_eq!(run("__DATE_STR_ISO8601__"), "\"2020-10-28T14:17:42Z\"");
    assert_eq!(run("__TIME__"), "15:17:42");
    assert_eq!(run("__TIME_UTC__"), "14:17:42");
    assert_eq!(run("__DAY__ __MONTH__ __YEAR__"), "28 10 2020");
    assert_eq!(run("__TIMESTAMP_UTC__"), "1603894662");
}

#[test]
fn game_version_macros_match_wiki_formats() {
    assert_eq!(run("__GAME_VER__"), "02.00.146790");
    assert_eq!(run("__GAME_VER_MAJ__"), "02");
    assert_eq!(run("__GAME_VER_MIN__"), "00");
    assert_eq!(run("__GAME_BUILD__"), "146790");
}

#[test]
fn default_game_version_is_2_22() {
    assert_eq!(pp("__GAME_VER__"), "02.22.154103");
}

#[test]
fn arma_flags() {
    assert_eq!(pp("__ARMA__ __ARMA3__"), "1 1");
    assert_eq!(pp("__A3_DEBUG__"), "__A3_DEBUG__");
}

#[test]
fn file_macros() {
    // wiki: "userconfig\file1.sqf", "file1.sqf", "file1"
    assert_eq!(run("__FILE__"), "\"userconfig\\file1.sqf\"");
    assert_eq!(run("__FILE_NAME__"), "\"file1.sqf\"");
    assert_eq!(run("__FILE_SHORT__"), "\"file1\"");
}

#[test]
fn file_macros_name_the_included_file() {
    let files =
        MemoryResolver::new().with_file("\\a\\b\\inc.test.hpp", "__FILE_SHORT__ __LINE__\n");
    let out = Preprocessor::new(&files)
        .preprocess_str(
            "\\a\\main.sqf",
            "\n#include \"b\\inc.test.hpp\"\n__FILE_NAME__ __LINE__",
        )
        .unwrap();
    assert_eq!(out.text, "\n\"inc.test\" 1\n\n\"main.sqf\" 3");
}

#[test]
fn line_counts_physical_lines() {
    assert_eq!(pp("__LINE__\n\n#define L __LINE__\nL"), "1\n\n\n4");
}

#[test]
fn line_inside_multi_line_call_is_the_call_line() {
    assert_eq!(pp("#define F(a) a\nF(__LINE__\n)\n__LINE__"), "\n2\n\n4");
}

#[test]
fn counter_increments_and_resets() {
    // wiki example
    assert_eq!(
        pp(
            "__COUNTER__ __COUNTER__ __COUNTER__ __COUNTER__\n__COUNTER_RESET__\n__COUNTER__ __COUNTER__"
        ),
        "0 1 2 3\n\n0 1"
    );
}

#[test]
fn random_macros_fit_their_range() {
    let text = run(
        "__RAND_INT8__ __RAND_INT16__ __RAND_INT32__ __RAND_INT64__ __RAND_UINT8__ __RAND_UINT16__ __RAND_UINT32__ __RAND_UINT64__",
    );
    let values: Vec<&str> = text.split(' ').collect();
    assert_eq!(values.len(), 8);
    assert!(values[0].parse::<i8>().is_ok());
    assert!(values[1].parse::<i16>().is_ok());
    assert!(values[2].parse::<i32>().is_ok());
    assert!(values[3].parse::<i64>().is_ok());
    assert!(values[4].parse::<u8>().is_ok());
    assert!(values[5].parse::<u16>().is_ok());
    assert!(values[6].parse::<u32>().is_ok());
    assert!(values[7].parse::<u64>().is_ok());
}

#[test]
fn random_macros_are_deterministic_for_a_seed() {
    assert_eq!(
        run("__RAND_UINT32__ __RAND_UINT32__"),
        run("__RAND_UINT32__ __RAND_UINT32__")
    );
    let two = run("__RAND_UINT64__ __RAND_UINT64__");
    let (a, b) = two.split_once(' ').unwrap();
    assert_ne!(a, b);
}

#[test]
fn builtins_are_not_expanded_in_strings() {
    assert_eq!(pp("\"__LINE__\" '__LINE__'"), "\"__LINE__\" '1'");
}

#[test]
fn builtins_can_be_stringified_through_a_macro() {
    assert_eq!(pp("#define Q(x) #x\nQ(__LINE__)"), "\n\"2\"");
}
