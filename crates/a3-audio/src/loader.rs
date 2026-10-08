//! Loading config sound paths from the VFS, with a cache of decoded clips.

use std::collections::HashMap;
use std::sync::Mutex;

use a3_vfs::{Vfs, VfsPath};

use crate::{Clip, Source, Stream};

/// Extensions tried, in order, for a config path written without one.
pub const SOUND_EXTENSIONS: [&str; 3] = ["wss", "ogg", "wav"];

/// Ogg files longer than this (in seconds) are streamed instead of decoded whole.
pub const STREAM_SECONDS: f64 = 20.0;

/// Finds the file a config sound path names: the path itself if it exists, else the path with
/// each of [`SOUND_EXTENSIONS`] appended. A leading `\` is ignored.
pub fn resolve_sound_path(vfs: &Vfs, path: &str) -> Option<VfsPath> {
    let path = path.trim().trim_start_matches(['\\', '/']);
    if path.is_empty() {
        return None;
    }
    let direct = VfsPath::new(path);
    if direct.extension().is_some() && vfs.exists(direct.as_str()) {
        return Some(direct);
    }
    SOUND_EXTENSIONS
        .iter()
        .map(|ext| VfsPath::new(&format!("{path}.{ext}")))
        .find(|p| vfs.exists(p.as_str()))
}

/// Loads sounds by config path and keeps decoded clips for reuse.
pub struct SoundLoader {
    vfs: Vfs,
    clips: Mutex<HashMap<VfsPath, Option<Clip>>>,
}

impl SoundLoader {
    /// A loader over `vfs` with an empty cache.
    pub fn new(vfs: Vfs) -> Self {
        Self {
            vfs,
            clips: Mutex::new(HashMap::new()),
        }
    }

    /// The VFS.
    pub fn vfs(&self) -> &Vfs {
        &self.vfs
    }

    /// A playable source for a config sound path: a cached clip, or a new stream for long Ogg
    /// files (`looping` makes the stream loop). `None` when the file is missing or broken
    /// (logged once per path).
    pub fn source(&self, path: &str, looping: bool) -> Option<Source> {
        let resolved = resolve_sound_path(&self.vfs, path)?;
        let data = self.vfs.open(resolved.as_str()).ok()?;
        if resolved.extension() == Some("ogg")
            && let Ok(info) = a3_audio_formats::probe(&data)
        {
            let seconds = info.frames.unwrap_or(0) as f64 / f64::from(info.sample_rate.max(1));
            if seconds > STREAM_SECONDS {
                return match Stream::open(data, looping) {
                    Ok(stream) => Some(Source::Stream(stream)),
                    Err(e) => {
                        log::warn!("sound {resolved}: {e}");
                        None
                    }
                };
            }
        }
        self.clip_of(&resolved, &data).map(Source::Clip)
    }

    /// The decoded clip for a config sound path, cached.
    pub fn clip(&self, path: &str) -> Option<Clip> {
        let resolved = resolve_sound_path(&self.vfs, path)?;
        if let Some(cached) = self.clips.lock().expect("clip cache").get(&resolved) {
            return cached.clone();
        }
        let data = self.vfs.open(resolved.as_str()).ok()?;
        self.clip_of(&resolved, &data)
    }

    fn clip_of(&self, path: &VfsPath, data: &[u8]) -> Option<Clip> {
        let mut cache = self.clips.lock().expect("clip cache");
        cache
            .entry(path.clone())
            .or_insert_with(|| match Clip::decode(data) {
                Ok(clip) => Some(clip),
                Err(e) => {
                    log::warn!("sound {path}: {e}");
                    None
                }
            })
            .clone()
    }
}
