//! The Landscape assembled from a parsed WRP and its layer materials.

use std::time::Instant;

use a3_core::VfsPath;
use a3_landscape::TerrainLayers;
use a3_render::TextureData;
use a3_wrp::Terrain;

use crate::heights::HeightField;
use crate::satellite::TileTable;

/// Pixels per satellite tile core in the overview atlas (Altis: 64 tiles -> 3840 px, 8 m/px).
pub const OVERVIEW_CORE_PIXELS: u32 = 60;

/// Overview colour of land without a satellite tile (sea floor).
pub const SEABED_RGBA: [u8; 4] = [120, 112, 90, 255];

/// What the terrain renderer needs from a terrain.
#[derive(Debug, Clone)]
pub struct Landscape {
    pub heights: HeightField,
    /// Land cell edge in metres.
    pub land_cell: f32,
    /// Land cells per axis.
    pub land_cells: u32,
    /// Terrain edge in metres.
    pub world_size: f32,
    pub tiles: TileTable,
    /// Low-resolution satellite image of the whole terrain (north up), if it has tiles.
    pub overview: Option<TextureData>,
}

impl Landscape {
    /// Build from a parsed terrain and its layer materials; `read` returns VFS file contents
    /// (satellite tiles for the overview).
    pub fn from_terrain<B: AsRef<[u8]>>(
        terrain: &Terrain,
        layers: &TerrainLayers,
        read: impl Fn(&VfsPath) -> Option<B>,
    ) -> Landscape {
        let start = Instant::now();
        let world_size = terrain.world_size();
        let tiles = TileTable::build(world_size, &layers.materials, &terrain.material_indices);
        let tables = start.elapsed();
        let overview = tiles.overview(OVERVIEW_CORE_PIXELS, SEABED_RGBA, &read);
        log::info!(
            "landscape: {} tiles ({} with satellite), grid {:?}; tables {:.0?}, overview {:.0?}",
            tiles.tiles.len(),
            tiles.tiles.iter().filter(|t| t.satellite.is_some()).count(),
            tiles.grid,
            tables,
            start.elapsed() - tables
        );
        Landscape {
            heights: HeightField::from_terrain(terrain),
            land_cell: terrain.land_cell_size,
            land_cells: terrain.land_grid.width,
            world_size,
            tiles,
            overview,
        }
    }

    /// Terrain surface height at world `(x, z)` (engine triangulation, clamped at the edge).
    pub fn surface_height(&self, x: f32, z: f32) -> f32 {
        self.heights.sample(x, z)
    }
}
