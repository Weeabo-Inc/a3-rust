//! `stringtable.xml` parsing and the per-key language choice.

use a3_stringtable::{Error, Stringtable};

const TABLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<Project name="Test">
  <Package name="Main">
    <Container name="Weapons">
      <Key ID="STR_Rifle">
        <Original>Rifle</Original>
        <English>Rifle (EN)</English>
        <German>Gewehr</German>
        <Czech>Puška</Czech>
      </Key>
      <Key ID="str_only_english">
        <English>Only English</English>
        <French>Seulement anglais</French>
      </Key>
    </Container>
    <Key ID="STR_no_container">
      <Polish>Tylko polski</Polish>
      <Russian>Только русский</Russian>
    </Key>
    <Text ID="STR_legacy_text"><English>Old &lt;br /&gt; style</English></Text>
  </Package>
</Project>
"#;

fn table() -> Stringtable {
    Stringtable::from_xml(TABLE).unwrap()
}

#[test]
fn reads_keys_in_any_nesting_including_legacy_text_elements() {
    let table = table();
    let ids: Vec<&str> = table.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(
        ids,
        [
            "STR_Rifle",
            "str_only_english",
            "STR_no_container",
            "STR_legacy_text"
        ]
    );
}

#[test]
fn keeps_every_translation_in_file_order_with_entities_decoded() {
    let table = table();
    let rifle = &table.entries[0];
    let langs: Vec<&str> = rifle.translations.iter().map(|(l, _)| l.as_str()).collect();
    assert_eq!(langs, ["Original", "English", "German", "Czech"]);
    assert_eq!(rifle.get("czech"), Some("Puška"));
    assert_eq!(table.entries[3].get("English"), Some("Old <br /> style"));
}

#[test]
fn prefers_the_selected_language_then_original_then_english_then_the_first() {
    let table = table();
    let [rifle, only_english, no_container, _] = &table.entries[..] else {
        panic!("four entries expected");
    };
    assert_eq!(rifle.resolve("German"), Some("Gewehr"));
    assert_eq!(
        rifle.resolve("French"),
        Some("Rifle"),
        "Original beats English"
    );
    assert_eq!(only_english.resolve("Japanese"), Some("Only English"));
    assert_eq!(
        no_container.resolve("English"),
        Some("Tylko polski"),
        "first"
    );
}

#[test]
fn lists_the_languages_used() {
    let table = table();
    assert_eq!(
        table.languages(),
        [
            "Original", "English", "German", "Czech", "French", "Polish", "Russian"
        ]
    );
}

#[test]
fn accepts_a_byte_order_mark() {
    let with_bom = format!("\u{feff}{TABLE}");
    assert_eq!(
        Stringtable::read(with_bom.as_bytes())
            .unwrap()
            .entries
            .len(),
        4
    );
}

#[test]
fn reports_malformed_xml() {
    let err =
        Stringtable::from_xml("<Project><Key ID=\"a\"><English>x</Key></Project>").unwrap_err();
    assert!(matches!(err, Error::Xml(_)), "{err:?}");
}
