//! The merged config: every addon's [`Config`] patched together in load order, with
//! inheritance resolved at lookup time.
//!
//! Semantics (see `docs/re/config.md`; items marked there as assumptions need RE confirmation):
//! - Merging a class into an existing same-named class merges entries recursively. Values are
//!   replaced in place; new entries are appended. The class's base becomes the patch's base (a
//!   patch without `: Base` removes inheritance), with an "Updating base class" warning when it
//!   changes.
//! - `class X;` adds a placeholder only when X does not exist. A placeholder is skipped by
//!   lookups (they continue into the base chain), so it acts as a reference to an inherited or
//!   later-defined class.
//! - `delete X;` removes X unless some class uses it as its base.
//! - `x[] += {...}` extends an own array in place; otherwise it is kept and resolved at lookup as
//!   inherited value + items.
//! - A base name is looked up in the enclosing class (its own entries, then its base chain), then
//!   in the enclosing class's enclosing class, and so on up to the root; the class itself is never
//!   its own base (`class Turrets: Turrets` finds the inherited `Turrets`).

use std::collections::HashMap;
use std::ops::Shr;

use crate::{Config, ConfigClass, Entry, EntryKind, Value, text::parse_number};

/// Index of a node in a [`ConfigTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

/// Bound on base-chain walks; breaks inheritance cycles in malformed configs.
const MAX_CHAIN: usize = 64;

#[derive(Debug)]
struct Node {
    name: String,
    parent: Option<NodeId>,
    kind: NodeKind,
}

#[derive(Debug)]
enum NodeKind {
    Class(ClassNode),
    Value(Value),
    ArrayAppend(Vec<Value>),
}

#[derive(Debug, Default)]
struct ClassNode {
    base: Option<String>,
    /// Declared with `class X;` and not (yet) defined.
    external: bool,
    entries: Vec<NodeId>,
    index: HashMap<String, NodeId>,
}

/// The merged, queryable config of a whole game session (`configFile`).
#[derive(Debug)]
pub struct ConfigTree {
    nodes: Vec<Node>,
    /// Lower-cased base name -> classes declaring it; used to refuse deleting referenced classes.
    by_base: HashMap<String, Vec<NodeId>>,
    warnings: Vec<String>,
    /// Prefix of [`ConfigRef::path_string`], e.g. `bin\config.bin`.
    root_name: String,
}

const ROOT: NodeId = NodeId(0);

fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl Default for ConfigTree {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigTree {
    /// An empty tree, as `configFile` before any addon is loaded.
    pub fn new() -> Self {
        Self::with_root_name("bin\\config.bin")
    }

    /// An empty tree whose paths print under `root_name` (e.g. a mission's `description.ext`).
    pub fn with_root_name(root_name: &str) -> Self {
        Self {
            nodes: vec![Node {
                name: String::new(),
                parent: None,
                kind: NodeKind::Class(ClassNode::default()),
            }],
            by_base: HashMap::new(),
            warnings: Vec::new(),
            root_name: root_name.to_owned(),
        }
    }

    /// A tree holding just one config file.
    pub fn from_config(config: &Config) -> Self {
        let mut tree = Self::new();
        tree.merge(config);
        tree
    }

    /// Patches `config` into the tree, as the engine does for each addon in load order.
    pub fn merge(&mut self, config: &Config) {
        self.merge_class(ROOT, &config.root);
    }

    /// Diagnostics produced while merging (base class changes, refused deletes, ...).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The root class (`configFile`).
    pub fn root(&self) -> ConfigRef<'_> {
        ConfigRef {
            tree: self,
            path: vec![ROOT],
        }
    }

    /// The entry reached by `path` (node ids from the root, as returned by
    /// [`ConfigRef::node_path`]), or the null config when the path is empty or does not
    /// belong to this tree. Lets a script VM store configs as plain ids.
    pub fn from_node_path(&self, path: &[NodeId]) -> ConfigRef<'_> {
        let valid =
            path.first() == Some(&ROOT) && path.iter().all(|id| (id.0 as usize) < self.nodes.len());
        ConfigRef {
            tree: self,
            path: if valid { path.to_vec() } else { Vec::new() },
        }
    }

    /// Number of nodes ever created (classes and values, including deleted ones).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn class(&self, id: NodeId) -> Option<&ClassNode> {
        match &self.nodes[id.0 as usize].kind {
            NodeKind::Class(c) => Some(c),
            _ => None,
        }
    }

    fn class_mut(&mut self, id: NodeId) -> &mut ClassNode {
        match &mut self.nodes[id.0 as usize].kind {
            NodeKind::Class(c) => c,
            _ => unreachable!("node is not a class"),
        }
    }

    fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    fn is_real_class(&self, id: NodeId) -> bool {
        self.class(id).is_some_and(|c| !c.external)
    }

    fn add(&mut self, parent: NodeId, name: &str, kind: NodeKind) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("config tree too large"));
        if let NodeKind::Class(c) = &kind {
            if let Some(base) = &c.base {
                self.by_base.entry(key(base)).or_default().push(id);
            }
        }
        self.nodes.push(Node {
            name: name.to_owned(),
            parent: Some(parent),
            kind,
        });
        let class = self.class_mut(parent);
        class.entries.push(id);
        class.index.insert(key(name), id);
        id
    }

    /// Replaces the node `old` in its parent's entry list with a fresh node, keeping position.
    fn replace(&mut self, parent: NodeId, old: NodeId, name: &str, kind: NodeKind) -> NodeId {
        self.forget_base(old);
        let id = self.add(parent, name, kind);
        let class = self.class_mut(parent);
        class.entries.pop();
        if let Some(slot) = class.entries.iter_mut().find(|e| **e == old) {
            *slot = id;
        }
        id
    }

    fn path_of(&self, id: NodeId) -> String {
        let mut names = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let node = self.node(c);
            if node.parent.is_some() {
                names.push(node.name.as_str());
            }
            cur = node.parent;
        }
        names.push(&self.root_name);
        names.reverse();
        names.join("/")
    }

    fn merge_class(&mut self, target: NodeId, patch: &ConfigClass) {
        for entry in &patch.entries {
            let existing = self
                .class(target)
                .and_then(|c| c.index.get(&key(&entry.name)).copied());
            match &entry.kind {
                EntryKind::Class(body) => {
                    let id = match existing {
                        Some(id) if self.class(id).is_some() => {
                            self.set_base(id, body.base.as_deref());
                            id
                        }
                        Some(old) => self.replace(target, old, &entry.name, new_class(body)),
                        None => self.add(target, &entry.name, new_class(body)),
                    };
                    self.merge_class(id, body);
                }
                EntryKind::External => {
                    if existing.is_none() {
                        let kind = NodeKind::Class(ClassNode {
                            external: true,
                            ..ClassNode::default()
                        });
                        self.add(target, &entry.name, kind);
                    }
                }
                EntryKind::Delete => {
                    if let Some(id) = existing {
                        self.delete(target, id);
                    }
                }
                EntryKind::Value(value) => {
                    let kind = NodeKind::Value(value.clone());
                    match existing {
                        Some(old) => {
                            self.replace(target, old, &entry.name, kind);
                        }
                        None => {
                            self.add(target, &entry.name, kind);
                        }
                    }
                }
                EntryKind::ArrayAppend(items) => match existing {
                    Some(id) => match &mut self.nodes[id.0 as usize].kind {
                        NodeKind::Value(Value::Array(own)) | NodeKind::ArrayAppend(own) => {
                            own.extend(items.iter().cloned());
                        }
                        _ => {
                            let kind = NodeKind::ArrayAppend(items.clone());
                            self.replace(target, id, &entry.name, kind);
                        }
                    },
                    None => {
                        self.add(target, &entry.name, NodeKind::ArrayAppend(items.clone()));
                    }
                },
            }
        }
    }

    fn set_base(&mut self, id: NodeId, base: Option<&str>) {
        let class = self.class(id).expect("class");
        let was_external = class.external;
        let old = class.base.clone();
        let changed = match (&old, base) {
            (Some(a), Some(b)) => !a.eq_ignore_ascii_case(b),
            (None, None) => false,
            _ => true,
        };
        if changed && !was_external {
            self.warnings.push(format!(
                "Updating base class {}->{}, by {}",
                old.as_deref().unwrap_or(""),
                base.unwrap_or(""),
                self.path_of(id)
            ));
        }
        if changed {
            self.forget_base(id);
            if let Some(new) = base {
                self.by_base.entry(key(new)).or_default().push(id);
            }
        }
        let class = self.class_mut(id);
        class.external = false;
        class.base = base.map(str::to_owned);
    }

    fn forget_base(&mut self, id: NodeId) {
        if let Some(old) = self.class(id).and_then(|c| c.base.as_deref()).map(key) {
            if let Some(list) = self.by_base.get_mut(&old) {
                list.retain(|&n| n != id);
            }
        }
    }

    /// Whether `id` is still reachable from the root (not inside a deleted or replaced subtree).
    fn is_attached(&self, id: NodeId) -> bool {
        let mut cur = id;
        while let Some(parent) = self.node(cur).parent {
            let name = key(&self.node(cur).name);
            if self.class(parent).and_then(|c| c.index.get(&name)) != Some(&cur) {
                return false;
            }
            cur = parent;
        }
        true
    }

    fn delete(&mut self, parent: NodeId, id: NodeId) {
        let name = self.node(id).name.clone();
        if self.class(id).is_some() {
            let users = self.by_base.get(&key(&name)).cloned().unwrap_or_default();
            if users
                .iter()
                .any(|&u| self.is_attached(u) && self.resolve_base(u) == Some(id))
            {
                self.warnings.push(format!(
                    "Cannot delete class {name}, it is referenced somewhere (used as a base class \
                     probably), at {}",
                    self.path_of(id)
                ));
                return;
            }
        }
        let class = self.class_mut(parent);
        class.entries.retain(|&e| e != id);
        class.index.remove(&key(&name));
    }

    /// Finds `name` in class `class` or its base chain, skipping placeholders and `skip`.
    fn find(&self, class: NodeId, name: &str, skip: Option<NodeId>) -> Option<NodeId> {
        let k = key(name);
        let mut cur = Some(class);
        for _ in 0..MAX_CHAIN {
            let c = cur?;
            let cn = self.class(c)?;
            if let Some(&id) = cn.index.get(&k) {
                if Some(id) != skip && self.class(id).is_none_or(|x| !x.external) {
                    return Some(id);
                }
            }
            cur = self.resolve_base(c);
        }
        None
    }

    /// The class `id` inherits from, resolved through the enclosing scopes.
    fn resolve_base(&self, id: NodeId) -> Option<NodeId> {
        self.resolve_base_depth(id, 0)
    }

    fn resolve_base_depth(&self, id: NodeId, depth: usize) -> Option<NodeId> {
        if depth > MAX_CHAIN {
            return None;
        }
        let base = self.class(id)?.base.as_deref()?;
        let k = key(base);
        let mut scope = self.node(id).parent;
        while let Some(s) = scope {
            // Search the scope's own entries, then its base chain, never returning `id` itself.
            let mut cur = Some(s);
            for _ in 0..MAX_CHAIN {
                let Some(c) = cur else { break };
                let Some(cn) = self.class(c) else { break };
                if let Some(&found) = cn.index.get(&k) {
                    if found != id && self.is_real_class(found) {
                        return Some(found);
                    }
                }
                cur = self.resolve_base_depth(c, depth + 1);
            }
            scope = self.node(s).parent;
        }
        None
    }

    /// The effective value of a value or `+=` node: own items appended to the inherited array.
    fn value_of(&self, id: NodeId) -> Option<Value> {
        self.value_of_depth(id, 0)
    }

    fn value_of_depth(&self, id: NodeId, depth: usize) -> Option<Value> {
        match &self.node(id).kind {
            NodeKind::Value(v) => Some(v.clone()),
            NodeKind::ArrayAppend(items) => {
                let mut out = Vec::new();
                if depth < MAX_CHAIN {
                    let node = self.node(id);
                    let inherited = node
                        .parent
                        .and_then(|p| self.resolve_base(p))
                        .and_then(|b| self.find(b, &node.name, None))
                        .and_then(|i| self.value_of_depth(i, depth + 1));
                    if let Some(Value::Array(base)) = inherited {
                        out = base;
                    }
                }
                out.extend(items.iter().cloned());
                Some(Value::Array(out))
            }
            NodeKind::Class(_) => None,
        }
    }
}

fn new_class(body: &ConfigClass) -> NodeKind {
    NodeKind::Class(ClassNode {
        base: body.base.clone(),
        ..ClassNode::default()
    })
}

/// How [`ConfigRef::export`] turns part of the merged tree back into a [`Config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    /// Own entries only, as left by patching: declared bases, `class X;` placeholders and
    /// unresolved `+=` appends are kept.
    Merged,
    /// Inheritance flattened: own and inherited entries, `+=` applied, no bases.
    Resolved,
}

/// A config entry reached by a path from the root, like an SQF `Config` value.
///
/// The access path is kept: an inherited subclass reached through a derived class reports the
/// derived path (`configHierarchy`, `str`), while its own base is resolved where it is defined.
/// A reference to nothing is the null config (`configNull`).
#[derive(Clone)]
pub struct ConfigRef<'a> {
    tree: &'a ConfigTree,
    /// Node ids from the root to this entry; empty for the null config.
    path: Vec<NodeId>,
}

impl std::fmt::Debug for ConfigRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.path_string())
    }
}

impl PartialEq for ConfigRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.tree, other.tree) && self.path == other.path
    }
}

impl<'a> ConfigRef<'a> {
    fn null(tree: &'a ConfigTree) -> Self {
        Self {
            tree,
            path: Vec::new(),
        }
    }

    fn at(tree: &'a ConfigTree, id: NodeId) -> Self {
        let mut path = vec![id];
        let mut cur = tree.node(id).parent;
        while let Some(p) = cur {
            path.push(p);
            cur = tree.node(p).parent;
        }
        path.reverse();
        Self { tree, path }
    }

    fn id(&self) -> Option<NodeId> {
        self.path.last().copied()
    }

    /// `config >> name`: the entry `name` of this class, searching the base chain.
    pub fn get(&self, name: &str) -> ConfigRef<'a> {
        let Some(id) = self.id() else {
            return Self::null(self.tree);
        };
        match self.tree.find(id, name, None) {
            Some(found) => {
                let mut path = self.path.clone();
                path.push(found);
                Self {
                    tree: self.tree,
                    path,
                }
            }
            None => Self::null(self.tree),
        }
    }

    /// The access path as node ids from the root (empty for null); see
    /// [`ConfigTree::from_node_path`].
    pub fn node_path(&self) -> &[NodeId] {
        &self.path
    }

    /// `isNull` for configs.
    pub fn is_null(&self) -> bool {
        self.path.is_empty()
    }

    /// `configName`: the entry's name in its original case; empty for the root and null.
    pub fn name(&self) -> &'a str {
        match self.id() {
            Some(id) => &self.tree.node(id).name,
            None => "",
        }
    }

    fn value(&self) -> Option<Value> {
        self.tree.value_of(self.id()?)
    }

    /// `isClass`.
    pub fn is_class(&self) -> bool {
        self.id().is_some_and(|id| self.tree.is_real_class(id))
    }

    /// `isNumber`: the entry holds a number (strings that look numeric do not count).
    pub fn is_number(&self) -> bool {
        matches!(
            self.value(),
            Some(Value::Int(_) | Value::Float(_) | Value::Int64(_))
        )
    }

    /// `isText`.
    pub fn is_text(&self) -> bool {
        matches!(self.value(), Some(Value::String(_) | Value::Expression(_)))
    }

    /// `isArray`.
    pub fn is_array(&self) -> bool {
        matches!(self.value(), Some(Value::Array(_)))
    }

    /// `getNumber`. Numbers convert directly. Strings are parsed as a number literal or
    /// `true`/`false`; other strings give 0 (the engine evaluates them as an expression — not
    /// implemented yet). Everything else gives 0.
    pub fn number(&self) -> f32 {
        match self.value() {
            Some(v) => value_number(&v),
            None => 0.0,
        }
    }

    /// `getText`. Numbers are formatted as text _(assumption)_; arrays, classes and missing
    /// entries give `""`.
    pub fn text(&self) -> String {
        match self.value() {
            Some(Value::String(s) | Value::Expression(s)) => s,
            Some(Value::Int(i)) => i.to_string(),
            Some(Value::Int64(i)) => i.to_string(),
            Some(Value::Float(f)) => f.to_string(),
            _ => String::new(),
        }
    }

    /// `getArray`: the array (with `+=` appends applied), or empty.
    pub fn array(&self) -> Vec<Value> {
        match self.value() {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        }
    }

    /// The base class name as written (`class X: Base`), before resolution.
    pub fn declared_base(&self) -> Option<&'a str> {
        self.tree.class(self.id()?)?.base.as_deref()
    }

    /// `inheritsFrom`: the base class at its definition path, or null.
    pub fn inherits_from(&self) -> ConfigRef<'a> {
        match self.id().and_then(|id| self.tree.resolve_base(id)) {
            Some(base) => Self::at(self.tree, base),
            None => Self::null(self.tree),
        }
    }

    /// The base chain, nearest first (like `BIS_fnc_returnParents` without the class itself).
    pub fn bases(&self) -> Vec<ConfigRef<'a>> {
        let mut out = Vec::new();
        let mut cur = self.inherits_from();
        while !cur.is_null() && out.len() < MAX_CHAIN {
            let next = cur.inherits_from();
            out.push(cur);
            cur = next;
        }
        out
    }

    /// `configHierarchy`: every config from the root down to this one along the access path.
    pub fn hierarchy(&self) -> Vec<ConfigRef<'a>> {
        (1..=self.path.len())
            .map(|n| Self {
                tree: self.tree,
                path: self.path[..n].to_vec(),
            })
            .collect()
    }

    /// The enclosing class along the access path, or null at the root.
    pub fn parent(&self) -> ConfigRef<'a> {
        let mut path = self.path.clone();
        path.pop();
        Self {
            tree: self.tree,
            path,
        }
    }

    /// The own entry ids of a class that [`entries`](Self::entries) lists.
    fn own_entry_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.id()
            .and_then(|id| self.tree.class(id))
            .into_iter()
            .flat_map(|class| class.entries.iter().copied())
            .filter(|&e| self.tree.class(e).is_none_or(|c| !c.external))
    }

    /// `count config`: the number of [`entries`](Self::entries), without building them.
    pub fn entry_count(&self) -> usize {
        self.own_entry_ids().count()
    }

    /// `config select index`: entry `index` of [`entries`](Self::entries), or null.
    pub fn entry_at(&self, index: usize) -> ConfigRef<'a> {
        match self.own_entry_ids().nth(index) {
            Some(e) => {
                let mut path = self.path.clone();
                path.push(e);
                Self {
                    tree: self.tree,
                    path,
                }
            }
            None => Self::null(self.tree),
        }
    }

    /// Own entries in order (`count` / `select` on a config), excluding inherited ones and
    /// unresolved `class X;` placeholders.
    pub fn entries(&self) -> Vec<ConfigRef<'a>> {
        let Some(class) = self.id().and_then(|id| self.tree.class(id)) else {
            return Vec::new();
        };
        class
            .entries
            .iter()
            .filter(|&&e| self.tree.class(e).is_none_or(|c| !c.external))
            .map(|&e| {
                let mut path = self.path.clone();
                path.push(e);
                Self {
                    tree: self.tree,
                    path,
                }
            })
            .collect()
    }

    /// This entry (a class with its subtree, or a value) as a standalone [`Config`] that
    /// [`crate::write_text`] can print. The root exports as its entries. `None` for null.
    pub fn export(&self, mode: ExportMode) -> Option<Config> {
        let id = self.id()?;
        if self.path.len() == 1 {
            let mut root = self.export_class(id, mode, 0);
            root.base = None;
            return Some(Config {
                root,
                enums: Vec::new(),
            });
        }
        let entry = self.export_entry(id, mode, 0)?;
        Some(Config {
            root: ConfigClass {
                base: None,
                entries: vec![entry],
            },
            enums: Vec::new(),
        })
    }

    fn export_entry(&self, id: NodeId, mode: ExportMode, depth: usize) -> Option<Entry> {
        let tree = self.tree;
        let node = tree.node(id);
        let kind = match (&node.kind, mode) {
            (NodeKind::Class(c), ExportMode::Merged) if c.external => EntryKind::External,
            (NodeKind::Class(_), _) => EntryKind::Class(self.export_class(id, mode, depth + 1)),
            (NodeKind::ArrayAppend(items), ExportMode::Merged) => {
                EntryKind::ArrayAppend(items.clone())
            }
            (_, _) => EntryKind::Value(tree.value_of(id)?),
        };
        Some(Entry::new(node.name.clone(), kind))
    }

    fn export_class(&self, id: NodeId, mode: ExportMode, depth: usize) -> ConfigClass {
        let tree = self.tree;
        // Inherited subclasses can nest without bound in a malformed config.
        if depth > MAX_CHAIN {
            return ConfigClass::default();
        }
        match mode {
            ExportMode::Merged => {
                let class = tree.class(id).expect("class");
                ConfigClass {
                    base: class.base.clone(),
                    entries: class
                        .entries
                        .iter()
                        .filter_map(|&e| self.export_entry(e, mode, depth))
                        .collect(),
                }
            }
            ExportMode::Resolved => ConfigClass {
                base: None,
                entries: ConfigRef::at(tree, id)
                    .entries_with_inherited()
                    .iter()
                    .filter_map(|e| self.export_entry(e.id()?, mode, depth))
                    .collect(),
            },
        }
    }

    /// Own and inherited entries (`configProperties` with inheritance): own entries first, then
    /// each base's entries not already present.
    pub fn entries_with_inherited(&self) -> Vec<ConfigRef<'a>> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        let chain = std::iter::once(self.clone()).chain(self.bases());
        for class in chain {
            for e in class.entries() {
                if seen.insert(key(e.name())) {
                    let mut path = self.path.clone();
                    path.push(e.id().expect("entry"));
                    out.push(Self {
                        tree: self.tree,
                        path,
                    });
                }
            }
        }
        out
    }

    /// `str config`: e.g. `bin\config.bin/CfgVehicles/B_Soldier_F`; empty for null.
    pub fn path_string(&self) -> String {
        if self.is_null() {
            return String::new();
        }
        let mut s = self.tree.root_name.clone();
        for &id in &self.path[1..] {
            s.push('/');
            s.push_str(&self.tree.node(id).name);
        }
        s
    }
}

fn value_number(v: &Value) -> f32 {
    match v {
        Value::Int(i) => *i as f32,
        Value::Int64(i) => *i as f32,
        Value::Float(f) => *f,
        Value::String(s) | Value::Expression(s) => {
            let s = s.trim();
            if s.eq_ignore_ascii_case("true") {
                1.0
            } else if s.eq_ignore_ascii_case("false") {
                0.0
            } else {
                match parse_number(s) {
                    Some(Value::Int(i)) => i as f32,
                    Some(Value::Int64(i)) => i as f32,
                    Some(Value::Float(f)) => f,
                    _ => 0.0,
                }
            }
        }
        Value::Array(_) => 0.0,
    }
}

impl<'a> Shr<&str> for ConfigRef<'a> {
    type Output = ConfigRef<'a>;

    fn shr(self, name: &str) -> ConfigRef<'a> {
        self.get(name)
    }
}

impl<'a> Shr<&str> for &ConfigRef<'a> {
    type Output = ConfigRef<'a>;

    fn shr(self, name: &str) -> ConfigRef<'a> {
        self.get(name)
    }
}
