//! The Move state machine of a Man: the move he plays now, the blend from the one before, and
//! the play queue `playMove` and friends build (`docs/re/sim-man-movement.md`).
//!
//! The machine plays one move of the moves type at a time. A move is left along the move graph
//! (a `connectTo` or `interpolateTo` edge) and the blend between the two is driven by their
//! `interpolationSpeed`. Movement ([`super::ManState::motion`]) comes out of the moves: each
//! contributes its RTM step scaled by its blend weight.

use a3_moves::{MoveId, Moves, Stance};

use super::ManInput;

/// The state machine of one Man: his current move, where in its animation he is, and the blend
/// from the previous move.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MoveState {
    /// The move he plays now. `None` until his first step, or while no moves type is loaded.
    current: Option<MoveId>,
}

impl MoveState {
    /// The move he plays now.
    pub fn current(&self) -> Option<MoveId> {
        self.current
    }

    /// The name of the move he plays now, in its original case (`Stand`, not `stand`), if any.
    pub fn move_name<'a>(&self, moves: &'a Moves) -> Option<&'a str> {
        Some(moves.get(self.current?).name.as_str())
    }

    /// One step of the machine: pick the move his input asks for, blend into it, and advance the
    /// play queue. `moves` is `None` while no moves type is loaded, and then he has no move.
    pub fn advance(&mut self, moves: Option<&Moves>, input: &ManInput, _dt: f64) {
        let Some(moves) = moves else {
            return;
        };
        // His first step: he starts in the idle move of his stance.
        if self.current.is_none() {
            self.current = idle_move(moves, self.wanted_stance(moves, input));
        }
    }

    /// The stance he asks to be in: the input's stance, or the stance of the move he plays (he
    /// keeps it), or standing when he has no move yet.
    fn wanted_stance(&self, moves: &Moves, input: &ManInput) -> Stance {
        if input.stance != Stance::Undefined {
            return input.stance;
        }
        self.current
            .and_then(|current| moves.get(current).actions)
            .map(|map| moves.action_map(map).stance)
            .filter(|stance| *stance != Stance::Undefined)
            .unwrap_or(Stance::Stand)
    }
}

/// The idle move of a stance: the `Stop` action of the stance's primary action map. Without a map
/// of that stance the first primary map answers (`CfgMovesBasic` `primaryActionMaps`).
fn idle_move(moves: &Moves, stance: Stance) -> Option<MoveId> {
    let maps = moves.primary_action_maps();
    let map = maps
        .iter()
        .map(|id| moves.action_map(*id))
        .find(|map| map.stance == stance)
        .or_else(|| maps.first().map(|id| moves.action_map(*id)))?;
    map.get_move("Stop")
}
