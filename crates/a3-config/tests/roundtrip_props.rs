//! Property tests: random config trees survive rapify/derap and print/parse.

use a3_config::{
    Config, ConfigClass, Entry, EntryKind, EnumEntry, Value, parse_text, read_rap, write_rap,
    write_text,
};
use proptest::prelude::*;

fn name() -> impl Strategy<Value = String> {
    "[A-Za-z_][A-Za-z0-9_]{0,8}".prop_filter("keyword", |s| {
        !matches!(s.as_str(), "class" | "delete" | "enum")
    })
}

/// Scalars both encodings represent exactly: finite floats, `Int64` outside the i32 range (an
/// in-range literal reads back as `Int`), and strings without NUL.
fn scalar(with_expressions: bool) -> BoxedStrategy<Value> {
    let mut options = vec![
        "[^\u{0}]{0,12}".prop_map(Value::String).boxed(),
        any::<f32>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(Value::Float)
            .boxed(),
        any::<i32>().prop_map(Value::Int).boxed(),
        prop_oneof![
            i64::MIN..i64::from(i32::MIN),
            i64::from(i32::MAX) + 1..=i64::MAX
        ]
        .prop_map(Value::Int64)
        .boxed(),
    ];
    if with_expressions {
        options.push("[^\u{0}]{0,8}".prop_map(Value::Expression).boxed());
    }
    proptest::strategy::Union::new(options).boxed()
}

fn array(with_expressions: bool) -> impl Strategy<Value = Vec<Value>> {
    let leaf = scalar(with_expressions);
    let value = leaf.prop_recursive(3, 24, 6, |inner| {
        prop::collection::vec(inner, 0..6).prop_map(Value::Array)
    });
    prop::collection::vec(value, 0..6)
}

fn class(with_expressions: bool) -> impl Strategy<Value = ConfigClass> {
    let leaf_entry = prop_oneof![
        (name(), scalar(with_expressions)).prop_map(|(n, v)| Entry::value(n, v)),
        (name(), array(with_expressions)).prop_map(|(n, a)| Entry::value(n, Value::Array(a))),
        (name(), array(with_expressions))
            .prop_map(|(n, a)| Entry::new(n, EntryKind::ArrayAppend(a))),
        name().prop_map(|n| Entry::new(n, EntryKind::External)),
        name().prop_map(|n| Entry::new(n, EntryKind::Delete)),
    ];
    let leaf_class = (
        proptest::option::of(name()),
        prop::collection::vec(leaf_entry, 0..6),
    )
        .prop_map(|(base, entries)| ConfigClass { base, entries });
    leaf_class.prop_recursive(4, 64, 6, move |inner| {
        (
            proptest::option::of(name()),
            prop::collection::vec(
                prop_oneof![
                    (name(), inner.clone()).prop_map(|(n, c)| Entry::class(n, c)),
                    (name(), scalar(with_expressions)).prop_map(|(n, v)| Entry::value(n, v)),
                ],
                0..6,
            ),
        )
            .prop_map(|(base, entries)| ConfigClass { base, entries })
    })
}

fn config(with_expressions: bool) -> impl Strategy<Value = Config> {
    (
        class(with_expressions),
        prop::collection::vec(
            (name(), any::<i32>()).prop_map(|(name, value)| EnumEntry { name, value }),
            0..4,
        ),
    )
        .prop_map(|(mut root, enums)| {
            root.base = None;
            Config { root, enums }
        })
}

proptest! {
    #[test]
    fn rapify_then_derap_is_identity(cfg in config(true)) {
        let bytes = write_rap(&cfg);
        prop_assert_eq!(read_rap(&bytes).unwrap(), cfg);
    }

    #[test]
    fn rap_writer_is_stable(cfg in config(true)) {
        let bytes = write_rap(&cfg);
        prop_assert_eq!(write_rap(&read_rap(&bytes).unwrap()), bytes);
    }

    #[test]
    fn print_then_parse_is_identity(cfg in config(false)) {
        let text = write_text(&cfg);
        let parsed = parse_text(&text).map_err(|e| TestCaseError::fail(format!("{e}\n{text}")))?;
        prop_assert_eq!(parsed, cfg);
    }

    #[test]
    fn rap_reader_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
        let mut input = b"\0raP".to_vec();
        input.extend(bytes);
        let _ = read_rap(&input);
    }

    #[test]
    fn text_parser_never_panics(src in "[ -~\n]{0,200}") {
        let _ = parse_text(&src);
    }
}
