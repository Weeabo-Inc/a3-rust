//! Placed objects and the single-model viewer, drawn by the `a3-render-models` feature from the
//! user's game data.

use a3_render::{Camera, DrawList, FreeFlyInput, Gpu, Renderer};
use a3_render_models::{
    LodSelector, ModelFeature, ModelId, ModelRenderer, ModelStats, ObjectsQuality, PlacedObject,
    TextureOptions,
};
use a3_vfs::Vfs;
use glam::{DAffine3, DMat3, DVec3};

/// Command-line settings for placed objects.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectOptions {
    /// `ObjectsQuality`: LOD coefficients.
    pub quality: ObjectsQuality,
    /// Object view distance in metres.
    pub view_distance: f32,
}

impl Default for ObjectOptions {
    fn default() -> Self {
        ObjectOptions {
            quality: ObjectsQuality::High,
            view_distance: 1600.0,
        }
    }
}

/// What the model viewer shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSpec {
    /// VFS path of the P3D.
    pub path: String,
    /// Camera heading and pitch in degrees.
    pub yaw: f32,
    pub pitch: f32,
    /// Camera distance in model radii.
    pub zoom: f32,
    /// Draw this Resolution LOD instead of choosing one.
    pub lod: Option<usize>,
    /// Pose the model as a Man in this Move (`CfgMovesMaleSdr` class) at this phase.
    pub pose: Option<(String, f32)>,
}

/// A terrain's placed objects, loaded with the World.
pub struct WorldObjects {
    pub vfs: Vfs,
    /// Model paths, indexed by `placed`.
    pub models: Vec<String>,
    /// (model index, world transform).
    pub placed: Vec<(u32, DAffine3)>,
    pub options: ObjectOptions,
}

impl WorldObjects {
    /// The objects of a parsed WRP.
    pub fn from_terrain(terrain: &a3_wrp::Terrain, vfs: Vfs, options: ObjectOptions) -> Self {
        WorldObjects {
            vfs,
            models: terrain
                .models
                .iter()
                .map(|p| p.as_str().to_owned())
                .collect(),
            placed: terrain
                .objects
                .iter()
                .map(|o| (o.model_index, object_transform(&o.transform)))
                .collect(),
            options,
        }
    }

    /// Register a model renderer with `renderer` drawing these objects.
    pub fn attach(self, gpu: &Gpu, renderer: &mut Renderer) -> ModelFeature {
        let feature = model_feature(gpu, renderer, self.vfs, 1024);
        {
            let mut m = feature.lock();
            m.settings.view_distance = self.options.view_distance;
            m.settings.lod = LodSelector::new(self.options.quality);
            let ids: Vec<ModelId> = self.models.iter().map(|p| m.model(p)).collect();
            let objects = self
                .placed
                .iter()
                .filter_map(|&(index, transform)| {
                    Some(PlacedObject {
                        model: *ids.get(index as usize)?,
                        transform,
                    })
                })
                .collect();
            m.set_static_objects(objects);
        }
        feature
    }
}

/// A WRP object transform (columns aside, up, dir, position) as a world transform.
pub fn object_transform(t: &a3_wrp::Transform) -> DAffine3 {
    let m = t.0.map(f64::from);
    DAffine3 {
        matrix3: DMat3::from_cols_array(&[m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7], m[8]]),
        translation: DVec3::new(m[9], m[10], m[11]),
    }
}

/// A model renderer reading from `vfs`, registered with `renderer`; textures capped at
/// `max_texture` pixels.
pub fn model_feature(
    gpu: &Gpu,
    renderer: &mut Renderer,
    vfs: Vfs,
    max_texture: u32,
) -> ModelFeature {
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .saturating_sub(2)
        .clamp(2, 12);
    model_feature_with(gpu, renderer, vfs, max_texture, threads)
}

/// Like [`model_feature`], with an explicit loader thread count. The player's one Man model
/// needs far fewer threads than a terrain's worth of objects.
pub fn model_feature_with(
    gpu: &Gpu,
    renderer: &mut Renderer,
    vfs: Vfs,
    max_texture: u32,
    threads: usize,
) -> ModelFeature {
    let mut models = ModelRenderer::new(
        &gpu.device,
        renderer.frame_layout(),
        vfs,
        TextureOptions {
            max_size: max_texture,
            bc_supported: Renderer::supports_bc(gpu),
        },
        threads,
    );
    models.settings.shadow_distance = if renderer.settings.shadows.enabled {
        renderer.settings.shadows.distance
    } else {
        0.0
    };
    let feature = ModelFeature::new(models);
    renderer.add_feature(Box::new(feature.clone()));
    feature
}

/// One overlay line of model renderer statistics.
pub fn stats_line(stats: &ModelStats) -> String {
    format!(
        "OBJ {} INST {} DRAWS {}  MODELS {} (+{} LOADING, {} FAILED)  TEX {} MAT {}",
        stats.candidates,
        stats.instances,
        stats.draw_calls,
        stats.models_ready,
        stats.models_pending,
        stats.models_failed,
        stats.textures,
        stats.materials,
    )
}

/// The model viewer: one model at `centre` with an orbit camera.
pub struct Orbit {
    pub spec: ModelSpec,
    model: ModelId,
    centre: DVec3,
    yaw: f32,
    pitch: f32,
    /// Camera distance in model radii.
    zoom: f32,
    radius: f32,
    lods: usize,
    /// The Man animation posing the model (`ModelSpec::pose`), if any.
    man: Option<crate::man::ManAnimation>,
}

/// How far below the orbit centre a posed Man's ground point goes: about his hip height, so the
/// camera circles his middle.
const POSED_CENTRE_HEIGHT: f64 = 0.9;

impl Orbit {
    /// Pose the model with `man` (already switched into its Move).
    pub fn with_man(mut self, man: crate::man::ManAnimation) -> Orbit {
        self.man = Some(man);
        self
    }

    /// Load `spec` into `models` and look at it from around `centre`.
    pub fn new(models: &ModelFeature, spec: ModelSpec, centre: DVec3) -> Orbit {
        let mut m = models.lock();
        m.settings.force_lod = spec.lod;
        let model = m.model(&spec.path);
        m.preload(model);
        Orbit {
            model,
            centre,
            yaw: spec.yaw.to_radians(),
            pitch: spec.pitch.to_radians(),
            zoom: spec.zoom.clamp(0.3, 20.0),
            radius: 2.0,
            lods: 0,
            spec,
            man: None,
        }
    }

    /// Turn and zoom from free-camera input (look turns, forward zooms) and place the camera.
    pub fn update(
        &mut self,
        camera: &mut Camera,
        fly: &FreeFlyInput,
        look: f32,
        dt: f64,
        models: &ModelFeature,
    ) {
        self.yaw += fly.yaw * look;
        self.pitch = (self.pitch + fly.pitch * look).clamp(-1.5, 1.5);
        self.zoom = (self.zoom * (1.0 - fly.forward * dt as f32)).clamp(0.3, 20.0);
        if let Some((radius, lods)) = models.lock().model_bounds(self.model) {
            self.radius = radius.max(0.05);
            self.lods = lods;
        }
        camera.yaw = self.yaw;
        camera.pitch = self.pitch;
        let distance = f64::from(self.radius * self.zoom);
        camera.position = self.centre - camera.forward().as_dvec3() * distance;
        camera.near = (self.radius * 0.01).clamp(0.01, 0.5);
    }

    /// Place the model for this frame and draw reference axes and a grid below it.
    pub fn draw(&self, draws: &mut DrawList, models: &ModelFeature) {
        let mut m = models.lock();
        m.clear_dynamic();
        m.clear_skinned();
        match self
            .man
            .as_ref()
            .and_then(|man| Some((man.palette()?, man.ground())))
        {
            Some((bones, ground)) => {
                let feet = self.centre - DVec3::new(0.0, POSED_CENTRE_HEIGHT, 0.0);
                let transform = DAffine3::from_translation(feet - ground.as_dvec3());
                m.add_skinned(
                    PlacedObject {
                        model: self.model,
                        transform,
                    },
                    &bones,
                );
            }
            None => m.add_dynamic(PlacedObject {
                model: self.model,
                transform: DAffine3::from_translation(self.centre),
            }),
        }
        let r = f64::from(self.radius);
        draws.lines.axes(self.centre, r * 0.5);
        draws.lines.grid(
            self.centre,
            self.centre.y - r,
            r * 0.25,
            8,
            [0.3, 0.32, 0.3, 1.0],
        );
    }

    /// Overlay line describing the model.
    pub fn describe(&self, models: &ModelFeature) -> String {
        if models.lock().model_failed(self.model) {
            return format!(
                "{}: FAILED TO LOAD (SEE LOG)",
                self.spec.path.to_uppercase()
            );
        }
        let move_ = self
            .man
            .as_ref()
            .map(|man| format!("  MOVE {}", man.move_name()))
            .unwrap_or_default();
        format!(
            "{}  RADIUS {:.2} M  {} LODS{move_}",
            self.spec.path.to_uppercase().replace('\\', "/"),
            self.radius,
            self.lods
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn wrp_transforms_keep_axes_and_position() {
        let mut t = a3_wrp::Transform::from_position(Vec3::new(10.0, 20.0, 30.0));
        // Heading 90 degrees: the model's z (dir) axis points east.
        t.0[..9].copy_from_slice(&[0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
        let a = object_transform(&t);
        assert_eq!(a.translation, DVec3::new(10.0, 20.0, 30.0));
        assert_eq!(a.transform_vector3(DVec3::Z), DVec3::X);
    }
}
