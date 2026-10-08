use std::io::{self, Write};

use sha1::{Digest, Sha1};

use crate::Properties;
use crate::entry::{METHOD_PROPERTIES, METHOD_STORED};

/// Builds a PBO from files and properties. Every file is stored uncompressed.
///
/// ```
/// let bytes = a3_pbo::PboWriter::new()
///     .property("prefix", r"my\addon")
///     .file("config.cpp", b"class CfgPatches {};".to_vec())
///     .to_bytes();
/// let pbo = a3_pbo::Pbo::from_bytes(bytes).unwrap();
/// assert_eq!(pbo.prefix().unwrap().as_str(), r"my\addon");
/// ```
#[derive(Debug, Clone, Default)]
pub struct PboWriter {
    properties: Properties,
    files: Vec<File>,
}

#[derive(Debug, Clone)]
struct File {
    name: String,
    data: Vec<u8>,
    timestamp: u32,
}

impl PboWriter {
    /// An empty archive with no properties.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a header property. Properties are written in the order they are added.
    pub fn property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.push(key, value);
        self
    }

    /// Adds a file with timestamp 0. `name` is written as given; use backslash separators.
    pub fn file(self, name: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        self.file_with_timestamp(name, data, 0)
    }

    /// Adds a file with a modification time in seconds since the Unix epoch.
    pub fn file_with_timestamp(
        mut self,
        name: impl Into<String>,
        data: impl Into<Vec<u8>>,
        timestamp: u32,
    ) -> Self {
        self.files.push(File {
            name: name.into(),
            data: data.into(),
            timestamp,
        });
        self
    }

    /// Writes the archive, including its SHA-1 trailer, to `out`.
    ///
    /// Fails with [`io::ErrorKind::InvalidInput`] for an empty name, a name or property
    /// containing NUL, or a file of 4 GiB or more.
    pub fn write_to(&self, out: impl Write) -> io::Result<()> {
        self.validate()?;
        let mut out = HashingWriter {
            inner: out,
            hasher: Sha1::new(),
        };
        if !self.properties.is_empty() {
            write_record(&mut out, "", METHOD_PROPERTIES, 0, 0, 0)?;
            for (key, value) in self.properties.iter() {
                write_cstr(&mut out, key)?;
                write_cstr(&mut out, value)?;
            }
            write_cstr(&mut out, "")?;
        }
        for file in &self.files {
            let size = file.data.len() as u32;
            write_record(&mut out, &file.name, METHOD_STORED, 0, file.timestamp, size)?;
        }
        write_record(&mut out, "", 0, 0, 0, 0)?;
        for file in &self.files {
            out.write_all(&file.data)?;
        }
        let digest = out.hasher.finalize();
        let mut inner = out.inner;
        inner.write_all(&[0])?;
        inner.write_all(&digest)?;
        inner.flush()
    }

    /// Writes the archive to a new byte vector.
    ///
    /// # Panics
    ///
    /// On the invalid input that makes [`PboWriter::write_to`] fail.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_to(&mut out).expect("invalid PBO writer input");
        out
    }

    fn validate(&self) -> io::Result<()> {
        let invalid = |msg: String| Err(io::Error::new(io::ErrorKind::InvalidInput, msg));
        for (key, value) in self.properties.iter() {
            if key.is_empty() || key.contains('\0') || value.contains('\0') {
                return invalid(format!("invalid PBO property {key:?}"));
            }
        }
        for file in &self.files {
            if file.name.is_empty() || file.name.contains('\0') {
                return invalid(format!("invalid PBO entry name {:?}", file.name));
            }
            if u32::try_from(file.data.len()).is_err() {
                return invalid(format!("PBO entry {:?} is 4 GiB or larger", file.name));
            }
        }
        Ok(())
    }
}

fn write_cstr(out: &mut impl Write, text: &str) -> io::Result<()> {
    out.write_all(text.as_bytes())?;
    out.write_all(&[0])
}

fn write_record(
    out: &mut impl Write,
    name: &str,
    method: u32,
    original_size: u32,
    timestamp: u32,
    data_size: u32,
) -> io::Result<()> {
    write_cstr(out, name)?;
    for field in [method, original_size, 0, timestamp, data_size] {
        out.write_all(&field.to_le_bytes())?;
    }
    Ok(())
}

struct HashingWriter<W> {
    inner: W,
    hasher: Sha1,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
