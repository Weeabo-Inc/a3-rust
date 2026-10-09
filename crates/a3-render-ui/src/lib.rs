//! 2D UI rendering: the [`a3_ui::DrawList`] as batched textured quads, drawn in
//! [`Phase::Ui`](a3_render::Phase::Ui) over the finished frame.
//!
//! [`UiRenderer`] resolves the draw list's textures — PAA files read through a [`UiAssets`]
//! source (the game [`Vfs`](a3_vfs::Vfs)) and `#(...)` procedural strings — keeps them across
//! frames, and draws the quads in painter order with per-batch scissor clipping. [`UiFeature`]
//! is the handle to hand to [`Renderer::add_feature`](a3_render::Renderer::add_feature); the app
//! keeps a clone and locks it to set each frame's asset source and draw list. See [`UiRenderer`]
//! for a worked example.

mod assets;
mod cache;
mod decode;
mod geometry;
mod renderer;

pub use assets::{MemoryAssets, UiAssets, vfs_with_dir};
pub use cache::TextureInfo;
pub use decode::{UiTextureError, decode_ui_texture};
pub use geometry::{Batch, Geometry, UiVertex, WHITE_SLOT, build_geometry};
pub use renderer::{UiFeature, UiRenderer, UiStats};
