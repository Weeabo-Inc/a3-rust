//! A terrain's description beyond the WRP: its world config (`CfgWorlds >> <world>`), the
//! surface types (`CfgSurfaces`, `CfgSurfaceCharacters`) and the layer materials (rvmats) that
//! tell the renderer which satellite tile, mask tile and surface layers each land cell uses.
//!
//! - [`WorldConfig::load`] reads one world class.
//! - [`Surfaces::load`] reads every surface type and clutter character.
//! - [`TerrainLayers::load`] reads the rvmat of every material of a [`Terrain`]; then
//!   [`TerrainLayers::cell`] answers "what do I draw on land cell `(x, z)`".
//!
//! See `docs/re/landscape.md`.

mod cfg;
mod error;
mod layers;
mod material;
mod surfaces;
mod world;

pub use error::Error;
pub use layers::{CellSurface, TerrainLayers};
pub use material::{LayerMaterial, SurfaceLayer, TextureStage, UvSource, UvTransform};
pub use surfaces::{Surface, SurfaceCharacter, Surfaces};
pub use world::{
    AmbientRadius, AmbientSpecies, ClutterModel, EnvMap, GridZoom, Location, MapGrid, OutsideLayer,
    OutsideTerrain, Sea, Sky, WorldConfig, world_classes,
};

pub use a3_wrp::Terrain;
