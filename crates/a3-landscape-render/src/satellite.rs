//! Satellite tiles: their grid over the terrain, which tile each land cell uses, and the
//! low-resolution overview atlas built from all of them.

use std::collections::HashMap;

use a3_core::VfsPath;
use a3_landscape::{LayerMaterial, UvSource, UvTransform};
use a3_paa::{PaaHeader, PixelFormat};
use a3_render::{TextureData, TextureFormat};
use a3_wrp::Grid;

/// Column (west to east) and row (counted from the **north** edge) of a satellite tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileCoord {
    pub col: u32,
    pub row: u32,
}

/// An affine map from world `x, z` (metres) to texture coordinates:
/// `u = u[0] * x + u[1] * z + u[2]`, `v = v[0] * x + v[1] * z + v[2]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileUv {
    pub u: [f32; 3],
    pub v: [f32; 3],
}

impl TileUv {
    /// The world-position transform of an rvmat stage, without its height term (the
    /// satellite and mask tiles are mapped straight down). `None` for other sources.
    pub fn from_transform(t: &UvTransform) -> Option<TileUv> {
        (t.source == UvSource::WorldPos).then_some(TileUv {
            u: [t.aside.x, t.dir.x, t.pos.x],
            v: [t.aside.y, t.dir.y, t.pos.y],
        })
    }

    /// Texture coordinates of world `(x, z)`.
    pub fn apply(&self, x: f32, z: f32) -> (f32, f32) {
        (
            self.u[0] * x + self.u[1] * z + self.u[2],
            self.v[0] * x + self.v[1] * z + self.v[2],
        )
    }
}

/// Marks a land cell without a satellite tile in [`TileTable::cell_tiles`].
pub const NO_TILE: u16 = u16::MAX;

/// The regular grid of satellite tiles. Tile `(col, row)` has its **core**, the square it is
/// responsible for, at `x = col * step .. (col + 1) * step` and, counting rows from the north,
/// `z = world - (row + 1) * step .. world - row * step`. Its texture covers `size` metres: the
/// core plus an overlap margin of `(size - step) / 2` on every side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SatelliteGrid {
    /// Tiles per axis.
    pub tiles: u32,
    /// Distance between tile origins in metres.
    pub step: f32,
    /// Edge of the area one tile texture covers in metres.
    pub size: f32,
}

impl SatelliteGrid {
    /// Derive the grid from a terrain of `world_size` metres and one tile's mapping.
    pub fn new(world_size: f32, tiles: u32, uv: &TileUv) -> Option<SatelliteGrid> {
        if tiles == 0 || uv.u[0] <= 0.0 {
            return None;
        }
        Some(SatelliteGrid {
            tiles,
            step: world_size / tiles as f32,
            size: 1.0 / uv.u[0],
        })
    }

    /// Overlap margin on each side of a tile in metres.
    pub fn overlap(&self) -> f32 {
        (self.size - self.step) * 0.5
    }

    /// For a tile texture of `px` pixels: the pixel offset where the core starts and the core
    /// width in pixels (rounded).
    pub fn core_pixels(&self, px: u32) -> (u32, u32) {
        let scale = px as f32 / self.size;
        (
            (self.overlap() * scale).round() as u32,
            (self.step * scale).round() as u32,
        )
    }
}

/// One satellite tile.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub coord: TileCoord,
    /// The tile's own satellite colour texture, if it has one.
    pub satellite: Option<VfsPath>,
    /// A satellite texture shared by many tiles instead (Altis' open-sea tiles all name the
    /// 32 px `a3\map_data\tiled_s_co.paa`); drawn as its average colour.
    pub shared_satellite: Option<VfsPath>,
    /// Layer mask texture.
    pub mask: Option<VfsPath>,
    /// World `x, z` to tile texture coordinates.
    pub uv: TileUv,
}

/// The satellite tiles of a terrain and which one every land cell uses.
#[derive(Debug, Clone, PartialEq)]
pub struct TileTable {
    pub grid: Option<SatelliteGrid>,
    pub tiles: Vec<Tile>,
    /// Per land cell (row-major, `z * width + x`, south first): index into `tiles` or
    /// [`NO_TILE`].
    pub cell_tiles: Grid<u16>,
}

impl TileTable {
    /// Build from the terrain's layer materials (indexed like the WRP material list, see
    /// `a3_landscape::TerrainLayers`) and its land-cell material indices.
    pub fn build(
        world_size: f32,
        materials: &[Option<LayerMaterial>],
        material_indices: &Grid<u16>,
    ) -> TileTable {
        let mut tiles: Vec<Tile> = Vec::new();
        let mut by_coord: HashMap<TileCoord, u16> = HashMap::new();
        let mut material_tile = vec![NO_TILE; materials.len()];
        let texture = |t: &str| (!t.is_empty() && !t.starts_with('#')).then(|| VfsPath::new(t));
        for (index, material) in materials.iter().enumerate() {
            let Some(material) = material else { continue };
            let Some((col, row)) = material.tile else {
                continue;
            };
            let coord = TileCoord { col, row };
            if let Some(&tile) = by_coord.get(&coord) {
                material_tile[index] = tile;
                continue;
            }
            let Some(uv) = TileUv::from_transform(&material.satellite.uv) else {
                continue;
            };
            let Ok(tile) = u16::try_from(tiles.len()) else {
                break;
            };
            tiles.push(Tile {
                coord,
                satellite: texture(&material.satellite.texture),
                shared_satellite: None,
                mask: texture(&material.mask.texture),
                uv,
            });
            by_coord.insert(coord, tile);
            material_tile[index] = tile;
        }
        // A satellite texture named by several tiles is a filler, not tile imagery.
        let mut uses: HashMap<VfsPath, u32> = HashMap::new();
        for path in tiles.iter().filter_map(|t| t.satellite.clone()) {
            *uses.entry(path).or_default() += 1;
        }
        for tile in &mut tiles {
            if tile.satellite.as_ref().is_some_and(|p| uses[p] > 1) {
                tile.shared_satellite = tile.satellite.take();
            }
        }
        let count = tiles
            .iter()
            .map(|t| t.coord.col.max(t.coord.row) + 1)
            .max()
            .unwrap_or(0);
        let grid = tiles
            .first()
            .and_then(|t| SatelliteGrid::new(world_size, count, &t.uv));
        let cells = material_indices
            .as_slice()
            .iter()
            .map(|&m| {
                material_tile
                    .get(usize::from(m))
                    .copied()
                    .unwrap_or(NO_TILE)
            })
            .collect();
        TileTable {
            grid,
            tiles,
            cell_tiles: Grid::from_vec(material_indices.size(), cells)
                .expect("same size as the material grid"),
        }
    }

    /// Build the overview atlas: every tile's core at about `core_target` pixels, placed by
    /// column and row (north at the top), with a full mip chain. Tiles with a shared texture,
    /// or too small a texture (Altis has 4x4 placeholders along the coast), get its average
    /// colour; land without a tile is `fallback`. Returns `None` without a tile grid.
    pub fn overview<B: AsRef<[u8]>>(
        &self,
        core_target: u32,
        fallback: [u8; 4],
        read: impl Fn(&VfsPath) -> Option<B>,
    ) -> Option<TextureData> {
        let grid = self.grid?;
        // Mip width giving a core of about `core_target` pixels.
        let want = (core_target as f32 * grid.size / grid.step).round() as u32;
        let (offset, core) = grid.core_pixels(want);
        let side = core * grid.tiles;
        let mut atlas: Vec<u8> = fallback
            .iter()
            .copied()
            .cycle()
            .take((side * side * 4) as usize)
            .collect();
        let mut averages: HashMap<VfsPath, Option<[u8; 4]>> = HashMap::new();
        let mut average_of = |path: &VfsPath| -> Option<[u8; 4]> {
            *averages.entry(path.clone()).or_insert_with(|| {
                let bytes = read(path)?;
                let c = PaaHeader::read(bytes.as_ref()).ok()?.meta.average_color?;
                Some([c.r, c.g, c.b, 255])
            })
        };
        let mut placeholders = 0;
        for tile in &self.tiles {
            let (x0, y0) = (tile.coord.col * core, tile.coord.row * core);
            let len = (core * 4) as usize;
            let pixels = tile
                .satellite
                .as_ref()
                .and_then(|path| read(path).and_then(|b| decode_mip(b.as_ref(), want)));
            if let Some(pixels) = pixels {
                for y in 0..core {
                    let src = (((y + offset) * want + offset) * 4) as usize;
                    let dst = (((y0 + y) * side + x0) * 4) as usize;
                    atlas[dst..dst + len].copy_from_slice(&pixels[src..src + len]);
                }
                continue;
            }
            let path = tile.satellite.as_ref().or(tile.shared_satellite.as_ref());
            let Some(average) = path.and_then(&mut average_of) else {
                continue;
            };
            if tile.satellite.is_some() {
                placeholders += 1;
            }
            for y in 0..core {
                let dst = (((y0 + y) * side + x0) * 4) as usize;
                for px in atlas[dst..dst + len].chunks_exact_mut(4) {
                    px.copy_from_slice(&average);
                }
            }
        }
        if placeholders > 0 {
            log::debug!("{placeholders} satellite tiles are placeholders without a {want} px mip");
        }
        Some(with_mips(side, atlas))
    }
}

/// Decode the `width`-pixel mip of a PAA to RGBA8.
fn decode_mip(bytes: &[u8], width: u32) -> Option<Vec<u8>> {
    let header = PaaHeader::read(bytes).ok()?;
    let index = header
        .mips
        .iter()
        .position(|m| u32::from(m.width) == width && u32::from(m.height) == width)?;
    let mip = header.read_mip(bytes, index).ok()?;
    let format = header.meta.format;
    let mut rgba = a3_paa::decode_rgba8(format, &mip).ok()?;
    if let Some(swizzle) = header.meta.swizzle {
        swizzle.restore(&mut rgba);
    }
    // Satellite tiles are opaque; DXT1 punch-through alpha would only confuse filtering.
    if format == PixelFormat::Dxt1 {
        rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Some(rgba)
}

/// A square RGBA8 image with a 2x2 box-filtered mip chain.
fn with_mips(side: u32, level0: Vec<u8>) -> TextureData {
    let mut mips = vec![level0];
    let mut size = side;
    while size > 1 {
        let next = (size / 2).max(1);
        let prev = mips.last().expect("level 0 exists");
        let mut out = vec![0u8; (next * next * 4) as usize];
        for y in 0..next {
            for x in 0..next {
                for c in 0..4 {
                    let at = |dx: u32, dy: u32| {
                        let (sx, sy) = ((2 * x + dx).min(size - 1), (2 * y + dy).min(size - 1));
                        u32::from(prev[((sy * size + sx) * 4 + c) as usize])
                    };
                    out[((y * next + x) * 4 + c) as usize] =
                        ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                }
            }
        }
        mips.push(out);
        size = next;
    }
    TextureData {
        format: TextureFormat::Rgba8,
        width: side,
        height: side,
        mips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_paa::{EncodeOptions, encode_rgba8};
    use a3_wrp::GridSize;

    /// The satellite stage of Altis' `p_030-030_l01_l02_l10_l14_l15.rvmat`.
    const ALTIS_RVMAT: &str = r#"
        class Stage0 { texture = "a3\map_altis\data\layers\00_00\s_030_030_lco.paa"; texGen = 3; };
        class Stage1 { texture = "a3\map_altis\data\layers\00_00\m_030_030_lca.paa"; texGen = 3; };
        class TexGen3 {
            uvSource = "worldPos";
            class uvTransform {
                aside[] = {0.001953125, 0, 0};
                up[] = {0, 0, 0.001953125};
                dir[] = {0, -0.001953125, 0};
                pos[] = {-28.09375, 31.90625, 0};
            };
        };
    "#;

    #[test]
    fn tile_uv_maps_world_position_onto_the_tile() {
        let config = a3_config::parse_text(ALTIS_RVMAT).unwrap();
        let material = LayerMaterial::from_config("p_030-030_l01.rvmat", &config).unwrap();
        let uv = TileUv::from_transform(&material.satellite.uv).unwrap();
        // Tile 30 spans x 14384..14896 (480 m step, 16 m overlap) and, being row 30 from the
        // north of a 30720 m map, z 15824..16336 with v = 0 at its north edge.
        let (u, v) = uv.apply(14_384.0, 16_336.0);
        assert!(u.abs() < 1e-4 && v.abs() < 1e-4, "{u} {v}");
        let (u, v) = uv.apply(14_896.0, 15_824.0);
        assert!((u - 1.0).abs() < 1e-4 && (v - 1.0).abs() < 1e-4, "{u} {v}");
        let tex = UvTransform::identity(UvSource::Tex);
        assert_eq!(TileUv::from_transform(&tex), None);
    }

    #[test]
    fn satellite_grid_follows_the_tile_mapping() {
        let s = 1.0 / 512.0;
        let altis = TileUv {
            u: [s, 0.0, -28.09375],
            v: [0.0, -s, 31.90625],
        };
        let grid = SatelliteGrid::new(30_720.0, 64, &altis).unwrap();
        assert_eq!(grid.step, 480.0);
        assert_eq!(grid.size, 512.0);
        assert_eq!(grid.overlap(), 16.0);
        assert_eq!(grid.core_pixels(64), (2, 60));
        assert_eq!(grid.core_pixels(512), (16, 480));
    }

    /// A 2x2-tile world of 2 land cells per axis: tile (0, 0) is north-west.
    struct World {
        materials: Vec<Option<LayerMaterial>>,
        indices: Grid<u16>,
        files: HashMap<VfsPath, Vec<u8>>,
    }

    fn rvmat(col: u32, row: u32, with_satellite: bool) -> String {
        // 100 m world, tiles of 50 m step and 60 m size (5 m overlap).
        let s = 1.0 / 60.0;
        let (ou, ov) = (
            -(col as f32 * 50.0 - 5.0) * s,
            (100.0 - row as f32 * 50.0 + 5.0) * s,
        );
        let texture = if with_satellite {
            format!(r"w\s_{col:03}_{row:03}_lco.paa")
        } else {
            r"w\tiled_s_co.paa".to_owned()
        };
        let stage0 = format!(r#"texture = "{texture}";"#);
        format!(
            r#"class Stage0 {{ {stage0} texGen = 3; }};
            class Stage1 {{ texture = ""; texGen = 3; }};
            class TexGen3 {{ uvSource = "worldPos"; class uvTransform {{
                aside[] = {{{s}, 0, 0}}; up[] = {{0, 0, {s}}}; dir[] = {{0, -{s}, 0}};
                pos[] = {{{ou}, {ov}, 0}}; }}; }};"#
        )
    }

    fn solid_paa(rgb: [u8; 3], px: u16) -> Vec<u8> {
        let pixels: Vec<u8> = (0..u32::from(px) * u32::from(px))
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect();
        encode_rgba8(px, px, &pixels, &EncodeOptions::new(PixelFormat::Argb8888))
            .unwrap()
            .to_bytes()
            .unwrap()
    }

    fn world() -> World {
        let mut files = HashMap::new();
        let mut materials = vec![None];
        for (col, row, sat) in [(0, 0, true), (1, 0, false), (0, 1, false), (1, 1, true)] {
            // Tiles (1, 0) and (0, 1) name the shared filler texture.
            let path = format!(r"w\p_{col:03}-{row:03}_l00.rvmat");
            let config = a3_config::parse_text(&rvmat(col, row, sat)).unwrap();
            materials.push(Some(LayerMaterial::from_config(&path, &config).unwrap()));
        }
        files.insert(
            VfsPath::new(r"w\s_000_000_lco.paa"),
            solid_paa([255, 0, 0], 12),
        );
        files.insert(
            VfsPath::new(r"w\s_001_001_lco.paa"),
            solid_paa([0, 0, 255], 12),
        );
        files.insert(VfsPath::new(r"w\tiled_s_co.paa"), solid_paa([9, 9, 9], 4));
        // Land cells (x, z), z = 0 south: south-west cell uses tile (0, 1), north-east (1, 0).
        let size = GridSize {
            width: 2,
            height: 2,
        };
        let indices = Grid::from_vec(size, vec![3, 4, 1, 2]).unwrap();
        World {
            materials,
            indices,
            files,
        }
    }

    #[test]
    fn land_cells_use_the_tile_of_their_layer_material() {
        let w = world();
        let table = TileTable::build(100.0, &w.materials, &w.indices);
        assert_eq!(table.tiles.len(), 4);
        let filler = &table.tiles[1];
        assert_eq!(filler.coord, TileCoord { col: 1, row: 0 });
        assert_eq!(
            (filler.satellite.clone(), filler.shared_satellite.clone()),
            (None, Some(VfsPath::new(r"w\tiled_s_co.paa"))),
            "tile (1, 0) shares the filler texture"
        );
        let coord = |x, z| {
            let t = *table.cell_tiles.get(x, z).unwrap();
            table.tiles[usize::from(t)].coord
        };
        assert_eq!(coord(0, 0), TileCoord { col: 0, row: 1 });
        assert_eq!(coord(1, 1), TileCoord { col: 1, row: 0 });
        let grid = table.grid.unwrap();
        assert_eq!((grid.tiles, grid.step), (2, 50.0));
        assert!((grid.size - 60.0).abs() < 1e-3);
    }

    #[test]
    fn overview_places_tile_cores_north_up_and_fills_gaps() {
        let w = world();
        let table = TileTable::build(100.0, &w.materials, &w.indices);
        // 12 px mips: 1 px overlap, 10 px core.
        let atlas = table
            .overview(10, [1, 2, 3, 255], |p| w.files.get(p))
            .unwrap();
        assert_eq!((atlas.width, atlas.height), (20, 20));
        assert_eq!(atlas.validate(), Ok(()));
        let px = |x: u32, y: u32| {
            let i = ((y * 20 + x) * 4) as usize;
            [atlas.mips[0][i], atlas.mips[0][i + 1], atlas.mips[0][i + 2]]
        };
        assert_eq!(px(0, 0), [255, 0, 0], "tile (0, 0) at the top left");
        assert_eq!(
            px(19, 0),
            [9, 9, 9],
            "tile (1, 0): the filler's average colour"
        );
        assert_eq!(px(0, 19), [9, 9, 9]);
        assert_eq!(px(19, 19), [0, 0, 255]);
        assert_eq!(atlas.mips.last().unwrap().len(), 4, "mips down to 1x1");
    }
}
