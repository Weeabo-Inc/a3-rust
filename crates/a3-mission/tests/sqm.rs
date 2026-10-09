//! `mission.sqm` parsing, on a synthetic file with the exact header and class shapes the shipped
//! campaign missions use.

use a3_config::write_rap;
use a3_mission::{Mission, MissionError, MissionVariable, parse_sqm, side_from_sqm};
use a3_world::Side;

/// A mission shaped like the shipped ones: `version`, `Mission` with its `Intel`, `Groups`
/// (units and waypoints), ungrouped `Vehicles`, `Markers`, `Sensors`, and a second top-level
/// class (`Intro`).
const SQM: &str = r#"version=12;
class Mission
{
	addOns[]=
	{
		"A3_Characters_F",
		"A3_Modules_F"
	};
	addOnsAuto[]=
	{
		"A3_Characters_F"
	};
	randomSeed=1234;
	class Intel
	{
		briefingName="Live Fire Exercise";
		timeOfChanges=1800;
		startWeather=0.25;
		startWind=0.1;
		startWaves=0.05;
		forecastWeather=0.4;
		forecastWind=0.2;
		forecastWaves=0.1;
		forecastLightnings=0.05;
		rainForced=0;
		lightningsForced=0;
		year=2035;
		month=6;
		day=24;
		hour=9;
		minute=30;
		startFogDecay=0.01;
		forecastFogDecay=0.02;
	};
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
					position[]={1000,2000,0};
					azimut=45;
					id=7;
					side="WEST";
					vehicle="B_Soldier_TL_F";
					leader=1;
					rank="SERGEANT";
					skill=0.60000002;
					text="boss";
					init="bossInit = true;";
				};
				class Item1
				{
					id=3;
					position[]={1010,2020,5.5};
					vehicle="B_soldier_AR_F";
					player="PLAYER COMMANDER";
					init="";
				};
			};
			class Waypoints
			{
				items=1;
				class Item0
				{
					position[]={1100,2100,0};
					type="MOVE";
					speed="NORMAL";
					combatMode="YELLOW";
					behaviour="SAFE";
					formation="WEDGE";
					description="move on";
					showWP="NEVER";
					timeout=30;
					synchronizations[]={1,2};
				};
			};
		};
		class Item1
		{
			side="EAST";
			init="groupEastInit = true;";
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
					vehicle="O_Soldier_F";
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
			position[]={500,600,1.5};
			azimut=-90;
			vehicle="Land_Cargo10_F";
		};
	};
	class Markers
	{
		items=2;
		class Item0
		{
			position[]={100,200,0};
			name="marker_start";
			text="Start";
			markerType="mil_dot";
			colorName="ColorRed";
			fillName="Solid";
			a=20;
			b=30;
			angle=15;
			drawBorder=1;
		};
		class Item1
		{
			position[]={110,210,0};
			name="BIS_return";
			text="BIS_return";
			type="Empty";
		};
	};
	class Sensors
	{
		items=1;
		class Item0
		{
			position[]={4000,5000,0};
			a=50;
			b=60;
			angle=30;
			name="trigger1";
			text="East detected";
			activationBy="EAST";
			activationType="PRESENT";
			repeating=1;
			interruptable=1;
			age="UNKNOWN";
			idVehicle=0;
			expActiv="hint ""go""";
			expCond="true";
			expDesactiv="hint ""stop""";
			synchronizations[]={2,1,0};
			syncId=3;
			class Effects
			{
			};
		};
	};
};
class Intro
{
	class Intel
	{
		briefingName="Intro";
	};
};
"#;

fn mission() -> Mission {
    Mission::parse(SQM.as_bytes()).expect("the fixture parses")
}

/// Config numbers are 32-bit in the engine and in the parser, so compare with a tolerance.
fn near(value: Option<f64>, expected: f64) -> bool {
    value.is_some_and(|v| (v - expected).abs() < 1e-6)
}

#[test]
fn header_and_intel() {
    let m = mission();
    assert_eq!(m.version, 12);
    assert_eq!(m.addons, ["A3_Characters_F", "A3_Modules_F"]);
    assert_eq!(m.addons_auto, ["A3_Characters_F"]);
    assert_eq!(m.random_seed, 1234);
    assert_eq!(m.intel.briefing_name.as_deref(), Some("Live Fire Exercise"));
    assert_eq!(m.intel.year, Some(2035));
    assert_eq!(m.intel.month, Some(6));
    assert_eq!(m.intel.day, Some(24));
    assert_eq!(m.intel.hour, Some(9));
    assert_eq!(m.intel.minute, Some(30));
    assert!(near(m.intel.start_weather, 0.25));
    assert!(near(m.intel.forecast_weather, 0.4));
    assert!(near(m.intel.forecast_lightnings, 0.05));
    assert!(near(m.intel.rain_forced, 0.0));
    assert!(near(m.intel.start_fog_decay, 0.01));
    assert!(near(m.intel.forecast_fog_decay, 0.02));
    // Only `class Mission` is read; `Intro` is a separate scene.
    assert_eq!(m.folder, "");
    assert_eq!(m.terrain, None);
}

#[test]
fn groups_units_and_leaders() {
    let m = mission();
    assert_eq!(m.groups.len(), 2);

    let west = &m.groups[0];
    assert_eq!(west.side, Side::West);
    assert_eq!(west.units.len(), 2);
    assert_eq!(west.units[0].id, 7);
    assert_eq!(west.units[1].id, 3);
    assert_eq!(west.units[0].class, "B_Soldier_TL_F");
    assert_eq!(west.units[0].rank.as_deref(), Some("SERGEANT"));
    assert!((west.units[0].skill.unwrap() - 0.6).abs() < 1e-6);
    assert_eq!(west.units[0].text.as_deref(), Some("boss"));
    assert_eq!(west.units[0].init.as_deref(), Some("bossInit = true;"));
    assert!(west.units[0].leader);
    assert!(!west.units[1].leader);
    assert_eq!(
        west.units[1].player.as_deref(),
        Some("PLAYER COMMANDER"),
        "the player unit carries the editor's player string"
    );

    // `position[]` is {east, north, height}: world (x, z, y).
    assert_eq!(
        west.units[0].position,
        glam::DVec3::new(1000.0, 0.0, 2000.0)
    );
    assert!(!west.units[0].on_surface, "3 components keep their height");
    assert_eq!(west.units[0].azimut, Some(45.0));
    assert_eq!(
        west.units[1].position,
        glam::DVec3::new(1010.0, 5.5, 2020.0)
    );

    let east = &m.groups[1];
    assert_eq!(east.side, Side::East);
    assert_eq!(east.init.as_deref(), Some("groupEastInit = true;"));
    assert!(east.units[0].is_absent(), "presence=0 means not spawned");
    // 2 components place him on the surface.
    assert!(east.units[1].on_surface);
    assert_eq!(
        east.units[1].position,
        glam::DVec3::new(3010.0, 0.0, 4010.0)
    );
    assert_eq!(east.units[1].azimut, None);
}

#[test]
fn waypoints() {
    let m = mission();
    let waypoints = &m.groups[0].waypoints;
    assert_eq!(waypoints.len(), 1);
    let wp = &waypoints[0];
    assert_eq!(wp.position, glam::DVec3::new(1100.0, 0.0, 2100.0));
    assert_eq!(wp.kind.as_deref(), Some("MOVE"));
    assert_eq!(wp.speed.as_deref(), Some("NORMAL"));
    assert_eq!(wp.combat_mode.as_deref(), Some("YELLOW"));
    assert_eq!(wp.behaviour.as_deref(), Some("SAFE"));
    assert_eq!(wp.formation.as_deref(), Some("WEDGE"));
    assert_eq!(wp.description.as_deref(), Some("move on"));
    assert_eq!(wp.show_wp.as_deref(), Some("NEVER"));
    assert_eq!(wp.timeout, Some(30.0));
    assert_eq!(wp.synchronizations, [1, 2]);
    assert_eq!(wp.sync_id, None);
}

#[test]
fn ungrouped_objects_are_units_too() {
    let m = mission();
    assert_eq!(m.objects.len(), 1);
    let cargo = &m.objects[0];
    assert_eq!(cargo.id, 32);
    assert_eq!(cargo.class, "Land_Cargo10_F");
    assert_eq!(cargo.position, glam::DVec3::new(500.0, 1.5, 600.0));
    assert_eq!(cargo.azimut, Some(-90.0));
    assert_eq!(m.units().count(), 5);
    assert_eq!(m.unit(32).map(|u| u.class.as_str()), Some("Land_Cargo10_F"));
    assert!(m.group_of(32).is_none());
    assert_eq!(m.group_of(7).map(|g| g.side), Some(Side::West));
}

#[test]
fn markers_and_sensors() {
    let m = mission();
    assert_eq!(m.markers.len(), 2);
    let marker = &m.markers[0];
    assert_eq!(marker.name, "marker_start");
    assert_eq!(marker.text.as_deref(), Some("Start"));
    assert_eq!(marker.marker_type.as_deref(), Some("mil_dot"));
    assert_eq!(marker.color.as_deref(), Some("ColorRed"));
    assert_eq!(marker.fill.as_deref(), Some("Solid"));
    assert_eq!(marker.a, Some(20.0));
    assert_eq!(marker.b, Some(30.0));
    assert_eq!(marker.angle, Some(15.0));
    assert!(marker.draw_border);
    assert_eq!(marker.position, glam::DVec3::new(100.0, 0.0, 200.0));
    // A marker without `markerType` falls back to `type`.
    assert_eq!(m.markers[1].marker_type.as_deref(), Some("Empty"));
    assert!(!m.markers[1].draw_border);

    assert_eq!(m.triggers.len(), 1);
    let trigger = &m.triggers[0];
    assert_eq!(trigger.name.as_deref(), Some("trigger1"));
    assert_eq!(trigger.text.as_deref(), Some("East detected"));
    assert_eq!(trigger.activation_by.as_deref(), Some("EAST"));
    assert_eq!(trigger.activation_type.as_deref(), Some("PRESENT"));
    assert!(trigger.repeating);
    assert!(trigger.interruptable);
    assert_eq!(trigger.age.as_deref(), Some("UNKNOWN"));
    assert_eq!(trigger.id_vehicle, Some(0));
    assert_eq!(trigger.a, Some(50.0));
    assert_eq!(trigger.b, Some(60.0));
    assert_eq!(trigger.angle, Some(30.0));
    assert_eq!(trigger.exp_activ.as_deref(), Some("hint \"go\""));
    assert_eq!(trigger.exp_cond.as_deref(), Some("true"));
    assert_eq!(trigger.exp_desactiv.as_deref(), Some("hint \"stop\""));
    assert_eq!(trigger.synchronizations, [2, 1, 0]);
    assert_eq!(trigger.sync_id, Some(3));
}

#[test]
fn variables_are_the_named_units_and_markers() {
    let m = mission();
    let variables = m.variables();
    assert_eq!(
        variables,
        [
            ("boss".to_owned(), MissionVariable::Unit(7)),
            (
                "marker_start".to_owned(),
                MissionVariable::Marker("marker_start".to_owned())
            ),
            (
                "BIS_return".to_owned(),
                MissionVariable::Marker("BIS_return".to_owned())
            ),
        ]
    );
    assert_eq!(m.player().map(|u| u.id), Some(3));
}

#[test]
fn sides_are_spelled_as_the_engine_spells_them() {
    assert_eq!(side_from_sqm("WEST"), Side::West);
    assert_eq!(side_from_sqm("EAST"), Side::East);
    assert_eq!(side_from_sqm("GUER"), Side::Independent);
    assert_eq!(side_from_sqm("CIV"), Side::Civilian);
    assert_eq!(side_from_sqm("LOGIC"), Side::Logic);
    assert_eq!(side_from_sqm("AMBIENT LIFE"), Side::AmbientLife);
    assert_eq!(side_from_sqm("empty"), Side::Empty);
    assert_eq!(side_from_sqm("nonsense"), Side::Unknown);
}

#[test]
fn a_config_without_class_mission_is_refused() {
    let err = Mission::parse(b"version=12;").unwrap_err();
    assert!(matches!(err, MissionError::NoMission));
}

#[test]
fn broken_text_is_a_parse_error() {
    let err = Mission::parse(b"class Mission {").unwrap_err();
    assert!(matches!(err, MissionError::Sqm(_)), "{err}");
}

#[test]
fn a_unit_without_a_class_or_position_is_refused() {
    let no_class =
        b"class Mission { class Vehicles { items=1; class Item0 { id=1; position[]={0,0,0}; }; }; };";
    assert!(matches!(
        Mission::parse(no_class).unwrap_err(),
        MissionError::NoClass { id: 1 }
    ));
    let no_position =
        b"class Mission { class Vehicles { items=1; class Item0 { id=2; vehicle=\"B_Soldier_F\"; }; }; };";
    assert!(matches!(
        Mission::parse(no_position).unwrap_err(),
        MissionError::NoPosition { id: 2, .. }
    ));
}

#[test]
fn a_rapified_sqm_parses_to_the_same_mission() {
    let text = parse_sqm(SQM.as_bytes()).unwrap();
    let rap = write_rap(&text);
    assert!(a3_config::is_rap(&rap));
    assert_eq!(parse_sqm(&rap).unwrap(), text);
    assert_eq!(Mission::parse(&rap).unwrap(), mission());
}
