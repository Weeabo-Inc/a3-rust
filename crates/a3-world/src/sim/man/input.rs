//! What a Man is asked to do this frame, filled by the controller that drives him (the player's
//! input on a client, AI later). The move state machine turns it into moves.

use a3_moves::Stance;

/// The input of one Man: his movement axes, whether he sprints, and the stance he asks for.
///
/// A player controller writes it every frame ([`crate::World::set_man_input`]); an AI controller
/// will fill it the same way. An all-default input stands him still.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ManInput {
    /// Forward (`1.0`) and backward (`-1.0`) movement, `-1.0..=1.0`.
    pub forward: f32,
    /// Strafe right (`1.0`) and left (`-1.0`), `-1.0..=1.0`, in his own frame.
    pub strafe: f32,
    /// Turn right (`1.0`) and left (`-1.0`), `-1.0..=1.0`: he follows it at a limited rate, and
    /// the move he plays scales it by its `turnSpeed` ([`super::Turning`]).
    pub turn: f32,
    /// `true` while the sprint key is held: `RunF` instead of `WalkF` where the move graph links
    /// both. Sprinting uphill is limited (`CfgSlopeLimits`, `docs/re/sim-man-movement.md` §4).
    pub sprint: bool,
    /// The stance he asks for; [`Stance::Undefined`] (the default) keeps the stance he is in.
    pub stance: Stance,
}

impl ManInput {
    /// Whether there is no movement request at all: nothing to walk, run, strafe or turn by.
    pub fn is_idle(&self) -> bool {
        self.forward == 0.0 && self.strafe == 0.0 && self.turn == 0.0
    }
}
