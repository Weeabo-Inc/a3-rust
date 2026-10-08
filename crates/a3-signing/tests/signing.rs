//! Keys, signatures and verification with a generated 1024-bit test key.

use a3_pbo::{Pbo, PboWriter};
use a3_signing::{
    Error, Part, PboHashes, PrivateKey, PublicKey, Signature, SignatureVersion, hashed_by_v2,
    hashed_by_v3, verify,
};
use num_bigint::BigUint;

fn test_key() -> PrivateKey {
    let hex = |s: &str| BigUint::parse_bytes(s.as_bytes(), 16).unwrap();
    let p = hex(
        "eb063397db95cfc30815e364736648600fcd0ec9395aa28725eb4cbdd3ab2442\
         646b866ef241a09a97f4e0429b0c94c00d25e3d00655dd23e4cde9b4adb4b6d3",
    );
    let q = hex(
        "e6abf783de5642702c9fa11ade9a384476597f20a39166a15d75c65968218f2d\
         7edd26ae4e3745800c2b733dd5366b300ffeba4e31d61b6ef6741bd375cdcafd",
    );
    PrivateKey::from_primes("test", p, q, 65537).unwrap()
}

fn pbo(script: &str) -> Pbo {
    let bytes = PboWriter::new()
        .property("prefix", r"test\addon")
        .file("empty.txt", Vec::new())
        .file(r"Scripts\Init.sqf", script.as_bytes().to_vec())
        .file("notes.txt", b"NOTES".to_vec())
        .file("texture.paa", b"PAADATA".to_vec())
        .to_bytes();
    Pbo::from_bytes(bytes).unwrap()
}

fn hex(bytes: &[u8; 20]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn hashes_match_independently_computed_values() {
    // Expected values computed with Python's hashlib from the PBO byte layout.
    let pbo = pbo(r#"hint "hi";"#);
    let v2 = PboHashes::of(&pbo, SignatureVersion::V2).unwrap();
    let v3 = PboHashes::of(&pbo, SignatureVersion::V3).unwrap();
    assert_eq!(hex(&v2.archive), "960e52ab678742e2c6bd93e4f58be6b76a4b659b");
    assert_eq!(hex(&v2.names), "cbce637767824d2af533a111c12f1df278764372");
    assert_eq!(hex(&v2.content), "5a2d505665c033028d88230b25e640199896e175");
    assert_eq!(v3.names, v2.names);
    assert_eq!(hex(&v3.content), "2dfd164a5370c1bbf58fa0e96bee0baf5520b59b");
}

#[test]
fn extension_lists_follow_the_version() {
    assert!(hashed_by_v2("config.cpp") && hashed_by_v2(r"a\b.sqf"));
    assert!(!hashed_by_v2(r"data\tex_co.PAA") && !hashed_by_v2("model.p3d"));
    assert!(hashed_by_v3(r"fn\fn_x.sqf") && hashed_by_v3("script_component.HPP"));
    assert!(!hashed_by_v3("config.cpp") && !hashed_by_v3("config.bin"));
    assert!(!hashed_by_v3("README"));
}

#[test]
fn a_signed_pbo_verifies() {
    let key = test_key();
    let pbo = pbo("hint 1;");
    for version in [SignatureVersion::V2, SignatureVersion::V3] {
        let signature = key.sign(&pbo, version).unwrap();
        verify(&key.public, &signature, &pbo).unwrap();
    }
}

#[test]
fn a_changed_pbo_fails_and_names_the_hash() {
    let key = test_key();
    let signature = key.sign(&pbo("hint 1;"), SignatureVersion::V3).unwrap();
    let err = verify(&key.public, &signature, &pbo("hint 2;")).unwrap_err();
    assert!(matches!(err, Error::Mismatch(Part::Archive)), "{err:?}");
}

#[test]
fn a_signature_by_another_key_is_reported() {
    let key = test_key();
    let signature = key.sign(&pbo("x"), SignatureVersion::V3).unwrap();
    let mut other = key.public.clone();
    other.authority = "other".into();
    other.modulus += 2u32;
    let err = verify(&other, &signature, &pbo("x")).unwrap_err();
    assert!(matches!(err, Error::WrongKey { .. }), "{err:?}");
}

#[test]
fn keys_and_signatures_round_trip_through_their_files() {
    let key = test_key();
    let bikey = key.public.to_bytes();
    assert_eq!(bikey.len(), 5 + 4 + 148, "name, length, 148-byte blob");
    assert_eq!(&bikey[5 + 4..5 + 4 + 8], [6, 2, 0, 0, 0, 0x24, 0, 0]);
    assert_eq!(PublicKey::read(&bikey).unwrap(), key.public);
    assert_eq!(PrivateKey::read(&key.to_bytes()).unwrap(), key);

    let signature = key.sign(&pbo("x"), SignatureVersion::V2).unwrap();
    let bisign = signature.to_bytes();
    assert_eq!(bisign.len(), bikey.len() + 3 * (4 + 128) + 4);
    assert_eq!(Signature::read(&bisign).unwrap(), signature);
}

#[test]
fn rejects_unknown_signature_versions_and_truncation() {
    let key = test_key();
    let mut bisign = key
        .sign(&pbo("x"), SignatureVersion::V3)
        .unwrap()
        .to_bytes();
    let version_at = 157 + 4 + 128;
    assert_eq!(bisign[version_at], 3);
    for len in 0..bisign.len() {
        assert!(Signature::read(&bisign[..len]).is_err(), "length {len}");
    }
    bisign[version_at] = 4;
    assert!(matches!(Signature::read(&bisign), Err(Error::Malformed(_))));
}

#[test]
fn recover_digest_rejects_bad_padding() {
    let key = test_key();
    let good = key.sign_digest(&[7; 20]);
    assert_eq!(key.public.recover_digest(&good), Some([7; 20]));
    assert_eq!(key.public.recover_digest(&(good + 1u32)), None);
}
