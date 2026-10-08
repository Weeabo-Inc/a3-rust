//! Round-trip properties of the identifier encodings.

use a3_world::{ClientId, EntitySpec, NetworkId, ObjectRef, SimulationClass, StaticKey, World};
use glam::DVec3;
use proptest::prelude::*;

proptest! {
    #[test]
    fn network_ids_round_trip_through_their_text_form(creator in any::<u32>(), id in any::<u32>()) {
        let net = NetworkId::new(creator, id);
        prop_assert_eq!(net.to_string().parse::<NetworkId>().unwrap(), net);
    }

    #[test]
    fn static_keys_round_trip_through_handle_ids(x in 0u32..1024, z in 0u32..1024, i in 0u32..2048) {
        let key = StaticKey::new(x, z, i).unwrap();
        prop_assert_eq!((key.cell(), key.index()), ((x, z), i));
        let r = ObjectRef::Static(key);
        prop_assert_eq!(ObjectRef::from_handle_id(r.to_handle_id()), Some(r));
        prop_assert_eq!(StaticKey::from_network_id(key.network_id()), Some(key));
    }

    #[test]
    fn entity_handles_stay_distinct_across_deletes(ops in prop::collection::vec(any::<bool>(), 1..200)) {
        let mut world = World::new(ClientId::SERVER);
        let mut live = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for spawn in ops {
            if spawn || live.is_empty() {
                let id = world.spawn(EntitySpec::new("T", SimulationClass::Thing, DVec3::ZERO));
                let h = ObjectRef::Entity(id).to_handle_id();
                prop_assert!(h != 0 && seen.insert(h), "handle reused");
                prop_assert_eq!(ObjectRef::from_handle_id(h), Some(ObjectRef::Entity(id)));
                live.push(id);
            } else {
                let id = live.swap_remove(0);
                prop_assert!(world.delete(id));
            }
        }
    }
}
