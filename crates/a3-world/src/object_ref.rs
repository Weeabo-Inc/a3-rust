//! What an SQF `Object` value refers to, and its encoding as an `a3-sqf` handle id.

use crate::world::ENTITY_GENERATION_MASK;
use crate::{EntityId, StaticKey};

/// An Object in the World: an Entity, or a Static object that has not been promoted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectRef {
    Entity(EntityId),
    Static(StaticKey),
}

const STATIC_TAG: u64 = 1 << 63;

impl ObjectRef {
    /// The `id` of an `a3_sqf::Handle` of kind `Object`; never 0 (0 is `objNull`).
    ///
    /// Entities encode as `generation << 32 | (index + 1)`, Static objects as
    /// `1 << 63 | key`.
    pub fn to_handle_id(self) -> u64 {
        match self {
            ObjectRef::Entity(id) => u64::from(id.generation) << 32 | (u64::from(id.index) + 1),
            ObjectRef::Static(key) => STATIC_TAG | u64::from(key.raw()),
        }
    }

    /// The inverse of [`to_handle_id`](Self::to_handle_id); `None` for 0 and malformed ids.
    /// Whether the Object still exists is a question for the World.
    pub fn from_handle_id(handle: u64) -> Option<ObjectRef> {
        if handle & STATIC_TAG != 0 {
            let raw = u32::try_from(handle & !STATIC_TAG).ok()?;
            return StaticKey::from_raw(raw).map(ObjectRef::Static);
        }
        let generation = (handle >> 32) as u32;
        let index = (handle as u32).checked_sub(1)?;
        if generation & !ENTITY_GENERATION_MASK != 0 {
            return None;
        }
        Some(ObjectRef::Entity(EntityId { index, generation }))
    }
}
