//! The move state machine of a Man: his move, the blend to the next one, the input a player
//! controller fills in and the movement that comes out of the animation.
//!
//! Everything runs on the synthetic moves config in `tests/common`, so it runs everywhere.

mod common;

use a3_moves::{MoveId, Stance};
use a3_world::{EntityId, ManInput, World};
use glam::DVec3;

use common::{create_man, world};

/// Steps the World `frames` times at 1/15 s: one step of a Man per call, the step these tests
/// are written in.
fn run(world: &mut World, frames: usize) {
    for _ in 0..frames {
        world.simulate(1.0 / 15.0);
    }
}

/// Where he faces: the World's front, his orientation times `Z`.
fn front_of(world: &World, man: EntityId) -> DVec3 {
    world.entity(man).unwrap().orientation() * DVec3::Z
}

/// A freshly created Man plays the idle move of his stance.
#[test]
fn a_man_stands_in_the_idle_move_of_his_stance() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));

    world.simulate(1.0 / 15.0);

    assert_eq!(world.animation_state(man), "stand");
}

/// Without a moves type there is nothing to play: he still stands on the ground (the motion of
/// `sim::man::ground` does not depend on the animation).
#[test]
fn without_moves_a_man_has_no_move() {
    let mut world = a3_world::World::new(a3_world::ClientId::SERVER);
    world
        .load_terrain(std::sync::Arc::new(common::flat_terrain(100.0)))
        .unwrap();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));

    world.simulate(1.0 / 15.0);

    assert_eq!(world.animation_state(man), "");
    assert_eq!(world.man(man).unwrap().input, ManInput::default());
}

/// Pushing forward, he plays the walk move and moves north — his front — at the speed the
/// animation gives him: the RTM step of the walk (1.62 m per cycle) times its phase rate
/// (0.85 cycles/s), i.e. 1.377 m/s (`docs/re/sim-man-movement.md` §3).
#[test]
fn walking_forward_moves_him_at_the_speed_of_his_move() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 20.0);
    let man = create_man(&mut world, start);
    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );

    for _ in 0..15 {
        world.simulate(1.0 / 15.0);
    }

    assert_eq!(world.animation_state(man), "walk");
    let entity = world.entity(man).unwrap();
    let speed = 1.62 * 0.85;
    assert!(
        (entity.velocity().z - speed).abs() < 1e-3,
        "{:?}",
        entity.velocity()
    );
    let moved = entity.position() - start;
    // A little short of the walk's speed: the first steps are slower while the walk blends in.
    assert!((moved.z - speed).abs() < 0.1, "{moved:?}");
    assert!(moved.x.abs() < 1e-9 && moved.y.abs() < 1e-9, "{moved:?}");
}

/// Pushing backward he plays the walk-back move and moves south, behind his front: 1.12 m per
/// cycle × 0.8 cycles/s. `Stand` has no direct edge to `WalkBack`, so the request goes through
/// the route `Stand → Walk → WalkBack` (`docs/re/sim-man-movement.md` §2).
#[test]
fn walking_backward_moves_him_backwards() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 20.0);
    let man = create_man(&mut world, start);
    world.set_man_input(
        man,
        ManInput {
            forward: -1.0,
            ..ManInput::default()
        },
    );

    // The route takes two hops and the blend has to reverse his velocity, so let him settle
    // first.
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "walkback");
    assert!(
        world.entity(man).unwrap().position().z < start.z,
        "he went back"
    );

    let before = world.entity(man).unwrap().position();
    run(&mut world, 15);
    let entity = world.entity(man).unwrap();
    let speed = 1.12 * 0.8;
    assert!(
        (entity.velocity().z + speed).abs() < 1e-3,
        "{:?}",
        entity.velocity()
    );
    let moved = entity.position() - before;
    assert!((moved.z + speed).abs() < 1e-3, "{moved:?}");
    assert!(moved.x.abs() < 1e-9 && moved.y.abs() < 1e-9, "{moved:?}");
}

/// Strafing left plays the left move and moves him west, his left while he faces north: 1.0 m
/// per cycle × 0.5 cycles/s. He keeps his front.
#[test]
fn strafing_left_moves_him_left() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            strafe: -1.0,
            ..ManInput::default()
        },
    );

    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "walkleft");

    let before = world.entity(man).unwrap().position();
    run(&mut world, 15);
    let entity = world.entity(man).unwrap();
    let speed = 1.0 * 0.5;
    assert!(
        (entity.velocity().x + speed).abs() < 1e-3,
        "{:?}",
        entity.velocity()
    );
    let moved = entity.position() - before;
    assert!((moved.x + speed).abs() < 1e-3, "{moved:?}");
    assert!(moved.z.abs() < 1e-9 && moved.y.abs() < 1e-9, "{moved:?}");
}

/// Turning right swings his front to the right, north towards east, without playing another
/// move or moving him: `docs/re/sim-man-locomotion.md` §2.
#[test]
fn turning_right_moves_his_front_to_the_right() {
    let mut world = world();
    let start = DVec3::new(10.0, 100.0, 20.0);
    let man = create_man(&mut world, start);
    world.set_man_input(
        man,
        ManInput {
            turn: 1.0,
            ..ManInput::default()
        },
    );

    run(&mut world, 1);

    let front = front_of(&world, man);
    assert!(front.x > 0.0 && front.z > 0.0, "{front:?}");
    assert_eq!(world.animation_state(man), "stand");
    let moved = world.entity(man).unwrap().position() - start;
    assert_eq!(moved.x, 0.0);
    assert_eq!(moved.z, 0.0);
}

/// Turning left takes him the other way, north towards west.
#[test]
fn turning_left_moves_his_front_to_the_left() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            turn: -1.0,
            ..ManInput::default()
        },
    );

    run(&mut world, 1);

    let front = front_of(&world, man);
    assert!(front.x < 0.0 && front.z > 0.0, "{front:?}");
}

/// A full turn is the move's `turnSpeed`: `StandActions` has 2, half a turn per second of
/// holding it. The turn he applies follows what he asks for at up to 6 per second, so it ramps
/// in first (`docs/re/sim-man-locomotion.md` §2).
#[test]
fn a_full_turn_is_the_moves_turn_speed() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            turn: 1.0,
            ..ManInput::default()
        },
    );

    run(&mut world, 15); // the ramp in
    let before = front_of(&world, man);
    run(&mut world, 15); // one second of full deflection
    let after = front_of(&world, man);

    let turned = before.angle_between(after);
    assert!((turned - std::f64::consts::PI).abs() < 0.05, "{turned}");
}

/// He does not jump from standing to the walk's speed: the blend runs over the walk's
/// `interpolationSpeed`, so the speed builds up from the idle he was in.
#[test]
fn starting_to_walk_blends_in() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );

    world.simulate(1.0 / 15.0);
    let first = world.entity(man).unwrap().velocity().z;
    let walk = 1.62 * 0.85;
    assert!(first > 0.0 && first < walk, "{first}");

    for _ in 0..15 {
        world.simulate(1.0 / 15.0);
    }
    let later = world.entity(man).unwrap().velocity().z;
    assert!(
        later > first,
        "the blend must build up: {first} then {later}"
    );
    assert!((later - walk).abs() < 1e-3, "{later}");
}

/// Lying down: he plays the stand-down move into the prone idle. `Stand` has no direct edge to
/// `Prone`, so the route is `Stand → StandDown → Prone`, and the hop into `Prone` is a
/// `connectTo`: it may only start once the stand-down has played out
/// (`docs/re/sim-man-anim-state.md` §5).
#[test]
fn lying_down_plays_the_stand_down_move_first() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            stance: Stance::Prone,
            ..ManInput::default()
        },
    );

    run(&mut world, 5);
    assert_eq!(world.animation_state(man), "standdown");
    // The stand-down carries him forward onto the ground: 0.5 m per cycle × 1 cycle/s.
    assert!((world.entity(man).unwrap().velocity().z - 0.5).abs() < 1e-3);

    // Its cycle lasts a second; only then does he lie down, and lying still he is.
    run(&mut world, 15);
    assert_eq!(world.animation_state(man), "prone");
    assert!(world.entity(man).unwrap().velocity().length() < 1e-3);
}

/// `playMove` asks for a move behind the machine's back: he plays it, and once its cycle is over
/// his input has him again (`docs/re/sim-man-anim-state.md` §3, §6.1).
#[test]
fn playmove_plays_the_move_and_hands_him_back_to_his_input() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    run(&mut world, 1);
    assert_eq!(world.animation_state(man), "stand");

    assert!(world.play_move(man, "walk"));
    run(&mut world, 1);
    assert_eq!(world.animation_state(man), "walk");

    // The walk is looped, but a request is done once its cycle is over (1/0.85 s): then nothing
    // asks for anything, and he stands again.
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "stand");
}

/// A second `playMove` queues behind the first: the queue is a FIFO and the next entry starts
/// only once the move he was asked for before it has played out.
#[test]
fn a_queued_move_waits_for_the_one_before_it_to_play_out() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    run(&mut world, 1);

    assert!(world.play_move(man, "walk"));
    assert!(world.play_move(man, "run"));
    let moves = world.moves().unwrap().clone();
    let queue: Vec<MoveId> = world.man(man).unwrap().moves.queue().collect();
    assert_eq!(
        queue,
        vec![moves.find("walk").unwrap(), moves.find("run").unwrap()],
        "the queue holds what the script asked for, in order"
    );

    run(&mut world, 3);
    assert_eq!(world.animation_state(man), "walk", "the run waits");

    // The run starts once the walk's cycle is over, and hands back to his input in turn.
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "run");
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "stand");
}

/// `playMoveNow` drops everything queued and arms the move at once — the route through the move
/// graph still applies (he blends his way there), but nothing queued before it plays.
#[test]
fn playmovnow_drops_the_queue_and_starts_at_once() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );
    run(&mut world, 5);
    assert_eq!(world.animation_state(man), "walk");

    assert!(world.play_move(man, "run"));
    assert!(world.play_move(man, "walkback"));
    run(&mut world, 3);
    assert_eq!(
        world.animation_state(man),
        "run",
        "the run was at the front"
    );

    assert!(world.play_move_now(man, "prone"));
    assert_eq!(
        world.man(man).unwrap().moves.queue().count(),
        0,
        "the queue is dropped"
    );
    run(&mut world, 1);
    assert_ne!(world.animation_state(man), "run", "he starts on the route");

    // The route walks the graph into the prone idle; the queued walk-back is gone for good.
    let mut seen = Vec::new();
    for _ in 0..40 {
        world.simulate(1.0 / 15.0);
        seen.push(world.animation_state(man));
    }
    assert!(!seen.iter().any(|s| s == "walkback"), "{seen:?}");
    assert!(seen.iter().any(|s| s == "prone"), "{seen:?}");
    // Pushing forward on the ground he crawls, so his input has him again.
    assert_eq!(world.animation_state(man), "crawl");
}

/// `switchMove` resets him to the move on the spot: no route through the move graph, no blend,
/// and nothing queued plays — the move is just what he plays now
/// (`docs/re/sim-man-anim-state.md` §6.3).
#[test]
fn switchmove_puts_him_in_the_move_on_the_spot() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );
    run(&mut world, 5);
    assert_eq!(world.animation_state(man), "walk");

    assert!(world.play_move(man, "run"));
    assert!(world.switch_move(man, "walkback"));
    assert_eq!(
        world.animation_state(man),
        "walkback",
        "he is in it without a step"
    );
    let state = &world.man(man).unwrap().moves;
    assert_eq!(state.phase(), 0.0, "its cycle starts over");
    assert_eq!(state.weight(), 1.0, "nothing blends");
    assert_eq!(state.queue().count(), 0, "the queued run is dropped");

    // Nothing holds him there: his input pulls him out of it from the next step on, and the run
    // that was queued before the switch never plays.
    let mut seen = Vec::new();
    for _ in 0..20 {
        world.simulate(1.0 / 15.0);
        seen.push(world.animation_state(man));
    }
    assert!(!seen.iter().any(|s| s == "run"), "{seen:?}");
    assert_eq!(world.animation_state(man), "walk");
}

/// A `switchMove` name that is no move of the moves type — the empty string included — resets
/// him to the default move of his current action map, the engine's fallback for an unknown name
/// (`docs/re/sim-man-anim-state.md` §6.3).
#[test]
fn switchmove_with_an_unknown_name_uses_the_default_of_his_action_map() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    run(&mut world, 1);
    assert!(world.play_move(man, "walk"));
    run(&mut world, 3);
    assert_eq!(world.animation_state(man), "walk");

    assert!(world.switch_move(man, ""));
    assert_eq!(world.animation_state(man), "stand", "StandActions' idle");

    assert!(world.switch_move(man, "prone"));
    assert!(world.switch_move(man, "not a move"));
    assert_eq!(world.animation_state(man), "prone", "ProneActions' idle");
}

/// The array form of `switchMove` also writes the phase and the blend the script asks for:
/// `[move, time, blendFactor, resetAim]` (`docs/re/sim-man-anim-state.md` §6.3).
#[test]
fn switchmove_with_an_array_sets_the_phase_and_the_blend() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    run(&mut world, 1);

    assert!(world.switch_move_at(man, "walk", 0.5, 0.25));
    assert_eq!(world.animation_state(man), "walk");
    let state = &world.man(man).unwrap().moves;
    assert_eq!(state.phase(), 0.5, "the phase the script wrote");
    assert_eq!(state.weight(), 0.25, "the blend factor the script wrote");

    // The blend ramps from that factor up to all of him.
    run(&mut world, 1);
    let weight = world.man(man).unwrap().moves.weight();
    assert!(weight > 0.25 && weight < 1.0, "{weight}");
    run(&mut world, 20);
    assert_eq!(world.man(man).unwrap().moves.weight(), 1.0);
}

/// `playAction` asks through the action map of the move he plays first and through the move
/// names after it; `playActionNow` clears the queue like `playMoveNow`
/// (`docs/re/sim-man-anim-state.md` §6.2).
#[test]
fn playaction_asks_through_the_action_map_of_his_move() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    run(&mut world, 1);
    assert_eq!(world.animation_state(man), "stand");

    // `StandActions` has `WalkF = "Walk"`: the same move his input would ask for.
    assert!(world.play_action(man, "WalkF"));
    run(&mut world, 1);
    assert_eq!(world.animation_state(man), "walk");

    // A name that is no action of the map is looked up as a move name.
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "stand");
    assert!(world.play_action(man, "Run"));
    run(&mut world, 1);
    assert_eq!(world.animation_state(man), "run");

    // A hop waits for the blend into the current move to finish, so give it time to settle; the
    // immediate forms below then take effect on the very next step.
    run(&mut world, 3);

    // `playActionNow` drops what was queued.
    assert!(world.play_move(man, "walkback"));
    assert!(world.play_action_now(man, "WalkF"));
    run(&mut world, 1);
    assert_eq!(world.animation_state(man), "walk");
    let mut seen = Vec::new();
    for _ in 0..20 {
        world.simulate(1.0 / 15.0);
        seen.push(world.animation_state(man));
    }
    assert!(!seen.iter().any(|s| s == "walkback"), "{seen:?}");

    // A name neither the map nor the moves type knows changes nothing.
    assert!(!world.play_action(man, "no such action"));
    assert!(!world.play_action_now(man, "no such action"));
}

/// A gesture action belongs on the engine's action layer (`Man+0x1830`), which this engine does
/// not model: asking for one leaves the move layer alone
/// (`docs/re/sim-man-anim-state.md` §2, §6.2).
#[test]
fn a_gesture_action_leaves_the_move_layer_alone() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    run(&mut world, 1);

    assert!(!world.play_action(man, "reloadMagazine"));
    assert!(!world.play_action_now(man, "reloadMagazine"));
    assert_eq!(world.animation_state(man), "stand");
    assert_eq!(world.man(man).unwrap().moves.queue().count(), 0);
}

/// He keeps the stance he is in while his axes drive him: on the ground, forward is the crawl.
#[test]
fn lying_down_and_pushing_forward_crawls() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    let prone = ManInput {
        stance: Stance::Prone,
        ..ManInput::default()
    };
    world.set_man_input(man, prone);
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "prone");

    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..prone
        },
    );
    run(&mut world, 15);

    assert_eq!(world.animation_state(man), "crawl");
    // The crawl step is 0.5 m per cycle at 0.5 cycles/s.
    assert!((world.entity(man).unwrap().velocity().z - 0.25).abs() < 1e-3);
}

/// Getting up again: the stance he is asked for is a request like any other, so asking for the
/// stand while prone blends him back up through the graph and his input drives him standing.
#[test]
fn standing_up_blends_back_out_of_the_prone() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            stance: Stance::Prone,
            ..ManInput::default()
        },
    );
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "prone");

    world.set_man_input(
        man,
        ManInput {
            stance: Stance::Stand,
            ..ManInput::default()
        },
    );
    run(&mut world, 1);
    assert_eq!(world.animation_state(man), "stand");

    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );
    run(&mut world, 20);
    assert_eq!(world.animation_state(man), "walk");
}

/// The blend a pose is built from, at the public seam `a3_pose::MoveBlend` reads: while he fades
/// out of the walk and into the stand, the state names both moves, each with the phase of its own
/// cycle, and how far the blend has gone. The move being left keeps playing its own cycle while
/// it fades, and once the blend is over there is none to name.
#[test]
fn a_blend_exposes_both_moves_and_the_phase_of_each() {
    let mut world = world();
    let man = create_man(&mut world, DVec3::new(10.0, 100.0, 20.0));
    world.set_man_input(
        man,
        ManInput {
            forward: 1.0,
            ..ManInput::default()
        },
    );
    run(&mut world, 20);
    let walk = world.man(man).unwrap().moves.current().unwrap();
    assert_eq!(world.animation_state(man), "walk");

    // Standing asks for the stand, and the walk fades out under it: their edge interpolates.
    world.set_man_input(man, ManInput::default());
    run(&mut world, 1);
    let after_one = {
        let state = &world.man(man).unwrap().moves;
        assert_eq!(state.previous(), Some(walk), "the move being left");
        assert_eq!(
            world.animation_state(man),
            "stand",
            "the move being entered"
        );
        let weight = state.weight();
        assert!(weight > 0.0 && weight < 1.0, "mid-fade: {weight}");
        state.previous_phase()
    };

    // It advances at its own phase rate while the blend runs (Walk: 0.85 cycles/s, 1/15 s a
    // step), not at the rate of the move taking over (Stand: 1.0, which would be 0.0667).
    run(&mut world, 1);
    let advanced = (world.man(man).unwrap().moves.previous_phase() - after_one).rem_euclid(1.0);
    assert!((advanced - 0.85 / 15.0).abs() < 1e-3, "{advanced}");

    // The blend over, he is in one move: there is none being left, and the previous sample of a
    // pose is the current move at its own phase.
    run(&mut world, 3);
    let state = &world.man(man).unwrap().moves;
    assert_eq!(state.previous(), None);
    assert_eq!(state.previous_phase(), state.phase());
}
