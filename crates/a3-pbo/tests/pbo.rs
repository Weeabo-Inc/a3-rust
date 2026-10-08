//! Behaviour of the PBO reader and writer, on synthetic archives built by the writer.

use a3_compress::lzss::ChecksumKind;
use a3_pbo::{Error, METHOD_COMPRESSED, PackingMethod, Pbo, PboWriter};

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

/// A PBO with one entry `a` written by hand: no properties, no trailer.
fn single_raw_entry(method: u32, original_size: u32, data: &[u8]) -> Vec<u8> {
    let mut b = b"a\0".to_vec();
    for field in [method, original_size, 0, 0, data.len() as u32] {
        b.extend_from_slice(&field.to_le_bytes());
    }
    b.extend_from_slice(&[0; 21]);
    b.extend_from_slice(data);
    b
}

/// LZSS of `"\xF0\x01ABCDEF"`: one flag byte (8 literals) and the 8 bytes, then `checksum`.
fn lzss_literals(checksum: u32) -> Vec<u8> {
    let mut b = vec![0xFF, 0xF0, 0x01, b'A', b'B', b'C', b'D', b'E', b'F'];
    b.extend_from_slice(&checksum.to_le_bytes());
    b
}

const LITERALS: &[u8] = b"\xF0\x01ABCDEF";
/// Byte sum of `LITERALS` with 0xF0 read as -16.
const SIGNED_SUM: u32 = 0x186;
/// Byte sum of `LITERALS` with 0xF0 read as 240.
const UNSIGNED_SUM: u32 = 0x286;

#[test]
fn a_hand_built_cprs_entry_is_decompressed() {
    let bytes = single_raw_entry(METHOD_COMPRESSED, 8, &lzss_literals(SIGNED_SUM));
    let pbo = Pbo::from_bytes(bytes).unwrap();

    let entry = pbo.entry("a").unwrap();
    assert_eq!(entry.method(), PackingMethod::Compressed);
    assert_eq!(entry.size(), 8);
    assert_eq!(&pbo.read("a").unwrap()[..], LITERALS);
    assert_eq!(pbo.raw(entry).len(), 13);
}

#[test]
fn a_cprs_entry_with_an_unsigned_checksum_is_accepted() {
    let bytes = single_raw_entry(METHOD_COMPRESSED, 8, &lzss_literals(UNSIGNED_SUM));
    let pbo = Pbo::from_bytes(bytes).unwrap();

    assert_eq!(&pbo.read("a").unwrap()[..], LITERALS);
}

#[test]
fn a_cprs_entry_with_a_wrong_checksum_is_an_error() {
    let bytes = single_raw_entry(METHOD_COMPRESSED, 8, &lzss_literals(SIGNED_SUM + 1));
    let pbo = Pbo::from_bytes(bytes).unwrap();

    assert!(matches!(pbo.read("a"), Err(Error::Decompress { .. })));
}

#[test]
fn a_truncated_cprs_entry_is_an_error() {
    let data = lzss_literals(SIGNED_SUM);
    let bytes = single_raw_entry(METHOD_COMPRESSED, 8, &data[..data.len() - 2]);
    let pbo = Pbo::from_bytes(bytes).unwrap();

    assert!(matches!(pbo.read("a"), Err(Error::Decompress { .. })));
}

#[test]
fn compressed_files_written_by_the_writer_read_back() {
    let text = b"class CfgPatches { class A { units[] = {}; }; };\n".repeat(40);
    let bytes = PboWriter::new()
        .property("prefix", "p")
        .compressed_file("config.cpp", text.clone())
        .file("stored.txt", b"plain".to_vec())
        .to_bytes();
    let pbo = Pbo::from_bytes(bytes).unwrap();

    let entry = pbo.entry("config.cpp").unwrap();
    assert_eq!(entry.method(), PackingMethod::Compressed);
    assert_eq!(entry.original_size() as usize, text.len());
    assert!((entry.data_size() as usize) < text.len() / 4);
    assert_eq!(&pbo.read("config.cpp").unwrap()[..], &text[..]);
    assert_eq!(&pbo.read("stored.txt").unwrap()[..], b"plain");
    pbo.verify().unwrap();
}

#[test]
fn the_writer_uses_the_assumed_checksum_kind() {
    let bytes = PboWriter::new()
        .compressed_file("a", LITERALS.to_vec())
        .to_bytes();
    let pbo = Pbo::from_bytes(bytes).unwrap();

    let raw = pbo.raw(pbo.entry("a").unwrap());
    assert_eq!(a3_pbo::CPRS_CHECKSUM, ChecksumKind::Signed);
    assert_eq!(raw[raw.len() - 4..], SIGNED_SUM.to_le_bytes());
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
            (proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300), proptest::prelude::any::<bool>()), 0..12),
        prefix in "[a-z]{1,6}(\\\\[a-z_]{1,6}){0,2}",
    ) {
        let mut writer = PboWriter::new().property("prefix", prefix.clone());
        for (name, (data, compress)) in &files {
            writer = if *compress {
                writer.compressed_file(name.clone(), data.clone())
            } else {
                writer.file(name.clone(), data.clone())
            };
        }
        let pbo = Pbo::from_bytes(writer.to_bytes()).unwrap();

        proptest::prop_assert_eq!(pbo.prefix().unwrap().into_string(), prefix);
        proptest::prop_assert_eq!(pbo.entries().len(), files.len());
        for (name, (data, _)) in &files {
            proptest::prop_assert_eq!(&pbo.read(name).unwrap()[..], &data[..]);
        }
        proptest::prop_assert!(pbo.verify().is_ok());
    }
}
