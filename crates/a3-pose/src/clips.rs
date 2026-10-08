//! The bridge from a moves type to poses: `a3-moves` names a move's RTM by path and plays it to
//! a phase; [`MoveClips`] holds the RTMs themselves and [`MoveBlend`] is a blend state in those
//! terms.

use std::collections::HashMap;

use a3_moves::{MoveId, Moves};
use a3_rtm::Animation;

use crate::{MoveSample, MoveState};

/// The RTMs a moves type names, read once per distinct `file` and kept for posing.
///
/// [`Moves`] gives every move the path of its RTM ([`a3_moves::Move::file`]); a pose needs the
/// animation itself. Load the moves a Man can reach up front, or the whole type with
/// `moves.iter().map(|(id, _)| id)`.
#[derive(Debug, Clone, Default)]
pub struct MoveClips {
    by_file: HashMap<String, Animation>,
}

impl MoveClips {
    /// An empty set: nothing is loaded.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the RTM of every move in `ids` through `open` (VFS path to bytes), once per
    /// distinct file; a move with no `file` is skipped, and a file already loaded is kept rather
    /// than read again. Ids naming the same file (`WalkAgain` of `Walk`) are one entry.
    pub fn load(
        &mut self,
        moves: &Moves,
        ids: impl IntoIterator<Item = MoveId>,
        mut open: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> ClipLoadReport {
        let mut report = ClipLoadReport::default();
        for id in ids {
            let file = moves.get(id).file.clone();
            if file.is_empty() || self.by_file.contains_key(&file) {
                continue;
            }
            match open(&file) {
                None => report.missing.push(file),
                Some(bytes) => match Animation::read(&bytes) {
                    Ok(animation) => {
                        report.loaded += 1;
                        self.by_file.insert(file, animation);
                    }
                    Err(e) => report.failed.push((file, e.to_string())),
                },
            }
        }
        report
    }

    /// Number of RTMs loaded.
    pub fn len(&self) -> usize {
        self.by_file.len()
    }

    /// Whether no RTM is loaded.
    pub fn is_empty(&self) -> bool {
        self.by_file.is_empty()
    }

    /// The move's animation, or `None` when the move has no RTM or its file is not loaded.
    /// `id` from another moves type panics, as [`Moves::get`] does.
    pub fn animation(&self, moves: &Moves, id: MoveId) -> Option<&Animation> {
        self.by_file.get(&moves.get(id).file)
    }

    /// The move at `phase` of its own cycle, for [`MoveState`]; `None` when its RTM is not
    /// loaded (see [`MoveClips::animation`]).
    pub fn sample(&self, moves: &Moves, id: MoveId, phase: f32) -> Option<MoveSample<'_>> {
        self.animation(moves, id)
            .map(|animation| MoveSample::new(animation, phase))
    }
}

/// What one [`MoveClips::load`] call did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipLoadReport {
    /// RTMs read and added to the set.
    pub loaded: usize,
    /// Files `open` did not find.
    pub missing: Vec<String>,
    /// Files that did not parse, with the error.
    pub failed: Vec<(String, String)>,
}

/// A blend of two moves of a moves type: the move being left and the one being entered, the
/// phase each has played to in its own cycle, and how far the blend between them has gone.
///
/// The phases are independent — an RTM's cycle is its own — which is what lets the engine blend
/// a walk into an idle per bone; it is the [`MoveState`] the pose is built from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveBlend {
    /// The move being left (all of the blend at `blend` 0).
    pub previous: MoveId,
    /// Phase the previous move has played to in its own cycle.
    pub previous_phase: f32,
    /// The move being entered (all of the blend at `blend` 1).
    pub current: MoveId,
    /// Phase the current move has played to in its own cycle.
    pub current_phase: f32,
    /// Blend position between the moves, `0..=1` ([`MoveState::phase`]).
    pub blend: f32,
}

impl MoveBlend {
    /// The blend over the loaded clips as a [`MoveState`], or `None` when a move's RTM is not
    /// loaded. A Man in one move names it twice with `blend` 0.
    pub fn state<'a>(&self, clips: &'a MoveClips, moves: &Moves) -> Option<MoveState<'a>> {
        Some(MoveState::new(
            clips.sample(moves, self.previous, self.previous_phase)?,
            clips.sample(moves, self.current, self.current_phase)?,
            self.blend,
        ))
    }
}
