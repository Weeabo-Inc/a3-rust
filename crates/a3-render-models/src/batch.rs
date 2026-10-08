//! Grouping this frame's visible instances into instanced draws.

use std::ops::Range;

/// One instanced draw: every instance of `key` this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch<K> {
    pub key: K,
    /// Range into the instance array returned by [`InstanceBatcher::finish`].
    pub instances: Range<u32>,
}

/// Collects (key, instance) pairs during culling and sorts them into per-key runs.
#[derive(Debug, Clone)]
pub struct InstanceBatcher<K, T> {
    entries: Vec<(K, T)>,
}

impl<K, T> Default for InstanceBatcher<K, T> {
    fn default() -> Self {
        InstanceBatcher {
            entries: Vec::new(),
        }
    }
}

impl<K: Ord + Copy, T: Copy> InstanceBatcher<K, T> {
    pub fn push(&mut self, key: K, instance: T) {
        self.entries.push((key, instance));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The instances ordered by key (keeping push order within a key) and one batch per key.
    /// Leaves the batcher empty, keeping its allocation.
    pub fn finish(&mut self) -> (Vec<T>, Vec<Batch<K>>) {
        self.entries.sort_by_key(|(k, _)| *k);
        let mut instances = Vec::with_capacity(self.entries.len());
        let mut batches: Vec<Batch<K>> = Vec::new();
        for (i, (key, instance)) in self.entries.drain(..).enumerate() {
            instances.push(instance);
            let i = i as u32;
            match batches.last_mut() {
                Some(b) if b.key == key => b.instances.end = i + 1,
                _ => batches.push(Batch {
                    key,
                    instances: i..i + 1,
                }),
            }
        }
        (instances, batches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_are_grouped_by_key_into_contiguous_ranges() {
        let mut b = InstanceBatcher::default();
        for (key, value) in [(7, 'a'), (3, 'b'), (7, 'c'), (5, 'd'), (3, 'e'), (7, 'f')] {
            b.push(key, value);
        }
        let (instances, batches) = b.finish();
        let groups: Vec<(u32, String)> = batches
            .iter()
            .map(|batch| {
                let r = batch.instances.start as usize..batch.instances.end as usize;
                (batch.key, instances[r].iter().collect())
            })
            .collect();
        assert_eq!(
            groups,
            vec![
                (3, "be".to_owned()),
                (5, "d".to_owned()),
                (7, "acf".to_owned())
            ]
        );
    }

    #[test]
    fn finishing_resets_the_batcher_for_the_next_frame() {
        let mut b = InstanceBatcher::default();
        b.push(1u32, 1.0f32);
        assert_eq!(b.len(), 1);
        let _ = b.finish();
        assert!(b.is_empty());
        let (instances, batches) = b.finish();
        assert!(instances.is_empty() && batches.is_empty());
    }
}
