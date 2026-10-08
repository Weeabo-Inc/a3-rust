//! World operations on the moves type and the Men animated by it: loading `CfgMovesMaleSdr`,
//! the move state machine of a Man, and what a controller asks him to do.

use std::sync::Arc;

use a3_moves::{ActionTarget, MoveId, Moves};

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

    /// `playMove`: queues the move `name` behind everything the Man was asked for already. It
    /// plays once the move before it has played out
    /// (`docs/re/sim-man-anim-state.md` §6.1). `false` when the Entity is not a Man of this
    /// World, no moves type is loaded, or `name` is no move of it — a name the engine would log
    /// and drop.
    pub fn play_move(&mut self, id: EntityId, name: &str) -> bool {
        match self.move_id(name) {
            Some(move_id) => match self.man_mut(id) {
                Some(man) => {
                    man.moves.play(move_id);
                    true
                }
                None => false,
            },
            None => false,
        }
    }

    /// `playMoveNow`: like [`Self::play_move`], but drops everything queued with it and arms
    /// `name` at once.
    pub fn play_move_now(&mut self, id: EntityId, name: &str) -> bool {
        match self.move_id(name) {
            Some(move_id) => match self.man_mut(id) {
                Some(man) => {
                    man.moves.play_now(move_id);
                    true
                }
                None => false,
            },
            None => false,
        }
    }

    /// `switchMove`: resets the Man to the move `name` on the spot — no transition move out of
    /// the one he plays, no blend, nothing queued. A name that is no move of the moves type (the
    /// empty string included) resets him to the default move of his current action map, the
    /// engine's fallback for an unknown name (`docs/re/sim-man-anim-state.md` §6.3). `false`
    /// when neither resolves, or the Entity is no Man of this World.
    pub fn switch_move(&mut self, id: EntityId, name: &str) -> bool {
        self.switch_move_at(id, name, 0.0, 1.0)
    }

    /// The array form of `switchMove`: as [`Self::switch_move`], plus the cycle phase `time`
    /// (`0.0..=1.0`) and the blend factor the script asks for. The blend ramps from that factor
    /// up to all of him.
    pub fn switch_move_at(
        &mut self,
        id: EntityId,
        name: &str,
        time: f64,
        blend_factor: f64,
    ) -> bool {
        let Some(moves) = self.moves.clone() else {
            return false;
        };
        let Some(man) = self.man_mut(id) else {
            return false;
        };
        let target = moves.find(name).or_else(|| man.moves.default_move(&moves));
        let Some(target) = target else {
            return false;
        };
        man.moves.switch_to(target, time, blend_factor);
        true
    }

    /// `playAction`: queues the move the action `name` asks for in the move the Man plays. The
    /// action map of that move answers first, a move of that name after it, as the engine
    /// resolves it (`docs/re/sim-man-anim-state.md` §6.2); the queued move plays exactly like
    /// [`Self::play_move`]. A gesture action is refused — gestures live on the engine's action
    /// layer, which this engine does not model — and so is a name that resolves to nothing, or
    /// an Entity that is no Man of this World.
    pub fn play_action(&mut self, id: EntityId, name: &str) -> bool {
        let Some(target) = self.action_move(id, name) else {
            return false;
        };
        match self.man_mut(id) {
            Some(man) => {
                man.moves.play(target);
                true
            }
            None => false,
        }
    }

    /// `playActionNow`: like [`Self::play_action`], but drops everything queued and arms it at
    /// once.
    pub fn play_action_now(&mut self, id: EntityId, name: &str) -> bool {
        let Some(target) = self.action_move(id, name) else {
            return false;
        };
        match self.man_mut(id) {
            Some(man) => {
                man.moves.play_now(target);
                true
            }
            None => false,
        }
    }

    /// The move the `playAction` name asks for in the move the Man with this id plays now.
    fn action_move(&self, id: EntityId, name: &str) -> Option<MoveId> {
        let moves = self.moves.as_ref()?;
        let current = self.man(id)?.moves.current()?;
        match moves.action(current, name) {
            Some(ActionTarget::Move(id)) => Some(*id),
            // A gesture is the action layer's (`Man+0x1830`), which is not modelled; the map
            // answered, so the move names are not tried.
            Some(ActionTarget::Gesture(_)) => None,
            None => moves.find(name),
        }
    }

    /// The id of the move called `name` in the loaded moves type, ignoring case.
    fn move_id(&self, name: &str) -> Option<MoveId> {
        self.moves.as_ref()?.find(name)
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
