//! Spatial and object queries over Entities and Static objects, and attachments.

use std::sync::Arc;

use a3_core::VfsPath;
use glam::DVec3;

use crate::{
    Attachment, EntityId, EntityType, Error, ObjectRef, SimulationClass, StaticKey, World,
};

/// Which Objects a proximity query considers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Near {
    /// Entities and Static objects (`nearestObjects`, `nearObjects`).
    All,
    /// Entities only (run-time objects, including promoted Static objects).
    Entities,
    /// Static objects only, promoted ones included (`nearestTerrainObjects`).
    Statics,
}

impl World {
    /// Whether `object` still exists (an Entity not yet removed, or a Static object not removed).
    pub fn exists(&self, object: ObjectRef) -> bool {
        match object {
            ObjectRef::Entity(id) => self.entity(id).is_some(),
            ObjectRef::Static(key) => self.static_object(key).is_some(),
        }
    }

    /// World-space position of an Entity or Static object.
    pub fn object_position(&self, object: ObjectRef) -> Option<DVec3> {
        match object {
            ObjectRef::Entity(id) => self.entity(id).map(|e| e.position()),
            ObjectRef::Static(key) => self.static_object(key).map(|o| o.position),
        }
    }

    /// The model path of a Static object.
    pub fn static_model(&self, key: StaticKey) -> Option<&VfsPath> {
        let object = self.static_object(key)?;
        self.terrain()?.models.get(object.model_index as usize)
    }

    /// The Entity a Static object was promoted to, or the Static object itself.
    pub fn object_ref_of_static(&self, key: StaticKey) -> ObjectRef {
        match self.promoted().get(&key) {
            Some(&id) => ObjectRef::Entity(id),
            None => ObjectRef::Static(key),
        }
    }

    /// Objects within `radius` of `center` (3D), nearest first. Entities scheduled for deletion
    /// are left out.
    pub fn objects_near(&self, center: DVec3, radius: f64, which: Near) -> Vec<(ObjectRef, f64)> {
        let mut out = Vec::new();
        if which != Near::Statics {
            for e in self.entities() {
                let d = e.position().distance(center);
                let is_static = e.network_id().is_some_and(|n| n.is_static());
                if d <= radius && !e.is_deleted() && (which == Near::All || !is_static) {
                    out.push((ObjectRef::Entity(e.id()), d));
                }
            }
        }
        if which != Near::Entities {
            for o in self.statics().near(center, radius) {
                let r = self.object_ref_of_static(o.key);
                if which == Near::All && matches!(r, ObjectRef::Entity(_)) {
                    continue; // already listed as an Entity
                }
                out.push((r, o.position.distance(center)));
            }
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }

    /// Makes a Static object an Entity so its state can change. The World's model type resolver
    /// ([`set_model_type_resolver`](World::set_model_type_resolver)) gives the type of its config
    /// class when there is one; without a resolver, or for a model no config class uses, it
    /// becomes the plain type named after its model (`Plain`, not simulated).
    pub fn promote_static_with_model_type(&mut self, key: StaticKey) -> Result<EntityId, Error> {
        if let Some(&id) = self.promoted().get(&key) {
            return Ok(id);
        }
        let model = self
            .static_model(key)
            .map(|m| m.as_str().to_owned())
            .unwrap_or_default();
        if let Some(ty) = self
            .model_type_resolver
            .as_mut()
            .and_then(|resolver| resolver.resolve(&model))
        {
            return self.promote_static(key, ty);
        }
        let name = model
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&model)
            .trim_end_matches(".p3d")
            .to_owned();
        self.promote_static(key, Arc::new(EntityType::new(name, SimulationClass::Plain)))
    }

    /// `attachTo`: `id` follows `to` at `offset` (in `to`'s model space) from the next attached
    /// positions phase on. Attaching to itself or to a missing Entity fails.
    pub fn attach(&mut self, id: EntityId, to: EntityId, offset: DVec3) -> Result<(), Error> {
        if id == to || self.entity(to).is_none() {
            return Err(Error::NoSuchEntity(to));
        }
        let e = self.entity_mut(id).ok_or(Error::NoSuchEntity(id))?;
        e.attachment = Some(Attachment { to, offset });
        Ok(())
    }

    /// `detach`.
    pub fn detach(&mut self, id: EntityId) {
        if let Some(e) = self.entity_mut(id) {
            e.attachment = None;
        }
    }

    /// `attachedObjects`: Entities attached to `to`, in arena order.
    pub fn attached_to(&self, to: EntityId) -> Vec<EntityId> {
        self.entities()
            .filter(|e| e.attachment.is_some_and(|a| a.to == to))
            .map(|e| e.id())
            .collect()
    }

    /// Moves every attached Entity to its parent (simulation phase 4). Attachments to Entities
    /// that are gone are dropped.
    pub(crate) fn update_attached_positions(&mut self) {
        let links: Vec<(EntityId, Attachment)> = self
            .entities()
            .filter_map(|e| e.attachment.map(|a| (e.id(), a)))
            .collect();
        for (id, a) in links {
            let parent = self
                .entity(a.to)
                .map(|p| (p.position() + p.orientation() * a.offset, p.orientation()));
            let Some(e) = self.entity_mut(id) else {
                continue;
            };
            match parent {
                Some((position, orientation)) => {
                    e.position = position;
                    e.orientation = orientation;
                }
                None => e.attachment = None,
            }
        }
    }
}
