//! The layer material of every land cell of a terrain.

use a3_config::{Config, is_rap, parse_text, read_rap};
use a3_vfs::Vfs;
use a3_wrp::Terrain;

use crate::Error;
use crate::material::{LayerMaterial, SurfaceLayer, TextureStage};

/// The layer materials of a terrain, indexed like [`Terrain::materials`].
#[derive(Debug, Default)]
pub struct TerrainLayers {
    /// One entry per terrain material; `None` for the empty entry 0 and for materials that
    /// failed to load.
    pub materials: Vec<Option<LayerMaterial>>,
    /// Materials that failed to load.
    pub errors: Vec<Error>,
}

/// What the renderer draws on one land cell.
#[derive(Debug, Clone, Copy)]
pub struct CellSurface<'a> {
    /// The land cell `(x, z)`.
    pub cell: (u32, u32),
    /// Index into [`Terrain::materials`].
    pub material_index: u16,
    /// The cell's layer material.
    pub material: &'a LayerMaterial,
}

impl<'a> CellSurface<'a> {
    /// The satellite colour tile and its world-position UV transform.
    pub fn satellite(&self) -> &'a TextureStage {
        &self.material.satellite
    }

    /// The layer mask tile and its world-position UV transform.
    pub fn mask(&self) -> &'a TextureStage {
        &self.material.mask
    }

    /// The satellite normal map tile, if the material has one.
    pub fn satellite_normal(&self) -> Option<&'a TextureStage> {
        self.material.satellite_normal.as_ref()
    }

    /// The surface layers blended by the mask.
    pub fn layers(&self) -> &'a [SurfaceLayer] {
        &self.material.layers
    }
}

impl TerrainLayers {
    /// Reads every layer material of `terrain` from the VFS (rapified or text rvmats).
    pub fn load(vfs: &Vfs, terrain: &Terrain) -> Self {
        Self::load_with(terrain, |path| {
            let bytes = vfs.open(path).map_err(|source| Error::Vfs {
                path: path.to_owned(),
                source,
            })?;
            parse_material(path, &bytes)
        })
    }

    /// Reads every layer material of `terrain` through `read`, which returns the parsed rvmat
    /// of a path.
    pub fn load_with(
        terrain: &Terrain,
        mut read: impl FnMut(&str) -> Result<Config, Error>,
    ) -> Self {
        let mut out = Self::default();
        for m in &terrain.materials {
            if m.path.is_root() {
                out.materials.push(None);
                continue;
            }
            let path = m.path.as_str();
            match read(path).and_then(|c| LayerMaterial::from_config(path, &c)) {
                Ok(material) => out.materials.push(Some(material)),
                Err(e) => {
                    out.errors.push(e);
                    out.materials.push(None);
                }
            }
        }
        out
    }

    /// The surface of land cell `(x, z)`, or `None` outside the grid or when the cell's
    /// material is missing.
    pub fn cell<'a>(&'a self, terrain: &Terrain, x: u32, z: u32) -> Option<CellSurface<'a>> {
        let material_index = *terrain.material_indices.get(x, z)?;
        let material = self.materials.get(usize::from(material_index))?.as_ref()?;
        Some(CellSurface {
            cell: (x, z),
            material_index,
            material,
        })
    }

    /// The surface under world position `(x, z)` in metres.
    pub fn at_world<'a>(&'a self, terrain: &Terrain, x: f32, z: f32) -> Option<CellSurface<'a>> {
        if x < 0.0 || z < 0.0 {
            return None;
        }
        let size = terrain.land_cell_size;
        self.cell(terrain, (x / size) as u32, (z / size) as u32)
    }
}

/// Parses rvmat bytes, rapified or text.
pub(crate) fn parse_material(path: &str, bytes: &[u8]) -> Result<Config, Error> {
    let parse_error = |detail: String| Error::Parse {
        path: path.to_owned(),
        detail,
    };
    if is_rap(bytes) {
        read_rap(bytes).map_err(|e| parse_error(e.to_string()))
    } else {
        parse_text(&a3_gamedata::decode_text(bytes)).map_err(|e| parse_error(e.to_string()))
    }
}
