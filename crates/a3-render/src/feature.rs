//! Extension point for render features (terrain, sky, particles, ...).

use glam::Mat4;

use crate::camera::Camera;
use crate::draw::DrawList;

/// The phases of a frame, in order.
///
/// 1. **Shadow**: one depth pass per sun shadow cascade (opaque meshes and features).
/// 2. **Opaque** and 3. **Alpha**: one render pass into the HDR scene colour
///    ([`Renderer::SCENE_COLOR_FORMAT`](crate::Renderer::SCENE_COLOR_FORMAT)) with the
///    reversed-Z depth buffer ([`Renderer::DEPTH_FORMAT`](crate::Renderer::DEPTH_FORMAT), clear
///    0, compare `Greater`). Alpha draws come after all opaque draws, without depth writes.
/// 4. **Post**: the renderer adds sky and fog into an HDR image, measures its luminance
///    (eye adaptation), tonemaps it and anti-aliases it into the output target.
/// 5. **Ui**: one render pass into the output target (no depth), for overlays and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Depth-only pass into one sun shadow cascade. Group 0 holds a copy of the frame uniforms
    /// whose `view_proj` is the cascade's light matrix; pipelines render depth only into
    /// [`SHADOW_FORMAT`](crate::shadow::SHADOW_FORMAT) (compare `Less`, cleared to 1).
    Shadow {
        cascade: u32,
    },
    Opaque,
    Alpha,
    Ui,
}

/// Per-frame information handed to features before any pass begins.
pub struct PrepareContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub camera: &'a Camera,
    /// Projection times rotation-only view: transforms camera-relative positions to clip space.
    pub view_projection: Mat4,
    /// Output size in pixels.
    pub viewport: (u32, u32),
    pub draws: &'a DrawList,
}

/// A renderer extension that owns its GPU resources and draws in one or more phases.
///
/// Pipelines must use [`Renderer::frame_layout`](crate::Renderer::frame_layout) as bind group
/// 0; the renderer binds the frame uniforms there before calling [`draw`](Self::draw). Group 0
/// also carries the sun shadow maps (bindings 1-3, see `shaders/mesh.wgsl` for the
/// `sun_visibility` lookup), so receivers such as terrain get shadows from the same data. All
/// positions must be camera-relative (`world - camera.position`, computed in `f64`).
pub trait RenderFeature {
    /// Upload this frame's data (instance buffers, uniforms). Called once per frame.
    fn prepare(&mut self, cx: &PrepareContext<'_>);

    /// Record draws for `phase` into `pass`. Group 0 is already bound.
    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>);
}
