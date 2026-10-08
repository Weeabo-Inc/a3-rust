//! `.bikey`, `.biprivatekey` and `.bisign` files: an authority name and CryptoAPI RSA key blobs.

use num_bigint::BigUint;

use crate::read::Reader;
use crate::{Error, PboHashes, Result};

/// `PUBLICKEYBLOB` / `PRIVATEKEYBLOB` header: blob type, version 2, reserved, `CALG_RSA_SIGN`.
const PUBLIC_BLOB: u8 = 0x06;
const PRIVATE_BLOB: u8 = 0x07;
const BLOB_VERSION: u8 = 0x02;
const CALG_RSA_SIGN: u32 = 0x2400;

/// ASN.1 `DigestInfo` prefix of a SHA-1 digest in a PKCS#1 v1.5 signature.
const SHA1_DIGEST_INFO: [u8; 15] = [
    0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14,
];

/// An RSA public key with the name of its authority (`.bikey`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    /// Authority (publisher) name, also the middle part of `.bisign` file names.
    pub authority: String,
    /// Key size in bits (1024 for every shipped key).
    pub bits: u32,
    /// Public exponent (65537 for every shipped key).
    pub exponent: u32,
    /// Modulus.
    pub modulus: BigUint,
}

/// An RSA private key with its authority name (`.biprivatekey`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateKey {
    /// The matching public key.
    pub public: PublicKey,
    /// First prime factor.
    pub p: BigUint,
    /// Second prime factor.
    pub q: BigUint,
    /// Private exponent.
    pub d: BigUint,
}

/// The signature scheme version of a `.bisign`: which files the content hash covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureVersion {
    /// Version 2: every file except textures, models, sounds and other listed binary types.
    V2,
    /// Version 3: only script and config source files.
    V3,
}

impl SignatureVersion {
    /// The number stored in the file.
    pub fn number(self) -> u32 {
        match self {
            Self::V2 => 2,
            Self::V3 => 3,
        }
    }
}

/// The signature of one PBO (`.bisign`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The public key of the signer, as embedded in the file.
    pub key: PublicKey,
    /// Scheme version.
    pub version: SignatureVersion,
    /// RSA signatures of the archive, names and content hashes, in that order.
    pub signatures: [BigUint; 3],
}

impl PublicKey {
    /// Reads a `.bikey` file.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let key = Self::read_from(&mut r)?;
        if !r.is_empty() {
            return Err(Error::Malformed("bytes after the key".into()));
        }
        Ok(key)
    }

    fn read_from(r: &mut Reader) -> Result<Self> {
        let authority = r.cstr()?;
        let blob = r.sized()?;
        let mut b = Reader::new(blob);
        let (bits, exponent) = blob_header(&mut b, PUBLIC_BLOB, b"RSA1")?;
        let modulus = BigUint::from_bytes_le(b.bytes(bits as usize / 8)?);
        Ok(Self {
            authority,
            bits,
            exponent,
            modulus,
        })
    }

    /// Encodes the key as a `.bikey` file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut blob = blob_header_bytes(PUBLIC_BLOB, b"RSA1", self.bits, self.exponent);
        blob.extend(le_fixed(&self.modulus, self.len()));
        let mut out = cstr_bytes(&self.authority);
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend(blob);
        out
    }

    /// Key size in bytes.
    pub fn len(&self) -> usize {
        self.bits as usize / 8
    }

    /// Whether the key has no modulus bytes (never true for a usable key).
    pub fn is_empty(&self) -> bool {
        self.bits == 0
    }

    /// Applies the public key to a signature and returns the SHA-1 digest it carries, or `None`
    /// when the result is not a PKCS#1 v1.5 SHA-1 signature block.
    pub fn recover_digest(&self, signature: &BigUint) -> Option<[u8; 20]> {
        if signature >= &self.modulus {
            return None;
        }
        let block = signature.modpow(&BigUint::from(self.exponent), &self.modulus);
        let block = be_fixed(&block, self.len());
        // 00 01 FF..FF 00 DigestInfo digest
        let digest_at = block.len().checked_sub(20)?;
        let info_at = digest_at.checked_sub(SHA1_DIGEST_INFO.len())?;
        let separator = info_at.checked_sub(1)?;
        let padding_ok = block.get(..2) == Some(&[0x00, 0x01][..])
            && separator >= 2 + 8
            && block[2..separator].iter().all(|&b| b == 0xff)
            && block[separator] == 0x00
            && block[info_at..digest_at] == SHA1_DIGEST_INFO;
        padding_ok.then(|| block[digest_at..].try_into().expect("20 bytes"))
    }
}

impl PrivateKey {
    /// Builds a key from its primes and public exponent. `None` when the exponent has no
    /// inverse modulo (p-1)(q-1).
    pub fn from_primes(authority: &str, p: BigUint, q: BigUint, exponent: u32) -> Option<Self> {
        let one = BigUint::from(1u32);
        let modulus = &p * &q;
        let phi = (&p - &one) * (&q - &one);
        let d = BigUint::from(exponent).modinv(&phi)?;
        let bits = u32::try_from(modulus.bits().div_ceil(8) * 8).ok()?;
        Some(Self {
            public: PublicKey {
                authority: authority.to_string(),
                bits,
                exponent,
                modulus,
            },
            p,
            q,
            d,
        })
    }

    /// Reads a `.biprivatekey` file.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let authority = r.cstr()?;
        let blob = r.sized()?;
        let mut b = Reader::new(blob);
        let (bits, exponent) = blob_header(&mut b, PRIVATE_BLOB, b"RSA2")?;
        let (full, half) = (bits as usize / 8, bits as usize / 16);
        let modulus = BigUint::from_bytes_le(b.bytes(full)?);
        let p = BigUint::from_bytes_le(b.bytes(half)?);
        let q = BigUint::from_bytes_le(b.bytes(half)?);
        b.bytes(half * 3)?; // d mod (p-1), d mod (q-1), q^-1 mod p: derived from the rest
        let d = BigUint::from_bytes_le(b.bytes(full)?);
        Ok(Self {
            public: PublicKey {
                authority,
                bits,
                exponent,
                modulus,
            },
            p,
            q,
            d,
        })
    }

    /// Encodes the key as a `.biprivatekey` file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let one = BigUint::from(1u32);
        let public = &self.public;
        let (full, half) = (public.len(), public.len() / 2);
        let mut blob = blob_header_bytes(PRIVATE_BLOB, b"RSA2", public.bits, public.exponent);
        let dp = &self.d % (&self.p - &one);
        let dq = &self.d % (&self.q - &one);
        let qinv = self.q.modinv(&self.p).unwrap_or_default();
        blob.extend(le_fixed(&public.modulus, full));
        for part in [&self.p, &self.q, &dp, &dq, &qinv] {
            blob.extend(le_fixed(part, half));
        }
        blob.extend(le_fixed(&self.d, full));
        let mut out = cstr_bytes(&public.authority);
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend(blob);
        out
    }

    /// Signs a SHA-1 digest (PKCS#1 v1.5).
    pub fn sign_digest(&self, digest: &[u8; 20]) -> BigUint {
        let len = self.public.len();
        let mut block = vec![0xff; len];
        block[0] = 0x00;
        block[1] = 0x01;
        let info_at = len - 20 - SHA1_DIGEST_INFO.len();
        block[info_at - 1] = 0x00;
        block[info_at..len - 20].copy_from_slice(&SHA1_DIGEST_INFO);
        block[len - 20..].copy_from_slice(digest);
        BigUint::from_bytes_be(&block).modpow(&self.d, &self.public.modulus)
    }

    /// Signs a PBO.
    pub fn sign(&self, pbo: &a3_pbo::Pbo, version: SignatureVersion) -> Result<Signature> {
        let hashes = PboHashes::of(pbo, version)?;
        Ok(Signature {
            key: self.public.clone(),
            version,
            signatures: [hashes.archive, hashes.names, hashes.content]
                .map(|h| self.sign_digest(&h)),
        })
    }
}

impl Signature {
    /// Reads a `.bisign` file.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let key = PublicKey::read_from(&mut r)?;
        let first = BigUint::from_bytes_le(r.sized()?);
        let version = match r.u32()? {
            2 => SignatureVersion::V2,
            3 => SignatureVersion::V3,
            other => {
                return Err(Error::Malformed(format!("signature version {other}")));
            }
        };
        let second = BigUint::from_bytes_le(r.sized()?);
        let third = BigUint::from_bytes_le(r.sized()?);
        if !r.is_empty() {
            return Err(Error::Malformed("bytes after the signatures".into()));
        }
        Ok(Self {
            key,
            version,
            signatures: [first, second, third],
        })
    }

    /// Encodes the signature as a `.bisign` file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let len = self.key.len();
        let mut out = self.key.to_bytes();
        let put = |out: &mut Vec<u8>, sig: &BigUint| {
            out.extend_from_slice(&(len as u32).to_le_bytes());
            out.extend(le_fixed(sig, len));
        };
        put(&mut out, &self.signatures[0]);
        out.extend_from_slice(&self.version.number().to_le_bytes());
        put(&mut out, &self.signatures[1]);
        put(&mut out, &self.signatures[2]);
        out
    }
}

fn blob_header(b: &mut Reader, blob_type: u8, magic: &[u8; 4]) -> Result<(u32, u32)> {
    let header = b.bytes(8)?;
    let alg = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes"));
    if header[0] != blob_type || header[1] != BLOB_VERSION || alg != CALG_RSA_SIGN {
        return Err(Error::Malformed(format!(
            "unexpected key blob header {header:02x?}"
        )));
    }
    if b.bytes(4)? != magic {
        return Err(Error::Malformed("key blob magic is not RSA1/RSA2".into()));
    }
    let bits = b.u32()?;
    let exponent = b.u32()?;
    if bits == 0 || bits % 16 != 0 {
        return Err(Error::Malformed(format!("key size {bits} bits")));
    }
    Ok((bits, exponent))
}

fn blob_header_bytes(blob_type: u8, magic: &[u8; 4], bits: u32, exponent: u32) -> Vec<u8> {
    let mut out = vec![blob_type, BLOB_VERSION, 0, 0];
    out.extend_from_slice(&CALG_RSA_SIGN.to_le_bytes());
    out.extend_from_slice(magic);
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(&exponent.to_le_bytes());
    out
}

fn cstr_bytes(s: &str) -> Vec<u8> {
    let mut out = s.as_bytes().to_vec();
    out.push(0);
    out
}

/// `n` as exactly `len` little-endian bytes (zero-padded; truncated if larger).
fn le_fixed(n: &BigUint, len: usize) -> Vec<u8> {
    let mut bytes = n.to_bytes_le();
    bytes.resize(len, 0);
    bytes
}

/// `n` as exactly `len` big-endian bytes (zero-padded on the left).
fn be_fixed(n: &BigUint, len: usize) -> Vec<u8> {
    let mut bytes = le_fixed(n, len);
    bytes.reverse();
    bytes
}
