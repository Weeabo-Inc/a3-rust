//! `mission.sqm` as the 3D editor (Eden) saves it, `version=52..54`: one `class Entities` list
//! whose items say what they are with `dataType` ("Group", "Object", "Logic", "Marker",
//! "Trigger", "Waypoint", "Layer"), positions in `class PositionInfo`, editor attributes in
//! `class Attributes`. Half of the shipped scenarios use it.

use a3_mission::{AttributeValue, Mission, MissionError, side_from_sqm};
use a3_sqf::Side;

const SQM: &str = r#"version=53;
class EditorData { moveGridStep=1; };
binarizationWanted=0;
addons[]={"A3_Characters_F","A3_Soft_F"};
randomSeed=1234567;
class ScenarioData { author="Tester"; };
class Mission
{
	class Intel
	{
		briefingName="Eden test";
		year=2035;
		month=7;
		hour=5;
	};
	class Entities
	{
		items=6;
		class Item0
		{
			dataType="Marker";
			position[]={3881.5,20.5,5505.25};
			name="BIS_insertion";
			type="mil_start";
			colorName="ColorBLUFOR";
			id=262;
			atlOffset=-0.0014705658;
		};
		class Item1
		{
			dataType="Group";
			side="West";
			class Entities
			{
				items=3;
				class Item0
				{
					dataType="Object";
					class PositionInfo
					{
						position[]={100,10,200};
						angles[]={0,1.5707964,0};
					};
					side="West";
					flags=4;
					class Attributes
					{
						name="BIS_second";
						init="this allowDamage false;";
						skill=0.6;
						rank="SERGEANT";
					};
					id=7;
					type="B_Soldier_F";
				};
				class Item1
				{
					dataType="Object";
					class PositionInfo
					{
						position[]={110,12,220};
					};
					side="West";
					flags=6;
					class Attributes
					{
						name="BIS_boss";
						isPlayer=1;
					};
					id=8;
					type="B_Soldier_F";
					atlOffset=2.5;
				};
				class Item2
				{
					dataType="Waypoint";
					position[]={500,0,600};
					type="Move";
					showWP="NEVER";
					id=9;
				};
			};
			class Attributes { };
			id=6;
		};
		class Item2
		{
			dataType="Trigger";
			position[]={300,5,400};
			class Attributes
			{
				name="BIS_trg";
				text="Send QRF";
				condition="this && BIS_ready";
				onActivation="hint ""go"";";
				onDeactivation="hint ""stop"";";
				sizeA=50;
				sizeB=30;
				activationBy="WEST";
				activationType="PRESENT";
				repeatable=1;
				type="SWITCH";
			};
			angle=0.5;
			id=10;
			type="EmptyDetector";
		};
		class Item3
		{
			dataType="Layer";
			name="Empty vehicles";
			class Entities
			{
				items=2;
				class Item0
				{
					dataType="Object";
					class PositionInfo
					{
						position[]={700,3,800};
						angles[]={0,3.1415927,0};
					};
					side="Empty";
					class Attributes { name="BIS_car"; };
					id=11;
					type="B_MRAP_01_F";
				};
				class Item1
				{
					dataType="Logic";
					class PositionInfo
					{
						position[]={710,3,810};
					};
					name="BIS_logic";
					init="BIS_logicReady = true;";
					id=12;
					type="Logic";
				};
			};
			id=13;
		};
		class Item4
		{
			dataType="Logic";
			class PositionInfo { position[]={720,3,820}; };
			presenceCondition="false";
			id=14;
			type="ModuleDoorOpen_F";
		};
		class Item5
		{
			dataType="Comment";
			position[]={1,2,3};
			title="A note";
			id=15;
		};
	};
};
"#;

fn mission() -> Mission {
    Mission::parse(SQM.as_bytes()).expect("the Eden fixture parses")
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn the_header_comes_from_the_root_and_the_intel_from_class_mission() {
    let m = mission();
    assert_eq!(m.version, 53);
    assert_eq!(m.random_seed, 1234567);
    assert_eq!(m.addons, ["A3_Characters_F", "A3_Soft_F"]);
    assert_eq!(m.intel.briefing_name.as_deref(), Some("Eden test"));
    assert_eq!(m.intel.year, Some(2035));
}

#[test]
fn groups_hold_their_objects_and_waypoints() {
    let m = mission();
    assert_eq!(m.groups.len(), 1);
    let g = &m.groups[0];
    assert_eq!(g.side, Side::West);
    assert_eq!(g.units.len(), 2);
    assert_eq!(g.waypoints.len(), 1);
    assert_eq!(g.waypoints[0].kind.as_deref(), Some("Move"));
    assert!(near(g.waypoints[0].position.x, 500.0) && near(g.waypoints[0].position.z, 600.0));

    let second = &g.units[0];
    assert_eq!((second.id, second.class.as_str()), (7, "B_Soldier_F"));
    assert_eq!(second.text.as_deref(), Some("BIS_second"));
    assert_eq!(second.init.as_deref(), Some("this allowDamage false;"));
    assert_eq!(second.rank.as_deref(), Some("SERGEANT"));
    assert!(second.skill.is_some_and(|s| near(s, 0.6)));
    // PositionInfo: {x, height ASL of the surface, y}; world space is (x, height, y).
    assert!(near(second.position.x, 100.0));
    assert!(near(second.position.y, 10.0));
    assert!(near(second.position.z, 200.0));
    assert!(!second.on_surface);
    // `angles[1]` is the heading in radians.
    assert!(second.azimut.is_some_and(|a| near(a, 90.0)));
    assert!(!second.leader);
    assert!(second.player.is_none());
}

#[test]
fn the_player_the_leader_and_the_height_above_the_surface() {
    let m = mission();
    let boss = &m.groups[0].units[1];
    assert!(boss.player.is_some(), "Attributes isPlayer=1");
    assert!(boss.leader, "flags bit 2");
    // atlOffset lifts the object above the surface height in position[].
    assert!(near(boss.position.y, 14.5));
    assert_eq!(m.player().map(|u| u.id), Some(8));
}

#[test]
fn objects_and_logics_outside_groups_come_out_of_layers_too() {
    let m = mission();
    let classes: Vec<&str> = m.objects.iter().map(|u| u.class.as_str()).collect();
    assert_eq!(classes, ["B_MRAP_01_F", "Logic", "ModuleDoorOpen_F"]);
    let car = &m.objects[0];
    assert_eq!(car.text.as_deref(), Some("BIS_car"));
    assert!(car.azimut.is_some_and(|a| near(a, 180.0)));
    // A Logic keeps its name and init on the item itself.
    let logic = &m.objects[1];
    assert_eq!(logic.text.as_deref(), Some("BIS_logic"));
    assert_eq!(logic.init.as_deref(), Some("BIS_logicReady = true;"));
    // `presenceCondition="false"` is never present.
    assert!(!logic.is_absent());
    assert!(m.objects[2].is_absent());
}

#[test]
fn markers_and_triggers() {
    let m = mission();
    assert_eq!(m.markers.len(), 1);
    assert_eq!(m.markers[0].name, "BIS_insertion");
    assert_eq!(m.markers[0].marker_type.as_deref(), Some("mil_start"));

    assert_eq!(m.triggers.len(), 1);
    let t = &m.triggers[0];
    assert_eq!(t.name.as_deref(), Some("BIS_trg"));
    assert_eq!(t.exp_cond.as_deref(), Some("this && BIS_ready"));
    assert_eq!(t.exp_activ.as_deref(), Some("hint \"go\";"));
    assert_eq!(t.exp_desactiv.as_deref(), Some("hint \"stop\";"));
    assert_eq!((t.a, t.b), (Some(50.0), Some(30.0)));
    assert_eq!(t.activation_by.as_deref(), Some("WEST"));
    assert_eq!(t.kind.as_deref(), Some("SWITCH"));
    assert!(t.repeating);
    assert!(near(t.position.x, 300.0) && near(t.position.z, 400.0));
}

#[test]
fn eden_spells_sides_in_words() {
    assert_eq!(side_from_sqm("West"), Side::West);
    assert_eq!(side_from_sqm("Independent"), Side::Independent);
    assert_eq!(side_from_sqm("Civilian"), Side::Civilian);
}

#[test]
fn a_scene_with_only_an_intro_loads_its_intro() {
    let sqm = r#"version=53;
class Intro
{
	class Intel { briefingName="Scene"; };
	class Entities
	{
		items=1;
		class Item0
		{
			dataType="Object";
			class PositionInfo { position[]={1,2,3}; };
			side="Empty";
			id=0;
			type="Land_Camera_01_F";
		};
	};
};
"#;
    let m = Mission::parse(sqm.as_bytes()).expect("an intro-only scene loads");
    assert_eq!(m.objects.len(), 1);
    assert_eq!(m.intel.briefing_name.as_deref(), Some("Scene"));
    assert!(matches!(
        Mission::parse(b"version=53;").unwrap_err(),
        MissionError::NoMission
    ));
}

/// `class CustomAttributes` holds the entity's editor attributes: SQF `expression`s run at
/// mission start with `_this` the entity and `_value` the typed `class Value >> class data`.
#[test]
fn custom_attributes_carry_their_expression_and_typed_value() {
    let sqm = r##"version=53;
class Mission
{
	class Entities
	{
		items=1;
		class Item0
		{
			dataType="Object";
			class PositionInfo { position[]={1,2,3}; };
			side="Empty";
			id=0;
			type="Land_Box_F";
			class CustomAttributes
			{
				class Attribute0
				{
					property="allowDamage";
					expression="_this allowdamage _value;";
					class Value { class data { class type { type[]={"BOOL"}; }; value=0; }; };
				};
				class Attribute1
				{
					property="pitch";
					expression="_this setpitch _value;";
					class Value { class data { class type { type[]={"SCALAR"}; }; value=0.95; }; };
				};
				class Attribute2
				{
					property="speaker";
					expression="_this setspeaker _value;";
					class Value { class data { class type { type[]={"STRING"}; }; value="Male07ENG"; }; };
				};
				class Attribute3
				{
					property="#doors";
					expression="_this setVariable ['doors', _value];";
					class Value
					{
						class data
						{
							class type { type[]={"ARRAY"}; };
							class value
							{
								items=2;
								class Item0 { class data { class type { type[]={"ARRAY"}; }; }; };
								class Item1 { class data { class type { type[]={"STRING"}; }; value="Door1"; }; };
							};
						};
					};
				};
				nAttributes=4;
			};
		};
	};
};
"##;
    let m = Mission::parse(sqm.as_bytes()).expect("parses");
    let attributes = &m.objects[0].attributes;
    let properties: Vec<&str> = attributes.iter().map(|a| a.property.as_str()).collect();
    assert_eq!(properties, ["allowDamage", "pitch", "speaker", "#doors"]);
    assert_eq!(attributes[0].expression, "_this allowdamage _value;");
    assert_eq!(attributes[0].value, AttributeValue::Bool(false));
    assert!(matches!(attributes[1].value, AttributeValue::Number(v) if near(v, 0.95)));
    assert_eq!(
        attributes[2].value,
        AttributeValue::String("Male07ENG".into())
    );
    assert_eq!(
        attributes[3].value,
        AttributeValue::Array(vec![
            AttributeValue::Array(vec![]),
            AttributeValue::String("Door1".into())
        ])
    );
}

/// Crew the editor put into a vehicle has no `position[]` of its own (`faction_blufor.altis`:
/// the AI of a UAV turret): `class CrewLinks` links it to its vehicle, whose position it takes.
#[test]
fn crew_without_a_position_takes_its_vehicles() {
    let sqm = r#"version=53;
class Mission
{
	class Entities
	{
		items=2;
		class Item0
		{
			dataType="Object";
			class PositionInfo { position[]={100,20,200}; };
			side="Empty";
			id=5;
			type="B_UAV_01_F";
		};
		class Item1
		{
			dataType="Group";
			side="West";
			class Entities
			{
				items=1;
				class Item0
				{
					dataType="Object";
					class PositionInfo { angles[]={0,0.8,0}; };
					side="West";
					flags=2;
					id=6;
					type="B_UAV_AI";
					atlOffset=185;
				};
			};
			class CrewLinks
			{
				class Links
				{
					items=1;
					class Item0 { linkID=0; item0=6; item1=5; class CustomData { role=1; }; };
				};
			};
			id=7;
		};
	};
};
"#;
    let m = Mission::parse(sqm.as_bytes()).expect("crew without a position loads");
    let crew = &m.groups[0].units[0];
    assert_eq!(crew.position, glam::DVec3::new(100.0, 20.0, 200.0));
}
