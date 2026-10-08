//! World operations on the terrain and its Static objects.

use std::sync::Arc;

use a3_wrp::Terrain;
use glam::DVec3;

use crate::statics::StaticObjects;
use crate::{
    ClientId, EntityId, EntityType, Error, Locality, ObjectRef, StaticKey, StaticObject, World,
};

impl World {
    /// Loads the terrain: its heights for placement and its placed objects as Static objects,
    /// replacing any loaded before.
    pub fn load_terrain(&mut self, terrain: Arc<Terrain>) -> Result<(), Error> {
        let statics = StaticObjects::from_terrain(&terrain)?;
        self.set_terrain(terrain, statics);
        Ok(())
    }

    /// Number of Static objects loaded (promoted ones included).
    pub fn static_object_count(&self) -> usize {
        self.statics().len()
    }

    /// The Static object with `key`, unless it was removed.
    pub fn static_object(&self, key: StaticKey) -> Option<&StaticObject> {
        self.statics().get(key)
    }

    /// The Object with WRP Object ID `object_id`, searched from the land cell under `near`
    /// outwards (`nearestObject [position, id]`). A promoted Static object is returned as its
    /// Entity.
    pub fn find_static(&self, near: DVec3, object_id: u32) -> Option<ObjectRef> {
        let key = self.statics().find(near, object_id)?;
        Some(match self.promoted().get(&key) {
            Some(&id) => ObjectRef::Entity(id),
            None => ObjectRef::Static(key),
        })
    }

    /// Turns a Static object into an Entity of `entity_type` that keeps its Network object ID
    /// `{1, key}` and its position. Promoting twice returns the same Entity.
    ///
    /// _Assumption_ (to confirm in issue #117): Static objects are owned by the server, so the
    /// Entity is Local on the server and Remote (owner server) elsewhere.
    pub fn promote_static(
        &mut self,
        key: StaticKey,
        entity_type: Arc<EntityType>,
    ) -> Result<EntityId, Error> {
        if let Some(&id) = self.promoted().get(&key) {
            return Ok(id);
        }
        let position = self
            .statics()
            .get(key)
            .ok_or(Error::NoSuchStatic(key))?
            .position;
        let locality = if self.local_client() == ClientId::SERVER {
            Locality::Local
        } else {
            Locality::Remote {
                owner: Some(ClientId::SERVER),
            }
        };
        Ok(self.insert_promoted(key, entity_type, position, locality))
    }
}
