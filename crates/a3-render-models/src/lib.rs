//! ODOL model rendering.
//!
//! - [`shader`]: the engine's `PixelShaderID` enum and our [`ShaderFamily`]s.
//! - [`material`]: a section's texture and embedded rvmat as a [`MaterialDesc`] (texture
//!   [`Slot`]s with UV transforms, colours, [`AlphaMode`]); [`rvmat`] parses loose rvmats.
//! - [`prepare`]: a decoded P3D as [`PreparedModel`]: per Resolution LOD one vertex/index
//!   buffer, index ranges per material, proxies.
//! - [`texture`]: PAA and procedural textures as upload-ready data, size-capped.
//! - [`lod`], [`cull`], [`batch`]: LOD selection by screen size, frustum and grid culling,
//!   instance batching.
//! - [`ModelRenderer`] / [`ModelFeature`]: the [`a3_render::RenderFeature`] drawing placed
//!   objects, with background loading through [`loader`].

pub mod batch;
pub mod cull;
pub mod loader;
pub mod lod;
pub mod material;
pub mod prepare;
mod renderer;
pub mod rvmat;
pub mod shader;
pub mod skin;
pub mod texture;

pub use lod::{LodSelector, ObjectsQuality};
pub use material::{AlphaMode, MaterialDesc, Slot};
pub use prepare::PreparedModel;
pub use renderer::{ModelFeature, ModelId, ModelRenderer, ModelSettings, ModelStats, PlacedObject};
pub use shader::{PixelShader, ShaderFamily};
pub use skin::{SkinData, SkinVertex};
pub use texture::TextureOptions;
