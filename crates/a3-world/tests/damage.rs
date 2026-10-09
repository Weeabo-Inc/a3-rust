//! Hit points, damage, destruction and ruins (`docs/re/sim-damage.md`, ADR 0006).
//!
//! The config, the terrain and the objects are synthetic, so every test runs everywhere; the
//! shipped config is only read in the `A3_ROOT` test at the bottom.

use std::sync::{Arc, Mutex};

use a3_config::{ConfigTree, parse_text};
use a3_world::{
    ClientId, Create, DamageHandler, DamageHit, DamageReply, DamageRequest, DamageState, Depends,
    DestructionType, EntityId, EntityType, NetworkId, ObjectRef, SimulationClass, TypeBank, World,
    WorldEvent,
};
use a3_wrp::{TerrainBuilder, Transform};
use glam::{DVec3, Vec3};

/// A synthetic config with four damage-carrying classes: a breakable house (a ruin whose class
/// the config names, a `depends` hit point, a negative armor), a house whose ruin model has no
/// class at all, a soldier (fatal `HitHead`/`HitBody`, a `Total` dependency), a car, the ruin
/// class itself, and a tree in `CfgNonAIVehicles`.
const CONFIG: &str = r#"
class CfgVehicles {
    class All { scope = 0; simulation = ""; };
    class House: All { simulation = "house"; armor = 20; armorStructural = 4; };
    class Land_House_Damage_F: House {
        scope = 2;
        model = "\A3\Structures_F\House_Damage_F.p3d";
        destrType = "DestructBuilding";
        class HitPoints {
            class HitWall { armor = 1.5; name = "wall"; radius = 0.3; passThrough = 0.6; material = 51; };
            class HitRoof { armor = -6; name = "roof"; depends = "HitWall max 0.25"; visual = "injury_roof"; };
        };
        class DestructionEffects {
            class Ruin1 { simulation = "ruin"; type = "\A3\Structures_F\House_Damage_ruins_F.p3d"; };
            class Smoke1 { simulation = "smoke"; type = "SmokeShell"; };
        };
    };
    class Land_House_Damage_ruins_F: House {
        scope = 2;
        model = "\A3\Structures_F\House_Damage_ruins_F.p3d";
        class HitPoints { class HitRuin { armor = 1; name = "ruin"; }; };
    };
    class Land_House_Fallback_F: House {
        scope = 2;
        model = "\A3\Structures_F\House_Fallback_F.p3d";
        class DestructionEffects {
            class Ruin1 { simulation = "ruin"; type = "\A3\Structures_F\House_Fallback_ruins_F.p3d"; };
        };
    };
    class Man_sim: All { simulation = "soldier"; };
    class B_Soldier_Damage_F: Man_sim {
        scope = 2;
        model = "\A3\Characters_F\soldier.p3d";
        class HitPoints {
            class HitHead { armor = 1; name = "head"; };
            class HitBody { armor = 1; name = "body"; depends = "Total"; };
            class HitFace { armor = 1; name = "face"; depends = "HitHead"; };
        };
    };
    class Car_sim: All { simulation = "carx"; };
    class C_Car_Damage_F: Car_sim {
        scope = 2;
        class HitPoints {
            class HitFuel { armor = 1; name = "fueltank"; };
            class HitHull { armor = 1; name = "hull"; };
            class HitEngine { armor = 1; name = "engine"; };
        };
    };
};
class CfgNonAIVehicles {
    class T_Pinus_F {
        scope = 2;
        simulation = "thing";
        model = "\A3\Plants_F\Tree\t_pinus.p3d";
    };
};
"#;

/// The Static objects of the terrain: the house's model at (120, 0, 160), a pine at (10, 0, 10)
/// whose class lives in `CfgNonAIVehicles`, and a rock at (30, 0, 30) no class uses.
fn terrain() -> a3_wrp::Terrain {
    let at = |x, z| Transform::from_position(Vec3::new(x, 0.0, z));
    TerrainBuilder::new(4, 8, 50.0)
        .object(r"a3\structures_f\house_damage_f.p3d", at(120.0, 160.0))
        .object(r"a3\plants_f\tree\t_pinus.p3d", at(10.0, 10.0))
        .object(r"a3\rocks_f\stone.p3d", at(30.0, 30.0))
        .build()
}

fn bank() -> TypeBank {
    let config = parse_text(CONFIG).unwrap();
    TypeBank::new(Arc::new(ConfigTree::from_config(&config)))
}

/// A World over the synthetic terrain with a resolver installed: a Static object changed by a
/// hit becomes an Entity of its config class, and a ruin finds its class by model path.
fn world(client: ClientId) -> World {
    let mut world = World::new(client);
    world.load_terrain(Arc::new(terrain())).unwrap();
    world.set_model_type_resolver(Some(Box::new(bank())));
    world
}

/// The same World with no resolver (nothing resolves model paths to config classes).
fn world_without_resolver(client: ClientId) -> World {
    let mut world = World::new(client);
    world.load_terrain(Arc::new(terrain())).unwrap();
    world
}

fn create(world: &mut World, class: &str) -> EntityId {
    let ty = bank().get(class).unwrap();
    world.create(Create::new(ty, DVec3::ZERO)).unwrap()
}

/// The type of a class, as the damage code sees it.
fn damage_of(types: &mut TypeBank, class: &str) -> a3_world::DamageModel {
    types.get(class).unwrap().damage().clone()
}

/// The hit point names of a model, as plain strings.
fn names(model: &a3_world::DamageModel) -> Vec<&str> {
    model.hit_points().iter().map(|p| p.name.as_str()).collect()
}

/// A model with no hit points, for the total-only behaviour.
fn total_only_model() -> a3_world::DamageModel {
    a3_world::DamageModel::new()
}

/// The Static key of terrain object `object_id`, from the house at the origin outwards.
fn static_key(world: &World, object_id: u32) -> a3_world::StaticKey {
    match world.find_static(DVec3::ZERO, object_id) {
        Some(ObjectRef::Static(key)) => key,
        other => panic!("no static object {object_id}: {other:?}"),
    }
}

// ---------------------------------------------------------------- the damage model

#[test]
fn a_house_type_reads_its_hit_points_armor_and_ruin_from_config() {
    let mut types = bank();
    let ty = types.get("Land_House_Damage_F").unwrap();
    let model = ty.damage();

    assert_eq!(names(model), ["HitWall", "HitRoof"]);
    let wall = &model.hit_points()[0];
    assert_eq!(wall.selection, "wall");
    assert_eq!(wall.armor, 1.5);
    assert_eq!(wall.radius, 0.3);
    assert_eq!(wall.pass_through, 0.6);
    assert_eq!(wall.material, 51);
    let roof = &model.hit_points()[1];
    assert_eq!(roof.visual, "injury_roof");
    assert_eq!(roof.radius, -1.0, "the default radius");
    assert_eq!(roof.pass_through, 1.0, "the default passThrough");
    assert_eq!(roof.minimal_hit, 0.01, "the default minimalHit");
    assert_eq!(
        roof.depends.as_ref().map(Depends::source),
        Some("HitWall max 0.25")
    );
    assert_eq!(model.armor(), 20.0);
    assert_eq!(model.armor_structural(), 4.0);
    assert_eq!(*model.destruction(), DestructionType::Building);
    assert_eq!(model.ruins().len(), 1, "only the ruin entry counts");
    assert_eq!(
        model.ruins()[0].model,
        r"\A3\Structures_F\House_Damage_ruins_F.p3d"
    );
    assert_eq!(model.ruins()[0].class_name(), "Land_House_Damage_ruins_F");
    let ruin_class = types.class_of_model(&model.ruins()[0].model);
    assert_eq!(ruin_class.as_deref(), Some("Land_House_Damage_ruins_F"));
}

#[test]
fn a_type_without_damage_config_has_the_defaults() {
    let ty = EntityType::new("Thing", SimulationClass::Thing);
    let model = ty.damage();

    assert!(model.hit_points().is_empty());
    assert!(model.ruins().is_empty());
    assert_eq!(model.armor(), 30.0);
    assert_eq!(model.min_total_damage_threshold(), 0.001);
    assert_eq!(model.explosion_shielding(), 1.0);
    assert_eq!(*model.destruction(), DestructionType::Default);
}

#[test]
fn hit_point_armor_is_relative_to_the_type_unless_it_is_negative() {
    let mut types = bank();
    let model = damage_of(&mut types, "Land_House_Damage_F");

    assert_eq!(model.armor_of(0), 30.0, "1.5 x the type's 20");
    assert_eq!(model.armor_of(1), 6.0, "-6 is absolute");
    assert_eq!(model.armor_of(9), 0.0, "no such hit point");
}

#[test]
fn soldiers_get_fatal_head_and_body_hit_points_and_vehicles_hull_and_fuel() {
    let mut types = bank();

    let soldier = damage_of(&mut types, "B_Soldier_Damage_F");
    let fatal: Vec<(usize, f32)> = soldier
        .fatal_hit_points()
        .iter()
        .map(|f| (f.index, f.threshold))
        .collect();
    assert_eq!(fatal, [(0, 1.0), (1, 1.0)], "HitHead, HitBody");

    let car = damage_of(&mut types, "C_Car_Damage_F");
    let fatal: Vec<&str> = car
        .fatal_hit_points()
        .iter()
        .map(|f| car.hit_points()[f.index].name.as_str())
        .collect();
    assert_eq!(fatal, ["HitHull", "HitFuel"], "the engine's check order");
    assert!(car.fatal_hit_points().iter().all(|f| f.threshold == 1.0));

    let house = damage_of(&mut types, "Land_House_Damage_F");
    assert!(
        house.fatal_hit_points().is_empty(),
        "no known fatal hit point"
    );
}

#[test]
fn hit_points_are_found_by_name_and_by_selection_case_insensitively() {
    let mut types = bank();
    let model = damage_of(&mut types, "Land_House_Damage_F");

    assert_eq!(model.hit_point_index("hitwall"), Some(0));
    assert_eq!(model.hit_point_index_of_selection("WALL"), Some(0));
    assert_eq!(model.hit_point_index_of_selection("HitWall"), None);
    assert_eq!(model.hit_point_index("Nope"), None);
}

#[test]
fn depends_expressions_parse_and_evaluate() {
    // A binary `max` chain, as `B_Soldier_F`'s `HitHead` reads it.
    let depends = Depends::parse("HitFace max HitNeck").unwrap();
    assert_eq!(depends.source(), "HitFace max HitNeck");
    let read: Vec<&str> = depends.names().iter().map(String::as_str).collect();
    assert_eq!(read, ["HitFace", "HitNeck"]);
    let lookup = |name: &str| match name {
        "HitFace" => 0.3,
        "HitNeck" => 0.7,
        _ => 0.0,
    };
    assert!((depends.eval(lookup) - 0.7).abs() < 1e-6);

    // A four-name chain: `HitBody` is the worst of the torso hit points.
    let depends = Depends::parse("HitPelvis max HitAbdomen max HitDiaphragm max HitChest").unwrap();
    assert_eq!(depends.names().len(), 4);
    let eval = depends.eval(|name| if name == "HitDiaphragm" { 0.4 } else { 0.1 });
    assert!((eval - 0.4).abs() < 1e-6);

    // The downed state: parentheses, `Total`, `max 0` clamping and a scale.
    let depends =
        Depends::parse("(((Total - 0.25) max 0) + ((HitHead - 0.25) max 0)) * 2").unwrap();
    let read: Vec<&str> = depends.names().iter().map(String::as_str).collect();
    assert_eq!(read, ["Total", "HitHead"]);
    let eval = depends.eval(|name| if name == "Total" { 0.5 } else { 0.1 });
    assert!(
        (eval - 0.5).abs() < 1e-6,
        "the head's excess is clamped away"
    );
    assert_eq!(depends.eval(|_| 0.1), 0.0, "nothing above the threshold");

    // A sum scaled by a number, as the twin fuel tanks read it.
    let depends = Depends::parse("(HitFuelL + HitFuelR)*0.5").unwrap();
    let eval = depends.eval(|name| if name == "HitFuelL" { 0.3 } else { 0.5 });
    assert!((eval - 0.4).abs() < 1e-6);
}

#[test]
fn an_empty_zero_or_malformed_depends_expression_is_no_dependency() {
    assert!(Depends::parse("").is_none());
    assert!(Depends::parse(" 0 ").is_none());
    assert!(Depends::parse("Total +").is_none());
    assert!(Depends::parse("HitWall $ 2").is_none());
    assert!(Depends::parse("(HitWall").is_none());
    // The config expression language has no function calls: `max`/`min` are binary. No shipped
    // config uses a comma, so an expression with one is treated as no dependency (a dropped
    // dependency, never a wrong value).
    assert!(Depends::parse("max(HitBody, 0.2)").is_none());
}

#[test]
fn destruction_types_parse_their_config_values() {
    assert_eq!(
        DestructionType::from_config("DestructBuilding"),
        DestructionType::Building
    );
    assert_eq!(
        DestructionType::from_config("destructno"),
        DestructionType::No
    );
    assert_eq!(
        DestructionType::from_config("DestructBush"),
        DestructionType::Tree
    );
    assert_eq!(DestructionType::from_config(""), DestructionType::Default);
    assert_eq!(
        DestructionType::from_config("DestructModThing"),
        DestructionType::Other("DestructModThing".to_owned())
    );
}

// ---------------------------------------------------------------- the damage state

#[test]
fn the_total_ignores_tiny_changes_and_is_capped_at_a_thousand() {
    let mut state = DamageState::new(&total_only_model());

    assert!(state.set_total(0.5));
    assert!(!state.set_total(0.5 + 1e-9), "inside the dead band");
    assert_eq!(state.total(), 0.5);
    assert!(state.set_total(5000.0));
    assert_eq!(state.stored_total(), 1000.0);
    assert_eq!(state.total(), 1.0, "scripts read 0..1");
    assert!(state.set_total(-3.0));
    assert_eq!(state.stored_total(), 0.0);
    assert!(!state.set_total(f32::NAN));
}

#[test]
fn hit_point_damage_is_clamped_to_zero_and_one() {
    let mut types = bank();
    let model = damage_of(&mut types, "Land_House_Damage_F");
    let mut state = DamageState::new(&model);

    assert_eq!(state.hit_points().len(), 2);
    assert!(state.set_hit_point(0, 1.7));
    assert_eq!(state.hit_point(0), 1.0);
    assert!(state.set_hit_point(0, -2.0));
    assert_eq!(state.hit_point(0), 0.0);
    assert!(state.add_hit_point(0, 0.4));
    assert_eq!(state.hit_point(0), 0.4);
    assert!(!state.set_hit_point(7, 1.0), "no such hit point");
    assert_eq!(state.hit_point(7), 0.0);
}

#[test]
fn an_entity_is_destroyed_by_its_total_or_by_a_fatal_hit_point() {
    let mut types = bank();
    let man = damage_of(&mut types, "B_Soldier_Damage_F");
    let mut state = DamageState::new(&man);

    assert!(!state.is_destroyed(&man));
    state.set_total(0.99);
    assert!(!state.is_destroyed(&man));
    state.set_total(1.0);
    assert!(state.is_destroyed(&man));

    let mut state = DamageState::new(&man);
    state.set_hit_point(0, 1.0); // HitHead
    assert!(state.is_destroyed(&man));
    state.set_hit_point(0, 0.0);
    state.set_hit_point(2, 1.0); // HitFace is not fatal on its own
    assert!(!state.is_destroyed(&man));

    // The house has no fatal hit point: only its total destroys it.
    let house = damage_of(&mut types, "Land_House_Damage_F");
    let mut state = DamageState::new(&house);
    state.set_hit_point(0, 1.0);
    assert!(!state.is_destroyed(&house));
}

#[test]
fn depends_hit_points_are_recomputed_from_the_hit_points_they_read() {
    let mut types = bank();
    let man = damage_of(&mut types, "B_Soldier_Damage_F");
    let mut state = DamageState::new(&man);

    // HitFace depends on HitHead: damaging the head fills the face too.
    state.set_hit_point(0, 0.7);
    state.recompute_depends(&man, &[0], false);
    assert_eq!(state.hit_point(2), 0.7);
    assert_eq!(state.hit_point(1), 0.0, "Total did not change");

    // HitBody depends on Total.
    state.set_total(0.5);
    state.recompute_depends(&man, &[], true);
    assert_eq!(state.hit_point(1), 0.5);

    // A hit point that reads a name which did not change is left alone.
    state.set_hit_point(1, 0.1);
    state.recompute_depends(&man, &[1], false);
    assert_eq!(state.hit_point(1), 0.1);
}

#[test]
fn a_direct_write_to_a_dependent_hit_point_wins_over_recomputation() {
    let mut types = bank();
    let man = damage_of(&mut types, "B_Soldier_Damage_F");
    let mut state = DamageState::new(&man);
    state.set_hit_point(0, 1.0);

    // `setHitPointDamage ["HitFace", 0.1]`: the written hit point is not recomputed away.
    state.set_hit_point(2, 0.1);
    state.recompute_depends(&man, &[0, 2], false);
    assert_eq!(state.hit_point(2), 0.1);

    // Without a direct write the chain runs in config order: HitFace follows HitHead.
    let mut state = DamageState::new(&man);
    state.set_hit_point(0, 1.0);
    state.recompute_depends(&man, &[0], false);
    assert_eq!(state.hit_point(2), 1.0);
}

// ---------------------------------------------------------------- applying hits

#[test]
fn an_engine_hit_adds_to_the_total_and_to_the_hit_points_and_records_dammaged() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "Land_House_Damage_F");
    world.drain_events();

    let outcome = world
        .apply_damage_to(id, DamageHit::added(0.25).at(0, 0.5))
        .unwrap();

    assert_eq!(outcome.damage, 0.25);
    assert_eq!(
        outcome.hit_points,
        [(0, 0.5), (1, 0.5)],
        "HitRoof depends on HitWall, so it followed"
    );
    assert!(!outcome.destroyed && !outcome.ruined);
    assert_eq!(world.entity(id).unwrap().damage(), 0.25);
    assert_eq!(
        world.hit_point_damage(ObjectRef::Entity(id), "HitRoof"),
        Some(0.5)
    );
    assert_eq!(
        world.drain_events(),
        [
            WorldEvent::Dammaged {
                entity: id,
                hit_point: Some(0),
                damage: 0.5,
                source: None,
            },
            WorldEvent::Dammaged {
                entity: id,
                hit_point: Some(1),
                damage: 0.5,
                source: None,
            },
        ]
    );
}

#[test]
fn an_engine_hit_that_changes_only_the_total_is_reported_as_a_total_change() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");
    world.drain_events();

    let outcome = world.apply_damage_to(id, DamageHit::added(0.4)).unwrap();

    assert_eq!(outcome.damage, 0.4);
    assert_eq!(
        world.drain_events(),
        [WorldEvent::Dammaged {
            entity: id,
            hit_point: None,
            damage: 0.4,
            source: None,
        }]
    );
}

#[test]
fn a_scripted_set_of_the_total_records_no_dammaged() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");
    world.drain_events();

    world
        .apply_damage_to(id, DamageHit::total(0.5).scripted())
        .unwrap();

    assert_eq!(world.entity(id).unwrap().damage(), 0.5);
    // `setDamage` fires Killed, not Dammaged; nothing here was destroyed either.
    assert!(world.drain_events().is_empty());
}

#[test]
fn the_replace_mode_sets_the_total_and_the_add_mode_adds_to_it() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");

    world.apply_damage_to(id, DamageHit::total(0.5)).unwrap();
    world.apply_damage_to(id, DamageHit::added(0.25)).unwrap();
    assert_eq!(world.entity(id).unwrap().damage(), 0.75);
    world.apply_damage_to(id, DamageHit::total(0.1)).unwrap();
    assert_eq!(world.entity(id).unwrap().damage(), 0.1);
}

#[test]
fn allow_damage_gates_engine_hits_but_not_scripted_ones() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");

    assert!(world.set_damage_allowed(ObjectRef::Entity(id), false));
    assert!(world.apply_damage_to(id, DamageHit::added(0.5)).is_none());
    assert_eq!(world.entity(id).unwrap().damage(), 0.0);

    world
        .apply_damage_to(id, DamageHit::total(0.5).scripted())
        .unwrap();
    assert_eq!(world.entity(id).unwrap().damage(), 0.5);

    assert!(world.set_damage_allowed(ObjectRef::Entity(id), true));
    world.apply_damage_to(id, DamageHit::added(0.25)).unwrap();
    assert_eq!(world.entity(id).unwrap().damage(), 0.75);
}

#[test]
fn a_destroyed_entity_takes_no_more_engine_damage_but_scripts_can_restore_it() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");

    world
        .apply_damage_to(id, DamageHit::total(1.0).scripted())
        .unwrap();
    assert!(world.entity(id).unwrap().is_destroyed());
    assert!(world.apply_damage_to(id, DamageHit::added(0.5)).is_none());

    world
        .apply_damage_to(id, DamageHit::total(0.0).scripted())
        .unwrap();
    assert!(!world.entity(id).unwrap().is_destroyed());
    assert!(world.entity(id).unwrap().is_alive());
}

#[test]
fn a_remote_entity_takes_no_damage_on_this_machine() {
    let mut world = world(ClientId(5000));
    let ty = bank().get("C_Car_Damage_F").unwrap();
    let id = world
        .spawn_remote(ty, DVec3::ZERO, NetworkId::new(2, 7), Some(ClientId(2)))
        .unwrap();

    assert!(world.apply_damage_to(id, DamageHit::added(0.5)).is_none());
    assert!(
        world
            .apply_damage_to(id, DamageHit::total(0.5).scripted())
            .is_none()
    );
}

// ---------------------------------------------------------------- the handler

#[derive(Debug, Clone)]
struct Recorder {
    seen: Arc<Mutex<Vec<DamageRequest>>>,
    halve: bool,
}

impl DamageHandler for Recorder {
    fn handle_damage(&mut self, _world: &World, request: &DamageRequest) -> DamageReply {
        self.seen.lock().unwrap().push(request.clone());
        if !self.halve {
            return DamageReply::allow();
        }
        DamageReply {
            damage: Some(request.damage / 2.0),
            hit_points: request
                .hit_points
                .iter()
                .map(|&(index, damage)| (index, damage / 2.0))
                .collect(),
        }
    }
}

#[test]
fn a_handler_sees_an_engine_hit_and_can_replace_the_values_it_stores() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");
    let seen = Arc::new(Mutex::new(Vec::new()));
    world.set_damage_handler(Some(Box::new(Recorder {
        seen: seen.clone(),
        halve: true,
    })));

    let outcome = world
        .apply_damage_to(id, DamageHit::added(0.4).at(1, 0.6).caused_by(id))
        .unwrap();

    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].entity, id);
    assert_eq!(requests[0].damage, 0.4);
    assert_eq!(requests[0].hit_points, [(1, 0.6)]);
    assert_eq!(requests[0].source, Some(id));
    assert_eq!(requests[0].context, a3_world::DamageContext::HitPoint);
    drop(requests);

    assert_eq!(outcome.damage, 0.2, "the handler halved it");
    assert_eq!(outcome.hit_points, [(1, 0.3)]);
    assert_eq!(
        world.hit_point_damage(ObjectRef::Entity(id), "HitHull"),
        Some(0.3)
    );
}

#[test]
fn a_scripted_hit_bypasses_the_handler() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");
    let seen = Arc::new(Mutex::new(Vec::new()));
    world.set_damage_handler(Some(Box::new(Recorder {
        seen: seen.clone(),
        halve: true,
    })));

    world
        .apply_damage_to(id, DamageHit::total(0.5).scripted())
        .unwrap();

    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(world.entity(id).unwrap().damage(), 0.5);
}

#[test]
fn a_handler_that_allows_a_hit_leaves_what_it_computed() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");
    world.set_damage_handler(Some(Box::new(Recorder {
        seen: Arc::new(Mutex::new(Vec::new())),
        halve: false,
    })));

    world.apply_damage_to(id, DamageHit::added(0.4)).unwrap();

    assert_eq!(world.entity(id).unwrap().damage(), 0.4);
}

// ---------------------------------------------------------------- destruction and ruins

#[test]
fn a_destroyed_house_becomes_its_ruin_and_records_killed() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "Land_House_Damage_F");
    world.drain_events();

    let outcome = world
        .apply_damage_to(id, DamageHit::total(1.0).scripted().caused_by(id))
        .unwrap();

    assert!(outcome.destroyed);
    assert!(outcome.ruined);
    let entity = world.entity(id).unwrap();
    assert_eq!(entity.type_name(), "Land_House_Damage_ruins_F");
    assert_eq!(entity.class(), SimulationClass::House, "the class is kept");
    assert_eq!(
        entity.entity_type().model(),
        r"\A3\Structures_F\House_Damage_ruins_F.p3d"
    );
    assert_eq!(entity.damage(), 1.0, "the ruin is still destroyed");
    assert!(!entity.is_alive());
    // The ruin's own class resolved by model path, so it brings its own hit points.
    assert_eq!(
        names(entity.entity_type().damage()),
        ["HitRuin"],
        "the ruin's config class, not a plain fallback"
    );
    assert_eq!(
        world.hit_point_damage(ObjectRef::Entity(id), "HitRuin"),
        Some(0.0),
        "the ruin starts undamaged apart from its total"
    );
    assert_eq!(
        world.drain_events(),
        [WorldEvent::Killed {
            entity: id,
            killer: Some(id),
            instigator: None,
            use_effects: true,
        }]
    );

    // A second hit does not fire Killed again.
    assert!(world.apply_damage_to(id, DamageHit::added(1.0)).is_none());
    assert!(world.drain_events().is_empty());
}

#[test]
fn a_ruin_model_no_class_uses_gets_the_plain_land_type() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "Land_House_Fallback_F");

    let outcome = world
        .apply_damage_to(id, DamageHit::total(1.0).scripted())
        .unwrap();
    assert!(outcome.ruined);

    let entity = world.entity(id).unwrap();
    assert_eq!(
        entity.type_name(),
        "Land_House_Fallback_ruins_F",
        "`Land_` + the model file stem (docs/re/sim-damage.md §7.2)"
    );
    assert_eq!(entity.class(), SimulationClass::House);
    assert!(entity.damage_state().hit_points().is_empty());
}

#[test]
fn destruction_without_effects_leaves_the_destroyed_object_as_it_is() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "Land_House_Damage_F");
    world.drain_events();

    let outcome = world
        .apply_damage_to(id, DamageHit::total(1.0).scripted().without_effects())
        .unwrap();

    assert!(outcome.destroyed && !outcome.ruined);
    assert_eq!(world.entity(id).unwrap().type_name(), "Land_House_Damage_F");
    assert_eq!(
        world.drain_events(),
        [WorldEvent::Killed {
            entity: id,
            killer: None,
            instigator: None,
            use_effects: false,
        }]
    );
}

#[test]
fn a_type_without_a_ruin_entry_keeps_the_destroyed_entity() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "C_Car_Damage_F");

    let outcome = world
        .apply_damage_to(id, DamageHit::total(1.0).scripted())
        .unwrap();

    assert!(outcome.destroyed && !outcome.ruined);
    assert_eq!(world.entity(id).unwrap().type_name(), "C_Car_Damage_F");
}

// ---------------------------------------------------------------- Static objects

#[test]
fn a_hit_on_a_static_object_promotes_it_with_its_config_type() {
    let mut world = world(ClientId::SERVER);
    let key = static_key(&world, 0);

    let outcome = world
        .apply_damage(ObjectRef::Static(key), DamageHit::added(0.4).at(0, 0.5))
        .unwrap();

    let id = match world.resolve(key.network_id()) {
        Some(ObjectRef::Entity(id)) => id,
        other => panic!("not promoted: {other:?}"),
    };
    let entity = world.entity(id).unwrap();
    assert_eq!(
        entity.type_name(),
        "Land_House_Damage_F",
        "the model path resolved to its class"
    );
    assert_eq!(
        entity.position(),
        DVec3::new(120.0, 0.0, 160.0),
        "the promoted object keeps its place"
    );
    assert_eq!(
        outcome.hit_points,
        [(0, 0.5), (1, 0.5)],
        "the house's `depends` hit point came with the class"
    );
    assert_eq!(
        world.hit_point_damage(ObjectRef::Entity(id), "HitWall"),
        Some(0.5)
    );
}

#[test]
fn a_static_object_whose_model_is_in_cfg_non_aivehicles_promotes_with_that_class() {
    let mut world = world(ClientId::SERVER);
    let key = static_key(&world, 1);

    world
        .apply_damage(ObjectRef::Static(key), DamageHit::added(0.5))
        .unwrap();

    let Some(ObjectRef::Entity(id)) = world.resolve(key.network_id()) else {
        panic!("not promoted");
    };
    let entity = world.entity(id).unwrap();
    assert_eq!(entity.type_name(), "T_Pinus_F");
    assert_eq!(entity.class(), SimulationClass::Thing);
}

#[test]
fn a_static_object_without_a_config_class_promotes_to_a_plain_type() {
    let mut world = world(ClientId::SERVER);
    let key = static_key(&world, 2);

    let outcome = world
        .apply_damage(ObjectRef::Static(key), DamageHit::added(0.5))
        .unwrap();

    assert_eq!(outcome.damage, 0.5);
    let Some(ObjectRef::Entity(id)) = world.resolve(key.network_id()) else {
        panic!("not promoted");
    };
    let entity = world.entity(id).unwrap();
    assert_eq!(entity.type_name(), "stone", "named after its model file");
    assert_eq!(entity.class(), SimulationClass::Plain);
    assert!(entity.damage_state().hit_points().is_empty());
}

#[test]
fn without_a_resolver_a_promoted_static_object_gets_no_hit_points() {
    let mut world = world_without_resolver(ClientId::SERVER);
    let key = static_key(&world, 0);

    world
        .apply_damage(ObjectRef::Static(key), DamageHit::added(0.5))
        .unwrap();

    let Some(ObjectRef::Entity(id)) = world.resolve(key.network_id()) else {
        panic!("not promoted");
    };
    let entity = world.entity(id).unwrap();
    assert_eq!(entity.type_name(), "house_damage_f");
    assert_eq!(entity.class(), SimulationClass::Plain);
    assert!(entity.damage_state().hit_points().is_empty());
    assert_eq!(entity.damage(), 0.5);
}

#[test]
fn a_ruined_static_object_gives_up_its_terrain_record_but_keeps_its_handle() {
    let mut world = world(ClientId::SERVER);
    let key = static_key(&world, 0);

    let outcome = world
        .apply_damage(ObjectRef::Static(key), DamageHit::total(1.0).scripted())
        .unwrap();
    assert!(outcome.ruined);

    let Some(ObjectRef::Entity(id)) = world.resolve(key.network_id()) else {
        panic!("the promoted handle stopped resolving");
    };
    assert_eq!(
        world.entity(id).unwrap().type_name(),
        "Land_House_Damage_ruins_F"
    );
    assert_eq!(
        world.entity(id).unwrap().position(),
        DVec3::new(120.0, 0.0, 160.0),
        "the ruin stands where the house did"
    );
    assert_eq!(world.static_object(key), None, "the terrain object is gone");
    assert_eq!(world.find_static(DVec3::ZERO, 0), None);
    assert!(
        world.find_static(DVec3::ZERO, 1).is_some() && world.find_static(DVec3::ZERO, 2).is_some(),
        "the pine and the rock stay"
    );
}

#[test]
fn static_objects_do_not_allow_damage() {
    let mut world = world(ClientId::SERVER);
    let key = static_key(&world, 0);

    assert_eq!(world.damage_allowed(ObjectRef::Static(key)), Some(false));
    assert!(!world.set_damage_allowed(ObjectRef::Static(key), true));
}

// ---------------------------------------------------------------- the readers

#[test]
fn the_readers_report_the_hit_point_state() {
    let mut world = world(ClientId::SERVER);
    let id = create(&mut world, "Land_House_Damage_F");
    let object = ObjectRef::Entity(id);
    world
        .apply_damage_to(id, DamageHit::added(0.0).at(0, 0.4).at(1, 0.75))
        .unwrap();

    assert_eq!(world.damage_of(object), Some(0.0));
    assert_eq!(world.hit_point_damage(object, "HitWall"), Some(0.4));
    assert_eq!(
        world.hit_point_damage_of_selection(object, "wall"),
        Some(0.4)
    );
    assert_eq!(world.hit_point_damage_of_index(object, 1), Some(0.75));
    assert_eq!(world.hit_point_damage(object, "Nope"), None);
    let all = world.all_hit_points_damage(object).unwrap();
    let hit_points: Vec<&str> = all.hit_points.iter().map(String::as_str).collect();
    let selections: Vec<&str> = all.selections.iter().map(String::as_str).collect();
    assert_eq!(hit_points, ["HitWall", "HitRoof"]);
    assert_eq!(selections, ["wall", "roof"]);
    assert_eq!(all.damage, [0.4, 0.75]);
    assert_eq!(all.damage.len(), all.hit_points.len());

    let key = static_key(&world, 1);
    assert_eq!(world.damage_of(ObjectRef::Static(key)), Some(0.0));
}

// ---------------------------------------------------------------- the shipped config

/// Every `depends` value of the shipped CfgVehicles/CfgAmmo/CfgNonAIVehicles parses, so no
/// vanilla hit point silently loses its dependency. Skipped when `A3_ROOT` is unset.
#[test]
fn every_shipped_depends_expression_parses() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    fn walk(
        cfg: &a3_config::ConfigRef<'_>,
        seen: &mut std::collections::BTreeSet<String>,
        bad: &mut Vec<String>,
    ) {
        let depends = cfg.get("depends").text();
        if !depends.is_empty() {
            seen.insert(depends.clone());
            if Depends::parse(&depends).is_none() && depends.trim() != "0" {
                bad.push(depends);
            }
        }
        for child in cfg.entries_with_inherited() {
            if child.is_class() {
                walk(&child, seen, bad);
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut bad = Vec::new();
    for root_name in ["CfgVehicles", "CfgAmmo", "CfgNonAIVehicles"] {
        walk(&data.config.root().get(root_name), &mut seen, &mut bad);
    }
    eprintln!(
        "{} distinct depends values, {} unparsed",
        seen.len(),
        bad.len()
    );
    for value in &bad {
        eprintln!("  unparsed: {value}");
    }
    assert!(seen.len() > 5, "the config was read");
    assert!(bad.is_empty());
}

/// The hit points, armor and ruin of a vanilla house, and the fatal hit points of a soldier.
/// Skipped when `A3_ROOT` is unset.
#[test]
fn the_shipped_house_and_soldier_read_their_damage_config() {
    let Some(root) = std::env::var_os("A3_ROOT") else {
        eprintln!("skipping: A3_ROOT not set");
        return;
    };
    let data = a3_gamedata::GameData::load(&a3_gamedata::LoadOptions::new(root)).unwrap();
    let mut bank = TypeBank::new(data.config.clone());

    let house = bank.get("Land_i_House_Big_01_V1_F").unwrap();
    let model = house.damage();
    assert!(model.armor() >= 1.0, "armor {}", model.armor());
    assert!(!model.hit_points().is_empty());
    assert!(
        model.hit_points().iter().any(|p| p.radius > 0.0),
        "a hit point with a radius"
    );
    assert!(
        model
            .hit_points()
            .iter()
            .any(|p| p.pass_through != 1.0 && p.pass_through > 0.0),
        "a hit point passing damage through"
    );
    let ruin = model.ruins().first().expect("the house has a ruin");
    let ruin_class = bank
        .class_of_model(&ruin.model)
        .unwrap_or_else(|| panic!("no class for {}", ruin.model));
    assert!(
        ruin_class.to_ascii_lowercase().starts_with("land_"),
        "{ruin_class}"
    );
    eprintln!("{} -> {ruin_class}", ruin.model);

    let soldier = bank.get("B_Soldier_F").unwrap();
    let model = soldier.damage();
    assert!(model.armor() > 0.0);
    let fatal: Vec<&str> = model
        .fatal_hit_points()
        .iter()
        .filter_map(|f| model.hit_points().get(f.index).map(|p| p.name.as_str()))
        .collect();
    assert_eq!(fatal, ["HitHead", "HitBody"]);
    assert!(
        model.hit_point_index_of_selection("head").is_some(),
        "the head selection resolves"
    );
}
