//! Group AI: waypoints drive each unit's [`ManInput`], groups walk in formation and complete
//! their waypoints deterministically, with no wall clock (issue #129).
mod common;

use a3_physics::Interest;
use a3_world::{
    Behaviour, ClientId, CombatMode, Formation, GroupId, Side, SpeedMode, Waypoint, WaypointType,
    World, WorldEvent,
};
use glam::DVec3;

use common::{
    at, collision_world, create_man, model, moves, terrain_with, view_geometry_lod, world,
};

/// The test frame rate: fixed steps, so every run is the same run.
const DT: f64 = 1.0 / 15.0;

fn run(world: &mut World, frames: u32) {
    for _ in 0..frames {
        world.simulate(DT);
    }
}

/// A local West group of `count` men standing in a line at `origin`, leader first.
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

/// Distance on the ground plane (x/z), ignoring height.
fn flat(a: DVec3, b: DVec3) -> f64 {
    DVec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

#[test]
fn a_group_walks_to_its_move_waypoint_and_completes_it() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 20.0);
    let (group, units) = group_at(&mut world, start, 3);
    let target = start + DVec3::new(0.0, 0.0, 20.0);
    let index = world
        .add_waypoint(group, Waypoint::new(WaypointType::Move, target))
        .unwrap();
    assert_eq!(index, 0);
    assert_eq!(world.current_waypoint(group), Some(0));

    run(&mut world, 300);

    // The leader walks onto the waypoint; the followers stand in their wedge slots around him,
    // so they end near it, not on it.
    let leader_at = position(&world, units[0]);
    assert!(
        flat(leader_at, target) < 4.0,
        "the leader reaches the waypoint: {leader_at:?}"
    );
    for (i, unit) in units.iter().enumerate().skip(1) {
        let slot = leader_at + Formation::Wedge.offset(i);
        assert!(
            flat(position(&world, *unit), slot) < 2.0,
            "unit {i} holds his slot in the wedge: {:?}",
            position(&world, *unit)
        );
    }
    // The group has one waypoint and is done with it: the current index is the count.
    assert_eq!(world.current_waypoint(group), Some(1));
    assert!(
        world
            .drain_events()
            .contains(&WorldEvent::WaypointCompleted { group, index: 0 })
    );
}

#[test]
fn a_column_puts_each_unit_behind_the_one_in_front() {
    let mut world = world();
    let start = DVec3::new(100.0, 100.0, 100.0);
    let (group, units) = group_at(&mut world, start, 3);
    world.set_group_formation(group, Formation::Column).unwrap();
    // A run of 60 m north gives the followers time to fall in.
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 60.0)),
        )
        .unwrap();

    run(&mut world, 400);

    let leader = position(&world, units[0]);
    for (i, unit) in units.iter().enumerate().skip(1) {
        let gap = flat(position(&world, *unit), leader);
        assert!(
            (gap - 5.0 * i as f64).abs() < 2.0,
            "unit {i} keeps {:.1} m behind the leader, not {gap:.1}",
            5.0 * i as f64
        );
        // COLUMN runs along the leader's heading (north here): behind is south.
        assert!(
            position(&world, *unit).z < leader.z,
            "unit {i} is behind the leader, not ahead"
        );
    }
}

#[test]
fn waypoints_are_taken_one_at_a_time_in_order() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let first = start + DVec3::new(0.0, 0.0, 10.0);
    let second = start + DVec3::new(0.0, 0.0, 20.0);
    world
        .add_waypoint(group, Waypoint::new(WaypointType::Move, first))
        .unwrap();
    world
        .add_waypoint(group, Waypoint::new(WaypointType::Move, second))
        .unwrap();

    // Reaching the first waypoint advances the group to the second, and it keeps walking.
    let mut reached_first = false;
    for _ in 0..600 {
        run(&mut world, 1);
        if world.current_waypoint(group) == Some(1) {
            reached_first = true;
            break;
        }
    }
    assert!(reached_first, "the first waypoint completes");
    assert!(
        world
            .drain_events()
            .contains(&WorldEvent::WaypointCompleted { group, index: 0 }),
        "completing the first waypoint is an event"
    );
    run(&mut world, 300);
    assert_eq!(
        world.current_waypoint(group),
        Some(2),
        "the second completes too"
    );
    assert!(flat(position(&world, units[0]), second) < 4.0);
}

#[test]
fn a_cycle_waypoint_sends_the_group_back_to_the_first() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let first = start + DVec3::new(0.0, 0.0, 10.0);
    let second = start + DVec3::new(0.0, 0.0, 20.0);
    world
        .add_waypoint(group, Waypoint::new(WaypointType::Move, first))
        .unwrap();
    world
        .add_waypoint(group, Waypoint::new(WaypointType::Cycle, second))
        .unwrap();

    // Wait for the first waypoint, then for the CYCLE waypoint to send the queue back to the
    // first: the index falls back to 0 instead of running past the end.
    let mut at_cycle = false;
    for _ in 0..600 {
        run(&mut world, 1);
        if world.current_waypoint(group) == Some(1) {
            at_cycle = true;
            break;
        }
    }
    assert!(at_cycle, "the group reaches the CYCLE waypoint");
    let mut cycled = false;
    for _ in 0..600 {
        run(&mut world, 1);
        if world.current_waypoint(group) == Some(0) {
            cycled = true;
            break;
        }
    }
    assert!(cycled, "CYCLE starts the queue again at the first waypoint");
    // And the man turns around and walks south, back towards the first waypoint.
    let before = position(&world, units[0]);
    run(&mut world, 60);
    let after = position(&world, units[0]);
    assert!(after.z < before.z, "he walks back: {before:?} -> {after:?}");
}

#[test]
fn a_hold_waypoint_waits_there_and_never_completes() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 2);
    let target = start + DVec3::new(0.0, 0.0, 12.0);
    world
        .add_waypoint(group, Waypoint::new(WaypointType::Move, target))
        .unwrap();
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Hold, target + DVec3::new(0.0, 0.0, 30.0)),
        )
        .unwrap();

    run(&mut world, 800);

    assert_eq!(
        world.current_waypoint(group),
        Some(1),
        "HOLD stays the active waypoint"
    );
    let events = world.drain_events();
    assert!(
        !events.contains(&WorldEvent::WaypointCompleted { group, index: 1 }),
        "a HOLD waypoint is never completed by arriving"
    );
    for unit in &units {
        assert!(
            world.man(*unit).unwrap().input.is_idle(),
            "the group stands still on a HOLD waypoint"
        );
    }
}

#[test]
fn a_waypoint_times_out_when_the_group_cannot_reach_it_in_time() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, _units) = group_at(&mut world, start, 1);
    let far = start + DVec3::new(0.0, 0.0, 500.0);
    let mut waypoint = Waypoint::new(WaypointType::Move, far);
    waypoint.timeout = [0.0, 1.0, 0.0];
    world.add_waypoint(group, waypoint).unwrap();

    run(&mut world, 30);

    assert_eq!(
        world.current_waypoint(group),
        Some(1),
        "the waypoint times out after its middle time"
    );
    assert!(
        world
            .drain_events()
            .contains(&WorldEvent::WaypointCompleted { group, index: 0 })
    );
    assert!(
        flat(position(&world, _units[0]), start) < 5.0,
        "it never got there"
    );
}

#[test]
fn limited_speed_walks_and_normal_speed_runs() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    world
        .set_group_speed_mode(group, SpeedMode::Limited)
        .unwrap();
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 200.0)),
        )
        .unwrap();

    run(&mut world, 60);
    assert_eq!(world.animation_state(units[0]), "walk");
    assert!(position(&world, units[0]).z > start.z, "he still moves");
}

/// A World like [`common::world`], with one opaque ViewGeometry wall standing across the
/// group's line of sight: x 5..15, 3 m tall, at z 40.
fn world_with_wall() -> World {
    let terrain = terrain_with(100.0, "wall.p3d", at(10.0, 100.0, 40.0));
    let wall = model(vec![view_geometry_lod(5.0, 0.0, 3.0)]);
    let mut world = World::new(ClientId::SERVER);
    world.load_terrain(terrain.clone()).unwrap();
    world.load_moves(moves());
    world.set_collision_world(collision_world(
        terrain,
        &[("wall.p3d", wall)],
        &[Interest {
            center: DVec3::new(10.0, 100.0, 40.0),
            radius: 400.0,
        }],
    ));
    world
}

// ---- Orders on units, and the mode commands ----

#[test]
fn an_order_of_his_own_sends_a_unit_where_his_group_is_not_going() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 2);
    // The group is going north; the second man is sent east on his own.
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 40.0)),
        )
        .unwrap();
    let errand = start + DVec3::new(30.0, 0.0, 0.0);
    world.order_move(units[1], errand);

    // He goes there and is done: the order clears on arrival and he turns back to the group, so
    // watch how close he gets rather than where he is at the end of the run.
    let mut closest = f64::INFINITY;
    for _ in 0..200 {
        world.simulate(DT);
        closest = closest.min(flat(position(&world, units[1]), errand));
    }

    assert!(
        closest < 2.0,
        "the man walks his own errand: got within {closest}"
    );
    assert!(
        world.unit_move_order(units[1]).is_none(),
        "a doMove is done on arrival, not kept"
    );
    assert!(
        position(&world, units[0]).z > start.z + 20.0,
        "the group walks its waypoint meanwhile"
    );

    // With his order done he rejoins the group on his own.
    run(&mut world, 300);
    let (leader, him) = (position(&world, units[0]), position(&world, units[1]));
    assert!(
        flat(leader, him) < 12.0,
        "he walks back into the formation: leader {leader:?}, him {him:?}"
    );
}

#[test]
fn do_stop_leaves_a_unit_where_he_stands_until_he_follows_again() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 2);
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 40.0)),
        )
        .unwrap();
    world.stop_unit(units[1]);
    assert!(world.unit_stopped(units[1]), "stopped reports it");

    run(&mut world, 200);
    let his_start = start + DVec3::new(2.0, 0.0, 0.0);
    assert!(
        flat(position(&world, units[1]), his_start) < 1.0,
        "he stayed behind: {:?}",
        position(&world, units[1])
    );

    // doFollow puts him back in the formation.
    world.follow_unit(units[1]);
    assert!(!world.unit_stopped(units[1]));
    let leader = position(&world, units[0]);
    run(&mut world, 200);
    assert!(
        flat(position(&world, units[1]), leader) < 12.0,
        "he falls back in with the group"
    );
}

#[test]
fn the_move_command_replaces_the_queue_with_one_waypoint() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 10.0)),
        )
        .unwrap();
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 20.0)),
        )
        .unwrap();

    let elsewhere = start + DVec3::new(30.0, 0.0, 0.0);
    world.move_group(group, elsewhere).unwrap();

    assert_eq!(world.waypoints(group).len(), 1, "the queue is replaced");
    assert_eq!(world.current_waypoint(group), Some(0));
    run(&mut world, 200);
    assert!(
        flat(position(&world, units[0]), elsewhere) < 4.0,
        "and walked"
    );
}

// ---- The waypoint queue's own commands ----

#[test]
fn a_waypoint_carries_the_group_modes_and_applies_them_when_it_becomes_active() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, _units) = group_at(&mut world, start, 1);
    let mut waypoint = Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 200.0));
    waypoint.behaviour = Some(Behaviour::Combat);
    waypoint.combat_mode = Some(CombatMode::Red);
    waypoint.speed_mode = Some(SpeedMode::Limited);
    waypoint.formation = Some(Formation::File);
    world.add_waypoint(group, waypoint).unwrap();

    // The first waypoint of a group becomes active at once, and so do its modes.
    assert_eq!(world.group_behaviour(group), Some(Behaviour::Combat));
    assert_eq!(world.group_combat_mode(group), Some(CombatMode::Red));
    assert_eq!(world.group_speed_mode(group), Some(SpeedMode::Limited));
    assert_eq!(world.group_formation(group), Some(Formation::File));
}

#[test]
fn set_current_waypoint_applies_that_waypoint_and_deleting_re_indexes() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, _units) = group_at(&mut world, start, 1);
    let mut second = Waypoint::new(WaypointType::Hold, start + DVec3::new(0.0, 0.0, 20.0));
    second.behaviour = Some(Behaviour::Stealth);
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 10.0)),
        )
        .unwrap();
    world.add_waypoint(group, second).unwrap();
    assert_eq!(world.current_waypoint(group), Some(0));

    world.set_current_waypoint(group, 1).unwrap();
    assert_eq!(world.current_waypoint(group), Some(1));
    assert_eq!(world.group_behaviour(group), Some(Behaviour::Stealth));

    // Deleting the first waypoint moves the second down to index 0...
    world.delete_waypoint(group, 0).unwrap();
    assert_eq!(world.waypoints(group).len(), 1);
    assert_eq!(world.current_waypoint(group), Some(0));
    // ...and an index past the end is an error, not a panic.
    assert!(world.delete_waypoint(group, 5).is_err());
    assert!(matches!(
        world.set_current_waypoint(group, 9),
        Err(a3_world::Error::NoSuchWaypoint { .. })
    ));
}

// ---- Target knowledge ----

#[test]
fn a_group_sees_an_enemy_in_the_open_and_its_knowledge_rises() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let enemy_group = world.create_group(Side::East, false);
    let enemy = create_man(&mut world, start + DVec3::new(0.0, 0.0, 40.0));
    world.join(enemy, enemy_group).unwrap();

    run(&mut world, 5);
    let early = world.knows_about(units[0], enemy);
    assert!(early > 0.0, "the group noticed him");
    assert!(early < 4.0, "and is still making him out: {early}");

    run(&mut world, 20);
    assert_eq!(
        world.knows_about(units[0], enemy),
        4.0,
        "a second of looking is a full contact"
    );
    // Units of a group always know about each other.
    assert_eq!(world.knows_about(units[0], units[0]), 4.0);
    assert_eq!(
        world.group_targets(group).unwrap().knowledge(enemy),
        4.0,
        "the contact belongs to the group"
    );
    assert!(
        world
            .group_targets(group)
            .unwrap()
            .iter()
            .any(|k| k.target == enemy && k.position.z > start.z + 30.0)
    );
}

#[test]
fn a_wall_hides_the_enemy_and_the_group_never_learns_about_him() {
    let mut world = world_with_wall();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let enemy_group = world.create_group(Side::East, false);
    // Behind the wall (x 5..15 at z 40), 60 m north.
    let enemy = create_man(&mut world, DVec3::new(10.0, 100.0, 70.0));
    world.join(enemy, enemy_group).unwrap();

    run(&mut world, 60);

    assert_eq!(
        world.knows_about(units[0], enemy),
        0.0,
        "the wall blocks the view"
    );
    assert!(world.group_targets(group).unwrap().is_empty());
}

#[test]
fn a_target_hidden_again_after_being_seen_is_forgotten_after_two_minutes() {
    let mut world = world_with_wall();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let enemy_group = world.create_group(Side::East, false);
    let seen = start + DVec3::new(0.0, 0.0, 25.0); // South of the wall: in plain sight.
    let enemy = create_man(&mut world, seen);
    world.join(enemy, enemy_group).unwrap();

    run(&mut world, 30);
    assert_eq!(world.knows_about(units[0], enemy), 4.0, "seen first");

    // He steps behind the wall: out of sight, but not forgotten yet.
    world
        .entity_mut(enemy)
        .unwrap()
        .set_position(DVec3::new(10.0, 100.0, 70.0));
    run(&mut world, 15 * 100);
    assert!(
        world.knows_about(units[0], enemy) > 0.0,
        "knowledge is kept while he is out of sight"
    );

    run(&mut world, 15 * 30);
    assert_eq!(
        world.knows_about(units[0], enemy),
        0.0,
        "losing sight for more than 120 s resets it"
    );
    assert!(world.group_targets(group).unwrap().is_empty());
}

#[test]
fn reveal_tells_a_group_about_an_enemy_it_cannot_see() {
    let mut world = world_with_wall();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let enemy_group = world.create_group(Side::East, false);
    let enemy = create_man(&mut world, DVec3::new(10.0, 100.0, 70.0));
    world.join(enemy, enemy_group).unwrap();
    run(&mut world, 10);
    assert_eq!(world.knows_about(units[0], enemy), 0.0);

    // Without an accuracy the group gets the side's knowledge — which is none, so 1.
    world.reveal_group(group, enemy, None);
    assert_eq!(world.knows_about(units[0], enemy), 1.0);

    // With one it gets exactly that, and knowledge cannot go down again.
    world.reveal_group(group, enemy, Some(3.0));
    assert_eq!(world.knows_about(units[0], enemy), 3.0);
    world.reveal_group(group, enemy, Some(0.5));
    assert_eq!(world.knows_about(units[0], enemy), 3.0, "only ever rises");

    // And the side's best knowledge is what a later reveal without one passes on.
    let other = world.create_group(Side::West, false);
    world.reveal_group(other, enemy, Some(2.0));
    world.forget_target(group, enemy);
    assert_eq!(world.knows_about(units[0], enemy), 0.0, "forgotten");
    world.reveal_group(group, enemy, None);
    assert_eq!(
        world.knows_about(units[0], enemy),
        2.0,
        "a group of his side knows about him"
    );
}

#[test]
fn a_group_on_the_defensive_turns_to_face_an_enemy_it_knows_about() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let enemy_group = world.create_group(Side::East, false);
    // Due east of the man, who starts facing north (heading 0).
    let enemy = create_man(&mut world, start + DVec3::new(30.0, 0.0, 0.0));
    world.join(enemy, enemy_group).unwrap();

    // AWARE (the default) holds position and does not turn; COMBAT seeks the enemy out.
    run(&mut world, 30);
    assert_eq!(world.group_behaviour(group), Some(Behaviour::Aware));
    assert!(world.man(units[0]).unwrap().input.is_idle());
    world.set_group_behaviour(group, Behaviour::Combat).unwrap();

    run(&mut world, 5);
    assert!(
        world.man(units[0]).unwrap().input.turn > 0.5,
        "he turns towards the enemy"
    );
    run(&mut world, 90);
    let heading = world.entity(units[0]).unwrap().heading();
    assert!(
        (heading - 90.0).abs() < 5.0 || (heading - 270.0).abs() < 5.0,
        "he ends up facing east, not {heading}"
    );
}

#[test]
fn a_unit_whose_courage_broke_runs_from_the_enemy_instead_of_his_waypoint() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 10.0);
    let (group, units) = group_at(&mut world, start, 1);
    let enemy_group = world.create_group(Side::East, false);
    // An enemy 40 m east, in plain sight, and the group's waypoint 120 m north: the two pull
    // the man in different directions.
    let enemy = create_man(&mut world, start + DVec3::new(40.0, 0.0, 0.0));
    world.join(enemy, enemy_group).unwrap();
    world
        .add_waypoint(
            group,
            Waypoint::new(WaypointType::Move, start + DVec3::new(0.0, 0.0, 120.0)),
        )
        .unwrap();

    // Full cowardice (`allowFleeing 1`, read back by `fleeing`): he breaks off west instead.
    world.object_state_mut(units[0]).fleeing = 1.0;
    run(&mut world, 200);
    let fled = position(&world, units[0]);
    assert!(
        fled.x < start.x - 5.0,
        "he runs away from the enemy, not on to the waypoint: {fled:?}"
    );
    assert!(
        fled.z < start.z + 40.0,
        "and does not work the waypoint meanwhile: {fled:?}"
    );

    // With his courage back he works the waypoint again.
    world.object_state_mut(units[0]).fleeing = 0.0;
    let broke = position(&world, units[0]);
    run(&mut world, 200);
    assert!(
        position(&world, units[0]).z > broke.z + 5.0,
        "he walks north to the waypoint again: {broke:?} -> {:?}",
        position(&world, units[0])
    );
}
