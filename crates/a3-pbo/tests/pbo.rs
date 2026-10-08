//! Behaviour of the PBO reader and writer, on synthetic archives built by the writer.

use a3_pbo::{Error, PackingMethod, Pbo, PboWriter};

fn sample() -> Vec<u8> {
    PboWriter::new()
        .property("prefix", r"a3\test_f")
        .property("product", "Arma 3")
        .file(r"config.bin", b"raP-config".to_vec())
        .file(r"Data\Tex_CO.paa", vec![7u8; 1000])
        .to_bytes()
}

/// A one-file PBO assembled by hand from the format description; the digest was computed
/// independently with Python's hashlib.
fn hand_made() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"\0sreV");
    b.extend_from_slice(&[0; 16]);
    b.extend_from_slice(b"prefix\0p\0\0");
    b.extend_from_slice(b"a\0");
    for field in [0u32, 0, 0, 5, 2] {
        b.extend_from_slice(&field.to_le_bytes());
    }
    b.extend_from_slice(&[0; 21]);
    b.extend_from_slice(b"xy");
    b.push(0);
    b.extend_from_slice(&hex("a91dac71e2be0112fc7544dbfbdf94660fdd7df1"));
    b
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn writer_output_matches_the_documented_layout() {
    let written = PboWriter::new()
        .property("prefix", "p")
        .file_with_timestamp("a", b"xy".to_vec(), 5)
        .to_bytes();
    assert_eq!(written, hand_made());
}

#[test]
fn parses_a_hand_made_archive() {
    let pbo = Pbo::from_bytes(hand_made()).unwrap();

    assert_eq!(pbo.properties().get("prefix"), Some("p"));
    let entry = &pbo.entries()[0];
    assert_eq!(entry.name(), "a");
    assert_eq!(entry.method(), PackingMethod::Stored);
    assert_eq!(entry.timestamp(), 5);
    assert_eq!(entry.data_size(), 2);
    assert_eq!(entry.data_offset(), 74);
    assert_eq!(&pbo.read("A").unwrap()[..], b"xy");
    assert_eq!(
        pbo.stored_hash().unwrap().to_vec(),
        hex("a91dac71e2be0112fc7544dbfbdf94660fdd7df1")
    );
    pbo.verify().unwrap();
}

#[test]
fn reads_back_files_written_by_the_writer() {
    let pbo = Pbo::from_bytes(sample()).unwrap();

    let names: Vec<_> = pbo.entries().iter().map(|e| e.name()).collect();
    assert_eq!(names, ["config.bin", r"Data\Tex_CO.paa"]);
    assert_eq!(&pbo.read("config.bin").unwrap()[..], b"raP-config");
    assert_eq!(pbo.read(r"data\tex_co.paa").unwrap().len(), 1000);
}

#[test]
fn exposes_properties_and_the_normalised_prefix() {
    let pbo = Pbo::from_bytes(sample()).unwrap();

    let props: Vec<_> = pbo.properties().iter().collect();
    assert_eq!(props, [("prefix", r"a3\test_f"), ("product", "Arma 3")]);
    assert_eq!(pbo.prefix().unwrap().as_str(), r"a3\test_f");
}

#[test]
fn a_pbo_without_properties_has_no_prefix() {
    let pbo = Pbo::from_bytes(PboWriter::new().file("x.txt", b"x".to_vec()).to_bytes()).unwrap();
    assert!(pbo.properties().is_empty());
    assert_eq!(pbo.prefix(), None);
    assert_eq!(&pbo.read("x.txt").unwrap()[..], b"x");
}

#[test]
fn lookup_ignores_case_and_separator_style() {
    let pbo = Pbo::from_bytes(sample()).unwrap();
    assert!(pbo.entry("DATA/tex_co.PAA").is_some());
    assert!(pbo.entry(r"\data\tex_co.paa").is_some());
    assert!(pbo.entry("missing.paa").is_none());
    assert!(matches!(pbo.read("missing.paa"), Err(Error::NotFound(_))));
}

#[test]
fn verify_detects_a_modified_archive() {
    let mut bytes = sample();
    let pos = bytes.len() - 30;
    bytes[pos] ^= 0xff;
    let pbo = Pbo::from_bytes(bytes).unwrap();
    assert!(matches!(pbo.verify(), Err(Error::HashMismatch { .. })));
}

#[test]
fn an_archive_without_trailer_still_reads_but_cannot_be_verified() {
    let mut bytes = sample();
    bytes.truncate(bytes.len() - 21);
    let pbo = Pbo::from_bytes(bytes).unwrap();
    assert_eq!(pbo.stored_hash(), None);
    assert!(matches!(pbo.verify(), Err(Error::MissingHash)));
    assert_eq!(&pbo.read("config.bin").unwrap()[..], b"raP-config");
}

#[test]
fn a_truncated_header_is_an_error() {
    let bytes = sample();
    assert!(matches!(
        Pbo::from_bytes(bytes[..40].to_vec()),
        Err(Error::Truncated { .. })
    ));
    assert!(matches!(
        Pbo::from_bytes(Vec::new()),
        Err(Error::Truncated { .. })
    ));
}

#[test]
fn entry_data_past_the_end_of_file_is_malformed() {
    let bytes = sample();
    let cut = bytes.len() - 21 - 500;
    assert!(matches!(
        Pbo::from_bytes(bytes[..cut].to_vec()),
        Err(Error::Malformed(_))
    ));
}

/// Replaces the packing method of the first file record (named `name`) in a written PBO.
fn set_method(bytes: &mut [u8], name: &str, method: u32) {
    let needle = [name.as_bytes(), b"\0"].concat();
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .unwrap()
        + needle.len();
    bytes[pos..pos + 4].copy_from_slice(&method.to_le_bytes());
}

#[test]
fn compressed_entries_are_reported_unsupported() {
    let mut bytes = sample();
    set_method(&mut bytes, "config.bin", u32::from_le_bytes(*b"srpC"));
    let pbo = Pbo::from_bytes(bytes).unwrap();

    assert_eq!(
        pbo.entry("config.bin").unwrap().method(),
        PackingMethod::Compressed
    );
    assert!(matches!(
        pbo.read("config.bin"),
        Err(Error::Unsupported {
            method: PackingMethod::Compressed,
            ..
        })
    ));
    assert_eq!(
        &pbo.raw(pbo.entry("config.bin").unwrap())[..],
        b"raP-config"
    );
}

#[test]
fn encrypted_entries_are_reported_encrypted() {
    let mut bytes = sample();
    set_method(&mut bytes, "config.bin", u32::from_le_bytes(*b"ocnE"));
    let pbo = Pbo::from_bytes(bytes).unwrap();
    assert_eq!(
        pbo.entry("config.bin").unwrap().method(),
        PackingMethod::Encrypted
    );
    assert!(matches!(pbo.read("config.bin"), Err(Error::Encrypted)));
}

#[test]
fn duplicate_names_resolve_to_the_first_entry() {
    let bytes = PboWriter::new()
        .file("a.txt", b"first".to_vec())
        .file("A.TXT", b"second".to_vec())
        .to_bytes();
    let pbo = Pbo::from_bytes(bytes).unwrap();
    assert_eq!(pbo.entries().len(), 2);
    assert_eq!(&pbo.read("a.txt").unwrap()[..], b"first");
}

#[test]
fn opens_a_pbo_file_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_f.pbo");
    std::fs::write(&path, sample()).unwrap();

    let pbo = Pbo::open(&path).unwrap();
    assert_eq!(&pbo.read("config.bin").unwrap()[..], b"raP-config");
    pbo.verify().unwrap();
    assert_eq!(a3_pbo::read_properties(&path).unwrap(), *pbo.properties());
}

#[test]
fn an_ebo_is_encrypted_but_its_properties_are_readable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret.ebo");
    let mut bytes = PboWriter::new().property("prefix", r"x\secret").to_bytes();
    bytes.extend_from_slice(&[0xa5; 64]); // stands in for the encrypted header
    std::fs::write(&path, bytes).unwrap();

    assert!(matches!(Pbo::open(&path), Err(Error::Encrypted)));
    let props = a3_pbo::read_properties(&path).unwrap();
    assert_eq!(props.prefix().unwrap().as_str(), r"x\secret");
}

#[test]
fn opening_an_empty_file_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.pbo");
    std::fs::write(&path, b"").unwrap();
    assert!(matches!(Pbo::open(&path), Err(Error::Truncated { .. })));
}

#[test]
fn writer_rejects_invalid_names() {
    let mut out = Vec::new();
    let err = PboWriter::new()
        .file("", b"x".to_vec())
        .write_to(&mut out)
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let err = PboWriter::new()
        .file("a\0b", b"x".to_vec())
        .write_to(&mut out)
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

proptest::proptest! {
    #[test]
    fn writer_and_reader_round_trip(
        files in proptest::collection::btree_map("[a-z]{1,8}(\\\\[a-z0-9_]{1,8}){0,3}\\.[a-z]{1,3}",
            proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300), 0..12),
        prefix in "[a-z]{1,6}(\\\\[a-z_]{1,6}){0,2}",
    ) {
        let mut writer = PboWriter::new().property("prefix", prefix.clone());
        for (name, data) in &files {
            writer = writer.file(name.clone(), data.clone());
        }
        let pbo = Pbo::from_bytes(writer.to_bytes()).unwrap();

        proptest::prop_assert_eq!(pbo.prefix().unwrap().into_string(), prefix);
        proptest::prop_assert_eq!(pbo.entries().len(), files.len());
        for (name, data) in &files {
            proptest::prop_assert_eq!(&pbo.read(name).unwrap()[..], &data[..]);
        }
        proptest::prop_assert!(pbo.verify().is_ok());
    }
}
