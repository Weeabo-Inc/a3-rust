//! What a group knows about its enemies: acquiring a target by seeing it, keeping it while it
//! stays in sight, and forgetting it 120 seconds later (`docs/re/ai.md`).
//!
//! Knowledge is the group's, not the unit's: the engine shares contact instantly among the men
//! of a group, and `knowsAbout` on any of them answers the same. The scale is the original's
//! 0..=4 (`docs/re/ai.md` has where the numbers come from).

use a3_physics::ObjectKey;

use crate::EntityId;

/// How far a group at full alert sees, in metres — the mission's `viewDistance` in the engine's
/// default setup. Behaviour scales it down ([`super::Behaviour::view_scale`]).
pub const VIEW_RANGE: f64 = 500.0;

/// Where the eyes of a standing man are, above his feet.
pub const EYE_HEIGHT: f64 = 1.55;

/// How fast seeing something turns into knowing it: a full 4 takes a second of looking at it.
pub const KNOWLEDGE_PER_SECOND: f64 = 4.0;

/// How long a target that is out of sight is kept before the group forgets it, in seconds — the
/// engine's loss-of-sight reset, measured (`knowsAbout` notes).
pub const FORGET_TIME: f64 = 120.0;

/// What a group knows about one enemy.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetKnowledge {
    pub target: EntityId,
    /// 0..=4; 0 is not stored (the group forgot it). The engine's "magic number" for opening
    /// fire is far below 4 — weapons come with #127.
    pub knowledge: f64,
    /// Where the target was when it was last seen, in world metres.
    pub position: glam::DVec3,
    /// World time the target was last seen in line of sight.
    pub last_seen: f64,
}

/// Everything a group knows about its enemies, oldest contact first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Targets {
    known: Vec<TargetKnowledge>,
}

impl Targets {
    /// How much the group knows about `target`, 0..=4.
    pub fn knowledge(&self, target: EntityId) -> f64 {
        self.known
            .iter()
            .find(|k| k.target == target)
            .map_or(0.0, |k| k.knowledge)
    }

    /// The contact the group knows best, in acquisition order on a tie.
    pub fn best(&self) -> Option<&TargetKnowledge> {
        self.known
            .iter()
            .reduce(|a, b| if b.knowledge > a.knowledge { b } else { a })
    }

    /// The contacts the group knows about.
    pub fn iter(&self) -> impl Iterator<Item = &TargetKnowledge> {
        self.known.iter()
    }

    /// Whether the group knows about anything at all.
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// The contact of `target`, to update.
    pub fn get_mut(&mut self, target: EntityId) -> Option<&mut TargetKnowledge> {
        self.known.iter_mut().find(|k| k.target == target)
    }

    /// Records or updates a contact (`reveal`).
    pub fn insert(&mut self, knowledge: TargetKnowledge) {
        match self.get_mut(knowledge.target) {
            Some(k) => {
                if knowledge.knowledge > k.knowledge {
                    k.knowledge = knowledge.knowledge;
                }
                k.position = knowledge.position;
                k.last_seen = knowledge.last_seen;
            }
            None => self.known.push(knowledge),
        }
    }

    /// Drops one contact (`forgetTarget`).
    pub fn forget(&mut self, target: EntityId) {
        self.known.retain(|k| k.target != target);
    }

    /// Drops the targets not seen for [`FORGET_TIME`] — the group has forgotten them — and
    /// returns the ones it dropped. The gone and the dead the caller forgets with [`Self::forget`],
    /// which needs the World to answer.
    pub fn retain_recent(&mut self, now: f64) -> Vec<EntityId> {
        let mut dropped = Vec::new();
        self.known.retain(|k| {
            let keep = now - k.last_seen <= FORGET_TIME;
            if !keep {
                dropped.push(k.target);
            }
            keep
        });
        dropped
    }
}

/// The physics key of an Entity for line-of-sight queries (`ObjectKey::Entity`).
pub fn target_key(entity: EntityId) -> ObjectKey {
    ObjectKey::Entity(crate::ObjectRef::Entity(entity).to_handle_id())
}
