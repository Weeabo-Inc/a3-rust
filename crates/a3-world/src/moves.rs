//! World operations on the moves type and the Men animated by it: loading `CfgMovesMaleSdr`,
//! the move state machine of a Man, and what a controller asks him to do.

use std::sync::Arc;

use a3_moves::Moves;

use crate::{ClassState, EntityId, ManInput, ManState, World};

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

    /// The state of the Man with this id, if it is one: his move state machine, his motion on the
    /// ground and what he is asked to do.
    pub fn man(&self, id: EntityId) -> Option<&ManState> {
        man_of(self.entity(id)?)
    }

    /// The state of the Man with this id, to change.
    pub fn man_mut(&mut self, id: EntityId) -> Option<&mut ManState> {
        man_of_mut(self.entity_mut(id)?)
    }

    /// Asks the Man with this id to do `input` ([`ManInput`]); the state machine reads it on his
    /// next step. Does nothing when the Entity is not a Man, or is gone.
    pub fn set_man_input(&mut self, id: EntityId, input: ManInput) {
        if let Some(man) = self.man_mut(id) {
            man.input = input;
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

fn man_of(entity: &crate::Entity) -> Option<&ManState> {
    match entity.class_state() {
        ClassState::Man(man) => Some(man),
        _ => None,
    }
}

fn man_of_mut(entity: &mut crate::Entity) -> Option<&mut ManState> {
    match entity.class_state_mut() {
        ClassState::Man(man) => Some(man),
        _ => None,
    }
}
