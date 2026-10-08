//! `a3-tools pbo ...`: list, unpack and pack PBO archives.

use std::path::{Path, PathBuf};

use a3_pbo::{Pbo, PboWriter};
use anyhow::{Context, Result, bail};

/// Name of the file that carries the PBO prefix in an unpacked folder (a common tool
/// convention).
pub const PREFIX_FILE: &str = "$PBOPREFIX$";

/// Prints the properties and entries of the PBO at `file`.
pub fn list(file: &Path, verify: bool) -> Result<()> {
    let pbo = Pbo::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    for (key, value) in pbo.properties().iter() {
        println!("{key} = {value}");
    }
    println!("{:>12}  {:>8}  {:>10}  name", "size", "method", "timestamp");
    let mut total = 0u64;
    for entry in pbo.entries() {
        total += u64::from(entry.size());
        println!(
            "{:>12}  {:>8}  {:>10}  {}",
            entry.size(),
            entry.method().to_string(),
            entry.timestamp(),
            entry.name()
        );
    }
    println!("{} entries, {total} bytes", pbo.entries().len());
    match pbo.stored_hash() {
        Some(hash) => println!("sha1 {}", a3_pbo::to_hex(&hash)),
        None => println!("no sha1 trailer"),
    }
    if verify {
        pbo.verify()?;
        println!("sha1 OK");
    }
    Ok(())
}

/// Extracts every entry of the PBO at `file` below `out`, and writes the prefix to
/// [`PREFIX_FILE`]. Entries whose names would escape `out` are skipped; their names are
/// returned.
pub fn unpack(file: &Path, out: &Path) -> Result<Vec<String>> {
    let pbo = Pbo::open(file).with_context(|| format!("cannot open {}", file.display()))?;
    let mut skipped = Vec::new();
    for entry in pbo.entries() {
        let Some(rel) = safe_relative_path(entry.name()) else {
            skipped.push(entry.name().to_owned());
            continue;
        };
        let target = out.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let data = pbo
            .read_entry(entry)
            .with_context(|| format!("cannot read {}", entry.name()))?;
        std::fs::write(&target, &data)
            .with_context(|| format!("cannot write {}", target.display()))?;
    }
    std::fs::create_dir_all(out)?;
    if let Some(prefix) = pbo.properties().get("prefix") {
        std::fs::write(out.join(PREFIX_FILE), prefix)?;
    }
    Ok(skipped)
}

/// Builds a PBO at `file` from every file below `dir`. The prefix is `prefix` if given, else
/// the content of `dir/$PBOPREFIX$` if present. `properties` are extra `key=value` pairs.
pub fn pack(dir: &Path, file: &Path, prefix: Option<&str>, properties: &[String]) -> Result<()> {
    let mut files = Vec::new();
    collect_files(dir, Path::new(""), &mut files)?;
    files.sort_by_key(|(name, _)| name.to_lowercase());

    let prefix_file = files
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case(PREFIX_FILE));
    let file_prefix = match prefix_file {
        Some(i) => {
            let (_, path) = files.remove(i);
            let text = std::fs::read_to_string(&path)?;
            text.lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(|line| line.strip_prefix("prefix=").unwrap_or(line).to_owned())
        }
        None => None,
    };

    let mut writer = PboWriter::new();
    if let Some(prefix) = prefix.map(str::to_owned).or(file_prefix) {
        writer = writer.property("prefix", prefix);
    }
    for property in properties {
        let Some((key, value)) = property.split_once('=') else {
            bail!("property {property:?} is not of the form key=value");
        };
        writer = writer.property(key, value);
    }
    for (name, path) in files {
        let data =
            std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let timestamp = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| u32::try_from(d.as_secs()).ok())
            .unwrap_or(0);
        writer = writer.file_with_timestamp(name, data, timestamp);
    }
    let out =
        std::fs::File::create(file).with_context(|| format!("cannot create {}", file.display()))?;
    writer.write_to(std::io::BufWriter::new(out))?;
    Ok(())
}

/// Every file below `root/rel` as (backslash-separated entry name, OS path).
fn collect_files(root: &Path, rel: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let dir = root.join(rel);
    for entry in
        std::fs::read_dir(&dir).with_context(|| format!("cannot read {}", dir.display()))?
    {
        let entry = entry?;
        let child = rel.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            collect_files(root, &child, out)?;
        } else {
            let name = child
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("\\");
            out.push((name, entry.path()));
        }
    }
    Ok(())
}

/// Converts an entry name to a relative OS path, or `None` if it could leave the output
/// folder (`..` components, drive letters).
fn safe_relative_path(name: &str) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for component in name.split(['\\', '/']) {
        match component {
            "" | "." => {}
            ".." => return None,
            c if c.contains(':') => return None,
            c => path.push(c),
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_then_unpack_round_trips_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("Data")).unwrap();
        std::fs::write(src.join("config.cpp"), "class CfgPatches {};").unwrap();
        std::fs::write(src.join("Data").join("Tex_CO.paa"), [1u8, 2, 3]).unwrap();
        let pbo = tmp.path().join("out.pbo");

        pack(&src, &pbo, Some(r"my\addon"), &[]).unwrap();
        let parsed = a3_pbo::Pbo::open(&pbo).unwrap();
        assert_eq!(parsed.prefix().unwrap().as_str(), r"my\addon");
        let names: Vec<_> = parsed
            .entries()
            .iter()
            .map(|e| e.name().to_owned())
            .collect();
        assert_eq!(names, [r"config.cpp", r"Data\Tex_CO.paa"]);
        drop(parsed);

        let out = tmp.path().join("out");
        let skipped = unpack(&pbo, &out).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(
            std::fs::read(out.join("Data").join("Tex_CO.paa")).unwrap(),
            [1, 2, 3]
        );
        assert_eq!(
            std::fs::read_to_string(out.join(PREFIX_FILE)).unwrap(),
            r"my\addon"
        );

        // Re-packing the unpacked folder picks the prefix up from $PBOPREFIX$.
        let again = tmp.path().join("again.pbo");
        pack(&out, &again, None, &[]).unwrap();
        let parsed = a3_pbo::Pbo::open(&again).unwrap();
        assert_eq!(parsed.prefix().unwrap().as_str(), r"my\addon");
        assert_eq!(parsed.entries().len(), 2);
    }

    #[test]
    fn unpack_refuses_entries_that_escape_the_output_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let pbo = tmp.path().join("evil.pbo");
        let bytes = a3_pbo::PboWriter::new()
            .file(r"..\escape.txt", b"x".to_vec())
            .file(r"C:\abs.txt", b"x".to_vec())
            .file(r"ok\fine.txt", b"x".to_vec())
            .to_bytes();
        std::fs::write(&pbo, bytes).unwrap();

        let out = tmp.path().join("out");
        let skipped = unpack(&pbo, &out).unwrap();
        assert_eq!(skipped, [r"..\escape.txt", r"C:\abs.txt"]);
        assert!(out.join("ok").join("fine.txt").exists());
        assert!(!tmp.path().join("escape.txt").exists());
    }
}
