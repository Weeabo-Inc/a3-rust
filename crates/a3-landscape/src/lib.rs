//! A terrain's description beyond the WRP: its world config (`CfgWorlds >> <world>`), the
//! surface types (`CfgSurfaces`, `CfgSurfaceCharacters`) and the layer materials (rvmats) that
//! tell the renderer which satellite tile, mask tile and surface layers each land cell uses.
//!
//! - [`WorldConfig::load`] reads one world class.
//! - [`Surfaces::load`] reads every surface type and clutter character.
//! - [`TerrainLayers::load`] reads the rvmat of every material of a [`Terrain`]; then
//!   [`TerrainLayers::cell`] answers "what do I draw on land cell `(x, z)`".
//! - [`RoadNetwork::load`] reads the road polylines (`roads.shp`, `roads.dbf`) and their types
//!   (`RoadsLib.cfg`); [`shapefile`] holds the generic ESRI shapefile and dBase readers.
//!
//! See `docs/re/landscape.md`.

mod cfg;
mod error;
mod layers;
mod material;
mod roads;
pub mod shapefile;
mod surfaces;
mod world;

pub use error::Error;
pub use layers::{CellSurface, TerrainLayers};
pub use material::{LayerMaterial, SurfaceLayer, TextureStage, UvSource, UvTransform};
pub use roads::{EASTING_OFFSET, Road, RoadNetwork, RoadType, RoadsError, RoadsLib, to_world};
pub use surfaces::{Surface, SurfaceCharacter, Surfaces};
pub use world::{
    AmbientRadius, AmbientSpecies, ClutterModel, EnvMap, GridZoom, Location, MapGrid, OutsideLayer,
    OutsideTerrain, Sea, Sky, WorldConfig, world_classes,
};

pub use a3_wrp::Terrain;
