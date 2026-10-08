//! Binarized `stringtable.bin` (`BLMX`).

use a3_stringtable::{Entry, Error, Stringtable};
use proptest::prelude::*;

fn cstr(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

/// Two languages, two keys; the English column comes first in the file.
fn hand_built() -> Vec<u8> {
    let mut out = b"BLMX".to_vec();
    out.extend_from_slice(&2u32.to_le_bytes());
    cstr(&mut out, "English");
    cstr(&mut out, "German");
    out.extend_from_slice(&2u32.to_le_bytes());
    let offsets_at = out.len();
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&2u32.to_le_bytes());
    cstr(&mut out, "STR_a");
    cstr(&mut out, "STR_b");
    let mut columns = Vec::new();
    for column in [["A", "B"], ["Ä", "Bé"]] {
        columns.push(out.len() as i32);
        out.extend_from_slice(&2u32.to_le_bytes());
        for text in column {
            cstr(&mut out, text);
        }
    }
    for (i, offset) in columns.iter().enumerate() {
        out[offsets_at + 4 * i..offsets_at + 4 * i + 4].copy_from_slice(&offset.to_le_bytes());
    }
    out
}

#[test]
fn reads_languages_keys_and_per_language_columns() {
    let table = Stringtable::read(&hand_built()).unwrap();
    assert_eq!(table.languages(), ["English", "German"]);
    assert_eq!(table.entries.len(), 2);
    assert_eq!(table.entries[0].key, "STR_a");
    assert_eq!(table.entries[1].get("german"), Some("Bé"));
    assert_eq!(table.entries[0].resolve("Czech"), Some("A"));
}

#[test]
fn rejects_a_column_with_the_wrong_number_of_texts() {
    let mut bytes = hand_built();
    // Shorten the key list to one key; the columns still hold two texts.
    let keys_at = bytes.windows(5).position(|w| w == b"STR_a").unwrap() - 4;
    bytes[keys_at] = 1;
    let err = Stringtable::read(&bytes).unwrap_err();
    assert!(matches!(err, Error::Malformed(_)), "{err:?}");
}

#[test]
fn rejects_truncated_files() {
    let bytes = hand_built();
    for len in 4..bytes.len() {
        assert!(Stringtable::read(&bytes[..len]).is_err(), "length {len}");
    }
}

fn arb_table() -> impl Strategy<Value = Stringtable> {
    let languages =
        prop::sample::subsequence(vec!["Original", "English", "German", "Czech"], 1..=4);
    (
        languages,
        prop::collection::vec("[A-Za-z0-9_ ]{0,12}", 0..6),
    )
        .prop_flat_map(|(languages, keys)| {
            let cells = languages.len() * keys.len();
            prop::collection::vec("[^\u{0}]{0,10}", cells).prop_map(move |texts| {
                let mut texts = texts.into_iter();
                let entries = keys
                    .iter()
                    .enumerate()
                    .map(|(i, key)| Entry {
                        key: format!("STR_{i}_{key}"),
                        translations: languages
                            .iter()
                            .map(|l| (l.to_string(), texts.next().unwrap()))
                            .collect(),
                    })
                    .collect();
                Stringtable { entries }
            })
        })
}

proptest! {
    #[test]
    fn bin_round_trips(table in arb_table()) {
        prop_assume!(!table.entries.is_empty());
        let bytes = table.to_bin();
        prop_assert_eq!(Stringtable::read(&bytes).unwrap(), table);
    }
}
