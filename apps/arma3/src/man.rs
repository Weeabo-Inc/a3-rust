//! The Man's animation: which Move the player's stance, pace and direction play, how the cycle
//! runs, and the rigged Man they pose.
//!
//! A Man is always in exactly one Move (CONTEXT.md), possibly blending out of the previous one,
//! so the client picks the Move from the player's own state rather than searching the Move graph
//! the way the engine's Action maps do: the unarmed states are named by stance (`Perc` standing,
//! `Pknl` kneeling, `Ppne` prone), gait (`Mstp` standing, `Mwlk` walking, `Mrun` running) and
//! direction (`Df`/`Db`/`Dl`/`Dr` and the diagonals), which is what [`move_names`] builds.
//!
//! The cycle the Move plays at is derived from the ground speed the player is moving at and the
//! distance the Move's RTM carries him per cycle, so the feet stay on the ground on a slope or a
//! straight (the shipped forward states' own `speed` agrees: 0.350 cycles/s times the walk's
//! 5.64 m step is the client's 1.98 m/s walk).
//!
//! The RTMs themselves come from the game data. They are read once each, the first time the Man
//! reaches the Move (`MoveClips::load` keeps a file already loaded), so a session pays only for
//! the moves it plays.

use a3_moves::{MoveId, Moves};
use a3_p3d::Model;
use a3_pose::{ManRig, MoveBlend, MoveClips};
use a3_vfs::Vfs;
use glam::{Affine3A, Vec3};

use crate::player::{Motion, Pace, Stance};

/// The `pivotsModel` of the Man's CfgSkeletonParameters: the rest pivots its moves are authored
/// over.
pub const PIVOTS_MODEL: &str = r"a3\anims_f\data\skeleton\skeletonpivots.p3d";

/// The `weaponBone` of the Man's CfgSkeletonParameters (`OFP2_ManSkeleton`): empty in the
/// shipped config, so the engine converts every bone's translation, `weapon` included.
pub const WEAPON_BONE: &str = "";

/// The Move every Moves type has, the last fallback: the standing idle.
const STANDING: &str = "AmovPercMstpSnonWnonDnon";

/// How long a change of Move blends over, in seconds: the middle of the 0.2-0.5 s the engine's
/// unarmed interpolate edges take.
pub const BLEND_TIME: f32 = 0.3;

/// The Move names' prefix for a stance: `AmovPerc` standing, `AmovPknl` kneeling, `AmovPpne`
/// prone.
fn prefix(stance: Stance) -> &'static str {
    match stance {
        Stance::Stand => "AmovPerc",
        Stance::Crouch => "AmovPknl",
        Stance::Prone => "AmovPpne",
    }
}

/// The Moves a Man in `stance`, moving as `motion` (or standing still), plays: the state the
/// engine's Action map would pick first, then the states to fall back to when the Moves type
/// has no such state — a gait it does not have (a prone walk is the crawl), then the stance's
/// own idle, then the standing idle every Moves type has.
pub fn move_names(stance: Stance, motion: Motion) -> Vec<String> {
    let prefix = prefix(stance);
    let idle = format!("{prefix}MstpSnonWnonDnon");
    let Some(direction) = motion.direction else {
        return vec![idle, STANDING.to_owned()];
    };
    // Sprint has no Move of its own: it plays the run.
    let (gait, other) = match motion.pace {
        Pace::Walk => ("Mwlk", "Mrun"),
        Pace::Run | Pace::Sprint => ("Mrun", "Mwlk"),
    };
    let one = direction.suffix();
    vec![
        format!("{prefix}{gait}SnonWnon{one}"),
        format!("{prefix}{gait}SnonWnonDf"),
        format!("{prefix}{other}SnonWnon{one}"),
        format!("{prefix}{other}SnonWnonDf"),
        idle,
        STANDING.to_owned(),
    ]
}

/// The first of `names` the Moves type has, as its Move. `None` when it has none of them.
pub fn resolve(moves: &Moves, names: &[String]) -> Option<MoveId> {
    names.iter().find_map(|name| moves.find(name))
}

/// The male soldier Moves type of the merged config, `CfgMovesMaleSdr`. `None` (with a warning)
/// when the game data has none, so the Man is drawn unposed.
pub fn moves_of(config: &a3_config::ConfigTree) -> Option<Moves> {
    match Moves::from_config(&config.root().get("CfgMovesMaleSdr")) {
        Ok(moves) => Some(moves),
        Err(error) => {
            log::warn!("the game data has no usable CfgMovesMaleSdr: {error}");
            None
        }
    }
}

/// The cycle rate in cycles per second that keeps a Move's feet on the ground while the Man
/// moves at `speed` metres per second: one cycle carries him `step` metres. A Move that carries
/// him nowhere (a standing idle) does not cycle.
pub fn cycle_rate(speed: f32, step: f32) -> f32 {
    if step <= 0.0 { 0.0 } else { speed / step }
}

/// Where the Man is in his Moves: the Move playing, the one he is blending out of, and how far
/// each has played into its own cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveClock {
    /// The Move being left: the same as `current` when nothing is changing.
    pub previous: MoveId,
    /// The phase `previous` has played to.
    pub previous_phase: f32,
    /// The Move playing.
    pub current: MoveId,
    /// The phase `current` has played to, `0` (start) to `1` (end) of its cycle.
    pub current_phase: f32,
    /// How far the change has blended, `0` (all `previous`) to `1` (all `current`).
    pub blend: f32,
    /// Cycles per second `previous` advances at while the blend lasts.
    previous_rate: f32,
    /// Cycles per second `current` advances at.
    current_rate: f32,
}

impl MoveClock {
    /// A Man at the start of `first`'s cycle.
    pub fn new(first: MoveId) -> MoveClock {
        MoveClock {
            previous: first,
            previous_phase: 0.0,
            current: first,
            current_phase: 0.0,
            blend: 1.0,
            previous_rate: 0.0,
            current_rate: 0.0,
        }
    }

    /// Advance by `dt` with the Move to play and its cycle rate in cycles per second. A change
    /// of Move blends out of the one playing over [`BLEND_TIME`], which keeps playing its own
    /// cycle behind the blend.
    pub fn advance(&mut self, next: MoveId, rate: f32, dt: f32) {
        if next != self.current {
            self.previous = self.current;
            self.previous_phase = self.current_phase;
            self.previous_rate = self.current_rate;
            self.current = next;
            self.current_phase = 0.0;
            self.blend = 0.0;
        }
        self.current_rate = rate;
        self.previous_phase = wrap(self.previous_phase + self.previous_rate * dt);
        self.current_phase = wrap(self.current_phase + rate * dt);
        self.blend = (self.blend + dt / BLEND_TIME).min(1.0);
    }

    /// The blend these moves and phases are, for [`MoveBlend::state`].
    pub fn blend(&self) -> MoveBlend {
        MoveBlend {
            previous: self.previous,
            previous_phase: self.previous_phase,
            current: self.current,
            current_phase: self.current_phase,
            blend: self.blend,
        }
    }
}

/// A position in a Move's cycle, wrapped back to the start at the end ([`MoveState`]'s phases
/// are `0..=1`).
///
/// [`MoveState`]: a3_pose::MoveState
fn wrap(phase: f32) -> f32 {
    phase - phase.floor()
}

/// The Man's animation: his Skeleton rigged with the game's rest pivots, his Moves type, the
/// RTMs reached so far and his place in the cycle.
pub struct ManAnimation {
    rig: ManRig,
    /// Where the model's origin sits in the rest pivots' space: its `bounding_center` (ODOL
    /// vertices are stored relative to it).
    model_offset: Vec3,
    moves: Moves,
    clips: MoveClips,
    clock: MoveClock,
    vfs: Vfs,
}

impl ManAnimation {
    /// The animation for the Man model at `model_path`, starting in his standing idle. `None`
    /// when the model, its Skeleton, the rest pivots or the idle Move is missing from the game
    /// data; the caller then draws the unposed mesh.
    pub fn load(vfs: &Vfs, moves: Moves, model_path: &str) -> Option<ManAnimation> {
        let soldier = Model::from_bytes(&vfs.open(model_path).ok()?).ok()?;
        let skeleton = soldier.skeleton.clone()?;
        let pivots = Model::from_bytes(&vfs.open(PIVOTS_MODEL).ok()?).ok()?;
        let rig = ManRig::new(&skeleton, &pivots, WEAPON_BONE);
        let idle = resolve(&moves, &move_names(Stance::Stand, Motion::default()))?;
        let mut animation = ManAnimation {
            rig,
            model_offset: soldier.info.bounding_center,
            moves,
            clips: MoveClips::new(),
            clock: MoveClock::new(idle),
            vfs: vfs.clone(),
        };
        animation.load_clips([idle]);
        Some(animation)
    }

    /// Play the Move `motion` calls for in `stance`, one frame of `dt` seconds long.
    pub fn advance(&mut self, stance: Stance, motion: Motion, dt: f32) {
        let Some(move_) = resolve(&self.moves, &move_names(stance, motion)) else {
            return;
        };
        self.load_clips([move_]);
        let step = self
            .clips
            .sample(&self.moves, move_, 0.0)
            .map_or(0.0, |sample| sample.step().length());
        let rate = cycle_rate(motion.speed as f32, step);
        self.clock.advance(move_, rate, dt);
    }

    /// The Man's bone palette for this frame: his pose at the clock's place in the cycle,
    /// composed down his Skeleton, as the renderer's skinned instances take it (in the model's
    /// own space). `None` when the Move's RTM is not loaded.
    pub fn palette(&self) -> Option<Vec<Affine3A>> {
        let state = self.clock.blend().state(&self.clips, &self.moves)?;
        let pose = self.rig.pose(&state);
        Some(self.rig.skinning_pose(&pose, self.model_offset).bones)
    }

    /// The point of the model's space that stands on the entity's position when posed: the
    /// ground under the Man's feet (`ManRig::ground_offset`).
    pub fn ground(&self) -> Vec3 {
        ManRig::ground_offset(self.model_offset)
    }

    /// Put the Man straight into the Move named `name` at `phase` of its cycle, with no blend
    /// (the `switchMove` of a script). He stays there until [`ManAnimation::advance`] picks
    /// another Move. `false` when the Moves type has no such Move; he keeps his Move then.
    pub fn switch_move(&mut self, name: &str, phase: f32) -> bool {
        let Some(id) = self.moves.find(name) else {
            return false;
        };
        self.load_clips([id]);
        let mut clock = MoveClock::new(id);
        clock.current_phase = wrap(phase);
        clock.previous_phase = clock.current_phase;
        self.clock = clock;
        true
    }

    /// The name of the Move playing, for the overlay.
    pub fn move_name(&self) -> &str {
        &self.moves.get(self.clock.current).name
    }

    /// Read the RTMs of `ids` that are not loaded yet.
    fn load_clips(&mut self, ids: impl IntoIterator<Item = MoveId>) {
        let ManAnimation {
            clips, moves, vfs, ..
        } = self;
        let report = clips.load(moves, ids, |path| vfs.open(path).ok().map(|b| b.to_vec()));
        for path in &report.missing {
            log::warn!("Man move animation {path} is not in the game data");
        }
        for (path, error) in &report.failed {
            log::warn!("Man move animation {path} does not read: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::Direction;

    fn walking(pace: Pace, direction: Direction) -> Motion {
        Motion {
            direction: Some(direction),
            pace,
            speed: 0.0,
        }
    }

    #[test]
    fn standing_still_is_the_stances_idle() {
        for (stance, idle) in [
            (Stance::Stand, "AmovPercMstpSnonWnonDnon"),
            (Stance::Crouch, "AmovPknlMstpSnonWnonDnon"),
            (Stance::Prone, "AmovPpneMstpSnonWnonDnon"),
        ] {
            assert_eq!(move_names(stance, Motion::default())[0], idle);
        }
    }

    #[test]
    fn a_move_names_the_stance_the_gait_and_the_direction() {
        assert_eq!(
            move_names(Stance::Stand, walking(Pace::Walk, Direction::Forward))[0],
            "AmovPercMwlkSnonWnonDf"
        );
        assert_eq!(
            move_names(Stance::Crouch, walking(Pace::Walk, Direction::Right))[0],
            "AmovPknlMwlkSnonWnonDr"
        );
        assert_eq!(
            move_names(Stance::Prone, walking(Pace::Run, Direction::BackLeft))[0],
            "AmovPpneMrunSnonWnonDbl"
        );
        // Sprint has no Move of its own: it is the run.
        assert_eq!(
            move_names(Stance::Stand, walking(Pace::Sprint, Direction::Forward))[0],
            "AmovPercMrunSnonWnonDf"
        );
    }

    #[test]
    fn a_move_the_type_does_not_have_falls_back_to_a_more_general_one() {
        // Prone has no walk, only the crawl (`Mrun`), and a direction may be missing too.
        let names = move_names(Stance::Prone, walking(Pace::Walk, Direction::Right));
        assert_eq!(names[0], "AmovPpneMwlkSnonWnonDr", "asked for first");
        assert!(
            names.contains(&"AmovPpneMrunSnonWnonDr".to_owned()),
            "the crawl, sideway"
        );
        assert!(
            names.contains(&"AmovPpneMrunSnonWnonDf".to_owned()),
            "the crawl, forward"
        );
        assert!(
            names.contains(&"AmovPpneMstpSnonWnonDnon".to_owned()),
            "prone idle as is"
        );
        assert_eq!(
            names.last().unwrap(),
            "AmovPercMstpSnonWnonDnon",
            "the standing idle ends every chain"
        );
    }

    /// A Moves type with two of the shipped unarmed states and nothing else.
    fn synthetic_moves() -> Moves {
        let config = a3_config::parse_text(
            r#"
class CfgMovesMaleSdr {
    class States {
        class AmovPercMstpSnonWnonDnon {
            file = "\a3\anims_f\data\anim\sdr\stp\erc\stp_idle.rtm";
            speed = 1e+10;
        };
        class AmovPercMwlkSnonWnonDf {
            file = "\a3\anims_f\data\anim\sdr\wlk\erc\wlk_f.rtm";
            speed = 0.350;
        };
    };
};"#,
        )
        .expect("the fixture parses");
        let tree = a3_config::ConfigTree::from_config(&config);
        Moves::from_config(&tree.root().get("CfgMovesMaleSdr")).expect("the states read")
    }

    #[test]
    fn resolve_picks_the_first_move_the_type_has() {
        let moves = synthetic_moves();
        let names = [
            "AmovPercMrunSnonWnonDf".to_owned(),
            "AmovPercMwlkSnonWnonDf".to_owned(),
        ];
        let id = resolve(&moves, &names).expect("the walk is in the fixture");
        assert_eq!(moves.get(id).name, "AmovPercMwlkSnonWnonDf");
        assert_eq!(
            resolve(&moves, &["AmovPercMsprSnonWnonDf".to_owned()]),
            None
        );
    }

    #[test]
    fn a_config_without_the_male_moves_type_has_no_moves() {
        let config = a3_config::parse_text("class CfgPatches {};").expect("the fixture parses");
        let tree = a3_config::ConfigTree::from_config(&config);
        assert!(moves_of(&tree).is_none());
    }

    #[test]
    fn the_cycle_rate_keeps_the_feet_on_the_ground() {
        // The shipped walk: 5.64 m per cycle at the client's 1.98 m/s is the state's own
        // 0.350 cycles/s (`docs/re/sim-man-movement.md`).
        assert!((cycle_rate(1.98, 5.64) - 0.350).abs() < 0.005);
        assert_eq!(cycle_rate(0.0, 5.64), 0.0, "standing still does not cycle");
        assert_eq!(cycle_rate(1.5, 0.0), 0.0, "a Move that carries him nowhere");
    }

    #[test]
    fn a_change_of_move_blends_from_the_one_before_it() {
        let (idle, walk) = (MoveId(0), MoveId(1));
        let mut clock = MoveClock::new(idle);
        // The frame that changes the Move is a quarter of the blend long, and counts towards it.
        clock.advance(walk, 0.35, BLEND_TIME / 4.0);
        assert_eq!(clock.current, walk);
        assert_eq!(clock.previous, idle, "the idle is what he is leaving");
        assert!(
            (clock.blend - 0.25).abs() < 1e-6,
            "a quarter across: {}",
            clock.blend
        );
        assert!((clock.current_phase - 0.35 * BLEND_TIME / 4.0).abs() < 1e-6);
        // Long enough for the blend to finish, he is in the walk alone.
        clock.advance(walk, 0.35, BLEND_TIME);
        assert_eq!(clock.blend, 1.0, "the blend is over");
    }

    #[test]
    fn a_move_advances_a_cycle_per_second_of_its_rate_and_wraps() {
        let idle = MoveId(0);
        let mut clock = MoveClock::new(idle);
        clock.advance(idle, 0.5, 1.0);
        assert!((clock.current_phase - 0.5).abs() < 1e-6);
        clock.advance(idle, 0.5, 1.0);
        assert!(
            clock.current_phase.abs() < 1e-6,
            "phase wrapped: {}",
            clock.current_phase
        );
    }

    #[test]
    fn the_move_being_left_keeps_playing_behind_the_blend() {
        let (walk, run) = (MoveId(1), MoveId(2));
        let mut clock = MoveClock::new(walk);
        clock.advance(walk, 0.4, 0.25);
        clock.advance(run, 1.0, 0.25);
        assert_eq!(clock.previous, walk);
        assert!(
            (clock.previous_phase - 0.2).abs() < 1e-6,
            "{}",
            clock.previous_phase
        );
        assert!(
            (clock.current_phase - 0.25).abs() < 1e-6,
            "{}",
            clock.current_phase
        );
    }
}
