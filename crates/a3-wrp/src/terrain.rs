//! The [`Terrain`] model and its (de)serialisation.

use std::collections::HashMap;

use a3_core::VfsPath;
use glam::Vec3;

use crate::cursor::{Codec, Reader, Writer};
use crate::geography::Geography;
use crate::grid::{Grid, GridSize};
use crate::map::{MapObject, read_map_object, write_map_object};
use crate::objects::{ObjectInstance, RoadConnection, RoadNet, RoadPart, StaticEntity, Transform};
use crate::{Error, Result, quadtree};

/// The file signature.
pub const SIGNATURE: &[u8; 4] = b"OPRW";

/// The version written by [`Terrain::to_bytes`] and found in every shipped terrain.
pub const LATEST_VERSION: u32 = 25;

/// The oldest version the reader accepts (the engine accepts 15 to 25, and 3).
pub const MIN_VERSION: u32 = 15;

/// The default `soundMapSizeCoef` of CfgWorlds: the sound map has this many cells per land cell
/// along each axis.
pub const SOUND_MAP_SIZE_COEF: u32 = 4;

/// Bytes of one object record.
const OBJECT_SIZE: usize = 60;

/// One entry of the terrain's material list: a surface layer material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainMaterial {
    /// The `.rvmat` path (`p_XXX-YYY_lNN...rvmat` for satellite/mask tiles). Entry 0 is
    /// usually empty.
    pub path: VfsPath,
    /// The "major texture" byte (version 17+; -1 before) _(meaning uncertain; 0 in shipped
    /// terrains)_.
    pub major: i8,
}

/// A binarized terrain (WRP, `OPRW`): everything the file holds.
///
/// Grid conventions: cell `(x, z)` with `x` east and `z` north; height sample `(i, j)` sits at
/// world `(i * terrain_cell_size, j * terrain_cell_size)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Terrain {
    /// File format version.
    pub version: u32,
    /// Steam app id of the game or DLC the terrain belongs to (version 25+).
    pub app_id: Option<u32>,
    /// Size of one land (layer) cell in metres.
    pub land_cell_size: f32,
    /// The land grid: geography, sound, material and object cells.
    pub land_grid: GridSize,
    /// Geography flags per land cell.
    pub geography: Grid<Geography>,
    /// Sound environment index per sound cell. The sound grid is
    /// [`SOUND_MAP_SIZE_COEF`] (a CfgWorlds value) times finer than the land grid.
    pub sound_map: Grid<u8>,
    /// Mountain peaks (positions of local height maxima).
    pub mountains: Vec<Vec3>,
    /// Index into [`Terrain::materials`] per land cell.
    pub material_indices: Grid<u16>,
    /// Randomisation values per land cell (versions before 21 only).
    pub random: Option<Grid<u16>>,
    /// Grass approximation per height sample (version 18+).
    pub grass_approx: Option<Grid<u8>>,
    /// Primary texture index per height sample (version 22+).
    pub primary_texture: Option<Grid<u8>>,
    /// Terrain heights above sea level in metres, one per height sample.
    pub heightmap: Grid<f32>,
    /// Surface layer materials.
    pub materials: Vec<TerrainMaterial>,
    /// Model paths of placed objects.
    pub models: Vec<VfsPath>,
    /// Static entities (version 15+).
    pub entities: Vec<StaticEntity>,
    /// Byte offset of each land cell's first object in the object block.
    pub object_offsets: Grid<u32>,
    /// Byte offset of each land cell's first map object in the map block.
    pub map_object_offsets: Grid<u32>,
    /// Persistence flags per land cell.
    pub persistent: Grid<u8>,
    /// Subdivision hints per height sample (zero in shipped terrains).
    pub subdivision_hints: Grid<u8>,
    /// The highest object id in use.
    pub max_object_id: u32,
    /// The road net.
    pub roads: RoadNet,
    /// Every placed object, in file order (grouped by land cell).
    pub objects: Vec<ObjectInstance>,
    /// Symbols for the 2D map, in file order (grouped by land cell).
    pub map_objects: Vec<MapObject>,
}

impl Terrain {
    /// Parses a whole WRP file.
    pub fn parse(data: &[u8]) -> Result<Self> {
        Parser::new(data)?.terrain()
    }

    /// Size of one height sample cell in metres.
    pub fn terrain_cell_size(&self) -> f32 {
        self.world_size() / self.heightmap.width() as f32
    }

    /// Edge length of the square terrain in metres.
    pub fn world_size(&self) -> f32 {
        self.land_cell_size * self.land_grid.width as f32
    }

    /// The model of `object`.
    pub fn model_of(&self, object: &ObjectInstance) -> Option<&VfsPath> {
        self.models.get(object.model_index as usize)
    }

    /// The material of the land cell `(x, z)`.
    pub fn material_at(&self, x: u32, z: u32) -> Option<&TerrainMaterial> {
        let index = *self.material_indices.get(x, z)?;
        self.materials.get(usize::from(index))
    }

    /// The height sample `(i, j)`, clamped to the grid edge outside it.
    pub fn grid_height(&self, i: i64, j: i64) -> f32 {
        *self.heightmap.get_clamped(i, j)
    }

    /// Terrain surface height at world `(x, z)`, interpolated the way the engine does.
    ///
    /// Each height cell is split into two triangles along the diagonal from `(i + 1, j)` to
    /// `(i, j + 1)`; the height is linear on each triangle. Outside the grid the edge heights
    /// are extended (the engine synthesises outside terrain instead; see `docs/re/wrp.md`).
    pub fn surface_height(&self, x: f32, z: f32) -> f32 {
        let inv = 1.0 / self.terrain_cell_size();
        let (gx, gz) = (x * inv, z * inv);
        let (fi, fj) = (gx.floor(), gz.floor());
        let (fx, fz) = (gx - fi, gz - fj);
        let (i, j) = (fi as i64, fj as i64);
        let h00 = self.grid_height(i, j);
        let h10 = self.grid_height(i + 1, j);
        let h01 = self.grid_height(i, j + 1);
        let h11 = self.grid_height(i + 1, j + 1);
        if fx + fz <= 1.0 {
            h00 + (h10 - h00) * fx + (h01 - h00) * fz
        } else {
            (h01 + h10 - h11) + (h11 - h01) * fx + (h11 - h10) * fz
        }
    }

    /// Serialises the terrain in its [`version`](Terrain::version).
    ///
    /// Compressed arrays are written raw, so every grid must stay below 1024 bytes (for
    /// example an 8x8 land grid and an 8x8 height grid); larger terrains return
    /// [`Error::Write`]. Objects, map objects and their offset grids are written as given.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        write_terrain(self)
    }
}

struct Parser<'a> {
    r: Reader<'a>,
    version: u32,
    codec: Codec,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let sig = r.array::<4>("signature")?;
        if &sig != SIGNATURE {
            return Err(Error::BadSignature(sig));
        }
        let version = r.u32("version")?;
        if !(MIN_VERSION..=LATEST_VERSION).contains(&version) {
            return Err(Error::UnsupportedVersion(version));
        }
        let codec = if version >= 23 {
            Codec::Lzo
        } else {
            Codec::Lzss
        };
        Ok(Self { r, version, codec })
    }

    fn byte_grid(&mut self, size: GridSize, what: &'static str) -> Result<Grid<u8>> {
        let bytes = self.r.compressed(size.len(), self.codec, what)?;
        Ok(Grid::from_vec(size, bytes.into_owned()).expect("length checked"))
    }

    fn terrain(mut self) -> Result<Terrain> {
        let v = self.version;
        let r = &mut self.r;
        let app_id = if v >= 25 {
            Some(r.u32("app id")?)
        } else {
            None
        };
        let land = GridSize::new(r.u32("land grid")?, r.u32("land grid")?);
        let terrain = GridSize::new(r.u32("terrain grid")?, r.u32("terrain grid")?);
        let land_cell_size = r.f32("cell size")?;
        for (size, what) in [(land, "land grid"), (terrain, "terrain grid")] {
            if size.width == 0
                || size.width != size.height
                || !size.width.is_power_of_two()
                || size.width > 1 << 16
            {
                return Err(Error::Invalid {
                    offset: 8,
                    what,
                    detail: format!(
                        "{}x{} is not a square power of two",
                        size.width, size.height
                    ),
                });
            }
        }

        let geography = quadtree::read::<u16>(r, land)?;
        let geography = Grid::from_vec(
            land,
            geography.into_vec().into_iter().map(Geography).collect(),
        )
        .expect("same size");
        let sound_size = sound_map_size(r, land)?;
        let sound_map = quadtree::read::<u8>(r, sound_size)?;
        let n = r.count(12, "mountain count")?;
        let mountains = (0..n)
            .map(|_| r.vec3("mountain"))
            .collect::<Result<Vec<_>>>()?;
        let material_indices = quadtree::read::<u16>(r, land)?;

        let random = if v < 21 {
            let bytes = self.r.compressed(land.len() * 2, self.codec, "random")?;
            Some(Grid::from_vec(land, le_u16s(&bytes)).expect("length"))
        } else {
            None
        };
        let grass_approx = if v >= 18 {
            Some(self.byte_grid(terrain, "grass approximation")?)
        } else {
            None
        };
        let primary_texture = if v >= 22 {
            Some(self.byte_grid(terrain, "primary texture")?)
        } else {
            None
        };
        let bytes = self
            .r
            .compressed(terrain.len() * 4, self.codec, "heightmap")?;
        let heightmap = Grid::from_vec(terrain, le_f32s(&bytes)).expect("length");
        drop(bytes);

        let r = &mut self.r;
        let n = r.count(2, "material count")?;
        let mut materials = Vec::with_capacity(n);
        for _ in 0..n {
            let path = VfsPath::new(&r.asciiz("material path")?);
            let major = r.u8("material major")? as i8;
            let major = if v < 17 { -1 } else { major };
            materials.push(TerrainMaterial { path, major });
        }
        let n = r.count(1, "model count")?;
        let models = (0..n)
            .map(|_| r.asciiz("model path").map(|s| VfsPath::new(&s)))
            .collect::<Result<Vec<_>>>()?;
        let entities = if v >= 15 {
            let n = r.count(18, "entity count")?;
            (0..n)
                .map(|_| {
                    Ok(StaticEntity {
                        class_name: r.asciiz("entity class")?,
                        shape: r.asciiz("entity shape")?,
                        position: r.vec3("entity position")?,
                        object_id: r.u32("entity id")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };

        let object_offsets = quadtree::read::<u32>(r, land)?;
        let objects_size = r.u32("objects size")? as usize;
        let map_object_offsets = quadtree::read::<u32>(r, land)?;
        let map_size = r.u32("map info size")? as usize;
        let persistent = self.byte_grid(land, "persistent flags")?;
        let subdivision_hints = self.byte_grid(terrain, "subdivision hints")?;
        let r = &mut self.r;
        let max_object_id = r.u32("max object id")?;
        let roads_size = r.u32("road net size")? as usize;

        let roads_start = r.pos();
        let roads = read_roads(r, v, land)?;
        if r.pos() - roads_start != roads_size {
            return Err(Error::Invalid {
                offset: roads_start,
                what: "road net",
                detail: format!(
                    "declared {roads_size} bytes, read {}",
                    r.pos() - roads_start
                ),
            });
        }

        if objects_size % OBJECT_SIZE != 0 {
            return Err(Error::Invalid {
                offset: r.pos(),
                what: "objects",
                detail: format!("size {objects_size} is not a multiple of {OBJECT_SIZE}"),
            });
        }
        let block = r.take(objects_size, "objects")?;
        let objects = block.chunks_exact(OBJECT_SIZE).map(read_object).collect();

        let map_start = r.pos();
        let map_block = r.take(map_size, "map info")?;
        let mut mr = Reader::new(map_block);
        let mut map_objects = Vec::with_capacity(map_size / 16);
        while mr.remaining() > 0 {
            map_objects
                .push(read_map_object(&mut mr, self.codec).map_err(|e| e.shifted(map_start))?);
        }
        if self.r.remaining() != 0 {
            return Err(Error::Invalid {
                offset: self.r.pos(),
                what: "end of file",
                detail: format!("{} trailing bytes", self.r.remaining()),
            });
        }

        Ok(Terrain {
            version: v,
            app_id,
            land_cell_size,
            land_grid: land,
            geography,
            sound_map,
            mountains,
            material_indices,
            random,
            grass_approx,
            primary_texture,
            heightmap,
            materials,
            models,
            entities,
            object_offsets,
            map_object_offsets,
            persistent,
            subdivision_hints,
            max_object_id,
            roads,
            objects,
            map_objects,
        })
    }
}

/// The sound map covers `land * soundMapSizeCoef` cells, where the coefficient comes from the
/// world's config (4 in every shipped world) and is not stored in the file. Picks 4 when the
/// tree fits it, else the smallest power of two that does.
fn sound_map_size(r: &Reader<'_>, land: GridSize) -> Result<GridSize> {
    let depth = quadtree::node_levels(r)?;
    [SOUND_MAP_SIZE_COEF, 1, 2, 8, 16]
        .into_iter()
        .map(|coef| GridSize::new(land.width * coef, land.height * coef))
        .find(|&size| quadtree::levels_for::<u8>(size) >= depth)
        .ok_or_else(|| Error::Invalid {
            offset: r.pos(),
            what: "sound map",
            detail: format!("quad tree of {depth} levels is too deep"),
        })
}

fn le_u16s(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn le_f32s(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn read_object(c: &[u8]) -> ObjectInstance {
    let word = |i: usize| [c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]];
    let mut m = [0.0; 12];
    for (k, v) in m.iter_mut().enumerate() {
        *v = f32::from_le_bytes(word(2 + k));
    }
    ObjectInstance {
        id: u32::from_le_bytes(word(0)),
        model_index: u32::from_le_bytes(word(1)),
        transform: Transform(m),
        shape_param: u32::from_le_bytes(word(14)),
    }
}

fn read_roads(r: &mut Reader<'_>, version: u32, land: GridSize) -> Result<RoadNet> {
    let mut net = RoadNet::default();
    let mut model_ids: HashMap<String, u32> = HashMap::new();
    for x in 0..land.width {
        for z in 0..land.height {
            let n = r.count(6, "road part count")?;
            for _ in 0..n {
                let ends = usize::from(r.u16("road connection count")?);
                let start = net.connections.len() as u32;
                for _ in 0..ends {
                    let position = r.vec3("road connection")?;
                    net.connections.push(RoadConnection { position, kind: 0 });
                }
                if version >= 24 {
                    for end in &mut net.connections[start as usize..] {
                        end.kind = r.u8("road connection type")?;
                    }
                }
                let object_id = r.u32("road object id")?;
                let (model, transform) = if version >= 16 {
                    (
                        r.asciiz("road model")?,
                        Transform(r.f32s::<12>("road transform")?),
                    )
                } else {
                    (String::new(), Transform::from_position(Vec3::ZERO))
                };
                let next = net.models.len() as u32;
                let model_index = *model_ids.entry(model).or_insert_with_key(|m| {
                    net.models.push(m.clone());
                    next
                });
                net.parts.push(RoadPart {
                    cell: (x as u16, z as u16),
                    object_id,
                    model_index,
                    transform,
                    connections: start..net.connections.len() as u32,
                });
            }
        }
    }
    Ok(net)
}

fn write_terrain(t: &Terrain) -> Result<Vec<u8>> {
    let v = t.version;
    if !(MIN_VERSION..=LATEST_VERSION).contains(&v) {
        return Err(Error::UnsupportedVersion(v));
    }
    let land = t.land_grid;
    let terrain = t.heightmap.size();
    let check = |ok: bool, what: &str| {
        if ok {
            Ok(())
        } else {
            Err(Error::Write(format!("{what} does not match its grid size")))
        }
    };
    check(t.geography.size() == land, "geography")?;
    check(t.material_indices.size() == land, "material indices")?;
    check(t.persistent.size() == land, "persistent flags")?;
    check(t.subdivision_hints.size() == terrain, "subdivision hints")?;
    check(
        (v >= 25) == t.app_id.is_some()
            && (v < 21) == t.random.is_some()
            && (v >= 18) == t.grass_approx.is_some()
            && (v >= 22) == t.primary_texture.is_some(),
        "the set of version-dependent arrays",
    )?;

    let mut w = Writer::default();
    w.bytes(SIGNATURE);
    w.u32(v);
    if let Some(app) = t.app_id {
        w.u32(app);
    }
    w.u32(land.width);
    w.u32(land.height);
    w.u32(terrain.width);
    w.u32(terrain.height);
    w.f32(t.land_cell_size);
    let geography = Grid::from_vec(land, t.geography.as_slice().iter().map(|g| g.0).collect())
        .expect("same size");
    quadtree::write(&mut w, &geography);
    quadtree::write(&mut w, &t.sound_map);
    w.u32(t.mountains.len() as u32);
    for m in &t.mountains {
        w.vec3(*m);
    }
    quadtree::write(&mut w, &t.material_indices);
    if let Some(random) = &t.random {
        check(random.size() == land, "random")?;
        let raw: Vec<u8> = random
            .as_slice()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        w.compressed(&raw, "random")?;
    }
    if let Some(grass) = &t.grass_approx {
        check(grass.size() == terrain, "grass approximation")?;
        w.compressed(grass.as_slice(), "grass approximation")?;
    }
    if let Some(prim) = &t.primary_texture {
        check(prim.size() == terrain, "primary texture")?;
        w.compressed(prim.as_slice(), "primary texture")?;
    }
    let raw: Vec<u8> = t
        .heightmap
        .as_slice()
        .iter()
        .flat_map(|h| h.to_le_bytes())
        .collect();
    w.compressed(&raw, "heightmap")?;
    w.u32(t.materials.len() as u32);
    for m in &t.materials {
        w.asciiz(m.path.as_str())?;
        w.u8(m.major as u8);
    }
    w.u32(t.models.len() as u32);
    for m in &t.models {
        w.asciiz(m.as_str())?;
    }
    w.u32(t.entities.len() as u32);
    for e in &t.entities {
        w.asciiz(&e.class_name)?;
        w.asciiz(&e.shape)?;
        w.vec3(e.position);
        w.u32(e.object_id);
    }

    check(t.object_offsets.size() == land, "object offsets")?;
    check(t.map_object_offsets.size() == land, "map object offsets")?;
    let mut object_block = Writer::default();
    for o in &t.objects {
        object_block.u32(o.id);
        object_block.u32(o.model_index);
        for f in o.transform.0 {
            object_block.f32(f);
        }
        object_block.u32(o.shape_param);
    }
    let mut map_block = Writer::default();
    for m in &t.map_objects {
        write_map_object(&mut map_block, m)?;
    }
    quadtree::write(&mut w, &t.object_offsets);
    w.u32(object_block.buf.len() as u32);
    quadtree::write(&mut w, &t.map_object_offsets);
    w.u32(map_block.buf.len() as u32);
    w.compressed(t.persistent.as_slice(), "persistent flags")?;
    w.compressed(t.subdivision_hints.as_slice(), "subdivision hints")?;
    w.u32(t.max_object_id);

    let mut roads = Writer::default();
    let mut by_cell: Vec<Vec<&RoadPart>> = vec![Vec::new(); land.len()];
    for part in &t.roads.parts {
        let (x, z) = (u32::from(part.cell.0), u32::from(part.cell.1));
        if x >= land.width || z >= land.height {
            return Err(Error::Write(format!(
                "road part cell {:?} outside the grid",
                part.cell
            )));
        }
        by_cell[(x * land.height + z) as usize].push(part);
    }
    for cell in &by_cell {
        roads.u32(cell.len() as u32);
        for part in cell {
            let ends = t.roads.connections_of(part);
            roads.u16(ends.len() as u16);
            for e in ends {
                roads.vec3(e.position);
            }
            if v >= 24 {
                for e in ends {
                    roads.u8(e.kind);
                }
            }
            roads.u32(part.object_id);
            if v >= 16 {
                roads.asciiz(t.roads.model_of(part))?;
                for f in part.transform.0 {
                    roads.f32(f);
                }
            }
        }
    }
    w.u32(roads.buf.len() as u32);
    w.bytes(&roads.buf);
    w.bytes(&object_block.buf);
    w.bytes(&map_block.buf);
    Ok(w.buf)
}

/// Builds small terrains, mainly for tests.
#[derive(Debug, Clone)]
pub struct TerrainBuilder {
    terrain: Terrain,
}

impl TerrainBuilder {
    /// A flat, empty version-25 terrain with a `land` x `land` cell grid of `cell_size` metres
    /// and a `heights` x `heights` height grid.
    pub fn new(land: u32, heights: u32, cell_size: f32) -> Self {
        let land_size = GridSize::new(land, land);
        let terrain_size = GridSize::new(heights, heights);
        Self {
            terrain: Terrain {
                version: LATEST_VERSION,
                app_id: Some(0),
                land_cell_size: cell_size,
                land_grid: land_size,
                geography: Grid::new(land_size),
                sound_map: Grid::new(GridSize::new(
                    land * SOUND_MAP_SIZE_COEF,
                    land * SOUND_MAP_SIZE_COEF,
                )),
                mountains: Vec::new(),
                material_indices: Grid::new(land_size),
                random: None,
                grass_approx: Some(Grid::new(terrain_size)),
                primary_texture: Some(Grid::new(terrain_size)),
                heightmap: Grid::new(terrain_size),
                materials: vec![TerrainMaterial {
                    path: VfsPath::root(),
                    major: 0,
                }],
                models: Vec::new(),
                entities: Vec::new(),
                object_offsets: Grid::new(land_size),
                map_object_offsets: Grid::new(land_size),
                persistent: Grid::new(land_size),
                subdivision_hints: Grid::new(terrain_size),
                max_object_id: 0,
                roads: RoadNet::default(),
                objects: Vec::new(),
                map_objects: Vec::new(),
            },
        }
    }

    /// Sets the format version, adding or dropping the version-dependent arrays.
    pub fn version(mut self, version: u32) -> Self {
        let t = &mut self.terrain;
        let (land, heights) = (t.land_grid, t.heightmap.size());
        t.version = version;
        t.app_id = (version >= 25).then_some(t.app_id.unwrap_or(0));
        t.random = (version < 21).then(|| t.random.take().unwrap_or_else(|| Grid::new(land)));
        t.grass_approx =
            (version >= 18).then(|| t.grass_approx.take().unwrap_or_else(|| Grid::new(heights)));
        t.primary_texture = (version >= 22).then(|| {
            t.primary_texture
                .take()
                .unwrap_or_else(|| Grid::new(heights))
        });
        if version < 17 {
            t.materials.iter_mut().for_each(|m| m.major = -1);
        }
        self
    }

    /// Sets every height from `f(i, j)`.
    pub fn heights(mut self, f: impl Fn(u32, u32) -> f32) -> Self {
        let size = self.terrain.heightmap.size();
        for j in 0..size.height {
            for i in 0..size.width {
                *self.terrain.heightmap.get_mut(i, j).expect("in grid") = f(i, j);
            }
        }
        self
    }

    /// Adds a material and returns the builder; its index is the number of materials before.
    pub fn material(mut self, path: &str) -> Self {
        self.terrain.materials.push(TerrainMaterial {
            path: VfsPath::new(path),
            major: if self.terrain.version < 17 { -1 } else { 0 },
        });
        self
    }

    /// Adds an object of `model` (added to the model list if new) at `transform`.
    pub fn object(mut self, model: &str, transform: Transform) -> Self {
        let t = &mut self.terrain;
        let path = VfsPath::new(model);
        let model_index = match t.models.iter().position(|m| *m == path) {
            Some(i) => i,
            None => {
                t.models.push(path);
                t.models.len() - 1
            }
        } as u32;
        let id = t.objects.len() as u32;
        t.objects.push(ObjectInstance {
            id,
            model_index,
            transform,
            shape_param: 2,
        });
        t.max_object_id = id;
        self
    }

    /// Gives mutable access to the terrain being built.
    pub fn edit(mut self, f: impl FnOnce(&mut Terrain)) -> Self {
        f(&mut self.terrain);
        self
    }

    /// The terrain.
    pub fn build(self) -> Terrain {
        self.terrain
    }
}
