//! Background loading: worker threads read models and textures from the VFS and decode them
//! into CPU data; the render thread uploads the results.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use a3_vfs::Vfs;

use crate::prepare::PreparedModel;
use crate::texture::{LoadedTexture, TextureOptions, decode_texture};

/// How a texture is sampled; part of its cache key because it decides the GPU format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextureUse {
    /// sRGB colour.
    Color,
    /// Linear data (specular, ambient shadow, masks, detail).
    Data,
    /// Normal map, converted to the `_nohq` layout.
    Normal,
}

/// A texture in the cache: lower-case path (or procedural string) and use.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureKey {
    pub path: String,
    pub usage: TextureUse,
}

impl TextureKey {
    pub fn new(path: &str, usage: TextureUse) -> Self {
        TextureKey {
            path: path
                .trim_start_matches(['\\', '/'])
                .replace('/', "\\")
                .to_ascii_lowercase(),
            usage,
        }
    }
}

/// Work for the loader threads.
#[derive(Debug, Clone)]
pub enum Job {
    Model { id: u32, path: String },
    Texture(TextureKey),
}

/// A finished job.
#[derive(Debug)]
pub enum Loaded {
    Model {
        id: u32,
        result: Result<Box<PreparedModel>, String>,
    },
    Texture {
        key: TextureKey,
        result: Result<LoadedTexture, String>,
    },
}

/// A pool of loader threads.
pub struct Loader {
    jobs: Option<Sender<Job>>,
    results: Receiver<Loaded>,
    workers: Vec<JoinHandle<()>>,
    in_flight: usize,
}

impl Loader {
    /// Start `threads` workers reading from `vfs`.
    pub fn new(vfs: Vfs, threads: usize, options: TextureOptions) -> Loader {
        let (job_tx, job_rx) = channel::<Job>();
        let (result_tx, results) = channel();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let workers = (0..threads.max(1))
            .map(|i| {
                let jobs = Arc::clone(&job_rx);
                let results = result_tx.clone();
                let vfs = vfs.clone();
                std::thread::Builder::new()
                    .name(format!("model loader {i}"))
                    .spawn(move || worker(&vfs, &jobs, &results, options))
                    .expect("spawning a loader thread")
            })
            .collect();
        Loader {
            jobs: Some(job_tx),
            results,
            workers,
            in_flight: 0,
        }
    }

    pub fn submit(&mut self, job: Job) {
        if let Some(jobs) = &self.jobs
            && jobs.send(job).is_ok()
        {
            self.in_flight += 1;
        }
    }

    /// The next finished job, if any.
    pub fn try_recv(&mut self) -> Option<Loaded> {
        let loaded = self.results.try_recv().ok()?;
        self.in_flight -= 1;
        Some(loaded)
    }

    /// Jobs submitted and not yet received.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        // Closing the job channel ends the workers once they finish their current job.
        self.jobs = None;
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn worker(
    vfs: &Vfs,
    jobs: &Mutex<Receiver<Job>>,
    results: &Sender<Loaded>,
    options: TextureOptions,
) {
    loop {
        let job = {
            let Ok(rx) = jobs.lock() else { return };
            match rx.recv() {
                Ok(job) => job,
                Err(_) => return,
            }
        };
        let loaded = match job {
            Job::Model { id, path } => Loaded::Model {
                id,
                result: load_model(vfs, &path).map(Box::new),
            },
            Job::Texture(key) => {
                let bytes = if a3_paa::Procedural::is_procedural(&key.path) {
                    None
                } else {
                    vfs.open(&key.path).ok()
                };
                let result = decode_texture(
                    &key.path,
                    bytes.as_deref(),
                    key.usage == TextureUse::Normal,
                    options,
                )
                .map_err(|e| e.to_string());
                Loaded::Texture { key, result }
            }
        };
        if results.send(loaded).is_err() {
            return;
        }
    }
}

fn load_model(vfs: &Vfs, path: &str) -> Result<PreparedModel, String> {
    let bytes = vfs.open(path).map_err(|e| e.to_string())?;
    let model = a3_p3d::Model::from_bytes(&bytes).map_err(|e| format!("{path}: {e}"))?;
    Ok(PreparedModel::new(&model))
}
