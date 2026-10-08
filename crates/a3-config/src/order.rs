//! Addon config load order from CfgPatches `requiredAddons`.
//!
//! Assumed engine behaviour (see `docs/re/config.md`): addons are discovered in a fixed order
//! (mod folders in load order, PBOs alphabetically within each). Their configs are then merged so
//! that every addon comes after the addons providing its `requiredAddons`; addons with no
//! dependency relation keep their discovery order. Requirements that no addon provides are
//! reported and ignored; dependency cycles are broken by loading the earliest-discovered addon of
//! the cycle first.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use crate::{Config, EntryKind, Value};

/// The CfgPatches declarations of one addon's config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddonPatches {
    /// Names of the CfgPatches classes the addon declares.
    pub patches: Vec<String>,
    /// Union of their `requiredAddons`, first occurrence order, duplicates removed.
    pub required: Vec<String>,
}

impl AddonPatches {
    /// Reads `CfgPatches` classes and their `requiredAddons[]` from an addon config.
    pub fn from_config(config: &Config) -> Self {
        let mut out = Self::default();
        let Some(patches) = config.root.class("CfgPatches") else {
            return out;
        };
        for entry in &patches.entries {
            let EntryKind::Class(class) = &entry.kind else {
                continue;
            };
            out.patches.push(entry.name.clone());
            let required = class.entries.iter().find_map(|e| match &e.kind {
                EntryKind::Value(Value::Array(items)) | EntryKind::ArrayAppend(items)
                    if e.name.eq_ignore_ascii_case("requiredAddons") =>
                {
                    Some(items)
                }
                _ => None,
            });
            for item in required.into_iter().flatten() {
                if let Value::String(name) = item {
                    if !out.required.iter().any(|r| r.eq_ignore_ascii_case(name)) {
                        out.required.push(name.clone());
                    }
                }
            }
        }
        out
    }
}

/// Result of [`load_order`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoadOrder {
    /// Indices into the input, in merge order.
    pub order: Vec<usize>,
    /// `(addon index, required name)` for requirements no addon provides.
    pub missing: Vec<(usize, String)>,
    /// Addons force-loaded to break a dependency cycle.
    pub cycles: Vec<usize>,
}

/// Orders addons so each is merged after the addons it requires. Stable: among addons whose
/// requirements are met, the one discovered first loads first.
pub fn load_order(addons: &[AddonPatches]) -> LoadOrder {
    let mut provider: HashMap<String, usize> = HashMap::new();
    for (i, a) in addons.iter().enumerate() {
        for p in &a.patches {
            provider.entry(p.to_ascii_lowercase()).or_insert(i);
        }
    }

    let mut result = LoadOrder::default();
    let mut pending = vec![0usize; addons.len()];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); addons.len()];
    for (i, a) in addons.iter().enumerate() {
        let mut deps: Vec<usize> = Vec::new();
        for r in &a.required {
            match provider.get(&r.to_ascii_lowercase()) {
                Some(&p) if p != i => {
                    if !deps.contains(&p) {
                        deps.push(p);
                    }
                }
                Some(_) => {}
                None => result.missing.push((i, r.clone())),
            }
        }
        pending[i] = deps.len();
        for d in deps {
            dependents[d].push(i);
        }
    }

    let mut ready: BinaryHeap<Reverse<usize>> = (0..addons.len())
        .filter(|&i| pending[i] == 0)
        .map(Reverse)
        .collect();
    let mut loaded = vec![false; addons.len()];
    while result.order.len() < addons.len() {
        let next = match ready.pop() {
            Some(Reverse(i)) if loaded[i] => continue,
            Some(Reverse(i)) => i,
            None => {
                let i = (0..addons.len())
                    .find(|&i| !loaded[i])
                    .expect("unloaded addon remains");
                result.cycles.push(i);
                i
            }
        };
        loaded[next] = true;
        result.order.push(next);
        for &d in &dependents[next] {
            pending[d] = pending[d].saturating_sub(1);
            if pending[d] == 0 && !loaded[d] {
                ready.push(Reverse(d));
            }
        }
    }
    result
}
