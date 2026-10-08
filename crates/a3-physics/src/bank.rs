//! The model collision cache: one [`ModelCollision`] per model path, plus scaled variants.

use std::collections::HashMap;
use std::sync::Arc;

use a3_config::ConfigTree;
use a3_p3d::{LodKind, LodResolution, Model};

use crate::files::{FileSource, normalize};
use crate::{ModelCollision, SurfaceBank};

/// Loads models through a [`FileSource`] and keeps their collision shapes for the session.
pub struct ModelBank {
    files: Arc<dyn FileSource>,
    surfaces: SurfaceBank,
    models: HashMap<String, Option<Arc<ModelCollision>>>,
    scaled: HashMap<(String, i64), Option<Arc<ModelCollision>>>,
}

impl std::fmt::Debug for ModelBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelBank")
            .field("models", &self.models.len())
            .field("scaled", &self.scaled.len())
            .field("surfaces", &self.surfaces.len())
            .finish()
    }
}

/// Scales within this of 1 use the unscaled shapes.
const SCALE_TOLERANCE: f64 = 1e-3;

/// The cache key of a model path: normalised, `.p3d` added when the path has no extension
/// (config `model` values often omit it).
pub(crate) fn model_key(path: &str) -> String {
    let mut key = normalize(path);
    let file = key.rsplit('\\').next().unwrap_or("");
    if !file.contains('.') {
        key.push_str(".p3d");
    }
    key
}

/// The LODs collision is built from; the others are not decoded.
fn is_collision_lod(resolution: LodResolution) -> bool {
    matches!(
        resolution.kind(),
        LodKind::Geometry
            | LodKind::GeometryPhysx
            | LodKind::FireGeometry
            | LodKind::ViewGeometry
            | LodKind::Roadway
    )
}

impl ModelBank {
    /// A bank reading `.p3d` and `.bisurf` files from `files`; `config` supplies `CfgSurfaces`.
    pub fn new(files: Arc<dyn FileSource>, config: Option<&ConfigTree>) -> Self {
        Self {
            surfaces: SurfaceBank::new(files.clone(), config),
            files,
            models: HashMap::new(),
            scaled: HashMap::new(),
        }
    }

    /// The collision of the model at `path`, loading it on first use. `None` if the file is
    /// missing, does not parse, or has no collision LOD.
    pub fn get(&mut self, path: &str) -> Option<Arc<ModelCollision>> {
        let key = model_key(path);
        if let Some(m) = self.models.get(&key) {
            return m.clone();
        }
        let built = self
            .files
            .read(&key)
            .and_then(|bytes| Model::from_bytes_with_lods(&bytes, is_collision_lod).ok())
            .map(|model| ModelCollision::from_model(&model, &mut self.surfaces))
            .filter(|m| !m.is_empty())
            .map(Arc::new);
        self.models.insert(key, built.clone());
        built
    }

    /// [`get`](Self::get) uniformly scaled by `scale`; cached per millimetre of scale.
    pub fn get_scaled(&mut self, path: &str, scale: f64) -> Option<Arc<ModelCollision>> {
        if (scale - 1.0).abs() < SCALE_TOLERANCE {
            return self.get(path);
        }
        let key = (model_key(path), (scale * 1000.0).round() as i64);
        if let Some(m) = self.scaled.get(&key) {
            return m.clone();
        }
        let scaled = self
            .get(path)
            .map(|m| Arc::new(m.scaled(key.1 as f64 / 1000.0)));
        self.scaled.insert(key, scaled.clone());
        scaled
    }

    /// Registers a model built elsewhere (synthetic models in tests, models made at run time).
    pub fn insert(&mut self, path: &str, model: ModelCollision) -> Arc<ModelCollision> {
        let model = Arc::new(model);
        self.models.insert(model_key(path), Some(model.clone()));
        model
    }

    /// Builds the collision of `model` with this bank's surfaces and registers it at `path`.
    pub fn insert_model(&mut self, path: &str, model: &Model) -> Arc<ModelCollision> {
        let collision = ModelCollision::from_model(model, &mut self.surfaces);
        self.insert(path, collision)
    }

    pub fn surfaces(&self) -> &SurfaceBank {
        &self.surfaces
    }

    pub fn surfaces_mut(&mut self) -> &mut SurfaceBank {
        &mut self.surfaces
    }

    /// Number of distinct model paths requested so far (including ones without collision).
    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }
}
