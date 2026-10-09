//! The binary mod/DLC block the official server packs into A2S_RULES (`docs/re/net-a2s.md`).
//!
//! Transport inside rules: serialise the block (at most 1024 bytes, dropping the mod list and then
//! the signature list when it does not fit), escape it (`0x01` becomes `01 01`, `0x00` becomes
//! `01 02`, `0xFF` becomes `01 03`), split the escaped bytes into 127-byte chunks and publish
//! chunk *i* of *n* under the two-byte key `{ i, n }`. Clients require one `n` across all keys and
//! `i < 'A'`, and concatenate the values in index order before unescaping.

use crate::bytes::Writer;
use crate::error::NetError;

/// The block version this build writes; clients accept 2 or 3.
pub const BLOCK_VERSION: u8 = 3;

/// The engine's limit on the serialised block (`DAT 0x20aecd0`).
pub const BLOCK_MAX: usize = 0x400;

/// The chunk size the escaped block is split into (`0x177960`).
pub const CHUNK: usize = 127;

/// The highest chunk index a client reads (`i < 'A'`), so at most 64 chunks.
pub const MAX_CHUNKS: usize = 64;

/// Flags bit 0: the mod list was dropped to fit.
pub const FLAG_MODS_DROPPED: u8 = 0x01;
/// Flags bit 1: the signature list was dropped to fit.
pub const FLAG_SIGNATURES_DROPPED: u8 = 0x02;
/// Flags bit 2: the world is Tanoa.
pub const FLAG_WORLD_TANOA: u8 = 0x04;
/// Flags bit 3: the world is Livonia (Enoch).
pub const FLAG_WORLD_LIVONIA: u8 = 0x08;

/// One chunk of the escaped block as it travels in A2S_RULES: the two-byte key `{ i, n }` and the
/// chunk's bytes.
pub type RulesChunk = (Vec<u8>, Vec<u8>);

/// One mod of the block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mod {
    /// The mod's hash as the server publishes it.
    pub hash: u32,
    /// The Steam workshop id; 0 for a mod that has none.
    pub workshop_id: u64,
    /// Bit 4 of the info byte: the entry is DLC rather than a workshop mod.
    pub is_dlc: bool,
    /// The mod's name.
    pub name: String,
}

/// The mod/DLC block, before escaping and chunking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RulesBlock {
    /// Extra flag bits to publish (the documented world/overflow bits).
    pub flags: u8,
    /// Bit *i* set means a u32 DLC hash follows; only bits 0..12 are read.
    pub dlc_mask: u16,
    /// One hash per set bit of [`RulesBlock::dlc_mask`], in ascending bit order.
    pub dlc_hashes: Vec<u32>,
    /// Bits 0-2 difficulty, bits 3-5 AI level, bit 6 and 7 flags whose meaning is medium.
    pub difficulty: u8,
    /// Read as a bool by the client (community name: crosshair; medium confidence).
    pub flag: u8,
    pub mods: Vec<Mod>,
    /// Bikey names of the loaded signatures.
    pub signatures: Vec<String>,
}

impl RulesBlock {
    /// Serialise the block, dropping the trailing lists only if it does not fit.
    ///
    /// The reader omits `sig_count` when either overflow flag is set, so the only states a
    /// well-formed block can be in are: everything present, signatures dropped, or both lists
    /// dropped. Signatures go first because they are the smaller, less useful half.
    pub fn encode(&self) -> Result<Vec<u8>, NetError> {
        if self.mods.len() > u8::MAX as usize {
            return Err(NetError::A2sFieldTooLong {
                field: "mod count",
                len: self.mods.len(),
                max: u8::MAX as usize,
            });
        }
        if self.signatures.len() > u8::MAX as usize {
            return Err(NetError::A2sFieldTooLong {
                field: "signature count",
                len: self.signatures.len(),
                max: u8::MAX as usize,
            });
        }
        for entry in &self.mods {
            if entry.name.len() > u8::MAX as usize {
                return Err(NetError::A2sFieldTooLong {
                    field: "mod name",
                    len: entry.name.len(),
                    max: u8::MAX as usize,
                });
            }
        }
        let complete = self.encode_with(self.flags);
        if complete.len() <= BLOCK_MAX {
            return Ok(complete);
        }
        let no_signatures = self.encode_with(self.flags | FLAG_SIGNATURES_DROPPED);
        if no_signatures.len() <= BLOCK_MAX {
            log::warn!(
                "a2s rules: signature list dropped, the block is larger than {BLOCK_MAX} bytes"
            );
            return Ok(no_signatures);
        }
        let bare = self.encode_with(self.flags | FLAG_MODS_DROPPED | FLAG_SIGNATURES_DROPPED);
        if bare.len() <= BLOCK_MAX {
            log::warn!(
                "a2s rules: mod and signature lists dropped, the block is larger than {BLOCK_MAX} bytes"
            );
            return Ok(bare);
        }
        Err(NetError::RulesBlockTooLong {
            len: bare.len(),
            max: BLOCK_MAX,
        })
    }

    /// The escaped block, split into the `{ i, n }` key/value chunks A2S_RULES carries.
    pub fn chunks(&self) -> Result<Vec<RulesChunk>, NetError> {
        let escaped = escape(&self.encode()?);
        let parts: Vec<&[u8]> = if escaped.is_empty() {
            vec![&[]]
        } else {
            escaped.chunks(CHUNK).collect()
        };
        let count = parts.len();
        if count > MAX_CHUNKS {
            return Err(NetError::RulesChunks {
                chunks: count,
                chunk: CHUNK,
            });
        }
        Ok(parts
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| (vec![index as u8 + 1, count as u8], chunk.to_vec()))
            .collect())
    }

    /// The raw block with the given overflow flags, before escaping.
    fn encode_with(&self, flags: u8) -> Vec<u8> {
        let mut writer = Writer::new()
            .u8(BLOCK_VERSION)
            .u8(flags)
            .u16(self.dlc_mask)
            .u8(self.difficulty)
            .u8(self.flag);
        for bit in 0..13 {
            if self.dlc_mask & (1 << bit) != 0 {
                let hash = self.dlc_hashes.get(bit as usize).copied().unwrap_or(0);
                writer = writer.u32(hash);
            }
        }
        if flags & FLAG_MODS_DROPPED == 0 {
            writer = writer.u8(self.mods.len() as u8);
            for entry in &self.mods {
                writer = writer.u32(entry.hash);
                let id = workshop_id_bytes(entry.workshop_id);
                let info = (id.len() as u8 & 0x0F) | if entry.is_dlc { 0x10 } else { 0 };
                writer = writer.u8(info).bytes(&id);
                writer = writer
                    .u8(entry.name.len() as u8)
                    .bytes(entry.name.as_bytes());
            }
        }
        // The reader skips `sig_count` when either flag is set, not only when bit 1 is.
        if flags & (FLAG_MODS_DROPPED | FLAG_SIGNATURES_DROPPED) == 0 {
            writer = writer.u8(self.signatures.len() as u8);
            for name in &self.signatures {
                writer = writer.u8(name.len() as u8).bytes(name.as_bytes());
            }
        }
        writer.finish()
    }
}

/// The little-endian workshop id trimmed of its leading zeros.
fn workshop_id_bytes(id: u64) -> Vec<u8> {
    let bytes = id.to_le_bytes();
    let len = bytes.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    bytes[..len].to_vec()
}

/// Escape a block for transport inside A2S strings.
///
/// The three bytes a NUL-terminated string cannot carry literally are escaped with `0x01` as the
/// prefix; everything else passes through.
pub fn escape(block: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(block.len());
    for byte in block {
        match byte {
            0x01 => out.extend_from_slice(&[0x01, 0x01]),
            0x00 => out.extend_from_slice(&[0x01, 0x02]),
            0xFF => out.extend_from_slice(&[0x01, 0x03]),
            other => out.push(*other),
        }
    }
    out
}

/// Undo [`escape`] over the concatenated chunk values.
pub fn unescape(escaped: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(escaped.len());
    let mut index = 0;
    while index < escaped.len() {
        let byte = escaped[index];
        index += 1;
        if byte != 0x01 {
            out.push(byte);
            continue;
        }
        let code = *escaped.get(index)?;
        index += 1;
        out.push(match code {
            0x01 => 0x01,
            0x02 => 0x00,
            0x03 => 0xFF,
            _ => return None,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).expect("test hex"))
            .collect()
    }

    /// An empty block: version 3, no flags, no DLC, no difficulty, no mods, no signatures.
    fn empty_block() -> RulesBlock {
        RulesBlock::default()
    }

    #[test]
    fn an_empty_block_is_the_documented_eight_bytes() {
        let block = empty_block().encode().expect("encode");
        assert_eq!(block, decode_hex("0300000000000000"));
    }

    #[test]
    fn a_full_block_has_the_documented_field_order() {
        let block = RulesBlock {
            flags: FLAG_WORLD_LIVONIA,
            dlc_mask: 0b101,
            dlc_hashes: vec![0xAAAA_AAAA, 0, 0xBBBB_BBBB],
            difficulty: 0x12,
            flag: 1,
            mods: vec![
                Mod {
                    hash: 0x1122_3344,
                    workshop_id: 0x1234,
                    is_dlc: false,
                    name: "CBA".into(),
                },
                Mod {
                    hash: 0x5566_7788,
                    workshop_id: 0x0102_0304_0506_0708,
                    is_dlc: true,
                    name: "DLC".into(),
                },
            ],
            signatures: vec!["a3.bikey".into()],
        };
        let encoded = block.encode().expect("encode");
        let mut expected = decode_hex("030805001201"); // version, flags, dlc_mask, difficulty, flag
        expected.extend_from_slice(&0xAAAA_AAAAu32.to_le_bytes()); // bit 0
        expected.extend_from_slice(&0xBBBB_BBBBu32.to_le_bytes()); // bit 2
        expected.push(2); // mod_count
        expected.extend_from_slice(&0x1122_3344u32.to_le_bytes());
        expected.push(2); // id length 2
        expected.extend_from_slice(&[0x34, 0x12]);
        expected.push(3);
        expected.extend_from_slice(b"CBA");
        expected.extend_from_slice(&0x5566_7788u32.to_le_bytes());
        expected.push(0x10 | 8); // is DLC, 8-byte id
        expected.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes());
        expected.push(3);
        expected.extend_from_slice(b"DLC");
        expected.push(1); // sig_count
        expected.push(8);
        expected.extend_from_slice(b"a3.bikey");
        assert_eq!(encoded, expected);
    }

    #[test]
    fn a_workshop_id_is_written_trimmed_and_little_endian() {
        assert_eq!(workshop_id_bytes(0), Vec::<u8>::new());
        assert_eq!(workshop_id_bytes(0x1234), vec![0x34, 0x12]);
        assert_eq!(workshop_id_bytes(1), vec![1]);
        assert_eq!(workshop_id_bytes(u64::MAX), vec![0xFF; 8]);
    }

    #[test]
    fn escape_round_trips_the_three_special_bytes() {
        let block: Vec<u8> = (0..=255u8).collect();
        let escaped = escape(&block);
        // 0x00, 0x01 and 0xFF each double, everything else passes through.
        assert_eq!(escaped.len(), 256 + 3);
        assert_eq!(unescape(&escaped), Some(block));
        // The escaped form carries no byte a NUL-terminated string could not hold.
        assert!(!escaped.contains(&0x00));
        assert!(!escaped.contains(&0xFF));
        assert_eq!(
            escape(&[0x01, 0x00, 0xFF]),
            vec![0x01, 0x01, 0x01, 0x02, 0x01, 0x03]
        );
        assert_eq!(unescape(&[0x01, 0x04]), None);
        assert_eq!(unescape(&[0x01]), None);
    }

    #[test]
    fn chunks_carry_the_one_based_index_and_the_count() {
        let chunks = empty_block().chunks().expect("chunks");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, vec![1, 1]);
        assert_eq!(
            unescape(&chunks[0].1),
            Some(empty_block().encode().unwrap())
        );

        // A block whose escaped form needs three chunks: ten mods with 20-character names.
        let block = RulesBlock {
            mods: (0..10)
                .map(|i| Mod {
                    hash: i,
                    workshop_id: 0,
                    is_dlc: false,
                    name: "x".repeat(20),
                })
                .collect(),
            ..RulesBlock::default()
        };
        let chunks = block.chunks().expect("chunks");
        assert_eq!(chunks.len(), 3);
        let count = chunks.len() as u8;
        for (index, (key, value)) in chunks.iter().enumerate() {
            assert_eq!(key, &vec![index as u8 + 1, count]);
            assert!(value.len() <= CHUNK, "chunks are at most {CHUNK} bytes");
        }
        let joined: Vec<u8> = chunks.iter().flat_map(|(_, value)| value.clone()).collect();
        assert_eq!(unescape(&joined), Some(block.encode().unwrap()));
    }

    #[test]
    fn an_oversized_list_is_dropped_with_its_flag() {
        // The mod list alone fits; the signatures push the block over the limit, so the signature
        // list is the one dropped.
        let block = RulesBlock {
            mods: (0..30)
                .map(|i| Mod {
                    hash: i,
                    workshop_id: u64::from(i),
                    is_dlc: false,
                    name: "n".repeat(20),
                })
                .collect(),
            signatures: vec!["a3.bikey".into(); 40],
            ..RulesBlock::default()
        };
        let encoded = block.encode().expect("encode");
        assert!(encoded.len() <= BLOCK_MAX);
        assert_eq!(
            encoded[1], FLAG_SIGNATURES_DROPPED,
            "only signatures dropped"
        );
        let mut cursor = crate::bytes::Cursor::new(&encoded);
        assert_eq!(cursor.u8(), Some(BLOCK_VERSION));
        assert_eq!(cursor.u8(), Some(FLAG_SIGNATURES_DROPPED));
        assert_eq!(cursor.u8(), Some(0), "dlc_mask, low byte");
        assert_eq!(cursor.u8(), Some(0), "dlc_mask, high byte");
        assert_eq!(cursor.u8(), Some(0), "difficulty");
        assert_eq!(cursor.u8(), Some(0), "flag");
        assert_eq!(cursor.u8(), Some(30), "the mod list survives");
        assert_eq!(
            cursor.remaining(),
            encoded.len() - 7,
            "the rest of the block is the mod list"
        );

        // A block whose mod list alone is too large loses both lists.
        let block = RulesBlock {
            mods: (0..200)
                .map(|i| Mod {
                    hash: i,
                    workshop_id: u64::from(i),
                    is_dlc: false,
                    name: "n".repeat(100),
                })
                .collect(),
            signatures: vec!["a3.bikey".into()],
            ..RulesBlock::default()
        };
        let encoded = block.encode().expect("encode");
        assert_eq!(
            encoded,
            decode_hex("030300000000"),
            "header only: version, both flags, empty dlc mask, difficulty, flag"
        );
    }
}
