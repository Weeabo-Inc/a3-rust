//! config.cpp text: parsing preprocessed source and pretty-printing it back.

use a3_config::{Config, ConfigClass, Entry, EntryKind, EnumEntry, Value, parse_text, write_text};

fn parse(src: &str) -> Config {
    parse_text(src).unwrap_or_else(|e| panic!("{e}"))
}

fn root_value(src: &str, name: &str) -> Value {
    match &parse(src).root.get(name).expect("entry").kind {
        EntryKind::Value(v) => v.clone(),
        other => panic!("not a value: {other:?}"),
    }
}

#[test]
fn parses_classes_values_and_arrays() {
    let config = parse(
        r#"
        class CfgPatches {
            class My_Addon {
                units[] = {"B_Soldier_F", "B_Soldier_2"};
                requiredVersion = 0.1;
                requiredAddons[] = {};
            };
        };
        class CfgVehicles {
            class Man;
            class B_Soldier_F: Man {
                displayName = "Rifleman";
                scope = 2;
            };
            delete Old;
        };
        "#,
    );
    let expected = Config {
        root: ConfigClass::new(
            None,
            vec![
                Entry::class(
                    "CfgPatches",
                    ConfigClass::new(
                        None,
                        vec![Entry::class(
                            "My_Addon",
                            ConfigClass::new(
                                None,
                                vec![
                                    Entry::value(
                                        "units",
                                        Value::Array(vec![
                                            Value::String("B_Soldier_F".into()),
                                            Value::String("B_Soldier_2".into()),
                                        ]),
                                    ),
                                    Entry::value("requiredVersion", Value::Float(0.1)),
                                    Entry::value("requiredAddons", Value::Array(vec![])),
                                ],
                            ),
                        )],
                    ),
                ),
                Entry::class(
                    "CfgVehicles",
                    ConfigClass::new(
                        None,
                        vec![
                            Entry::new("Man", EntryKind::External),
                            Entry::class(
                                "B_Soldier_F",
                                ConfigClass::new(
                                    Some("Man"),
                                    vec![
                                        Entry::value(
                                            "displayName",
                                            Value::String("Rifleman".into()),
                                        ),
                                        Entry::value("scope", Value::Int(2)),
                                    ],
                                ),
                            ),
                            Entry::new("Old", EntryKind::Delete),
                        ],
                    ),
                ),
            ],
        ),
        enums: vec![],
    };
    assert_eq!(config, expected);
}

#[test]
fn numbers_follow_int_float_hex_rules() {
    assert_eq!(root_value("a = 42;", "a"), Value::Int(42));
    assert_eq!(root_value("a = -7;", "a"), Value::Int(-7));
    assert_eq!(root_value("a = 0x1F;", "a"), Value::Int(31));
    assert_eq!(root_value("a = 1.5;", "a"), Value::Float(1.5));
    assert_eq!(root_value("a = .5;", "a"), Value::Float(0.5));
    assert_eq!(root_value("a = 2.;", "a"), Value::Float(2.0));
    assert_eq!(root_value("a = 1e3;", "a"), Value::Float(1000.0));
    assert_eq!(root_value("a = -2.5E-2;", "a"), Value::Float(-0.025));
    assert_eq!(
        root_value("a = 3000000000;", "a"),
        Value::Int64(3_000_000_000)
    );
}

#[test]
fn unquoted_text_falls_back_to_a_string() {
    assert_eq!(
        root_value("a = some text here;", "a"),
        Value::String("some text here".into())
    );
    assert_eq!(root_value("a = true;", "a"), Value::String("true".into()));
    assert_eq!(root_value("a = 1.5f;", "a"), Value::String("1.5f".into()));
    assert_eq!(
        root_value("a[] = {abc, def ghi, 1};", "a"),
        Value::Array(vec![
            Value::String("abc".into()),
            Value::String("def ghi".into()),
            Value::Int(1),
        ])
    );
}

/// Shipped campaign descriptions (Apex, Laws of War, Contact) write `lost = ;` and `cutscene = ;`,
/// and the game loads them: an empty value is the empty string.
#[test]
fn an_empty_value_is_the_empty_string() {
    let config = parse("class Missions { cutscene = ; end1 =  ;\n lost = ; firstMission = A; };");
    let missions = match &config.root.get("Missions").expect("class").kind {
        EntryKind::Class(class) => class.clone(),
        other => panic!("not a class: {other:?}"),
    };
    for name in ["cutscene", "end1", "lost"] {
        match &missions.get(name).expect(name).kind {
            EntryKind::Value(v) => assert_eq!(*v, Value::String(String::new()), "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    assert_eq!(root_value("a = ;", "a"), Value::String(String::new()));
}

/// The 3D editor saves multi-line text (an init field, a trigger's activation) as quoted lines
/// joined by `\n` tokens: `"line 1" \n "line 2"` is one string with a line break.
#[test]
fn quoted_lines_joined_by_newline_tokens_are_one_string() {
    assert_eq!(
        root_value("a=\"x = 1; \" \\n \"y = 2;\";", "a"),
        Value::String("x = 1; \ny = 2;".into())
    );
    assert_eq!(
        root_value("a=\"one\" \\n \"\" \\n \"three\";", "a"),
        Value::String("one\n\nthree".into())
    );
    // Inside an array too.
    assert_eq!(
        root_value("a[]={\"p\" \\n \"q\", \"r\"};", "a"),
        Value::Array(vec![
            Value::String("p\nq".into()),
            Value::String("r".into())
        ])
    );
}

/// East Wind's campaign description has `add[] = { {...}, {...}; };`: a `;` where a `,` or the
/// closing `}` belongs. The game loads it; the parser takes the `;` as a separator.
#[test]
fn a_semicolon_inside_an_array_separates_items() {
    let config = parse("class C { add[] = {\n{\"a\", 1},\n{\"b\", 2};\n};\n}; class D {};");
    let c = match &config.root.get("C").expect("C").kind {
        EntryKind::Class(class) => class.clone(),
        other => panic!("{other:?}"),
    };
    match &c.get("add").expect("add").kind {
        EntryKind::Value(Value::Array(items)) => assert_eq!(items.len(), 2),
        other => panic!("{other:?}"),
    }
    assert!(config.root.get("D").is_some(), "D stays at the root");
}

#[test]
fn quoted_strings_unescape_doubled_quotes() {
    assert_eq!(
        root_value(r#"a = "say ""hi"" now";"#, "a"),
        Value::String(r#"say "hi" now"#.into())
    );
    assert_eq!(
        root_value("a = 'single';", "a"),
        Value::String("single".into())
    );
    assert_eq!(
        root_value("a = \"two\nlines\";", "a"),
        Value::String("two\nlines".into())
    );
}

#[test]
fn nested_arrays_and_append() {
    let config = parse("a[] = {1, {2, {\"x\"}}, {}}; b[] += {3, 4,};");
    assert_eq!(
        config.root.get("a").unwrap().kind,
        EntryKind::Value(Value::Array(vec![
            Value::Int(1),
            Value::Array(vec![
                Value::Int(2),
                Value::Array(vec![Value::String("x".into())])
            ]),
            Value::Array(vec![]),
        ]))
    );
    assert_eq!(
        config.root.get("b").unwrap().kind,
        EntryKind::ArrayAppend(vec![Value::Int(3), Value::Int(4)])
    );
}

#[test]
fn enums_are_collected_into_the_enum_table() {
    let config = parse("enum { A, B = 5, C };");
    assert_eq!(
        config.enums,
        vec![
            EnumEntry {
                name: "A".into(),
                value: 0
            },
            EnumEntry {
                name: "B".into(),
                value: 5
            },
            EnumEntry {
                name: "C".into(),
                value: 6
            },
        ]
    );
}

#[test]
fn tolerates_missing_semicolon_after_class_and_preprocessor_line_markers() {
    let config = parse("#line 1 \"config.cpp\"\nclass A { x = 1; }\nclass B {};");
    assert!(config.root.class("A").is_some());
    assert!(config.root.class("B").is_some());
}

#[test]
fn errors_report_line_and_column() {
    let err = parse_text("class A {\n  x = 1;\n  class : B {};\n};").unwrap_err();
    assert_eq!((err.line, err.column), (3, 9));
    assert!(err.to_string().starts_with("3:9:"), "{err}");

    let err = parse_text("class A {\n  x = 1;\n").unwrap_err();
    assert_eq!(err.line, 3);
}

#[test]
fn writes_readable_text() {
    let config = parse(
        "enum { E = 2 }; x = 5; class A: B { s = \"q\"\"q\"; f = 1; g = 2.0; a[] = {1, {\"x\"}}; b[] += {}; class E; delete D; class C {}; };",
    );
    let expected = r#"enum
{
    E = 2
};
x = 5;
class A: B
{
    s = "q""q";
    f = 1;
    g = 2.0;
    a[] = {1, {"x"}};
    b[] += {};
    class E;
    delete D;
    class C {};
};
"#;
    assert_eq!(write_text(&config), expected);
}

#[test]
fn written_text_parses_back_to_the_same_config() {
    let src = "x = -0.0; y = 1e-7; z = 16777216.0; w = 9007199254740993; s = \"a\nb\";";
    let config = parse(src);
    assert_eq!(parse(&write_text(&config)), config);
}

#[test]
fn backslash_n_between_strings_joins_them_with_a_line_break() {
    // FSM Editor output: multi-line code as quoted lines joined by `\n`, inside editor comments.
    let src = "init = /*%FSM<STATEINIT\"\"\">*/\"a = 1;\" \\n\n   \"b = 2;\" \\n \"\"/*%FSM</STATEINIT\"\"\">*/;";
    assert_eq!(
        root_value(src, "init"),
        Value::String("a = 1;\nb = 2;\n".into())
    );
    // In an array too; a string without a continuation stays as it is.
    let src = "a[] = {\"x\" \\n \"y\", \"z\"};";
    assert_eq!(
        root_value(src, "a"),
        Value::Array(vec![
            Value::String("x\ny".into()),
            Value::String("z".into())
        ])
    );
}
