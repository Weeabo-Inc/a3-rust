//! A terrain's road network: `roads.shp` polylines typed by `RoadsLib.cfg`.

use a3_config::{ConfigTree, parse_text};
use a3_core::VfsPath;
use a3_vfs::Vfs;
use a3_wrp::MapType;
use glam::{DVec2, Vec2};

use crate::cfg::{array_n, classes, number, text};
use crate::shapefile::{Dbf, Shape, Shapefile, ShapefileError};

/// Shapefile x (easting) of world x = 0. World `x = X - EASTING_OFFSET`, `z = Y`.
pub const EASTING_OFFSET: f64 = 200_000.0;

/// Errors from loading a road network.
#[derive(Debug, thiserror::Error)]
pub enum RoadsError {
    /// A file is missing from the VFS.
    #[error("cannot read {path}: {source}")]
    Vfs {
        /// The file.
        path: VfsPath,
        /// The VFS error.
        source: a3_vfs::Error,
    },
    /// The shapefile or table is malformed.
    #[error(transparent)]
    Shapefile(#[from] ShapefileError),
    /// `RoadsLib.cfg` does not parse.
    #[error("cannot parse RoadsLib.cfg: {0}")]
    RoadsLib(String),
    /// The shapefile and the table disagree.
    #[error("roads.shp has {shapes} records but roads.dbf has {rows}")]
    RecordCount {
        /// Shapes.
        shapes: usize,
        /// Table rows.
        rows: usize,
    },
}

/// One road type (`RoadTypesLibrary >> RoadNNNN` of `RoadsLib.cfg`).
#[derive(Debug, Clone, PartialEq)]
pub struct RoadType {
    /// The type number: `ID` in the table, `NNNN` in the class name.
    pub id: u32,
    /// The class name, e.g. `Road0001`.
    pub class: String,
    /// Width in metres.
    pub width: f32,
    /// `mainStrTex`: the texture of straight segments.
    pub straight_texture: String,
    /// `mainTerTex`: the texture of road ends.
    pub end_texture: String,
    /// `mainMat`: the rvmat.
    pub material: String,
    /// `map`: how the 2D map draws it, e.g. `"main road"`, `"road"`, `"track"`.
    pub map: String,
    /// [`RoadType::map`] as a map type, when it names one.
    pub map_type: Option<MapType>,
    /// `AIpathOffset`: how far from the centre line AI drives.
    pub ai_path_offset: f32,
    /// `pedestriansOnly`.
    pub pedestrians_only: bool,
    /// `color[]`: the road's colour in the AI cost map, if given.
    pub color: Option<[f32; 4]>,
}

impl RoadType {
    /// The engine's fallback for an `ID` without a class.
    pub fn fallback(id: u32) -> Self {
        Self {
            id,
            class: format!("Road{id:04}"),
            width: 0.0,
            straight_texture: "a3\\roads_f\\roads\\data\\road_ca.paa".into(),
            end_texture: "a3\\roads_f\\roads\\data\\road_end_ca.paa".into(),
            material: "a3\\roads_f\\roads\\data\\road.rvmat".into(),
            map: "main road".into(),
            map_type: Some(MapType::MainRoad),
            ai_path_offset: 0.0,
            pedestrians_only: false,
            color: None,
        }
    }
}

/// The road types of a terrain (`RoadsLib.cfg`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadsLib {
    /// Types in file order.
    pub types: Vec<RoadType>,
}

impl RoadsLib {
    /// Parses `RoadsLib.cfg` text.
    pub fn parse(source: &str) -> Result<Self, RoadsError> {
        let config = parse_text(source).map_err(|e| RoadsError::RoadsLib(e.to_string()))?;
        let tree = ConfigTree::from_config(&config);
        let lib = tree.root().get("RoadTypesLibrary");
        let types = classes(&lib)
            .iter()
            .filter_map(|c| {
                let name = c.name();
                let id = name.get(4..)?.parse().ok()?;
                name.get(..4)?.eq_ignore_ascii_case("road").then_some(())?;
                let fallback = RoadType::fallback(id);
                let map = text(c, "map").unwrap_or(fallback.map);
                Some(RoadType {
                    id,
                    class: name.to_owned(),
                    width: number(c, "width").unwrap_or(fallback.width),
                    straight_texture: text(c, "mainStrTex").unwrap_or(fallback.straight_texture),
                    end_texture: text(c, "mainTerTex").unwrap_or(fallback.end_texture),
                    material: text(c, "mainMat").unwrap_or(fallback.material),
                    map_type: map_type_named(&map),
                    map,
                    ai_path_offset: number(c, "AIpathOffset").unwrap_or(0.0),
                    pedestrians_only: number(c, "pedestriansOnly").unwrap_or(0.0) != 0.0,
                    color: c
                        .get("color")
                        .is_array()
                        .then(|| array_n::<4>(c, "color", 1.0)),
                })
            })
            .collect();
        Ok(Self { types })
    }

    /// The type numbered `id`.
    pub fn get(&self, id: u32) -> Option<&RoadType> {
        self.types.iter().find(|t| t.id == id)
    }
}

fn map_type_named(name: &str) -> Option<MapType> {
    MapType::ALL
        .iter()
        .copied()
        .find(|t| t.name().eq_ignore_ascii_case(name.trim()))
}

/// One road: a polyline with its table attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Road {
    /// The shapefile record (and table row) it came from.
    pub record: usize,
    /// The road type number (`ID`, taken modulo 256 as the engine does).
    pub id: u32,
    /// `ORDER` (draw / priority order), 0 when absent.
    pub order: u8,
    /// `ROADMASK` decoded as the engine does: bit 0 = units digit set, bit 1 = tens digit set,
    /// bit 2 = hundreds digit set (values of 112 and more give 0) _(meaning uncertain)_.
    pub mask: u8,
    /// The centre line in world `(x, z)` metres.
    pub points: Vec<Vec2>,
}

impl Road {
    /// Length of the centre line in metres.
    pub fn length(&self) -> f32 {
        self.points.windows(2).map(|w| w[0].distance(w[1])).sum()
    }
}

/// A terrain's roads and their types.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoadNetwork {
    /// One road per polyline part with at least two points, in file order.
    pub roads: Vec<Road>,
    /// The road types.
    pub library: RoadsLib,
}

impl RoadNetwork {
    /// Builds the network from the contents of `roads.shp`, `roads.dbf` and (optionally)
    /// `RoadsLib.cfg`.
    pub fn from_files(shp: &[u8], dbf: &[u8], roads_lib: Option<&str>) -> Result<Self, RoadsError> {
        let shapes = Shapefile::parse(shp)?;
        let table = Dbf::parse(dbf)?;
        if shapes.shapes.len() != table.records.len() {
            return Err(RoadsError::RecordCount {
                shapes: shapes.shapes.len(),
                rows: table.records.len(),
            });
        }
        let library = match roads_lib {
            Some(src) => RoadsLib::parse(src)?,
            None => RoadsLib::default(),
        };
        let int = |row: usize, name: &str| {
            table
                .get(row, name)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|v| v as i64)
        };
        let mut roads = Vec::new();
        for (record, shape) in shapes.shapes.iter().enumerate() {
            if table.deleted[record] {
                continue;
            }
            let Shape::PolyLine(parts) = shape else {
                continue;
            };
            let id = (int(record, "ID").unwrap_or(0) & 0xff) as u32;
            let order = (int(record, "ORDER").unwrap_or(0) & 0xff) as u8;
            let mask = int(record, "ROADMASK").map_or(0, decode_mask);
            for part in parts.iter().filter(|p| p.len() >= 2) {
                roads.push(Road {
                    record,
                    id,
                    order,
                    mask,
                    points: part.iter().map(|&p| to_world(p)).collect(),
                });
            }
        }
        Ok(Self { roads, library })
    }

    /// Loads `shp` (a VFS path such as `newRoadsShape`), the `.dbf` next to it and the
    /// `RoadsLib.cfg` in the same folder (when present).
    pub fn load(vfs: &Vfs, shp: &VfsPath) -> Result<Self, RoadsError> {
        let read = |path: VfsPath| {
            vfs.open(path.as_str())
                .map_err(|source| RoadsError::Vfs { path, source })
        };
        let dbf_path = VfsPath::new(&format!(
            "{}.dbf",
            shp.as_str().strip_suffix(".shp").unwrap_or(shp.as_str())
        ));
        let lib_path = shp
            .parent()
            .map_or_else(|| VfsPath::new("roadslib.cfg"), |p| p.join("roadslib.cfg"));
        let shp_bytes = read(shp.clone())?;
        let dbf_bytes = read(dbf_path)?;
        let lib = vfs
            .open(lib_path.as_str())
            .ok()
            .map(|b| a3_gamedata::decode_text(&b));
        Self::from_files(&shp_bytes, &dbf_bytes, lib.as_deref())
    }

    /// The type of `road`: its RoadsLib class, or the engine's fallback.
    pub fn road_type(&self, road: &Road) -> RoadType {
        self.library
            .get(road.id)
            .cloned()
            .unwrap_or_else(|| RoadType::fallback(road.id))
    }
}

/// Shapefile coordinates to world `(x, z)`.
pub fn to_world(p: DVec2) -> Vec2 {
    Vec2::new((p.x - EASTING_OFFSET) as f32, p.y as f32)
}

fn decode_mask(v: i64) -> u8 {
    if !(0..112).contains(&v) {
        return 0;
    }
    let digit = |d: i64| u8::from((v / d) % 10 != 0);
    digit(1) | digit(10) << 1 | digit(100) << 2
}

#[cfg(test)]
mod tests {
    use super::decode_mask;

    #[test]
    fn road_mask_digits_become_bits() {
        assert_eq!(decode_mask(0), 0);
        assert_eq!(decode_mask(1), 1);
        assert_eq!(decode_mask(10), 2);
        assert_eq!(decode_mask(101), 5);
        assert_eq!(decode_mask(111), 7);
        assert_eq!(decode_mask(112), 0);
        assert_eq!(decode_mask(-1), 0);
    }
}
