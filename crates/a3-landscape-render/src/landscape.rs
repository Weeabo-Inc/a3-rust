//! The Landscape assembled from a parsed WRP and its layer materials.

use std::time::Instant;

use a3_core::VfsPath;
use a3_landscape::TerrainLayers;
use a3_render::TextureData;
use a3_wrp::{Grid, Terrain};

use crate::detail::DetailLayers;
use crate::heights::HeightField;
use crate::satellite::TileTable;

/// Pixels per satellite tile core in the overview atlas (Altis: 64 tiles -> 3840 px, 8 m/px).
pub const OVERVIEW_CORE_PIXELS: u32 = 60;

/// Overview colour of land without a satellite tile (sea floor).
pub const SEABED_RGBA: [u8; 4] = [120, 112, 90, 255];

/// World constants of the terrain shading (`CfgWorlds >> <world>`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainShading {
    /// Distance in metres up to which detail layers show fully (`fullDetailDist`).
    pub full_detail_dist: f32,
    /// Distance beyond which only the satellite shows (`noDetailDist`).
    pub no_detail_dist: f32,
    /// `terrainBlendMaxDarkenCoef`.
    pub max_darken: f32,
    /// `terrainBlendMaxBrightenCoef`.
    pub max_brighten: f32,
}

impl Default for TerrainShading {
    /// Altis' values.
    fn default() -> Self {
        TerrainShading {
            full_detail_dist: 10.0,
            no_detail_dist: 65.0,
            max_darken: 0.85,
            max_brighten: 0.15,
        }
    }
}

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
    /// The WRP material index of every land cell.
    pub material_indices: Grid<u16>,
    /// Every material's detail layers.
    pub detail: DetailLayers,
    pub shading: TerrainShading,
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
        let detail = DetailLayers::build(&layers.materials, &tiles.material_tiles);
        Landscape {
            heights: HeightField::from_terrain(terrain),
            material_indices: terrain.material_indices.clone(),
            detail,
            shading: TerrainShading::default(),
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
