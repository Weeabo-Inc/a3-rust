use std::fmt;

/// Packing-method value of a plain, uncompressed entry.
pub const METHOD_STORED: u32 = 0;
/// Packing-method value `Cprs`: LZSS-compressed entry.
pub const METHOD_COMPRESSED: u32 = u32::from_le_bytes(*b"srpC");
/// Packing-method value `Enco`: encrypted entry.
pub const METHOD_ENCRYPTED: u32 = u32::from_le_bytes(*b"ocnE");
/// Packing-method value `Vers`: the header-extension record that introduces the properties.
pub const METHOD_PROPERTIES: u32 = u32::from_le_bytes(*b"sreV");

/// How the bytes of an entry are stored in the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackingMethod {
    /// Stored as is (`0`).
    Stored,
    /// LZSS-compressed (`Cprs`).
    Compressed,
    /// Encrypted (`Enco`).
    Encrypted,
    /// Any other value.
    Other(u32),
}

impl PackingMethod {
    /// Classifies a raw header value.
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            METHOD_STORED => Self::Stored,
            METHOD_COMPRESSED => Self::Compressed,
            METHOD_ENCRYPTED => Self::Encrypted,
            other => Self::Other(other),
        }
    }

    /// The raw header value.
    pub fn raw(self) -> u32 {
        match self {
            Self::Stored => METHOD_STORED,
            Self::Compressed => METHOD_COMPRESSED,
            Self::Encrypted => METHOD_ENCRYPTED,
            Self::Other(raw) => raw,
        }
    }
}

impl fmt::Display for PackingMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stored => f.write_str("stored"),
            Self::Compressed => f.write_str("Cprs"),
            Self::Encrypted => f.write_str("Enco"),
            Self::Other(raw) => write!(f, "{raw:#010x}"),
        }
    }
}

/// One file record of a PBO header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub(crate) name: String,
    pub(crate) method: PackingMethod,
    pub(crate) original_size: u32,
    pub(crate) reserved: u32,
    pub(crate) timestamp: u32,
    pub(crate) data_size: u32,
    pub(crate) data_offset: u64,
}

impl Entry {
    /// The path as stored in the header (original case, backslash-separated).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How the entry's bytes are stored.
    pub fn method(&self) -> PackingMethod {
        self.method
    }

    /// The `original size` header field: the unpacked size for compressed entries, usually 0
    /// for stored ones.
    pub fn original_size(&self) -> u32 {
        self.original_size
    }

    /// The `reserved` header field (0 in every shipped PBO).
    pub fn reserved(&self) -> u32 {
        self.reserved
    }

    /// Modification time in seconds since the Unix epoch (0 when unknown).
    pub fn timestamp(&self) -> u32 {
        self.timestamp
    }

    /// Number of bytes the entry occupies in the archive.
    pub fn data_size(&self) -> u32 {
        self.data_size
    }

    /// Absolute byte offset of the entry's data in the archive.
    pub fn data_offset(&self) -> u64 {
        self.data_offset
    }

    /// Size of the entry once unpacked.
    pub fn size(&self) -> u32 {
        match self.method {
            PackingMethod::Compressed => self.original_size,
            _ => self.data_size,
        }
    }
}
