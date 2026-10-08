//! Draws the Landscape: the terrain of a World.
//!
//! - [`Landscape`]: what the renderer needs from a parsed WRP and its layer materials
//!   (`a3_landscape::TerrainLayers`): the [`HeightField`], the satellite [`TileTable`] and a
//!   low-resolution overview atlas of all satellite tiles.
//! - [`LodQuadtree`]: CDLOD level-of-detail selection over the heightmap.
//! - [`TerrainRenderer`]: the [`a3_render::RenderFeature`] that draws terrain patches with the
//!   overview and streamed full-resolution satellite tiles, and the sea.
//!
//! See `docs/re/render-terrain.md` and `docs/adr/0007-terrain-lod.md`.

pub mod heights;
pub mod landscape;
pub mod lod;
pub mod render;
pub mod residency;
pub mod satellite;
pub mod stream;

pub use heights::{HeightField, MinMaxPyramid};
pub use landscape::Landscape;
pub use lod::{LodQuadtree, LodSettings, SelectedNode};
pub use render::{TerrainRenderer, TerrainStats};
pub use satellite::{NO_TILE, SatelliteGrid, Tile, TileCoord, TileTable, TileUv};
pub use stream::{FileReader, TileFormat};
