//! Formations, the leader's speed, the SAFE file, paths, the unit formation FSM and the unit
//! commands of the AI (`docs/re/ai.md` §3-§4, `docs/re/ai-fsm.md`).
mod common;

use std::sync::Arc;

use a3_config::{ConfigTree, parse_text};
use a3_fsm::Fsm;
use a3_world::{
    AiFeatures, Behaviour, Formation, FormationTable, GroupId, Side, SpeedMode, UnitPos, Waypoint,
    WaypointType, World,
};
use glam::DVec3;

use common::{create_man, world};

const DT: f64 = 1.0 / 15.0;

fn run(world: &mut World, frames: u32) {
    for _ in 0..frames {
        world.simulate(DT);
    }
}

fn group_at(world: &mut World, origin: DVec3, count: usize) -> (GroupId, Vec<a3_world::EntityId>) {
    let group = world.create_group(Side::West, false);
    let units = (0..count)
        .map(|i| {
            let id = create_man(world, origin + DVec3::new(i as f64 * 2.0, 0.0, 0.0));
            world.join(id, group).unwrap();
            id
        })
        .collect::<Vec<_>>();
    (group, units)
}

fn position(world: &World, id: a3_world::EntityId) -> DVec3 {
    world.entity(id).unwrap().position()
}

fn flat(a: DVec3, b: DVec3) -> f64 {
    DVec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

/// `(x, z)` of every slot, in formation units for men (5 m).
fn units_of(formation: Formation, n: usize) -> Vec<(f64, f64)> {
    let men = vec![Some((5.0, 5.0)); n];
    FormationTable::shipped()
        .slots(formation, &men)
        .iter()
        .map(|s| (s.offset.x / 5.0, s.offset.z / 5.0))
        .collect()
}

#[test]
fn the_shipped_formations_place_slots_as_the_engine_does() {
    // `docs/re/ai.md` §4.1: x right of the formation direction, z ahead, slot 0 the leader's.
    assert_eq!(
        units_of(Formation::Wedge, 5),
        [
            (0.0, 0.0),
            (1.0, -1.0),
            (-1.0, -1.0),
            (2.0, -2.0),
            (-2.0, -2.0)
        ]
    );
    assert_eq!(
        units_of(Formation::StagColumn, 4),
        [(0.0, 0.0), (1.0, -1.0), (0.0, -2.0), (1.0, -3.0)]
    );
    assert_eq!(
        units_of(Formation::Vee, 5),
        [(0.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (2.0, 1.0), (-2.0, 2.0)]
    );
    assert_eq!(
        units_of(Formation::Line, 4),
        [(0.0, 0.0), (1.0, 0.0), (-1.0, 0.0), (2.0, 0.0)]
    );
    assert_eq!(
        units_of(Formation::Diamond, 6),
        [
            (0.0, 0.0),
            (0.5, -0.5),
            (-0.5, -0.5),
            (0.0, -1.0),
            (0.5, -1.5),
            (-0.5, -1.5)
        ]
    );
    assert_eq!(
        units_of(Formation::File, 3),
        [(0.0, 0.0), (0.0, -0.5), (0.0, -1.0)]
    );
    assert_eq!(
        units_of(Formation::EchLeft, 3),
        [(0.0, 0.0), (-1.0, -1.0), (-2.0, -2.0)]
    );
    assert_eq!(units_of(Formation::Column, 13)[12], (0.0, -12.0));
}

#[test]
fn a_formation_step_is_the_average_formation_unit_of_the_two_units_it_links() {
    // A 5 m man behind a 15 m vehicle in a COLUMN stands (5 + 15) / 2 = 10 m back; an empty
    // slot counts as one metre.
    let slots = FormationTable::shipped().slots(
        Formation::Column,
        &[Some((10.0, 15.0)), Some((5.0, 5.0)), None, Some((5.0, 5.0))],
    );
    assert_eq!(slots[1].offset, DVec3::new(0.0, 0.0, -10.0));
    assert_eq!(slots[2].offset, DVec3::new(0.0, 0.0, -13.0));
    assert_eq!(slots[3].offset, DVec3::new(0.0, 0.0, -16.0));
    // Watch directions: the column's second man looks a quarter right.
    assert!((slots[1].angle - std::f64::consts::FRAC_PI_4).abs() < 1e-6);
}

#[test]
fn a_formation_table_reads_cfg_formations_by_position() {
    let config = parse_text(
        r#"class cfgFormations { class West {
            class A { class Fixed { p1[] = {-1, 0, 0, 0}; }; class Pattern { p1[] = {-1, 2, -3, 0.5}; }; };
        }; };"#,
    )
    .unwrap();
    let tree = ConfigTree::from_config(&config);
    let table = FormationTable::from_config(&(tree.root() >> "cfgFormations" >> "West"));
    // The first class is COLUMN, whatever its name.
    let slots = table.slots(Formation::Column, &[Some((5.0, 5.0)); 3]);
    assert_eq!(slots[2].offset, DVec3::new(20.0, 0.0, -30.0));
    // Missing formations default to a line.
    let line = table.slots(Formation::File, &[Some((5.0, 5.0)); 3]);
    assert_eq!(line[2].offset, DVec3::new(-5.0, 0.0, 0.0));
}

#[test]
fn followers_take_their_slots_along_the_way_to_the_waypoint() {
    let mut world = world();
    let start = DVec3::new(60.0, 100.0, 20.0);
    let (group, units) = group_at(&mut world, start, 3);
    // East: the formation turns to face the waypoint.
    let target = start + DVec3::new(60.0, 0.0, 0.0);
    world
        .add_waypoint(group, Waypoint::new(WaypointType::Move, target))
        .unwrap();
    run(&mut world, 600);
    let heading = world.formation_direction(group).unwrap();
    assert!(
        (heading - 90.0).abs() < 1e-6,
        "formation faces east: {heading}"
    );
    // In a wedge facing east, slot 1 is right (south, -z) and behind (west, -x) of the leader.
    let leader = position(&world, units[0]);
    let slot1 = world.formation_position(units[1]).unwrap();
    assert!(flat(slot1, leader + DVec3::new(-5.0, 0.0, -5.0)) < 1e-6);
    assert!(
        flat(position(&world, units[1]), slot1) < 1.5,
        "the follower stands in his slot: {:?} vs {slot1:?}",
        position(&world, units[1])
    );
}

#[test]
fn the_leader_slows_down_for_a_follower_left_behind() {
    let mut world = world();
    let start = DVec3::new(20.0, 100.0, 20.0);
    let (group, units) = group_at(&mut world, start, 2);
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 150.0)),
        )
        .unwrap();
    run(&mut world, 30);
    let coef_free = world.group(group).unwrap().ai().formation_coef;
    assert!(coef_free > 1.0, "nobody lags: {coef_free}");

    // The follower is held to a standstill in his place in the formation: he falls behind his
    // slot, and the leader slows down for him (`AISubgroup_SetLeaderSpeed`).
    world.force_speed(units[1], 0.0);
    run(&mut world, 150);
    let coef = world.group(group).unwrap().ai().formation_coef;
    assert!(coef < 0.9, "the leader slows for him: {coef}");
}

#[test]
fn a_safe_group_walks_in_file_behind_its_leader() {
    let mut world = world();
    let start = DVec3::new(60.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 3);
    world.set_group_behaviour(group, Behaviour::Safe).unwrap();
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 120.0)),
        )
        .unwrap();
    run(&mut world, 600);
    // Each man walks on the trail of the man ahead: all three close to one line along the
    // way (north, x ≈ the leader's), one behind the other.
    let p: Vec<DVec3> = units.iter().map(|u| position(&world, *u)).collect();
    assert!(p[0].z > p[1].z && p[1].z > p[2].z, "in file: {p:?}");
    for i in 1..3 {
        assert!(
            (p[i].x - p[0].x).abs() < 2.0,
            "man {i} walks on the leader's line: {p:?}"
        );
    }
}

#[test]
fn limited_speed_holds_the_leader_to_a_walk_and_full_speed_runs() {
    let mut world = world();
    let start = DVec3::new(20.0, 100.0, 20.0);
    let (group, units) = group_at(&mut world, start, 1);
    world
        .set_group_speed_mode(group, SpeedMode::Limited)
        .unwrap();
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 150.0)),
        )
        .unwrap();
    run(&mut world, 10);
    assert!(!world.man(units[0]).unwrap().input.sprint, "LIMITED walks");
    world.set_group_speed_mode(group, SpeedMode::Full).unwrap();
    run(&mut world, 10);
    assert!(world.man(units[0]).unwrap().input.sprint, "FULL runs");
    // forceSpeed caps him again.
    world.force_speed(units[0], 1.0);
    run(&mut world, 2);
    assert!(
        !world.man(units[0]).unwrap().input.sprint,
        "forced to a walk"
    );
}

#[test]
fn a_path_leads_the_leader_around_a_blocked_area() {
    // A 200 m terrain of 5 m land cells: the navigation grid is the land grid.
    let terrain = Arc::new(
        a3_wrp::TerrainBuilder::new(40, 40, 5.0)
            .heights(|_, _| 100.0)
            .build(),
    );
    let mut world = World::new(a3_world::ClientId::SERVER);
    world.load_terrain(terrain.clone()).unwrap();
    world.load_moves(common::moves());
    world.set_collision_world(common::collision_world(terrain.clone(), &[], &[]));
    let mut navigator = a3_nav::Navigator::new(terrain);
    // A wall of blocked cells across the straight line, from the west edge to x = 60.
    let grid = navigator.grid_mut();
    for z in 0..grid.height() {
        for x in 0..grid.width() {
            let c = grid.cell_center(x, z);
            if (90.0..110.0).contains(&c.z) && c.x < 60.0 {
                grid.set_cost(x, z, 0);
            }
        }
    }
    world.set_navigator(navigator);
    let start = DVec3::new(30.0, 100.0, 60.0);
    let (group, units) = group_at(&mut world, start, 1);
    let target = DVec3::new(30.0, 100.0, 140.0);
    world
        .add_waypoint(group, Waypoint::new(WaypointType::Move, target))
        .unwrap();
    let path = loop {
        world.simulate(DT);
        if let Some(path) = world.man(units[0]).unwrap().ai.path.clone() {
            break path;
        }
    };
    assert!(
        path.points.len() > 1,
        "the path bends around the wall: {:?}",
        path.points
    );
    let crosses = |p: &DVec3| (90.0..110.0).contains(&p.z) && (0.0..60.0).contains(&p.x);
    assert!(!path.points.iter().any(crosses), "{:?}", path.points);
}

/// A small native FSM of our own over the engine's Man functions: down in COMBAT, back to
/// AUTO out of it.
const NATIVE: &str = r#"class CfgFSMs { class Drill { class States {
    class Start { name = "Start";
        class Init { function = "formationExcluded"; parameters[] = {}; thresholds[] = {}; };
        class Links { class Combat { priority = 1; to = "Down";
            class Condition { function = "behaviourCombat"; parameters[] = {}; threshold = 0; };
            class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; }; };
    };
    class Down { name = "Down";
        class Init { function = "setUnitPosToDown"; parameters[] = {}; thresholds[] = {}; };
        class Links { class Calm { priority = 1; to = "Start";
            class Condition { function = "1-behaviourCombat"; parameters[] = {}; threshold = 0; };
            class Action { function = "nothing"; parameters[] = {}; thresholds[] = {}; }; }; };
    };
}; initState = "Start"; finalStates[] = {}; }; };"#;

fn drill() -> Fsm {
    let tree = ConfigTree::from_config(&parse_text(NATIVE).unwrap());
    let mut fsm = Fsm::from_native_config(&(tree.root() >> "CfgFSMs" >> "Drill"))
        .unwrap()
        .fsm;
    // Our soldiers' `fsmFormation` is the engine's "Formation"; the drill stands in for it.
    fsm.name = "Formation".into();
    fsm
}

#[test]
fn the_formation_fsm_drives_the_stance_with_the_behaviour() {
    let mut world = world();
    world.add_native_fsm(drill());
    let (group, units) = group_at(&mut world, DVec3::new(20.0, 100.0, 20.0), 2);
    run(&mut world, 2);
    assert_eq!(world.effective_unit_pos(units[1]), UnitPos::Auto);
    world.set_group_behaviour(group, Behaviour::Combat).unwrap();
    run(&mut world, 2);
    assert_eq!(world.effective_unit_pos(units[1]), UnitPos::Down);
    assert_eq!(
        world.man(units[1]).unwrap().input.stance,
        a3_moves::Stance::Prone
    );
    // A script's stance beats the FSM's.
    world.set_unit_pos(units[1], UnitPos::Up);
    run(&mut world, 1);
    assert_eq!(world.effective_unit_pos(units[1]), UnitPos::Up);
    world.set_unit_pos(units[1], UnitPos::Auto);
    world.set_group_behaviour(group, Behaviour::Aware).unwrap();
    run(&mut world, 2);
    assert_eq!(world.effective_unit_pos(units[1]), UnitPos::Auto);
}

#[test]
fn disable_ai_fsm_stops_the_formation_fsm() {
    let mut world = world();
    world.add_native_fsm(drill());
    let (group, units) = group_at(&mut world, DVec3::new(20.0, 100.0, 20.0), 1);
    world.set_ai_feature(units[0], AiFeatures::FSM, false);
    world.set_group_behaviour(group, Behaviour::Combat).unwrap();
    run(&mut world, 3);
    assert_eq!(world.effective_unit_pos(units[0]), UnitPos::Auto);
    world.set_ai_feature(units[0], AiFeatures::FSM, true);
    run(&mut world, 3);
    assert_eq!(world.effective_unit_pos(units[0]), UnitPos::Down);
}

#[test]
fn the_shipped_formation_data_matches_and_the_formation_fsm_covers_in_combat() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    let formations = data.config.root() >> "cfgFormations";
    for side in ["West", "East", "Guer", "Civ"] {
        assert_eq!(
            FormationTable::from_config(&(&formations >> side)),
            FormationTable::shipped(),
            "cfgFormations >> {side}"
        );
    }
    let mut world = world();
    let warnings = world.load_native_fsms(&(data.config.root() >> "CfgFSMs"));
    assert!(warnings.is_empty(), "{warnings:?}");
    let (group, units) = group_at(&mut world, DVec3::new(20.0, 100.0, 20.0), 3);
    // Without cover objects, `coverReached` holds only with cover off; then the FSM goes into
    // cover: a stance down or crouched, picked at random.
    for unit in &units {
        world.set_ai_feature(*unit, AiFeatures::COVER, false);
    }
    world.set_group_behaviour(group, Behaviour::Combat).unwrap();
    run(&mut world, 10);
    for unit in &units[1..] {
        let pos = world.effective_unit_pos(*unit);
        assert!(
            matches!(pos, UnitPos::Down | UnitPos::Middle),
            "{unit:?} covers: {pos:?}"
        );
        assert_eq!(world.man(*unit).unwrap().ai.cover.mode, 2);
    }
    // Out of COMBAT, the FSM cleans up and leaves the stance to the AI again.
    world.set_group_behaviour(group, Behaviour::Aware).unwrap();
    run(&mut world, 5);
    for unit in &units {
        assert_eq!(world.effective_unit_pos(*unit), UnitPos::Auto);
    }
}

#[test]
fn a_type_without_config_moves_as_a_rifleman() {
    let ty = Arc::new(a3_world::EntityType::new(
        "B_Soldier_F",
        a3_world::SimulationClass::Soldier,
    ));
    let ai = ty.ai();
    assert_eq!(
        (ai.formation_x, ai.formation_z, ai.precision),
        (5.0, 5.0, 1.0)
    );
    assert!((ai.max_speed - 24.0 / 3.6).abs() < 1e-9);
    assert_eq!(ai.fsm_formation, "Formation");
}
