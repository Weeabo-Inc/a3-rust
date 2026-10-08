//! `a3-tools sign ...`: inspect keys and signatures, verify and sign PBOs.

use std::path::{Path, PathBuf};

use a3_pbo::Pbo;
use a3_signing::{PrivateKey, PublicKey, Signature, SignatureVersion};
use anyhow::{Context, bail};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum SignCommand {
    /// Print the authority, key size and (for a .bisign) version of a key or signature file.
    Info {
        /// A .bikey, .biprivatekey or .bisign file.
        file: PathBuf,
    },
    /// Verify a PBO against the .bisign files next to it (`<pbo>.<authority>.bisign`).
    Verify {
        /// The PBO file.
        pbo: PathBuf,
        /// A .bikey to verify with; repeatable. Default: every .bikey in the folders given by
        /// --keys-dir.
        #[arg(long = "key")]
        keys: Vec<PathBuf>,
        /// Folder of .bikey files (for example the game's `Keys`).
        #[arg(long)]
        keys_dir: Option<PathBuf>,
    },
    /// Sign a PBO with a .biprivatekey, writing `<pbo>.<authority>.bisign` next to it.
    Sign {
        /// The PBO file.
        pbo: PathBuf,
        /// The private key.
        key: PathBuf,
        /// Signature version (2 or 3).
        #[arg(long, default_value_t = 3)]
        version: u32,
    },
}

pub fn run(cmd: SignCommand) -> anyhow::Result<()> {
    match cmd {
        SignCommand::Info { file } => {
            let data = read(&file)?;
            print!("{}", info(&file, &data)?);
        }
        SignCommand::Verify {
            pbo,
            keys,
            keys_dir,
        } => verify(&pbo, &keys, keys_dir.as_deref())?,
        SignCommand::Sign { pbo, key, version } => {
            let version = match version {
                2 => SignatureVersion::V2,
                3 => SignatureVersion::V3,
                other => bail!("signature version must be 2 or 3, not {other}"),
            };
            let key = PrivateKey::read(&read(&key)?)?;
            let signature = key.sign(&Pbo::open(&pbo)?, version)?;
            let out = signature_path(&pbo, &key.public.authority);
            std::fs::write(&out, signature.to_bytes())
                .with_context(|| format!("writing {}", out.display()))?;
            eprintln!("wrote {}", out.display());
        }
    }
    Ok(())
}

fn read(path: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

fn signature_path(pbo: &Path, authority: &str) -> PathBuf {
    let mut name = pbo.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{authority}.bisign"));
    pbo.with_file_name(name)
}

fn info(path: &Path, data: &[u8]) -> anyhow::Result<String> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let describe = |key: &PublicKey| {
        format!(
            "authority {}\nkey       {} bits, exponent {}\n",
            key.authority, key.bits, key.exponent
        )
    };
    Ok(match ext.as_str() {
        "bikey" => describe(&PublicKey::read(data)?),
        "biprivatekey" => describe(&PrivateKey::read(data)?.public) + "private   yes\n",
        "bisign" => {
            let signature = Signature::read(data)?;
            describe(&signature.key) + &format!("version   {}\n", signature.version.number())
        }
        _ => bail!("expected a .bikey, .biprivatekey or .bisign file"),
    })
}

fn verify(pbo_path: &Path, key_files: &[PathBuf], keys_dir: Option<&Path>) -> anyhow::Result<()> {
    let mut key_files = key_files.to_vec();
    if let Some(dir) = keys_dir {
        for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("bikey"))
            {
                key_files.push(path);
            }
        }
    }
    let keys = key_files
        .iter()
        .map(|p| Ok(PublicKey::read(&read(p)?)?))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let pbo = Pbo::open(pbo_path)?;
    let dir = pbo_path.parent().unwrap_or(Path::new("."));
    let prefix = format!(
        "{}.",
        pbo_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    );
    let mut checked = 0;
    let mut failed = false;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if !(name.starts_with(&prefix) && name.ends_with(".bisign")) {
            continue;
        }
        checked += 1;
        let signature = Signature::read(&read(&path)?)?;
        let key = keys
            .iter()
            .find(|k| k.modulus == signature.key.modulus)
            .unwrap_or(&signature.key);
        let trusted = !std::ptr::eq(key, &signature.key);
        match a3_signing::verify(key, &signature, &pbo) {
            Ok(()) if trusted => println!("OK       {} ({})", path.display(), key.authority),
            Ok(()) => println!(
                "UNTRUSTED {} (valid, but key {} is not among the given keys)",
                path.display(),
                signature.key.authority
            ),
            Err(e) => {
                failed = true;
                println!("FAIL     {}: {e}", path.display());
            }
        }
    }
    if checked == 0 {
        bail!("no {prefix}*.bisign next to {}", pbo_path.display());
    }
    if failed {
        bail!("signature verification failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_files_are_named_after_pbo_and_authority() {
        assert_eq!(
            signature_path(Path::new("addons/foo.pbo"), "me"),
            Path::new("addons/foo.pbo.me.bisign")
        );
    }

    #[test]
    fn info_rejects_unknown_extensions() {
        assert!(info(Path::new("x.txt"), b"").is_err());
    }
}
