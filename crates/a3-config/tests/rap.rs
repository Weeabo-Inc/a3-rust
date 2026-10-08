//! Rapified (binary `raP`) config reading and writing.

use a3_config::{Config, ConfigClass, Entry, EntryKind, EnumEntry, Value, read_rap, write_rap};

/// `x = 5; class A { s = "hi"; };` rapified by hand, laid out as BI's tools do: each class body
/// is followed by a u32 end-of-subtree offset, then the bodies of its child classes.
fn small_fixture() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"\0raP");
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&8u32.to_le_bytes());
    b.extend_from_slice(&50u32.to_le_bytes()); // enum table offset
    // root body @16
    b.push(0); // no base
    b.push(2); // entry count
    b.extend_from_slice(&[1, 2, b'x', 0, 5, 0, 0, 0]); // x = 5 (int)
    b.extend_from_slice(&[0, b'A', 0]); // class A
    b.extend_from_slice(&37u32.to_le_bytes()); // body of A
    b.extend_from_slice(&50u32.to_le_bytes()); // end of root subtree
    // body of A @37
    b.push(0);
    b.push(1);
    b.extend_from_slice(&[1, 0, b's', 0, b'h', b'i', 0]); // s = "hi"
    b.extend_from_slice(&50u32.to_le_bytes());
    // enum table @50
    b.extend_from_slice(&0u32.to_le_bytes());
    b
}

fn small_config() -> Config {
    Config {
        root: ConfigClass::new(
            None,
            vec![
                Entry::value("x", Value::Int(5)),
                Entry::class(
                    "A",
                    ConfigClass::new(None, vec![Entry::value("s", Value::String("hi".into()))]),
                ),
            ],
        ),
        enums: vec![],
    }
}

#[test]
fn reads_hand_built_rap() {
    assert_eq!(read_rap(&small_fixture()).unwrap(), small_config());
}

#[test]
fn writes_bi_layout_byte_for_byte() {
    assert_eq!(write_rap(&small_config()), small_fixture());
}

#[test]
fn rejects_missing_magic() {
    let mut bytes = small_fixture();
    bytes[1] = b'X';
    assert!(read_rap(&bytes).is_err());
}

#[test]
fn truncated_input_is_an_error_not_a_panic() {
    let bytes = small_fixture();
    for len in 0..bytes.len() - 4 {
        assert!(read_rap(&bytes[..len]).is_err(), "len {len}");
    }
}

#[test]
fn round_trips_every_entry_kind() {
    let config = Config {
        root: ConfigClass::new(
            None,
            vec![
                Entry::new("Ext", EntryKind::External),
                Entry::new("Gone", EntryKind::Delete),
                Entry::class(
                    "Derived",
                    ConfigClass::new(
                        Some("Ext"),
                        vec![
                            Entry::value("f", Value::Float(1.5)),
                            Entry::value("i64", Value::Int64(1 << 40)),
                            Entry::value("e", Value::Expression("1+1".into())),
                            Entry::value(
                                "arr",
                                Value::Array(vec![
                                    Value::Int(1),
                                    Value::String("two".into()),
                                    Value::Array(vec![Value::Float(-0.25), Value::Int64(-1 << 40)]),
                                    Value::Expression("x".into()),
                                ]),
                            ),
                            Entry::new("more", EntryKind::ArrayAppend(vec![Value::Int(3)])),
                            Entry::class("Inner", ConfigClass::default()),
                        ],
                    ),
                ),
            ],
        ),
        enums: vec![
            EnumEntry {
                name: "ONE".into(),
                value: 1,
            },
            EnumEntry {
                name: "BIG".into(),
                value: -7,
            },
        ],
    };
    let bytes = write_rap(&config);
    assert_eq!(read_rap(&bytes).unwrap(), config);
}

#[test]
fn non_utf8_strings_decode_as_windows_1252() {
    let mut bytes = small_fixture();
    // "hi" -> "h\x97" (Windows-1252 em dash)
    bytes[44] = 0x97;
    let config = read_rap(&bytes).unwrap();
    let a = config.root.class("A").unwrap();
    assert_eq!(
        a.get("s").unwrap().kind,
        EntryKind::Value(Value::String("h\u{2014}".into()))
    );
}

#[test]
fn enum_table_shorter_than_its_count_is_tolerated() {
    let config = Config {
        enums: vec![EnumEntry {
            name: "A".into(),
            value: 3,
        }],
        ..small_config()
    };
    let mut bytes = write_rap(&config);
    bytes[50] = 5; // claim 5 constants; only 1 follows
    assert_eq!(read_rap(&bytes).unwrap(), config);
}

#[test]
fn compressed_entry_count_uses_7_bit_groups() {
    // 200 entries need a two-byte count: 0xC8 0x01.
    let entries = (0..200)
        .map(|i| Entry::value(format!("v{i}"), Value::Int(i)))
        .collect();
    let config = Config {
        root: ConfigClass::new(None, entries),
        enums: vec![],
    };
    let bytes = write_rap(&config);
    assert_eq!(&bytes[16..19], &[0, 0xC8, 0x01]);
    assert_eq!(read_rap(&bytes).unwrap(), config);
}

#[test]
fn class_body_referenced_twice_is_rejected() {
    // Two class entries sharing one body would let a tiny file expand exponentially.
    let mut b = Vec::new();
    b.extend_from_slice(b"\0raP");
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&8u32.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&[0, 2]); // root @16: no base, 2 entries
    b.extend_from_slice(&[0, b'A', 0]);
    b.extend_from_slice(&36u32.to_le_bytes());
    b.extend_from_slice(&[0, b'B', 0]);
    b.extend_from_slice(&36u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]); // empty body @36
    assert!(read_rap(&b).is_err());
}
