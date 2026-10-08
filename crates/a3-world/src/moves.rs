//! World operations on the moves type: the moves every Man of the session is animated by.

use std::sync::Arc;

use a3_moves::Moves;

use crate::{EntityId, World};

impl World {
    /// Loads the moves type the Men of this World play (`CfgMovesMaleSdr`), replacing any loaded
    /// before. Every Man created afterwards is animated by it; the animation state of existing
    /// Men is left as it is until their next step.
    pub fn load_moves(&mut self, moves: Arc<Moves>) {
        self.moves = Some(moves);
    }

    /// The loaded moves type.
    pub fn moves(&self) -> Option<&Arc<Moves>> {
        self.moves.as_ref()
    }

    /// The move state machine of the Man with this id, if it is one.
    pub fn man(&self, id: EntityId) -> Option<&crate::ManState> {
        match self.entity(id)?.class_state() {
            crate::ClassState::Man(man) => Some(man),
            _ => None,
        }
    }

    /// The current move of the Man with this id, in lower case as `animationState` returns it.
    /// Empty when he is not a Man, has no moves type loaded, or has not been stepped yet.
    pub fn animation_state(&self, id: EntityId) -> String {
        let (Some(man), Some(moves)) = (self.man(id), self.moves.as_ref()) else {
            return String::new();
        };
        man.moves
            .move_name(moves)
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    }
}
