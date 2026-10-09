//! wgpu renderer core.
//!
//! - [`Gpu`] / [`WindowSurface`]: device setup, surface resize and vsync.
//! - [`Camera`]: RV world space (left-handed, X east, Y up, Z north, `f64` positions), Arma-style
//!   `fovTop`, reversed-Z infinite projection, camera-relative rendering
//!   (`docs/adr/0003-coordinates-and-precision.md`); [`FreeFlyController`] for a debug camera.
//! - [`TextureData`] → [`Renderer::upload_texture`]: BC1/BC2/BC3/RGBA8 with mip chains, the
//!   shape PAA decoding produces.
//! - [`MeshData`] → [`Renderer::upload_mesh`]: indexed triangles with position, normal, UV and
//!   tangent; drawn per frame as instanced [`MeshDraw`]s.
//! - [`DrawList`]: this frame's meshes, [`DebugLines`] and debug text.
//! - [`RenderFeature`]: how further renderers (terrain, sky, particles) plug into the frame's
//!   phases (shadow, opaque, alpha, post, UI).
//! - [`HdrSettings`]: the post chain after the scene pass, following RV's HDR chain: sky and
//!   fog into HDR, log-average eye adaptation, bloom, `tonemapMethod` curves (`HDRNewPars`), FXAA.

pub mod camera;
mod draw;
mod feature;
pub mod font;
mod frustum;
mod gpu;
pub mod mesh;
mod post;
mod renderer;
pub mod residency;
pub mod roads;
pub mod shadow;
pub mod sky;
pub mod texture;

pub use camera::{Camera, Fov, FreeFlyController, FreeFlyInput};
pub use draw::{Color, DebugLines, DrawList, MeshDraw, MeshId, TextRun, TextureId};
pub use feature::{Phase, PrepareContext, RenderFeature};
pub use frustum::Frustum;
pub use gpu::{Gpu, RenderError, WindowSurface};
pub use mesh::{Mesh, MeshData, Vertex};
pub use post::{AntiAliasing, BloomSettings, FilmicCurve, HdrSettings, Tonemap};
pub use renderer::{ExposureReadout, HemisphereAmbient, RenderSettings, Renderer, WaterFog};
pub use residency::{
    PaaSource, ResidencyConfig, ResidencyStats, TextureHandle, TextureResidency, TextureSource,
};
pub use shadow::ShadowSettings;
pub use texture::{ColorSpace, GpuTexture, TextureData, TextureError, TextureFormat};

/// Re-export so users can name wgpu types without a direct dependency.
pub use wgpu;
