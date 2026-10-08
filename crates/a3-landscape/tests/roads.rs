//! Road networks from synthetic shapefiles, dBase tables and RoadsLib.cfg text.

use a3_landscape::shapefile::{Dbf, Shape, Shapefile};
use a3_landscape::{RoadNetwork, RoadsError, RoadsLib};
use a3_wrp::MapType;
use glam::{DVec2, Vec2};

/// Writes a polyline shapefile: one record per entry, each a list of parts.
fn shp(records: &[Vec<Vec<(f64, f64)>>]) -> Vec<u8> {
    let mut body = Vec::new();
    for (i, parts) in records.iter().enumerate() {
        let mut content = Vec::new();
        content.extend(3u32.to_le_bytes());
        content.extend([0u8; 32]); // bbox (unused by the reader)
        content.extend((parts.len() as u32).to_le_bytes());
        let points: usize = parts.iter().map(Vec::len).sum();
        content.extend((points as u32).to_le_bytes());
        let mut start = 0u32;
        for p in parts {
            content.extend(start.to_le_bytes());
            start += p.len() as u32;
        }
        for &(x, y) in parts.iter().flatten() {
            content.extend(x.to_le_bytes());
            content.extend(y.to_le_bytes());
        }
        body.extend((i as u32 + 1).to_be_bytes());
        body.extend(((content.len() / 2) as u32).to_be_bytes());
        body.extend(content);
    }
    let mut out = Vec::new();
    out.extend(9994u32.to_be_bytes());
    out.extend([0u8; 20]);
    out.extend((((100 + body.len()) / 2) as u32).to_be_bytes());
    out.extend(1000u32.to_le_bytes());
    out.extend(3u32.to_le_bytes());
    out.extend([0u8; 64]);
    out.extend(body);
    out
}

/// Writes a dBase III table of numeric/text columns.
fn dbf(fields: &[(&str, u8, u8)], rows: &[Vec<&str>]) -> Vec<u8> {
    let header_len = 32 + 32 * fields.len() + 1;
    let record_len = 1 + fields.iter().map(|f| usize::from(f.2)).sum::<usize>();
    let mut out = vec![3u8, 126, 1, 1];
    out.extend((rows.len() as u32).to_le_bytes());
    out.extend((header_len as u16).to_le_bytes());
    out.extend((record_len as u16).to_le_bytes());
    out.extend([0u8; 20]);
    for &(name, kind, len) in fields {
        let mut d = [0u8; 32];
        d[..name.len()].copy_from_slice(name.as_bytes());
        d[11] = kind;
        d[16] = len;
        out.extend(d);
    }
    out.push(0x0d);
    for row in rows {
        out.push(b' ');
        for (value, &(_, _, len)) in row.iter().zip(fields) {
            out.extend(format!("{value:>width$}", width = usize::from(len)).bytes());
        }
    }
    out.push(0x1a);
    out
}

const ROADS_LIB: &str = r#"
class RoadTypesLibrary
{
    class Road0001
    {
        width = 14;
        mainStrTex = "a3\roads_f\roads_ae\data\surf_roadtarmac_highway_ca.paa"; // lowercase!
        mainTerTex = "a3\roads_f\roads_ae\data\surf_roadtarmac_highway_end_ca.paa";
        mainMat = "a3\roads_f\roads_ae\data\surf_roadtarmac_highway.rvmat";
        map = "main road";
        AIpathOffset = 1;
        color[] = {0.0, 1.0, 1.0, 1.0};
    };
    /* class Road0009 { width = 99; }; */
    class Road0005
    {
        width = 7;
        map = "track";
        pedestriansOnly = 1;
    };
};
"#;

#[test]
fn shapefile_reads_polyline_parts() {
    let bytes = shp(&[vec![
        vec![(0.0, 1.0), (2.0, 3.0)],
        vec![(4.0, 5.0), (6.0, 7.0), (8.0, 9.0)],
    ]]);
    let file = Shapefile::parse(&bytes).unwrap();
    assert_eq!(file.shape_type, 3);
    assert_eq!(
        file.shapes,
        [Shape::PolyLine(vec![
            vec![DVec2::new(0.0, 1.0), DVec2::new(2.0, 3.0)],
            vec![
                DVec2::new(4.0, 5.0),
                DVec2::new(6.0, 7.0),
                DVec2::new(8.0, 9.0)
            ],
        ])]
    );
    assert!(Shapefile::parse(&bytes[..bytes.len() - 3]).is_err());
    assert!(Shapefile::parse(b"not a shapefile at all, clearly").is_err());
}

#[test]
fn dbf_reads_trimmed_columns() {
    let bytes = dbf(
        &[("__LAYER", b'C', 30), ("ID", b'N', 30)],
        &[vec!["Emptiness", "5"], vec!["MainIsland", "3"]],
    );
    let table = Dbf::parse(&bytes).unwrap();
    assert_eq!(table.fields[1].name, "ID");
    assert_eq!(table.fields[1].kind, 'N');
    assert_eq!(table.get(1, "__layer"), Some("MainIsland"));
    assert_eq!(table.get(0, "ID"), Some("5"));
    assert_eq!(table.get(0, "WIDTH"), None);
}

#[test]
fn roads_lib_reads_road_classes_and_skips_comments() {
    let lib = RoadsLib::parse(ROADS_LIB).unwrap();
    assert_eq!(lib.types.len(), 2);
    let highway = lib.get(1).unwrap();
    assert_eq!(highway.width, 14.0);
    assert_eq!(highway.map_type, Some(MapType::MainRoad));
    assert_eq!(highway.color, Some([0.0, 1.0, 1.0, 1.0]));
    assert_eq!(
        highway.material,
        "a3\\roads_f\\roads_ae\\data\\surf_roadtarmac_highway.rvmat"
    );
    let path = lib.get(5).unwrap();
    assert!(path.pedestrians_only);
    assert_eq!(path.map_type, Some(MapType::Track));
    assert!(lib.get(9).is_none());
}

#[test]
fn roads_are_in_world_coordinates_with_their_type() {
    let shp_bytes = shp(&[
        vec![vec![
            (200_100.0, 50.0),
            (200_100.0, 150.0),
            (200_200.0, 150.0),
        ]],
        vec![vec![(200_010.0, 10.0), (200_020.0, 10.0)]],
        vec![vec![(200_000.0, 0.0)]],
    ]);
    let dbf_bytes = dbf(
        &[
            ("ID", b'N', 30),
            ("ORDER", b'N', 30),
            ("ROADMASK", b'N', 30),
        ],
        &[
            vec!["1", "5", "101"],
            vec!["7", "0", "0"],
            vec!["1", "0", "0"],
        ],
    );
    let net = RoadNetwork::from_files(&shp_bytes, &dbf_bytes, Some(ROADS_LIB)).unwrap();
    // The single-point polyline is dropped.
    assert_eq!(net.roads.len(), 2);
    let main = &net.roads[0];
    assert_eq!(main.points[0], Vec2::new(100.0, 50.0));
    assert_eq!(main.length(), 200.0);
    assert_eq!((main.id, main.order, main.mask), (1, 5, 0b101));
    assert_eq!(net.road_type(main).width, 14.0);
    // ID 7 has no class: the engine's fallback type.
    let unknown = net.road_type(&net.roads[1]);
    assert_eq!(unknown.class, "Road0007");
    assert_eq!(unknown.material, "a3\\roads_f\\roads\\data\\road.rvmat");
}

#[test]
fn shape_and_table_must_agree() {
    let shp_bytes = shp(&[vec![vec![(0.0, 0.0), (1.0, 1.0)]]]);
    let dbf_bytes = dbf(&[("ID", b'N', 4)], &[vec!["1"], vec!["2"]]);
    assert!(matches!(
        RoadNetwork::from_files(&shp_bytes, &dbf_bytes, None),
        Err(RoadsError::RecordCount { shapes: 1, rows: 2 })
    ));
}
