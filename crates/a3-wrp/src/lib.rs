//! Reader for binarized terrains: WRP files with signature `OPRW`.
//!
//! [`Terrain::parse`] reads a whole `.wrp` file into a [`Terrain`]: the heightmap, the land
//! cell grids (geography, sound, surface materials), mountain peaks, the material and model
//! lists, static entities, the road net, every placed object and the symbols of the 2D map.
//! [`Terrain::surface_height`] samples the heightmap the way the engine does.
//!
//! Shipped terrains (game build 2.22) are all version 25; the reader accepts versions 15 to 25.
//! Versions 23 and later compress large arrays with LZO, earlier ones with LZSS.
//!
//! [`TerrainBuilder`] and [`Terrain::to_bytes`] build small synthetic terrains for tests.
//!
//! See `docs/re/wrp.md` for the file layout.

mod compress;
mod cursor;
mod error;
mod geography;
mod grid;
mod map;
mod objects;
mod quadtree;
mod terrain;

pub use error::{Error, Result};
pub use geography::Geography;
pub use grid::{Grid, GridSize};
pub use map::{MapObject, MapShape, MapShapeKind, MapType};
pub use objects::{ObjectInstance, RoadConnection, RoadNet, RoadPart, StaticEntity, Transform};
pub use terrain::{
    LATEST_VERSION, MIN_VERSION, SIGNATURE, SOUND_MAP_SIZE_COEF, Terrain, TerrainBuilder,
    TerrainMaterial,
};
