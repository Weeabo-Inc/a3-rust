//! The player: a Man standing on the terrain with a first- or third-person camera.
//!
//! The Man simulation (`crates/a3-world`) is not wired in yet, so this module drives the
//! player transform directly from input, with the speeds and slope limits recorded in
//! `docs/re/sim-man-movement.md`. When the Man sim lands, its `ManInput` replaces
//! [`Player::update`]'s movement step; the camera code stays as is.

use a3_input::{ActionMap, InputState, actions};
use a3_render::Camera;
use glam::{DAffine3, DQuat, DVec3};

/// Action names the player uses that `a3-input` does not name yet. The names are the ones the
/// game's `CfgDefaultKeysPresets` bind (`personView`, `turbo`, `crouch`, `prone`, `stand`).
pub const PERSON_VIEW: &str = "personView";
pub const TURBO: &str = "turbo";
pub const CROUCH: &str = "crouch";
pub const PRONE: &str = "prone";
pub const STAND: &str = "stand";
pub const WALK: &str = "walk";

/// Ground height in metres at a world position: the terrain under the player.
pub trait Ground {
    fn height(&self, x: f64, z: f64) -> f32;
}

/// Eye height above the feet in the standing stance, in metres.
pub const STAND_EYE_HEIGHT: f64 = 1.70;

/// Eye height crouched and prone, in metres.
pub const CROUCH_EYE_HEIGHT: f64 = 1.05;
pub const PRONE_EYE_HEIGHT: f64 = 0.35;

/// Movement speeds in metres per second, from the move states' `speed` times their RTM step
/// (`docs/re/sim-man-movement.md`). Unarmed speeds: the milestone renders no weapons.
pub const WALK_SPEED: f64 = 1.98;
pub const RUN_SPEED: f64 = 3.97;
/// Sprint has no state in `CfgMovesMaleSdr >> States`; this is the engine's documented
/// "sprint is ~3x walking" rounded to the fast end. Approximate.
pub const SPRINT_SPEED: f64 = 5.40;
/// Kneeling walk and run.
pub const CROUCH_WALK_SPEED: f64 = 1.28;
pub const CROUCH_RUN_SPEED: f64 = 3.24;
/// Crawling, prone. Approximate: the crawl states' speeds were not extracted.
pub const PRONE_SPEED: f64 = 0.70;

/// How fast A/D turn the man, radians per second. Approximate.
pub const TURN_RATE: f32 = 2.0;

/// Gravity, metres per second squared, as the engine uses for men.
pub const GRAVITY: f64 = 9.81;

/// Steepest terrain gradient (rise over run along the movement direction) that a man can still
/// run and sprint up, from `CfgSlopeLimits` (maxRun 0.6, maxSprint 0.3; the minRun/minSprint
/// downhill limits are not applied yet, see `docs/re/sim-man-movement.md` §4).
pub const MAX_RUN_GRADIENT: f64 = 0.6;
pub const MAX_SPRINT_GRADIENT: f64 = 0.3;

/// Distance ahead the slope is probed at, in metres.
const SLOPE_PROBE: f64 = 1.0;

/// How far up and down the aim can tilt, in radians: the free-fly camera's clamp.
pub const MAX_PITCH: f32 = 1.55;

/// The `CfgVehicles` class the player is: a BLUFOR Rifleman, so the third-person camera shows
/// the game's own Man model.
pub const PLAYER_CLASS: &str = "B_Soldier_F";

/// Third-person boom: how far behind the head the camera sits, in metres.
pub const THIRD_PERSON_BACK: f64 = 2.5;

/// Third-person boom: how far above the feet the camera pivot sits, in metres.
pub const THIRD_PERSON_UP: f64 = 1.5;

/// Which camera the player sees through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    #[default]
    FirstPerson,
    ThirdPerson,
}

impl std::str::FromStr for CameraMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "first" | "fp" | "firstperson" => Ok(CameraMode::FirstPerson),
            "third" | "tp" | "thirdperson" => Ok(CameraMode::ThirdPerson),
            _ => Err(format!("expected `first`/`fp` or `third`/`tp`, not `{s}`")),
        }
    }
}

impl CameraMode {
    /// The other mode: what the `personView` key toggles to.
    pub fn toggled(self) -> CameraMode {
        match self {
            CameraMode::FirstPerson => CameraMode::ThirdPerson,
            CameraMode::ThirdPerson => CameraMode::FirstPerson,
        }
    }
}

/// The player's stance, from the game's `crouch`/`prone`/`stand` actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stance {
    #[default]
    Stand,
    Crouch,
    Prone,
}

/// The direction the Man moves in, relative to where he faces: the eight ways the engine's moves
/// name with their `Df`/`Db`/`Dl`/`Dr` and diagonal suffixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Back,
    Left,
    Right,
    ForwardLeft,
    ForwardRight,
    BackLeft,
    BackRight,
}

impl Direction {
    /// Which of the eight directions a body-relative movement points in, or `None` at rest.
    /// `forward` is along the facing, `strafe` to the Man's right; the sectors are 45 degrees
    /// wide, centred on each direction, so a straight input is that direction and a diagonal
    /// input is its own.
    pub fn of(forward: f32, strafe: f32) -> Option<Direction> {
        const SECTORS: [Direction; 8] = [
            Direction::Forward,
            Direction::ForwardRight,
            Direction::Right,
            Direction::BackRight,
            Direction::Back,
            Direction::BackLeft,
            Direction::Left,
            Direction::ForwardLeft,
        ];
        if forward.hypot(strafe) < 1e-3 {
            return None;
        }
        // Clockwise from the facing: turning right is positive, like the heading is.
        let sector = (strafe.atan2(forward) / std::f32::consts::FRAC_PI_4).round() as i32;
        Some(SECTORS[sector.rem_euclid(8) as usize])
    }

    /// The suffix the engine's moves name this direction with.
    pub fn suffix(self) -> &'static str {
        match self {
            Direction::Forward => "Df",
            Direction::Back => "Db",
            Direction::Left => "Dl",
            Direction::Right => "Dr",
            Direction::ForwardLeft => "Dfl",
            Direction::ForwardRight => "Dfr",
            Direction::BackLeft => "Dbl",
            Direction::BackRight => "Dbr",
        }
    }
}

/// How the Man is moving this frame: the Move his animation plays is the one this selects.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Motion {
    /// The body-relative direction of travel; `None` while he stands.
    pub direction: Option<Direction>,
    /// The pace the movement keys asked for.
    pub pace: Pace,
    /// How fast his feet move over the ground, in metres per second (`0` while standing, and
    /// less than the pace speed when the slope slows him).
    pub speed: f64,
}

/// A Man standing somewhere in the world.
#[derive(Debug, Clone)]
pub struct Player {
    /// Feet position in world space.
    pub position: DVec3,
    /// Body facing in radians, clockwise from north.
    pub yaw: f32,
    /// Look elevation in radians, positive up.
    pub pitch: f32,
    pub stance: Stance,
    pub mode: CameraMode,
    /// The movement applied by the last [`update`](Self::update): what his animation plays.
    pub motion: Motion,
    /// Vertical speed while falling, metres per second (negative down).
    velocity_y: f64,
}

impl Player {
    /// A standing player with feet at `position`, facing north.
    pub fn new(position: DVec3) -> Player {
        Player {
            position,
            yaw: 0.0,
            pitch: 0.0,
            stance: Stance::Stand,
            mode: CameraMode::default(),
            motion: Motion::default(),
            velocity_y: 0.0,
        }
    }

    /// Turn and tilt the aim, as mouse motion does while the cursor is captured. `yaw_delta`
    /// turns clockwise, `pitch_delta` looks up; the pitch is clamped like the engine's.
    pub fn look(&mut self, yaw_delta: f32, pitch_delta: f32) {
        self.yaw += yaw_delta;
        self.pitch = (self.pitch + pitch_delta).clamp(-MAX_PITCH, MAX_PITCH);
    }

    pub fn switch_mode(&mut self, mode: CameraMode) {
        self.mode = mode;
    }

    /// The camera for the current mode: at the eyes in first person, on a boom behind the head
    /// in third person. Both look along the player's aim.
    ///
    /// The boom pivots `THIRD_PERSON_UP` above the feet — the engine's default view height —
    /// or at the eyes when the stance puts them lower, and points back along the aim, so
    /// looking down raises the camera behind the man.
    pub fn camera(&self) -> Camera {
        let direction = look_direction(self.yaw, self.pitch);
        let eye = self.eye();
        match self.mode {
            CameraMode::FirstPerson => Camera {
                position: eye,
                yaw: self.yaw,
                pitch: self.pitch,
                ..Camera::default()
            },
            CameraMode::ThirdPerson => {
                let pivot = self.position
                    + DVec3::new(0.0, self.stance.eye_height().min(THIRD_PERSON_UP), 0.0);
                Camera {
                    position: pivot - direction * THIRD_PERSON_BACK,
                    yaw: self.yaw,
                    pitch: self.pitch,
                    ..Camera::default()
                }
            }
        }
    }

    /// The player's eyes in world space for the current stance.
    pub fn eye(&self) -> DVec3 {
        self.position + DVec3::new(0.0, self.stance.eye_height(), 0.0)
    }

    /// The Man's world transform for the model renderer: feet at [`position`](Self::position),
    /// his front towards [`yaw`](Self::yaw).
    ///
    /// A model's front in raw P3D data is -Z, not +Z (`docs/re/p3d-odol.md`: the Offroad's front
    /// lights are at z ≈ -3.1, and the soldier's `leftshoulder` — his left, +X — is the memory
    /// point pair that fixes the side), while a heading is measured clockwise from north (+Z).
    /// So the rotation about Y is the heading plus a half turn: it carries the model's front,
    /// not its back, along the direction the player faces.
    pub fn transform(&self) -> DAffine3 {
        DAffine3::from_rotation_translation(
            DQuat::from_rotation_y(f64::from(self.yaw) + std::f64::consts::PI),
            self.position,
        )
    }

    /// Advance the player one frame: stance keys, turn, move over `ground`, fall to it.
    pub fn update(&mut self, map: &ActionMap, input: &InputState, ground: &dyn Ground, dt: f64) {
        // `personView` toggles first and third person, like the engine's view key.
        if map.just_triggered(input, PERSON_VIEW) {
            self.mode = self.mode.toggled();
        }
        // Stance keys toggle, like the engine's `crouch`/`prone`; the fallback `stand` key
        // always stands.
        for (action, stance) in [
            (STAND, Stance::Stand),
            (CROUCH, Stance::Crouch),
            (PRONE, Stance::Prone),
        ] {
            if map.just_triggered(input, action) {
                self.stance = if self.stance == stance {
                    Stance::Stand
                } else {
                    stance
                };
            }
        }
        let command = player_input(map, input);
        self.yaw += command.turn * TURN_RATE * dt as f32;
        let forward = horizontal_direction(self.yaw);
        let right = DVec3::new(forward.z, 0.0, -forward.x);
        let mut direction =
            forward * f64::from(command.forward) + right * f64::from(command.strafe);
        let moving = direction.length_squared() > 0.0;
        if moving {
            direction = direction.normalize();
        }
        let speed = self.speed(command.pace, ground, direction);
        self.position += direction * speed * dt;
        self.motion = Motion {
            direction: if moving {
                Direction::of(command.forward, command.strafe)
            } else {
                None
            },
            pace: command.pace,
            speed: if moving { speed } else { 0.0 },
        };
        self.settle(ground, dt);
    }

    /// The speed for a pace in this stance, reduced by the slope along `direction`.
    fn speed(&self, pace: Pace, ground: &dyn Ground, direction: DVec3) -> f64 {
        let here = f64::from(ground.height(self.position.x, self.position.z));
        let ahead = f64::from(ground.height(
            self.position.x + direction.x * SLOPE_PROBE,
            self.position.z + direction.z * SLOPE_PROBE,
        ));
        let gradient = if direction == DVec3::ZERO {
            0.0
        } else {
            (ahead - here) / SLOPE_PROBE
        };
        let clear = |limit: f64| gradient <= limit;
        let mut pace = pace;
        if pace == Pace::Sprint && !clear(MAX_SPRINT_GRADIENT) {
            pace = Pace::Run;
        }
        if pace == Pace::Run && !clear(MAX_RUN_GRADIENT) {
            pace = Pace::Walk;
        }
        self.stance.speed(pace)
    }

    /// Keep the feet on the ground: snap up the terrain, fall back down to it.
    fn settle(&mut self, ground: &dyn Ground, dt: f64) {
        let surface = f64::from(ground.height(self.position.x, self.position.z)).max(0.0);
        if self.position.y <= surface {
            self.position.y = surface;
            self.velocity_y = 0.0;
        } else {
            self.velocity_y -= GRAVITY * dt;
            self.position.y += self.velocity_y * dt;
            if self.position.y <= surface {
                self.position.y = surface;
                self.velocity_y = 0.0;
            }
        }
    }
}

/// The lift that stands a model's feet on the ground, from the lowest corner of its
/// model-space box.
///
/// A character's mesh is authored below its model origin — `b_soldier_01.p3d`'s body spans
/// `y = -1.852 .. -0.020`, the whole Man hanging off an origin at the crown of his head — and
/// the engine's stance animations (`ManPosStanding` and friends in the Man's `model.cfg`) lift
/// the body by that much so the feet rest on the ground plane. Until poses are evaluated (see
/// `a3-anim`), the mesh's own lowest point stands in for that lift.
pub fn ground_lift(lowest: DVec3) -> f64 {
    (-lowest.y).max(0.0)
}

/// The stance's eye height above the feet, in metres.
impl Stance {
    pub fn eye_height(self) -> f64 {
        match self {
            Stance::Stand => STAND_EYE_HEIGHT,
            Stance::Crouch => CROUCH_EYE_HEIGHT,
            Stance::Prone => PRONE_EYE_HEIGHT,
        }
    }

    /// The speed for a pace in this stance.
    fn speed(self, pace: Pace) -> f64 {
        match (self, pace) {
            (Stance::Stand, Pace::Walk) => WALK_SPEED,
            (Stance::Stand, Pace::Run) => RUN_SPEED,
            (Stance::Stand, Pace::Sprint) => SPRINT_SPEED,
            (Stance::Crouch, Pace::Walk) => CROUCH_WALK_SPEED,
            (Stance::Crouch, Pace::Run | Pace::Sprint) => CROUCH_RUN_SPEED,
            (Stance::Prone, _) => PRONE_SPEED,
        }
    }
}

/// How fast the man moves: walk, run or sprint. Running is the engine's default pace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pace {
    Walk,
    #[default]
    Run,
    Sprint,
}

/// One frame of movement input, from the game's user actions.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlayerCommand {
    /// -1 back, +1 forward; A/D strafe when the preset binds `moveLeft`/`moveRight`.
    pub forward: f32,
    pub strafe: f32,
    /// -1 left, +1 right, from `turnLeft`/`turnRight`.
    pub turn: f32,
    pub pace: Pace,
}

/// Read the movement actions: forward/back, strafing, turning, and the walk/sprint modifiers.
/// Sprinting only moves the man forward, like the engine's sprint.
pub fn player_input(map: &ActionMap, input: &InputState) -> PlayerCommand {
    let v = |name| map.value(input, name);
    let forward = v(actions::MOVE_FORWARD) - v(actions::MOVE_BACK);
    let pace = if forward > 0.5 && map.is_active(input, TURBO) {
        Pace::Sprint
    } else if map.is_active(input, WALK) {
        Pace::Walk
    } else {
        Pace::Run
    };
    PlayerCommand {
        forward,
        strafe: v(actions::MOVE_RIGHT) - v(actions::MOVE_LEFT),
        turn: v(actions::TURN_RIGHT) - v(actions::TURN_LEFT),
        pace,
    }
}

/// Unit horizontal direction for a heading clockwise from north: X east, Z north.
pub fn horizontal_direction(yaw: f32) -> DVec3 {
    let (sin, cos) = yaw.sin_cos();
    DVec3::new(f64::from(sin), 0.0, f64::from(cos))
}

/// Unit look direction for a heading (clockwise from north) and elevation, following the
/// renderer's convention: X east, Y up, Z north.
pub fn look_direction(yaw: f32, pitch: f32) -> DVec3 {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    DVec3::new(f64::from(sy * cp), f64::from(sp), f64::from(cy * cp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use a3_input::{Dik, InputCode, InputState, actions};

    /// A horizontal plane at a fixed height, for tests that do not need a terrain.
    struct FlatGround(f32);

    impl Ground for FlatGround {
        fn height(&self, _x: f64, _z: f64) -> f32 {
            self.0
        }
    }

    #[test]
    fn walking_forward_runs_along_the_facing_and_stays_on_the_ground() {
        let ground = FlatGround(10.0);
        let mut player = Player::new(DVec3::new(0.0, 10.0, 0.0));
        let mut input = InputState::new();
        input.press(InputCode::Key(Dik::W));
        player.update(&player_map(), &input, &ground, 1.0);
        // The unarmed run state's speed: 0.6845 cycles/s x 5.7437 m step
        // (docs/re/sim-man-movement.md).
        assert!(
            (player.position.z - RUN_SPEED).abs() < 1e-4,
            "moved north by {RUN_SPEED} m: {:?}",
            player.position
        );
        assert!(player.position.x.abs() < 1e-9);
        assert!(
            (player.position.y - 10.0).abs() < 1e-9,
            "feet snapped to the ground: {:?}",
            player.position
        );
    }

    /// The action map the client runs with: the built-in bindings plus the client's own
    /// fallback keys for the actions the shipped presets leave empty.
    fn player_map() -> ActionMap {
        let mut map = actions::default_map();
        crate::keys::ensure_client_defaults(&mut map);
        map
    }

    /// A plane tilted along +Z: the height rises by `rise` metres per metre north.
    struct SlopeGround(f32);

    impl Ground for SlopeGround {
        fn height(&self, _x: f64, z: f64) -> f32 {
            z as f32 * self.0
        }
    }

    fn press_forward(keys: &[Dik]) -> InputState {
        let mut input = InputState::new();
        input.press(InputCode::Key(Dik::W));
        for &key in keys {
            input.press(InputCode::Key(key));
        }
        input
    }

    #[test]
    fn a_gentle_uphill_allows_a_run_but_no_sprint() {
        // Gradient 0.4: runnable (maxRun 0.6) but not sprintable (maxSprint 0.3).
        let ground = SlopeGround(0.4);
        let mut player = Player::new(DVec3::ZERO);
        let input = press_forward(&[Dik::LSHIFT]);
        player.update(&player_map(), &input, &ground, 0.1);
        assert!(
            (player.position.z - RUN_SPEED * 0.1).abs() < 1e-3,
            "sprinted {RUN_SPEED} m/s up a 0.4 slope: {:?}",
            player.position
        );
    }

    #[test]
    fn a_steep_uphill_slows_the_man_to_a_walk() {
        // Gradient 1.0: above maxRun 0.6, only walking is possible.
        let ground = SlopeGround(1.0);
        let mut player = Player::new(DVec3::ZERO);
        let input = press_forward(&[Dik::LSHIFT]);
        player.update(&player_map(), &input, &ground, 0.1);
        assert!(
            (player.position.z - WALK_SPEED * 0.1).abs() < 1e-3,
            "walked {WALK_SPEED} m/s up a 1.0 slope: {:?}",
            player.position
        );
    }

    #[test]
    fn stance_keys_crouch_the_eye_and_slow_the_man() {
        let ground = FlatGround(0.0);
        let map = player_map();
        let mut player = Player::new(DVec3::ZERO);
        let key = |dik| {
            let mut i = InputState::new();
            i.press(InputCode::Key(dik));
            i
        };
        let crouch = key(Dik::C);
        player.update(&map, &crouch, &ground, 0.1);
        assert_eq!(player.stance, Stance::Crouch);
        assert!((player.eye().y - CROUCH_EYE_HEIGHT).abs() < 1e-9);

        // Kneeling run, from the moves states, while no stance key is being pressed.
        player.update(&map, &key(Dik::W), &ground, 1.0);
        assert_eq!(player.stance, Stance::Crouch, "stays crouched");
        assert!(
            (player.position.z - CROUCH_RUN_SPEED).abs() < 1e-4,
            "{:?}",
            player.position
        );

        // Pressing crouch again stands the man back up.
        player.update(&map, &crouch, &ground, 0.1);
        assert_eq!(player.stance, Stance::Stand);

        player.update(&map, &key(Dik::Z), &ground, 0.1);
        assert_eq!(player.stance, Stance::Prone);
        assert!((player.eye().y - PRONE_EYE_HEIGHT).abs() < 1e-9);
        player.update(&map, &key(Dik::X), &ground, 0.1);
        assert_eq!(player.stance, Stance::Stand);
    }

    #[test]
    fn camera_mode_names_parse() {
        use std::str::FromStr as _;
        assert_eq!(
            CameraMode::from_str("first").unwrap(),
            CameraMode::FirstPerson
        );
        assert_eq!(CameraMode::from_str("fp").unwrap(), CameraMode::FirstPerson);
        assert_eq!(
            CameraMode::from_str("third").unwrap(),
            CameraMode::ThirdPerson
        );
        assert_eq!(CameraMode::from_str("tp").unwrap(), CameraMode::ThirdPerson);
        assert_eq!(
            CameraMode::from_str("THIRD").unwrap(),
            CameraMode::ThirdPerson,
            "case-insensitive"
        );
        assert!(CameraMode::from_str("2D").is_err());
    }

    #[test]
    fn the_person_view_key_toggles_between_the_cameras() {
        let map = player_map();
        let mut player = Player::new(DVec3::ZERO);
        let ground = FlatGround(0.0);
        let mut input = InputState::new();
        input.press(InputCode::Key(Dik::NUMPADENTER));
        assert!(
            map.just_triggered(&input, PERSON_VIEW),
            "the profile's `personView` key is Numpad Enter"
        );
        player.update(&map, &input, &ground, 0.1);
        assert_eq!(player.mode, CameraMode::ThirdPerson);
        // The next frame, the key is still held: no further toggle.
        input.end_frame();
        player.update(&map, &input, &ground, 0.1);
        assert_eq!(player.mode, CameraMode::ThirdPerson, "one toggle per press");
        // Released and pressed again: back to first person.
        input.release(InputCode::Key(Dik::NUMPADENTER));
        input.end_frame();
        let mut again = InputState::new();
        again.press(InputCode::Key(Dik::NUMPADENTER));
        player.update(&map, &again, &ground, 0.1);
        assert_eq!(player.mode, CameraMode::FirstPerson);
    }

    #[test]
    fn strafing_steps_sideways_without_turning_the_man() {
        // #228: the strafe keys move him laterally; his facing only follows the aim and the turn
        // keys.
        let mut player = Player::new(DVec3::ZERO);
        let mut input = InputState::new();
        input.press(InputCode::Key(Dik::D));
        player.update(&player_map(), &input, &FlatGround(0.0), 1.0);
        // Facing north (+Z), his right is east (+X).
        assert!(
            (player.position.x - RUN_SPEED).abs() < 1e-4,
            "stepped east by the run speed: {:?}",
            player.position
        );
        assert!(
            player.position.z.abs() < 1e-9,
            "no motion along the facing: {:?}",
            player.position
        );
        assert_eq!(player.yaw, 0.0, "a strafe key does not turn him");
        assert_eq!(player.motion.direction, Some(Direction::Right));
        assert!((player.motion.speed - RUN_SPEED).abs() < 1e-4);
    }

    #[test]
    fn standing_still_has_no_direction_and_no_speed() {
        let mut player = Player::new(DVec3::ZERO);
        player.stance = Stance::Crouch;
        player.update(&player_map(), &InputState::new(), &FlatGround(0.0), 1.0);
        assert_eq!(player.motion.direction, None);
        assert_eq!(player.motion.speed, 0.0, "the idle's Move does not cycle");
    }

    #[test]
    fn the_direction_sectors_are_centred_on_the_eight_ways_a_man_moves() {
        assert_eq!(Direction::of(1.0, 0.0), Some(Direction::Forward));
        assert_eq!(Direction::of(1.0, 0.4), Some(Direction::Forward));
        assert_eq!(Direction::of(1.0, 0.5), Some(Direction::ForwardRight));
        assert_eq!(Direction::of(0.0, 1.0), Some(Direction::Right));
        assert_eq!(Direction::of(-1.0, 1.0), Some(Direction::BackRight));
        assert_eq!(Direction::of(-1.0, 0.0), Some(Direction::Back));
        assert_eq!(Direction::of(-1.0, -1.0), Some(Direction::BackLeft));
        assert_eq!(Direction::of(0.0, -1.0), Some(Direction::Left));
        assert_eq!(Direction::of(1.0, -1.0), Some(Direction::ForwardLeft));
        assert_eq!(
            Direction::of(0.0, 0.0),
            None,
            "at rest there is no direction"
        );
    }

    #[test]
    fn the_direction_suffixes_are_the_engines_own() {
        assert_eq!(Direction::Forward.suffix(), "Df");
        assert_eq!(Direction::Back.suffix(), "Db");
        assert_eq!(Direction::Left.suffix(), "Dl");
        assert_eq!(Direction::Right.suffix(), "Dr");
        assert_eq!(Direction::ForwardLeft.suffix(), "Dfl");
        assert_eq!(Direction::ForwardRight.suffix(), "Dfr");
        assert_eq!(Direction::BackLeft.suffix(), "Dbl");
        assert_eq!(Direction::BackRight.suffix(), "Dbr");
    }

    #[test]
    fn mouse_look_turns_the_aim_and_clamps_the_pitch() {
        let mut player = Player::new(DVec3::ZERO);
        player.look(0.5, 2.0);
        assert!((player.yaw - 0.5).abs() < 1e-6);
        assert!((player.pitch - MAX_PITCH).abs() < 1e-6, "{}", player.pitch);
        player.look(0.0, -10.0);
        assert!((player.pitch + MAX_PITCH).abs() < 1e-6);
    }

    #[test]
    fn the_third_person_camera_follows_the_walking_man() {
        let mut player = Player::new(DVec3::ZERO);
        player.switch_mode(CameraMode::ThirdPerson);
        let before = player.camera().position;
        player.update(&player_map(), &press_forward(&[]), &FlatGround(0.0), 1.0);
        let after = player.camera().position;
        assert!(
            (after - before - DVec3::new(0.0, 0.0, RUN_SPEED)).length() < 1e-6,
            "the boom keeps its offset: {:?}",
            after - before
        );
    }

    #[test]
    fn a_man_in_the_air_falls_to_the_ground() {
        let ground = FlatGround(0.0);
        let mut player = Player::new(DVec3::new(0.0, 5.0, 0.0));
        let map = player_map();
        let idle = InputState::new();
        player.update(&map, &idle, &ground, 0.05);
        assert!(
            player.position.y > 4.9,
            "not an instant drop: {:?}",
            player.position
        );
        for _ in 0..200 {
            player.update(&map, &idle, &ground, 0.05);
        }
        assert!(
            player.position.y.abs() < 1e-9,
            "landed: {:?}",
            player.position
        );
    }

    #[test]
    fn third_person_camera_sits_behind_the_man() {
        let mut player = Player::new(DVec3::new(100.0, 5.0, 200.0));
        player.switch_mode(CameraMode::ThirdPerson);
        player.yaw = 0.0; // Facing north (+Z).
        player.pitch = 0.0;
        let camera = player.camera();
        let offset = camera.position - player.position;
        assert!(
            (offset.z + THIRD_PERSON_BACK).abs() < 1e-9,
            "behind the man, so south: {offset:?}"
        );
        assert!(
            offset.x.abs() < 1e-9 && (offset.y - THIRD_PERSON_UP).abs() < 1e-9,
            "centred, {} m up: {offset:?}",
            THIRD_PERSON_UP
        );
        assert_eq!((camera.yaw, camera.pitch), (0.0, 0.0));
    }

    #[test]
    fn the_body_transform_stands_the_man_at_his_feet_facing_him() {
        let mut player = Player::new(DVec3::new(100.0, 20.0, 200.0));
        // A quarter turn clockwise from north: east.
        player.yaw = std::f32::consts::FRAC_PI_2;
        let t = player.transform();
        assert_eq!(t.translation, DVec3::new(100.0, 20.0, 200.0));
        // A model's front in raw P3D data is -Z (`docs/re/p3d-odol.md`), so the model's -z axis
        // is what must point along the direction the player faces.
        // Tolerance: the yaw is an f32, so a quarter turn is only f32-precise.
        let facing = t.transform_vector3(DVec3::NEG_Z);
        assert!((facing - DVec3::X).length() < 1e-6, "faces east: {facing}");
        player.yaw = 0.0;
        let north = player.transform().transform_vector3(DVec3::NEG_Z);
        assert!((north - DVec3::Z).length() < 1e-6, "faces north: {north}");
        player.yaw = std::f32::consts::PI;
        let south = player.transform().transform_vector3(DVec3::NEG_Z);
        assert!(
            (south - DVec3::NEG_Z).length() < 1e-6,
            "faces south: {south}"
        );
    }

    #[test]
    fn a_model_hanging_below_its_origin_is_lifted_onto_the_ground() {
        // The soldier's body box: the lowest vertex is 1.852 m under the model origin
        // (`docs/re/p3d-odol.md`'s box for b_soldier_01.p3d).
        let man = ground_lift(DVec3::new(-1.0307345, -1.8522594, 0.23840997));
        assert!((man - 1.8522594).abs() < 1e-9, "lifts the Man: {man}");
        // A model standing on its origin is left where it is.
        assert_eq!(ground_lift(DVec3::new(-0.5, 0.0, -0.5)), 0.0);
        // Geometry above the origin is not pushed into the ground.
        assert_eq!(ground_lift(DVec3::new(-0.5, 0.3, -0.5)), 0.0);
    }
}
