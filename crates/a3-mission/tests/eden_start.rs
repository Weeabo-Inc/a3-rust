//! Starting a 3D-editor mission: after the init fields, each entity's attribute expressions run
//! with `_this` the entity and `_value` the attribute's value.

mod common;

use a3_gamedata::VfsHost;
use a3_mission::{MissionVmHost, StartOptions, load_mission, spawn_mission, start_mission};
use a3_sqf::Vm;

const FOLDER: &str = "missions\\eden.altis";

const SQM: &str = r#"version=53;
class Mission
{
	class Entities
	{
		items=1;
		class Item0
		{
			dataType="Group";
			side="West";
			class Entities
			{
				items=1;
				class Item0
				{
					dataType="Object";
					class PositionInfo { position[]={100,12,200}; };
					side="West";
					flags=6;
					class Attributes
					{
						name="BIS_unit";
						init="order = [""init""];";
					};
					id=1;
					type="B_Soldier_F";
					class CustomAttributes
					{
						class Attribute0
						{
							property="probe";
							expression="order pushBack 'attribute'; seen = [_this isEqualTo BIS_unit, _value];";
							class Value { class data { class type { type[]={"SCALAR"}; }; value=0.5; }; };
						};
						class Attribute1
						{
							property="list";
							expression="list = _value;";
							class Value
							{
								class data
								{
									class type { type[]={"ARRAY"}; };
									class value
									{
										items=2;
										class Item0 { class data { class type { type[]={"BOOL"}; }; value=1; }; };
										class Item1 { class data { class type { type[]={"STRING"}; }; value="x"; }; };
									};
								};
							};
						};
						nAttributes=2;
					};
				};
			};
			id=0;
		};
	};
};
"#;

#[test]
fn attribute_expressions_run_after_the_init_fields_with_this_and_value() {
    let (_dir, vfs) = common::mount(&[("mission.sqm", SQM)], FOLDER);
    let mut loader = Vm::new(VfsHost::new(vfs.clone()));
    let mission = load_mission(&vfs, FOLDER, &mut loader).expect("the fixture loads");
    let (mut world, mut types) = common::world_and_types();
    let spawned = spawn_mission(&mut world, &mut types, &mission);
    assert_eq!(spawned.units.len(), 1);

    let mut vm = MissionVmHost::new(world, types, VfsHost::new(vfs)).vm();
    let report = start_mission(&mut vm, &mission, &spawned, StartOptions::default());

    assert!(report.errors.is_empty(), "{:#?}", report.errors);
    assert_eq!(
        vm.get_global("order").to_sqf_string(),
        "[\"init\",\"attribute\"]"
    );
    assert_eq!(vm.get_global("seen").to_sqf_string(), "[true,0.5]");
    assert_eq!(vm.get_global("list").to_sqf_string(), "[true,\"x\"]");
}
