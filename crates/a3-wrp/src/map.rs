//! Map info: the 2D map symbols (trees, houses, roads, fences, ...) the in-game map draws.

use glam::Vec2;

use crate::cursor::{Codec, Reader, Writer};
use crate::{Error, Result};

macro_rules! map_types {
    ($($(#[$doc:meta])* $name:ident = $value:literal, $label:literal;)*) => {
        /// The kind of a [`MapObject`], numbered as in the file. Each kind has one record
        /// layout ([`MapShape`]).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u32)]
        pub enum MapType {
            $($(#[$doc])* $name = $value,)*
        }

        impl MapType {
            /// Every map type, in numeric order.
            pub const ALL: &'static [MapType] = &[$(MapType::$name,)*];

            /// The type numbered `value`, if it exists.
            pub fn from_u32(value: u32) -> Option<Self> {
                match value {
                    $($value => Some(MapType::$name),)*
                    _ => None,
                }
            }

            /// The engine's display name (as in its debug tables, e.g. `"VIEW-TOWER"`).
            pub fn name(self) -> &'static str {
                match self {
                    $(MapType::$name => $label,)*
                }
            }
        }
    };
}

map_types! {
    Tree = 0, "TREE";
    SmallTree = 1, "SMALL TREE";
    Bush = 2, "BUSH";
    Building = 3, "BUILDING";
    House = 4, "HOUSE";
    ForestBorder = 5, "FOREST BORDER";
    ForestTriangle = 6, "FOREST TRIANGLE";
    ForestSquare = 7, "FOREST SQUARE";
    Church = 8, "CHURCH";
    Chapel = 9, "CHAPEL";
    Cross = 10, "CROSS";
    Rock = 11, "ROCK";
    Bunker = 12, "BUNKER";
    Fortress = 13, "FORTRESS";
    Fountain = 14, "FOUNTAIN";
    ViewTower = 15, "VIEW-TOWER";
    Lighthouse = 16, "LIGHTHOUSE";
    Quay = 17, "QUAY";
    FuelStation = 18, "FUELSTATION";
    Hospital = 19, "HOSPITAL";
    Fence = 20, "FENCE";
    Wall = 21, "WALL";
    Hide = 22, "HIDE";
    BusStop = 23, "BUSSTOP";
    Road = 24, "ROAD";
    Forest = 25, "FOREST";
    Transmitter = 26, "TRANSMITTER";
    Stack = 27, "STACK";
    Ruin = 28, "RUIN";
    Tourism = 29, "TOURISM";
    WaterTower = 30, "WATERTOWER";
    Track = 31, "TRACK";
    MainRoad = 32, "MAIN ROAD";
    Rocks = 33, "ROCKS";
    PowerLines = 34, "POWER LINES";
    RailWay = 35, "RAILWAY";
    PowerSolar = 36, "POWERSOLAR";
    PowerWave = 37, "POWERWAVE";
    PowerWind = 38, "POWERWIND";
    Shipwreck = 39, "SHIPWRECK";
    Trail = 40, "TRAIL";
    ForestLod1 = 41, "FOREST_LOD1";
    ForestLod2 = 42, "FOREST_LOD2";
    TownLod1 = 43, "TOWN_LOD1";
    River = 44, "RIVER";
}

/// The record layout of a map type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapShapeKind {
    /// Object id only.
    Plain,
    /// Object id and a position.
    Point,
    /// Object id and an angle.
    Angle,
    /// Object id and four corners.
    Rect,
    /// Object id, four corners and a colour.
    RectColored,
    /// Object id, four flag bytes and four values.
    Forest,
    /// Object id and a line segment.
    Line,
    /// Object id, six values and a flag byte.
    RailWay,
    /// Object id and a polyline.
    River,
}

impl MapType {
    /// The record layout used for this type (from the engine's map object factory).
    pub fn shape_kind(self) -> MapShapeKind {
        use MapType::*;
        match self {
            Tree | SmallTree | Bush | Cross | Rock | Bunker | Fortress | Fountain | ViewTower
            | Lighthouse | Quay | Hide | BusStop | Transmitter | Stack | WaterTower => {
                MapShapeKind::Point
            }
            Building | House | Church | Chapel | FuelStation | Hospital | Fence | Wall | Ruin
            | Tourism | PowerSolar | PowerWave | PowerWind | Shipwreck => MapShapeKind::RectColored,
            ForestBorder | ForestSquare => MapShapeKind::Plain,
            ForestTriangle => MapShapeKind::Angle,
            Road | Track | MainRoad | Trail => MapShapeKind::Rect,
            Forest | Rocks | ForestLod1 | ForestLod2 | TownLod1 => MapShapeKind::Forest,
            PowerLines => MapShapeKind::Line,
            RailWay => MapShapeKind::RailWay,
            River => MapShapeKind::River,
        }
    }
}

/// The geometry of a map object. Positions are world `(x, z)` in metres.
#[derive(Debug, Clone, PartialEq)]
pub enum MapShape {
    /// No geometry.
    Plain,
    /// A point symbol.
    Point(Vec2),
    /// An angle in radians _(uncertain)_.
    Angle(f32),
    /// An oriented rectangle, as four corners.
    Rect([Vec2; 4]),
    /// An oriented rectangle with an ARGB colour (`0xAARRGGBB` read little-endian).
    RectColored {
        /// The four corners.
        corners: [Vec2; 4],
        /// The fill colour.
        color: u32,
    },
    /// A forest-like area cell: four flag bytes and four values _(meaning uncertain)_.
    Forest {
        /// Four flag bytes.
        flags: [u8; 4],
        /// Four values.
        values: [f32; 4],
    },
    /// A line segment (power lines).
    Line(Vec2, Vec2),
    /// A railway segment: six values and a flag byte _(meaning uncertain)_.
    RailWay {
        /// Six values.
        values: [f32; 6],
        /// A flag byte.
        flag: u8,
    },
    /// A polyline (rivers).
    River(Vec<Vec2>),
}

/// One symbol of the 2D map.
#[derive(Debug, Clone, PartialEq)]
pub struct MapObject {
    /// What the symbol shows.
    pub kind: MapType,
    /// The id of the placed object it stands for (`0xFFFFFFFF` for none).
    pub object_id: u32,
    /// Its geometry; the variant matches [`MapType::shape_kind`].
    pub shape: MapShape,
}

fn vec2(r: &mut Reader<'_>) -> Result<Vec2> {
    let [x, z] = r.f32s::<2>("map object")?;
    Ok(Vec2::new(x, z))
}

fn corners(r: &mut Reader<'_>) -> Result<[Vec2; 4]> {
    Ok([vec2(r)?, vec2(r)?, vec2(r)?, vec2(r)?])
}

pub(crate) fn read_map_object(r: &mut Reader<'_>, codec: Codec) -> Result<MapObject> {
    let offset = r.pos();
    let raw = r.u32("map object type")?;
    let kind = MapType::from_u32(raw).ok_or(Error::UnknownMapType { kind: raw, offset })?;
    let object_id = r.u32("map object id")?;
    let shape = match kind.shape_kind() {
        MapShapeKind::Plain => MapShape::Plain,
        MapShapeKind::Point => MapShape::Point(vec2(r)?),
        MapShapeKind::Angle => MapShape::Angle(r.f32("map object")?),
        MapShapeKind::Rect => MapShape::Rect(corners(r)?),
        MapShapeKind::RectColored => MapShape::RectColored {
            corners: corners(r)?,
            color: r.u32("map object")?,
        },
        MapShapeKind::Forest => MapShape::Forest {
            flags: r.array::<4>("map object")?,
            values: r.f32s::<4>("map object")?,
        },
        MapShapeKind::Line => MapShape::Line(vec2(r)?, vec2(r)?),
        MapShapeKind::RailWay => MapShape::RailWay {
            values: r.f32s::<6>("map object")?,
            flag: r.u8("map object")?,
        },
        MapShapeKind::River => {
            let n = r.count(0, "river point count")?;
            let bytes = r.compressed(n * 8, codec, "river points")?;
            MapShape::River(
                bytes
                    .chunks_exact(8)
                    .map(|c| {
                        Vec2::new(
                            f32::from_le_bytes([c[0], c[1], c[2], c[3]]),
                            f32::from_le_bytes([c[4], c[5], c[6], c[7]]),
                        )
                    })
                    .collect(),
            )
        }
    };
    Ok(MapObject {
        kind,
        object_id,
        shape,
    })
}

pub(crate) fn write_map_object(w: &mut Writer, o: &MapObject) -> Result<()> {
    if !shape_matches(o.kind.shape_kind(), &o.shape) {
        return Err(Error::Write(format!(
            "map object {:?} needs a {:?} shape",
            o.kind,
            o.kind.shape_kind()
        )));
    }
    w.u32(o.kind as u32);
    w.u32(o.object_id);
    let v2 = |w: &mut Writer, v: Vec2| {
        w.f32(v.x);
        w.f32(v.y);
    };
    match &o.shape {
        MapShape::Plain => {}
        MapShape::Point(p) => v2(w, *p),
        MapShape::Angle(a) => w.f32(*a),
        MapShape::Rect(c) => c.iter().for_each(|p| v2(w, *p)),
        MapShape::RectColored { corners, color } => {
            corners.iter().for_each(|p| v2(w, *p));
            w.u32(*color);
        }
        MapShape::Forest { flags, values } => {
            w.bytes(flags);
            values.iter().for_each(|v| w.f32(*v));
        }
        MapShape::Line(a, b) => {
            v2(w, *a);
            v2(w, *b);
        }
        MapShape::RailWay { values, flag } => {
            values.iter().for_each(|v| w.f32(*v));
            w.u8(*flag);
        }
        MapShape::River(points) => {
            w.u32(points.len() as u32);
            let mut raw = Vec::with_capacity(points.len() * 8);
            for p in points {
                raw.extend_from_slice(&p.x.to_le_bytes());
                raw.extend_from_slice(&p.y.to_le_bytes());
            }
            w.compressed(&raw, "river points")?;
        }
    }
    Ok(())
}

fn shape_matches(kind: MapShapeKind, shape: &MapShape) -> bool {
    matches!(
        (kind, shape),
        (MapShapeKind::Plain, MapShape::Plain)
            | (MapShapeKind::Point, MapShape::Point(_))
            | (MapShapeKind::Angle, MapShape::Angle(_))
            | (MapShapeKind::Rect, MapShape::Rect(_))
            | (MapShapeKind::RectColored, MapShape::RectColored { .. })
            | (MapShapeKind::Forest, MapShape::Forest { .. })
            | (MapShapeKind::Line, MapShape::Line(..))
            | (MapShapeKind::RailWay, MapShape::RailWay { .. })
            | (MapShapeKind::River, MapShape::River(_))
    )
}
