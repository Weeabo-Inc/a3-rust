//! World operations on the Static objects of the loaded terrain.

use a3_wrp::Terrain;
use glam::DVec3;

use crate::statics::StaticObjects;
use crate::{
    ClientId, EntityId, EntitySpec, Error, Locality, ObjectRef, StaticKey, StaticObject, World,
};

impl World {
    /// Loads the terrain's placed objects as Static objects, replacing any loaded before.
    pub fn load_terrain(&mut self, terrain: &Terrain) -> Result<(), Error> {
        *self.statics_mut() = StaticObjects::from_terrain(terrain)?;
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

    /// Turns a Static object into an Entity that keeps its Network object ID `{1, key}`. Promoting
    /// twice returns the same Entity.
    ///
    /// _Assumption_ (to confirm in issue #117): Static objects are owned by the server, so the
    /// Entity is Local on the server and Remote (owner server) elsewhere.
    pub fn promote_static(&mut self, key: StaticKey, spec: EntitySpec) -> Result<EntityId, Error> {
        if let Some(&id) = self.promoted().get(&key) {
            return Ok(id);
        }
        if !self.statics().contains(key) {
            return Err(Error::NoSuchStatic(key));
        }
        let locality = if self.local_client() == ClientId::SERVER {
            Locality::Local
        } else {
            Locality::Remote {
                owner: Some(ClientId::SERVER),
            }
        };
        Ok(self.insert_promoted(key, spec, locality))
    }
}
