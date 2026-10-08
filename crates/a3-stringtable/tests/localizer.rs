//! Merging stringtables and the `localize` lookup rules.

use a3_stringtable::{Localizer, Stringtable};

fn xml(keys: &[(&str, &[(&str, &str)])]) -> Stringtable {
    let mut text = String::from("<Project><Package>");
    for (id, translations) in keys {
        text += &format!("<Key ID=\"{id}\">");
        for (lang, value) in *translations {
            text += &format!("<{lang}>{value}</{lang}>");
        }
        text += "</Key>";
    }
    text += "</Package></Project>";
    Stringtable::from_xml(&text).unwrap()
}

#[test]
fn english_is_the_default_language() {
    let mut loc = Localizer::default();
    loc.add(&xml(&[(
        "STR_hello",
        &[("English", "Hello"), ("German", "Hallo")],
    )]));
    assert_eq!(loc.language(), "English");
    assert_eq!(loc.get("STR_hello"), Some("Hello"));
}

#[test]
fn keys_are_case_insensitive() {
    let mut loc = Localizer::new("German");
    loc.add(&xml(&[(
        "STR_Hello",
        &[("English", "Hello"), ("German", "Hallo")],
    )]));
    assert_eq!(loc.get("str_HELLO"), Some("Hallo"));
    assert!(loc.contains("STR_hello"));
    assert_eq!(loc.len(), 1);
}

#[test]
fn the_first_registration_of_a_key_wins_and_duplicates_are_reported() {
    let mut loc = Localizer::default();
    assert!(
        loc.add(&xml(&[("STR_a", &[("English", "first")])]))
            .is_empty()
    );
    let duplicates = loc.add(&xml(&[
        ("str_A", &[("English", "second")]),
        ("STR_b", &[("English", "b")]),
    ]));
    assert_eq!(duplicates, ["str_A"]);
    assert_eq!(loc.get("STR_a"), Some("first"));
    assert_eq!(loc.get("STR_b"), Some("b"));
}

#[test]
fn localize_follows_the_script_command() {
    let mut loc = Localizer::default();
    loc.add(&xml(&[
        ("STR_a", &[("English", "Alpha")]),
        ("not_str_key", &[("English", "Plain")]),
    ]));
    assert_eq!(loc.localize("STR_a"), "Alpha");
    assert_eq!(loc.localize("$STR_a"), "Alpha", "a $STR prefix is dropped");
    assert_eq!(loc.localize("not_str_key"), "Plain", "any key works");
    assert_eq!(
        loc.localize("$not_str_key"),
        "",
        "$ only stripped before STR"
    );
    assert_eq!(loc.localize("STR_missing"), "");
    assert_eq!(loc.localize(""), "");
}

#[test]
fn config_text_resolves_only_dollar_str_references() {
    let mut loc = Localizer::default();
    loc.add(&xml(&[("STR_a", &[("English", "Alpha")])]));
    assert_eq!(loc.config_text("$STR_a"), "Alpha");
    assert_eq!(loc.config_text("$str_A"), "Alpha");
    assert_eq!(loc.config_text("STR_a"), "STR_a", "no $: literal text");
    assert_eq!(
        loc.config_text("$STR_missing"),
        "",
        "the lookup's not-found result"
    );
    assert_eq!(loc.config_text("Plain text"), "Plain text");
}
