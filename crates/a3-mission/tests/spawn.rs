//! Spawning a synthetic mission into a World: Entities at the SQM positions, groups, leaders.

mod common;

use a3_mission::{Mission, spawn_mission};
use a3_world::{Side, World};

/// A mission with a two-unit WEST group (a leader and the player), an EAST group with one
/// absent and one present soldier (the present one has no config class), and one ungrouped
/// object. Heights are 12 m everywhere, so an on-surface placement lands at 12.
const SQM: &str = r#"version=12;
class Mission
{
	class Groups
	{
		items=2;
		class Item0
		{
			side="WEST";
			class Vehicles
			{
				items=2;
				class Item0
				{
					position[]={1000,0,2000};
					azimut=45;
					id=7;
					side="WEST";
					vehicle="B_Soldier_F";
					leader=1;
					text="boss";
				};
				class Item1
				{
					id=3;
					position[]={1010,2020};
					vehicle="B_soldier_AR_F";
					player="PLAYER COMMANDER";
				};
			};
		};
		class Item1
		{
			side="EAST";
			class Vehicles
			{
				items=2;
				class Item0
				{
					id=11;
					position[]={3000,4000};
					vehicle="O_Soldier_F";
					presence=0;
				};
				class Item1
				{
					id=12;
					position[]={3010,4010};
					vehicle="O_Missing_F";
				};
			};
		};
	};
	class Vehicles
	{
		items=1;
		class Item0
		{
			id=32;
			position[]={500,1.5,600};
			azimut=-90;
			vehicle="Land_Cargo10_F";
		};
	};
	class Markers
	{
		items=1;
		class Item0
		{
			position[]={100,0,200};
			name="marker_start";
			text="Start";
			type="Empty";
		};
	};
};
"#;

fn position(world: &World, id: a3_world::EntityId) -> glam::DVec3 {
    world.entity(id).unwrap().position()
}

#[test]
fn creates_every_present_unit_at_its_place() {
    let (mut world, mut types) = common::world_and_types();
    let mission = Mission::parse(SQM.as_bytes()).unwrap();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    assert_eq!(spawned.units.len(), 3, "7, 3 and 32");
    assert_eq!(spawned.unspawned.len(), 1);
    assert_eq!(spawned.unspawned[0].id, 12);
    assert_eq!(spawned.unspawned[0].class, "O_Missing_F");
    assert!(spawned.unspawned[0].reason.contains("O_Missing_F"));

    // The 2D editor's stored height is not a place (`docs/re/missions.md` §Placement): the
    // engine stands the entity on the ground at its `x`/`z`, which is 12 m here.
    let boss = spawned.units[&7];
    assert_eq!(
        position(&world, boss),
        glam::DVec3::new(1000.0, 12.0, 2000.0)
    );
    assert_eq!(world.entity(boss).unwrap().type_name(), "B_Soldier_F");

    // A 2-component position likewise.
    let player = spawned.units[&3];
    assert_eq!(
        position(&world, player),
        glam::DVec3::new(1010.0, 12.0, 2020.0)
    );

    // The ungrouped object, with its own heading.
    let cargo = spawned.units[&32];
    assert_eq!(
        position(&world, cargo),
        glam::DVec3::new(500.0, 12.0, 600.0)
    );
    assert_eq!(world.entity(cargo).unwrap().type_name(), "Land_Cargo10_F");
    assert_eq!(world.entity(cargo).unwrap().group(), None);
}

/// The 2D editor's `special="FLY"` is the one case whose stored height the engine reads (as the
/// aircraft's altitude), so the Entity is created where the SQM put it, not on the ground.
#[test]
fn a_flying_entity_keeps_its_stored_altitude() {
    let sqm = r#"version=12;
class Mission
{
	class Vehicles
	{
		items=1;
		class Item0
		{
			position[]={500,200,600};
			special="FLY";
			id=40;
			vehicle="Land_Cargo10_F";
		};
	};
};
"#;
    let (mut world, mut types) = common::world_and_types();
    let mission = Mission::parse(sqm.as_bytes()).unwrap();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    assert_eq!(
        position(&world, spawned.units[&40]),
        glam::DVec3::new(500.0, 200.0, 600.0)
    );
}

/// Over sea the ground is the water surface, not the sea floor: a 2D-editor entity stands at
/// y = 0 above a terrain 30 m below it (measured on the Oracle, `docs/re/missions.md`
/// §Placement).
#[test]
fn an_entity_over_sea_stands_on_the_water_surface() {
    let (mut world, mut types) = common::world_and_types_at(-30.0);
    let mission = Mission::parse(SQM.as_bytes()).unwrap();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    assert_eq!(
        position(&world, spawned.units[&7]),
        glam::DVec3::new(1000.0, 0.0, 2000.0)
    );
}

#[test]
fn headings_come_from_azimut() {
    let (mut world, mut types) = common::world_and_types();
    let mission = Mission::parse(SQM.as_bytes()).unwrap();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    // Degrees clockwise from north (`getDir`), so 45 is north-east.
    assert!((world.entity(spawned.units[&7]).unwrap().heading() - 45.0).abs() < 1e-9);
    // A negative azimut wraps like `setDir`.
    assert!((world.entity(spawned.units[&32]).unwrap().heading() - 270.0).abs() < 1e-9);
    assert_eq!(world.entity(spawned.units[&3]).unwrap().heading(), 0.0);
}

#[test]
fn units_join_their_group_and_its_leader_is_the_one_marked() {
    let (mut world, mut types) = common::world_and_types();
    let mission = Mission::parse(SQM.as_bytes()).unwrap();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    assert_eq!(spawned.groups.len(), 2);
    let west = &spawned.groups[0];
    assert_eq!(west.side, Side::West);
    assert_eq!(west.units, vec![spawned.units[&7], spawned.units[&3]]);
    for unit in &west.units {
        assert_eq!(world.entity(*unit).unwrap().group(), Some(west.group));
    }
    let group = world.group(west.group).unwrap();
    assert_eq!(group.side(), Side::West);
    assert_eq!(group.leader(), Some(spawned.units[&7]));
    assert_eq!(group.units().len(), 2);

    // The absent unit is not created, so the EAST group holds no one.
    let east = &spawned.groups[1];
    assert_eq!(east.side, Side::East);
    assert!(east.units.is_empty());
    assert_eq!(world.group(east.group).unwrap().units().len(), 0);
}

#[test]
fn the_player_unit_is_the_one_the_sqm_marks() {
    let (mut world, mut types) = common::world_and_types();
    let mission = Mission::parse(SQM.as_bytes()).unwrap();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    assert_eq!(spawned.player, Some(spawned.units[&3]));
    assert_eq!(mission.player().map(|u| u.id), Some(3));
}

#[test]
fn markers_are_carried_for_the_caller() {
    let (mut world, mut types) = common::world_and_types();
    let mission = Mission::parse(SQM.as_bytes()).unwrap();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    assert_eq!(
        spawned.markers,
        vec![(
            "marker_start".to_owned(),
            glam::DVec3::new(100.0, 0.0, 200.0)
        )]
    );
}

#[test]
fn absent_units_are_not_created_and_unknown_classes_do_not_stop_the_rest() {
    let (mut world, mut types) = common::world_and_types();
    let mission = Mission::parse(SQM.as_bytes()).unwrap();
    let before = world.entities().count();
    let spawned = spawn_mission(&mut world, &mut types, &mission);

    assert_eq!(world.entities().count() - before, 3);
    assert!(!spawned.units.contains_key(&11), "presence=0");
    assert!(!spawned.units.contains_key(&12), "unknown class");
}
