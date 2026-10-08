//! Behaviour of the WRP reader on synthetic terrains built by the test writer.

use a3_wrp::{
    Error, Geography, MapObject, MapShape, MapShapeKind, MapType, RoadConnection, RoadPart,
    StaticEntity, Terrain, TerrainBuilder, Transform,
};
use glam::{Vec2, Vec3};
use proptest::prelude::*;

#[test]
fn flat_terrain_round_trips_its_header() {
    let terrain = TerrainBuilder::new(4, 8, 50.0).build();
    let bytes = terrain.to_bytes().unwrap();

    assert_eq!(&bytes[..4], b"OPRW");
    let parsed = Terrain::parse(&bytes).unwrap();
    assert_eq!(parsed.version, 25);
    assert_eq!(parsed.land_grid.width, 4);
    assert_eq!(parsed.land_grid.height, 4);
    assert_eq!(parsed.heightmap.width(), 8);
    assert_eq!(parsed.land_cell_size, 50.0);
    assert_eq!(parsed.terrain_cell_size(), 25.0);
    assert_eq!(parsed.world_size(), 200.0);
}

fn shape_for(kind: MapType, seed: f32) -> MapShape {
    let corners = [
        Vec2::new(seed, 1.0),
        Vec2::new(2.0, seed),
        Vec2::new(3.0, 4.0),
        Vec2::new(5.0, 6.0),
    ];
    match kind.shape_kind() {
        MapShapeKind::Plain => MapShape::Plain,
        MapShapeKind::Point => MapShape::Point(Vec2::new(seed, -seed)),
        MapShapeKind::Angle => MapShape::Angle(seed),
        MapShapeKind::Rect => MapShape::Rect(corners),
        MapShapeKind::RectColored => MapShape::RectColored {
            corners,
            color: 0xff10_2030,
        },
        MapShapeKind::Forest => MapShape::Forest {
            flags: [0, 0, 1, 1],
            values: [0.0, seed, 0.5, 1.25],
        },
        MapShapeKind::Line => MapShape::Line(Vec2::ZERO, Vec2::new(seed, 7.0)),
        MapShapeKind::RailWay => MapShape::RailWay {
            values: [1.0, 2.0, 3.0, 4.0, 5.0, seed],
            flag: 1,
        },
        MapShapeKind::River => MapShape::River(vec![Vec2::ZERO, Vec2::new(seed, 9.0)]),
    }
}

/// A terrain with every section populated.
fn full_terrain() -> Terrain {
    let rot = Transform([
        0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 120.0, 3.5, 60.0,
    ]);
    TerrainBuilder::new(8, 8, 25.0)
        .heights(|i, j| i as f32 * 2.0 - j as f32)
        .material("a3\\map_test\\data\\layers\\p_000-000_l00_l01.rvmat")
        .material("a3\\map_test\\data\\layers\\p_000-001_l00.rvmat")
        .object("a3\\structures_f\\house.p3d", rot)
        .object(
            "a3\\plants_f\\tree.p3d",
            Transform::from_position(Vec3::new(10.0, 2.0, 190.0)),
        )
        .object("a3\\structures_f\\house.p3d", Transform::from_position(Vec3::ONE))
        .edit(|t| {
            t.app_id = Some(107_410);
            for (k, g) in t.geography.as_mut_slice().iter_mut().enumerate() {
                *g = Geography((k as u16).wrapping_mul(0x9e37));
            }
            for (k, s) in t.sound_map.as_mut_slice().iter_mut().enumerate() {
                *s = (k % 3) as u8;
            }
            for (k, m) in t.material_indices.as_mut_slice().iter_mut().enumerate() {
                *m = (k % 3) as u16;
            }
            t.mountains = vec![Vec3::new(100.0, 55.5, 25.0), Vec3::new(1.0, 2.0, 3.0)];
            t.grass_approx.as_mut().unwrap().as_mut_slice()[5] = 7;
            t.primary_texture.as_mut().unwrap().as_mut_slice()[9] = 2;
            t.persistent.as_mut_slice()[3] = 1;
            t.subdivision_hints.as_mut_slice()[1] = 4;
            t.object_offsets.as_mut_slice()[63] = 180;
            t.map_object_offsets.as_mut_slice()[10] = 16;
            t.entities.push(StaticEntity {
                class_name: "Land_NavigLight".into(),
                shape: "a3\\roads_f\\runway\\naviglight.p3d".into(),
                position: Vec3::new(1.5, 2.5, 3.5),
                object_id: 0x9301_0016,
            });
            t.roads.models.push("a3\\roads_f\\road.p3d".into());
            t.roads.connections.extend([
                RoadConnection {
                    position: Vec3::new(1.0, 0.0, 2.0),
                    kind: 1,
                },
                RoadConnection {
                    position: Vec3::new(3.0, 0.0, 4.0),
                    kind: 0,
                },
            ]);
            t.roads.parts.push(RoadPart {
                cell: (2, 5),
                object_id: 0x9341_707a,
                model_index: 0,
                transform: rot,
                connections: 0..2,
            });
            t.roads.parts.push(RoadPart {
                cell: (7, 0),
                object_id: 7,
                model_index: 0,
                transform: rot,
                connections: 2..2,
            });
            t.map_objects = MapType::ALL
                .iter()
                .enumerate()
                .map(|(i, &kind)| MapObject {
                    kind,
                    object_id: i as u32,
                    shape: shape_for(kind, i as f32),
                })
                .collect();
        })
        .build()
}

#[test]
fn every_section_round_trips() {
    let terrain = full_terrain();
    let parsed = Terrain::parse(&terrain.to_bytes().unwrap()).unwrap();
    assert_eq!(parsed, terrain);
}

#[test]
fn objects_resolve_their_model_and_transform() {
    let parsed = Terrain::parse(&full_terrain().to_bytes().unwrap()).unwrap();
    assert_eq!(parsed.objects.len(), 3);
    let house = &parsed.objects[0];
    assert_eq!(
        parsed.model_of(house).unwrap().as_str(),
        "a3\\structures_f\\house.p3d"
    );
    assert_eq!(house.transform.position(), Vec3::new(120.0, 3.5, 60.0));
    // Its z axis points along world +x: east, 90 degrees.
    assert!((house.transform.heading_degrees() - 90.0).abs() < 1e-4);
    assert_eq!(parsed.objects[2].model_index, 0);
    assert_eq!(parsed.max_object_id, 2);
}

#[test]
fn materials_are_looked_up_per_land_cell() {
    let parsed = Terrain::parse(&full_terrain().to_bytes().unwrap()).unwrap();
    // Cell (1, 0) is index 1 in row-major order, so its material index is 1.
    assert_eq!(
        parsed.material_at(1, 0).unwrap().path.as_str(),
        "a3\\map_test\\data\\layers\\p_000-000_l00_l01.rvmat"
    );
    assert!(parsed.material_at(8, 0).is_none());
}

#[test]
fn road_parts_keep_their_cell_and_ends() {
    let parsed = Terrain::parse(&full_terrain().to_bytes().unwrap()).unwrap();
    let roads = &parsed.roads;
    assert_eq!(roads.parts.len(), 2);
    // Cells are listed with x outer, z inner: (2, 5) comes before (7, 0).
    assert_eq!(roads.parts[0].cell, (2, 5));
    let ends = roads.connections_of(&roads.parts[0]);
    assert_eq!(ends.len(), 2);
    assert_eq!(ends[0].kind, 1);
    assert_eq!(roads.model_of(&roads.parts[1]), "a3\\roads_f\\road.p3d");
}

#[test]
fn version_24_has_no_app_id_and_version_23_no_connection_types() {
    let v25_len = full_terrain().to_bytes().unwrap().len();
    let mut terrain = full_terrain();
    terrain.version = 24;
    terrain.app_id = None;
    assert_eq!(terrain.to_bytes().unwrap().len(), v25_len - 4);

    terrain.version = 23;
    let bytes = terrain.to_bytes().unwrap();
    // One type byte per road end (2) is gone too.
    assert_eq!(bytes.len(), v25_len - 4 - 2);
    let parsed = Terrain::parse(&bytes).unwrap();
    assert_eq!(parsed.app_id, None);
    assert!(parsed.roads.connections.iter().all(|c| c.kind == 0));
}

#[test]
fn older_versions_round_trip_their_optional_arrays() {
    for version in [15, 16, 17, 18, 20, 21, 22, 23, 24] {
        let terrain = TerrainBuilder::new(4, 8, 10.0)
            .version(version)
            .heights(|i, j| (i * j) as f32)
            .build();
        let parsed = Terrain::parse(&terrain.to_bytes().unwrap()).unwrap();
        assert_eq!(parsed, terrain, "version {version}");
        assert_eq!(parsed.random.is_some(), version < 21);
        assert_eq!(parsed.grass_approx.is_some(), version >= 18);
        assert_eq!(parsed.primary_texture.is_some(), version >= 22);
    }
}

#[test]
fn rejects_other_files() {
    assert!(matches!(
        Terrain::parse(b"8WVR\x00\x00\x00\x00"),
        Err(Error::BadSignature(sig)) if &sig == b"8WVR"
    ));
    assert!(matches!(
        Terrain::parse(b"OPRW\x1a\x00\x00\x00"),
        Err(Error::UnsupportedVersion(26))
    ));
}

#[test]
fn truncated_files_fail_cleanly() {
    let bytes = full_terrain().to_bytes().unwrap();
    for len in (0..bytes.len()).step_by(7) {
        assert!(Terrain::parse(&bytes[..len]).is_err(), "prefix of {len} bytes");
    }
}

#[test]
fn unknown_map_type_is_reported() {
    let mut bytes = TerrainBuilder::new(4, 8, 10.0)
        .edit(|t| {
            t.map_objects.push(MapObject {
                kind: MapType::Tree,
                object_id: 1,
                shape: MapShape::Point(Vec2::ZERO),
            })
        })
        .to_bytes_for_test();
    let at = bytes.len() - 16;
    bytes[at..at + 4].copy_from_slice(&99u32.to_le_bytes());
    assert!(matches!(
        Terrain::parse(&bytes),
        Err(Error::UnknownMapType { kind: 99, offset }) if offset == at
    ));
}

trait BuildBytes {
    fn to_bytes_for_test(self) -> Vec<u8>;
}

impl BuildBytes for TerrainBuilder {
    fn to_bytes_for_test(self) -> Vec<u8> {
        self.build().to_bytes().unwrap()
    }
}

#[test]
fn surface_height_interpolates_on_the_engine_triangles() {
    // One 10 m cell: h(0,0)=0, h(1,0)=10, h(0,1)=20, h(1,1)=40.
    let terrain = TerrainBuilder::new(2, 2, 10.0)
        .heights(|i, j| [[0.0, 10.0], [20.0, 40.0]][j as usize][i as usize])
        .build();
    let h = |x, z| terrain.surface_height(x, z);
    // Corners are exact.
    assert_eq!(h(0.0, 0.0), 0.0);
    assert_eq!(h(10.0, 0.0), 10.0);
    assert_eq!(h(0.0, 10.0), 20.0);
    // Lower-left triangle (x + z <= 1 in cell units) holds (0,0), (1,0), (0,1).
    assert!((h(2.5, 2.5) - 7.5).abs() < 1e-4);
    // Upper-right triangle holds (1,0), (0,1), (1,1).
    assert!((h(7.5, 7.5) - 27.5).abs() < 1e-4);
    // On the split diagonal both triangles agree: the midpoint of (1,0)-(0,1) is 15, not the
    // 17.5 of bilinear interpolation.
    assert!((h(5.0, 5.0) - 15.0).abs() < 1e-4);
}

#[test]
fn surface_height_extends_the_edge_outside_the_grid() {
    let terrain = TerrainBuilder::new(2, 2, 10.0)
        .heights(|i, j| (i + 2 * j) as f32)
        .build();
    assert_eq!(terrain.surface_height(-50.0, -50.0), 0.0);
    assert_eq!(terrain.surface_height(500.0, 500.0), 3.0);
}

#[test]
fn large_grids_cannot_be_written_without_compression() {
    let terrain = TerrainBuilder::new(4, 16, 10.0).build();
    assert!(matches!(terrain.to_bytes(), Err(Error::Write(_))));
}

proptest! {
    #[test]
    fn grids_round_trip_through_quad_trees(
        land_log in 0u32..4,
        values in proptest::collection::vec(any::<u32>(), 64),
        sparse in any::<bool>(),
    ) {
        let land = 1u32 << land_log;
        let pick = |k: usize| if sparse { values[k % 64] % 3 } else { values[k % 64] };
        let terrain = TerrainBuilder::new(land, 8, 4.0)
            .edit(|t| {
                for (k, g) in t.geography.as_mut_slice().iter_mut().enumerate() {
                    *g = Geography(pick(k) as u16);
                }
                for (k, s) in t.sound_map.as_mut_slice().iter_mut().enumerate() {
                    *s = pick(k + 1) as u8;
                }
                for (k, m) in t.material_indices.as_mut_slice().iter_mut().enumerate() {
                    *m = pick(k + 2) as u16;
                }
                for (k, o) in t.object_offsets.as_mut_slice().iter_mut().enumerate() {
                    *o = pick(k + 3);
                }
            })
            .build();
        let parsed = Terrain::parse(&terrain.to_bytes().unwrap()).unwrap();
        prop_assert_eq!(parsed, terrain);
    }
}
