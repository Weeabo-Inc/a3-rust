//! SQF config commands (`configFile`, `>>`, `getNumber`, `configClasses`, ...) over the
//! merged [`ConfigTree`]s.
//!
//! A config value in the VM is an opaque `Handle(HandleKind::Config, id)`. [`SqfConfigs`] owns
//! the three config roots (`configFile`, `missionConfigFile`, `campaignConfigFile`) and interns
//! each access path to one id, so the same entry reached twice is the same handle (`==` and
//! hash map keys work). Id 0 is `configNull`.
//!
//! A host opts in by implementing [`ConfigHost`] and registering [`register_config_commands`]
//! into its [`Registry`]:
//!
//! ```
//! use std::sync::Arc;
//! use a3_config::{ConfigTree, parse_text};
//! use a3_gamedata::{ConfigHost, SqfConfigs, VfsHost, register_config_commands};
//! use a3_sqf::{Registry, Vm};
//!
//! let cfg = parse_text("class CfgThings { class Box { mass = 5; }; };").unwrap();
//! let mut host = VfsHost::default();
//! host.configs = SqfConfigs::new(Arc::new(ConfigTree::from_config(&cfg)));
//! let mut reg = Registry::with_core();
//! register_config_commands(&mut reg);
//! let mut vm = Vm::with_registry(host, std::rc::Rc::new(reg));
//! let v = vm.eval("getNumber (configFile >> \"CfgThings\" >> \"Box\" >> \"mass\")").unwrap();
//! assert_eq!(v.to_sqf_string(), "5");
//! ```
//!
//! Behaviour (community wiki, `docs/re/sqf-semantics.md`):
//! - `getText` localizes `$STR_` values through [`Host::localize`]; `getTextRaw` does not.
//! - `getNumber` of a text entry that is not a number literal evaluates it as an SQF expression,
//!   as the engine does for entries such as `"(1 + 2)"`.
//! - `count` and `select` see a class's own entries; `configClasses` own subclasses;
//!   `configProperties` own and (by default) inherited entries.
//! - `getMissionConfigValue` reads `missionConfigFile`; scenario attributes set in the Eden
//!   Editor (mission.sqm) are not consulted yet.

use std::collections::HashMap;
use std::sync::Arc;

use a3_config::{ConfigRef, ConfigTree, NodeId, Value as CfgValue};
use a3_sqf::vm::Ctx;
use a3_sqf::{Handle, HandleKind, Host, Registry, SqfError, Sym, Type, TypeSet, Value};

/// Which config root a handle belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigRoot {
    /// `configFile`: the merged addon config.
    Game,
    /// `missionConfigFile`: the mission's description.ext.
    Mission,
    /// `campaignConfigFile`: the campaign's description.ext.
    Campaign,
}

impl ConfigRoot {
    fn index(self) -> usize {
        match self {
            ConfigRoot::Game => 0,
            ConfigRoot::Mission => 1,
            ConfigRoot::Campaign => 2,
        }
    }
}

/// The config roots a script can reach and the interned config handles.
#[derive(Debug)]
pub struct SqfConfigs {
    trees: [Arc<ConfigTree>; 3],
    paths: Vec<(ConfigRoot, Vec<NodeId>)>,
    ids: HashMap<(ConfigRoot, Vec<NodeId>), u64>,
}

impl Default for SqfConfigs {
    fn default() -> Self {
        SqfConfigs::new(Arc::new(ConfigTree::new()))
    }
}

impl SqfConfigs {
    /// Configs with `config_file` as `configFile` and empty mission and campaign configs.
    pub fn new(config_file: Arc<ConfigTree>) -> Self {
        SqfConfigs {
            trees: [
                config_file,
                Arc::new(ConfigTree::with_root_name("description.ext")),
                // No campaign loaded: `str campaignConfigFile` is "".
                Arc::new(ConfigTree::with_root_name("")),
            ],
            paths: Vec::new(),
            ids: HashMap::new(),
        }
    }

    /// Replaces a config root (e.g. when a mission loads its description.ext). Handles into
    /// the old tree become `configNull`.
    pub fn set(&mut self, root: ConfigRoot, tree: Arc<ConfigTree>) {
        self.trees[root.index()] = tree;
        let stale: Vec<u64> = self
            .ids
            .iter()
            .filter(|((r, _), _)| *r == root)
            .map(|(_, id)| *id)
            .collect();
        for id in stale {
            self.paths[(id - 1) as usize].1.clear();
        }
        self.ids.retain(|(r, _), _| *r != root);
    }

    /// The tree of a config root.
    pub fn tree(&self, root: ConfigRoot) -> &Arc<ConfigTree> {
        &self.trees[root.index()]
    }

    /// The handle for an entry of `root`'s tree.
    pub fn handle(&mut self, root: ConfigRoot, entry: &ConfigRef<'_>) -> Handle {
        self.intern(root, entry.node_path().to_vec())
    }

    fn intern(&mut self, root: ConfigRoot, path: Vec<NodeId>) -> Handle {
        if path.is_empty() {
            return Handle::null(HandleKind::Config);
        }
        let key = (root, path);
        if let Some(&id) = self.ids.get(&key) {
            return Handle {
                kind: HandleKind::Config,
                id,
            };
        }
        self.paths.push(key.clone());
        let id = self.paths.len() as u64;
        self.ids.insert(key, id);
        Handle {
            kind: HandleKind::Config,
            id,
        }
    }

    /// The root and node path of a handle; `None` for `configNull` and unknown ids.
    pub fn resolve(&self, handle: Handle) -> Option<(ConfigRoot, Arc<ConfigTree>, Vec<NodeId>)> {
        if handle.kind != HandleKind::Config || handle.id == 0 {
            return None;
        }
        let (root, path) = self.paths.get((handle.id - 1) as usize)?;
        if path.is_empty() {
            return None;
        }
        Some((*root, self.trees[root.index()].clone(), path.clone()))
    }

    /// `str config`: `bin\config.bin/CfgVehicles/Car`, `"<NULL-config>"` for `configNull`.
    pub fn format(&self, handle: Handle) -> String {
        match self.resolve(handle) {
            Some((_, tree, path)) => tree.from_node_path(&path).path_string(),
            None if handle.kind == HandleKind::Config => "<NULL-config>".to_owned(),
            None => String::new(),
        }
    }
}

/// A script host that can reach the merged configs.
///
/// Implementors should also route [`Host::format_handle`] for config handles to
/// [`SqfConfigs::format`] so that `str` prints config paths.
pub trait ConfigHost: Host {
    fn configs(&self) -> &SqfConfigs;
    fn configs_mut(&mut self) -> &mut SqfConfigs;

    /// The language `localize` resolves for (`language`), e.g. `English`.
    fn language(&self) -> &str {
        a3_stringtable::ENGLISH
    }
}

const CONFIG: TypeSet = TypeSet::of(Type::Config);
const STR: TypeSet = TypeSet::of(Type::String);
const NUM: TypeSet = TypeSet::NUMBER;
const BOOL: TypeSet = TypeSet::of(Type::Bool);
const ARR: TypeSet = TypeSet::of(Type::Array);

fn handle_of(v: &Value) -> Handle {
    match v {
        Value::Handle(h) => *h,
        _ => Handle::null(HandleKind::Config),
    }
}

fn config_value(h: Handle) -> Value {
    Value::Handle(h)
}

/// Runs `f` on the entry behind `v` (null when it does not resolve).
fn with_entry<H: ConfigHost, R>(
    ctx: &Ctx<'_, H>,
    v: &Value,
    f: impl FnOnce(Option<(ConfigRoot, &ConfigRef<'_>)>) -> R,
) -> R {
    match ctx.host.configs().resolve(handle_of(v)) {
        Some((root, tree, path)) => {
            let entry = tree.from_node_path(&path);
            f(Some((root, &entry)))
        }
        None => f(None),
    }
}

/// Maps entries of `v` to handles: `pick` returns node paths in the same root.
fn related<H: ConfigHost>(
    ctx: &mut Ctx<'_, H>,
    v: &Value,
    pick: impl FnOnce(&ConfigRef<'_>) -> Vec<Vec<NodeId>>,
) -> Vec<Handle> {
    let Some((root, tree, path)) = ctx.host.configs().resolve(handle_of(v)) else {
        return Vec::new();
    };
    let paths = pick(&tree.from_node_path(&path));
    let configs = ctx.host.configs_mut();
    paths.into_iter().map(|p| configs.intern(root, p)).collect()
}

fn related_one<H: ConfigHost>(
    ctx: &mut Ctx<'_, H>,
    v: &Value,
    pick: impl FnOnce(&ConfigRef<'_>) -> Vec<NodeId>,
) -> Value {
    let h = related(ctx, v, |e| vec![pick(e)])
        .into_iter()
        .next()
        .unwrap_or(Handle::null(HandleKind::Config));
    config_value(h)
}

fn handles_value(hs: Vec<Handle>) -> Value {
    Value::array(hs.into_iter().map(Value::Handle))
}

/// Converts a config value to SQF (`getArray`, `getMissionConfigValue`).
fn to_sqf(v: &CfgValue) -> Value {
    match v {
        CfgValue::String(s) | CfgValue::Expression(s) => Value::from(s.as_str()),
        CfgValue::Float(f) => Value::Number(*f),
        CfgValue::Int(i) => Value::Number(*i as f32),
        CfgValue::Int64(i) => Value::Number(*i as f32),
        CfgValue::Array(items) => Value::array(items.iter().map(to_sqf)),
    }
}

fn localized<H: ConfigHost>(ctx: &Ctx<'_, H>, text: String) -> String {
    match text.strip_prefix('$') {
        Some(key) if key.len() >= 4 && key[..4].eq_ignore_ascii_case("STR_") => {
            ctx.host.localize(key).unwrap_or(text)
        }
        _ => text,
    }
}

/// Whether `s` is a literal `getNumber` reads without evaluating it.
fn is_number_literal(s: &str) -> bool {
    let s = s.trim();
    s.parse::<f64>().is_ok()
        || s.eq_ignore_ascii_case("true")
        || s.eq_ignore_ascii_case("false")
        || s.strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .is_some_and(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Filters config entries with an SQF condition string (`configClasses`,
/// `configProperties`); `_x` is the entry.
///
/// The condition runs once per entry and errors do not stop the filter: a failing condition
/// drops that entry and is reported, as the engine does with script errors inside a condition
/// (issue #271). `false` (and `nil`) drop the entry, any other result reports a type error.
fn filter<H: ConfigHost>(
    ctx: &mut Ctx<'_, H>,
    condition: &str,
    items: Vec<Handle>,
) -> Result<Value, SqfError> {
    if condition.trim().eq_ignore_ascii_case("true") {
        return Ok(handles_value(items));
    }
    let code = ctx
        .compile("", condition)
        .map_err(|e| SqfError::Generic(e.message))?;
    let mut out = Vec::new();
    for handle in items {
        match ctx.call_unscheduled_with_locals(&code, None, vec![(Sym::X, Value::Handle(handle))]) {
            // The error is already reported to the host.
            Err(_) => {}
            Ok(Value::Bool(true)) => out.push(Value::Handle(handle)),
            Ok(Value::Bool(false)) | Ok(Value::Nil) | Ok(Value::Nothing) => {}
            Ok(other) => return Err(SqfError::type_error(&other, BOOL)),
        }
    }
    Ok(Value::array(out))
}

/// Registers the config commands.
pub fn register_config_commands<H: ConfigHost>(r: &mut Registry<H>) {
    fn root<H: ConfigHost>(ctx: &mut Ctx<'_, H>, which: ConfigRoot) -> Value {
        let tree = ctx.host.configs().tree(which).clone();
        let h = ctx.host.configs_mut().handle(which, &tree.root());
        config_value(h)
    }
    r.nular("configFile", CONFIG, |ctx| Ok(root(ctx, ConfigRoot::Game)));
    r.nular("missionConfigFile", CONFIG, |ctx| {
        Ok(root(ctx, ConfigRoot::Mission))
    });
    r.nular("campaignConfigFile", CONFIG, |ctx| {
        Ok(root(ctx, ConfigRoot::Campaign))
    });
    for name in [">>", "/"] {
        r.binary(name, CONFIG, STR, CONFIG, |ctx, a, b| {
            let key = b.as_str().unwrap_or("").to_owned();
            Ok(related_one(ctx, &a, |e| e.get(&key).node_path().to_vec()))
        });
    }
    r.unary("getNumber", CONFIG, NUM, |ctx, a| {
        let (n, expr) = with_entry(ctx, &a, |e| match e {
            Some((_, e)) if e.is_text() && !is_number_literal(&e.text()) => (0.0, Some(e.text())),
            Some((_, e)) => (e.number(), None),
            None => (0.0, None),
        });
        let Some(expr) = expr else {
            return Ok(Value::Number(n));
        };
        // The text is an SQF expression (`getNumber` of `"(1 + 2)"` is 3). Text that does not
        // compile or does not evaluate to a number gives 0, as the engine's error recovery does.
        let Ok(code) = ctx.compile("", &expr) else {
            return Ok(Value::Number(0.0));
        };
        Ok(match ctx.call_unscheduled(&code, None) {
            Ok(Value::Number(n)) => Value::Number(n),
            Ok(Value::Bool(b)) => Value::Number(if b { 1.0 } else { 0.0 }),
            _ => Value::Number(0.0),
        })
    });
    // Only a text entry has text: a number, array or class entry gives "".
    r.unary("getText", CONFIG, STR, |ctx, a| {
        let text = with_entry(ctx, &a, |e| match e {
            Some((_, e)) if e.is_text() => e.text(),
            _ => String::new(),
        });
        Ok(Value::from(localized(ctx, text)))
    });
    r.unary("getTextRaw", CONFIG, STR, |ctx, a| {
        Ok(Value::from(with_entry(ctx, &a, |e| match e {
            Some((_, e)) if e.is_text() => e.text(),
            _ => String::new(),
        })))
    });
    r.unary("getArray", CONFIG, ARR, |ctx, a| {
        Ok(with_entry(ctx, &a, |e| match e {
            Some((_, e)) => Value::array(e.array().iter().map(to_sqf)),
            None => Value::array([]),
        }))
    });
    r.unary("isClass", CONFIG, BOOL, |ctx, a| {
        Ok(Value::Bool(with_entry(ctx, &a, |e| {
            e.is_some_and(|(_, e)| e.is_class())
        })))
    });
    r.unary("isNumber", CONFIG, BOOL, |ctx, a| {
        Ok(Value::Bool(with_entry(ctx, &a, |e| {
            e.is_some_and(|(_, e)| e.is_number())
        })))
    });
    r.unary("isText", CONFIG, BOOL, |ctx, a| {
        Ok(Value::Bool(with_entry(ctx, &a, |e| {
            e.is_some_and(|(_, e)| e.is_text())
        })))
    });
    r.unary("isArray", CONFIG, BOOL, |ctx, a| {
        Ok(Value::Bool(with_entry(ctx, &a, |e| {
            e.is_some_and(|(_, e)| e.is_array())
        })))
    });
    r.unary("configName", CONFIG, STR, |ctx, a| {
        Ok(Value::from(with_entry(ctx, &a, |e| {
            e.map(|(_, e)| e.name().to_owned()).unwrap_or_default()
        })))
    });
    // `configHierarchy config` (parent to child) and
    // `configHierarchy [config, classesOnly, includeBases, reversedOrder, configNames]`.
    r.unary("configHierarchy", CONFIG, ARR, |ctx, a| {
        Ok(handles_value(related(ctx, &a, |e| {
            e.hierarchy()
                .iter()
                .map(|c| c.node_path().to_vec())
                .collect()
        })))
    });
    r.unary("configHierarchy", ARR, ARR, |ctx, a| {
        let args = a.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let config = args.first().cloned().unwrap_or(Value::Nil);
        if !matches!(config, Value::Handle(_)) {
            return Err(SqfError::type_error(&config, CONFIG));
        }
        let flag = |i: usize| args.get(i).and_then(Value::as_bool).unwrap_or(false);
        let (classes_only, include_bases, reversed, names) = (flag(1), flag(2), flag(3), flag(4));
        let handles = related(ctx, &config, |e| {
            // `include_bases` walks the inheritance chain (child first), the plain form the
            // access path from the root (parent first).
            let mut chain: Vec<ConfigRef<'_>> = if include_bases {
                let mut chain = vec![e.clone()];
                chain.extend(e.bases());
                chain
            } else {
                e.hierarchy()
            };
            if classes_only {
                chain.retain(|c| c.is_class());
            }
            if include_bases {
                chain.reverse();
            }
            if reversed {
                chain.reverse();
            }
            chain.iter().map(|c| c.node_path().to_vec()).collect()
        });
        if !names {
            return Ok(handles_value(handles));
        }
        let mut out = Vec::with_capacity(handles.len());
        for handle in handles {
            let name = with_entry(ctx, &config_value(handle), |e| {
                e.map(|(_, e)| e.name().to_owned()).unwrap_or_default()
            });
            out.push(Value::string(name));
        }
        Ok(Value::array(out))
    });
    r.unary("inheritsFrom", CONFIG, CONFIG, |ctx, a| {
        Ok(related_one(ctx, &a, |e| {
            e.inherits_from().node_path().to_vec()
        }))
    });
    r.unary("count", CONFIG, NUM, |ctx, a| {
        Ok(Value::Number(
            with_entry(ctx, &a, |e| e.map_or(0, |(_, e)| e.entry_count())) as f32,
        ))
    });
    r.binary("select", CONFIG, NUM, CONFIG, |ctx, a, b| {
        let n = b.as_number().unwrap_or(0.0);
        // Rounded like array select (half to even).
        let i = n.round_ties_even();
        Ok(related_one(ctx, &a, |e| {
            if i < 0.0 {
                return Vec::new();
            }
            e.entry_at(i as usize).node_path().to_vec()
        }))
    });
    r.binary("configClasses", STR, CONFIG, ARR, |ctx, a, b| {
        let classes = related(ctx, &b, |e| {
            e.entries()
                .iter()
                .filter(|c| c.is_class())
                .map(|c| c.node_path().to_vec())
                .collect()
        });
        filter(ctx, a.as_str().unwrap_or("true"), classes)
    });
    r.unary("configProperties", ARR, ARR, |ctx, a| {
        let Value::Array(args) = &a else {
            return Err(SqfError::type_error(&a, ARR));
        };
        let args = args.borrow().clone();
        let config = args.first().cloned().unwrap_or(Value::Nil);
        if !matches!(config, Value::Handle(_)) {
            return Err(SqfError::type_error(&config, CONFIG));
        }
        let condition = match args.get(1) {
            Some(Value::String(s)) => s.to_string(),
            _ => "true".to_owned(),
        };
        let inherit = !matches!(args.get(2), Some(Value::Bool(false)));
        let items = related(ctx, &config, |e| {
            let entries = if inherit {
                e.entries_with_inherited()
            } else {
                e.entries()
            };
            entries.iter().map(|c| c.node_path().to_vec()).collect()
        });
        filter(ctx, &condition, items)
    });
    r.unary("getMissionConfig", STR, CONFIG, |ctx, a| {
        let key = a.as_str().unwrap_or("").to_owned();
        let mission = root(ctx, ConfigRoot::Mission);
        Ok(related_one(ctx, &mission, |e| {
            e.get(&key).node_path().to_vec()
        }))
    });
    r.unary("getMissionConfigValue", STR, TypeSet::ANYTHING, |ctx, a| {
        Ok(mission_value(ctx, a.as_str().unwrap_or("")).unwrap_or(Value::Nil))
    });
    r.unary("getMissionConfigValue", ARR, TypeSet::ANYTHING, |ctx, a| {
        let Value::Array(args) = &a else {
            return Err(SqfError::type_error(&a, ARR));
        };
        let args = args.borrow().clone();
        let key = args
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let default = args.get(1).cloned().unwrap_or(Value::Nil);
        Ok(mission_value(ctx, &key).unwrap_or(default))
    });
    // `configOf object` needs the world; until then every object has no config.
    r.unary("configOf", TypeSet::of(Type::Object), CONFIG, |_, _| {
        Ok(config_value(Handle::null(HandleKind::Config)))
    });

    // `language`: the language the stringtables resolve for, e.g. "English".
    r.nular("language", STR, |ctx| {
        Ok(Value::string(ctx.host.language()))
    });

    // `"class" isKindOf "base"` and `"class" isKindOf ["base", targetConfig]`. The class is
    // looked up in CfgVehicles, then CfgAmmo, then CfgNonAIVehicles (the roots the engine
    // searches for the string forms).
    r.binary("isKindOf", STR, STR, BOOL, |ctx, a, b| {
        let (class, base) = (
            a.as_str().unwrap_or_default().to_owned(),
            b.as_str().unwrap_or_default().to_owned(),
        );
        Ok(Value::Bool(class_is_kind_of(ctx, &class, &base, None)))
    });
    r.binary("isKindOf", STR, ARR, BOOL, |ctx, a, b| {
        let class = a.as_str().unwrap_or_default().to_owned();
        let args = b.as_array().map(|x| x.borrow().clone()).unwrap_or_default();
        let base = args
            .first()
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let target = args.get(1).cloned();
        Ok(Value::Bool(class_is_kind_of(
            ctx,
            &class,
            &base,
            target.as_ref(),
        )))
    });
}

/// The config roots `isKindOf` searches for a class name (the engine's order).
const IS_KIND_OF_ROOTS: [&str; 3] = ["CfgVehicles", "CfgAmmo", "CfgNonAIVehicles"];

/// `class isKindOf base`: whether `class` is `base` or inherits from it. `target` is the config
/// to search when the caller named one (`isKindOf ["base", configFile >> "CfgWeapons"]`).
fn class_is_kind_of<H: ConfigHost>(
    ctx: &mut Ctx<'_, H>,
    class: &str,
    base: &str,
    target: Option<&Value>,
) -> bool {
    if let Some(Value::Handle(h)) = target
        .filter(|v| matches!(v, Value::Handle(h) if h.kind == HandleKind::Config && h.id != 0))
    {
        let Some((_, tree, path)) = ctx.host.configs().resolve(*h) else {
            return false;
        };
        let class_entry = tree.from_node_path(&path).get(class);
        return class_entry.is_class() && entry_is_kind_of(&class_entry, class, base);
    }
    let tree = ctx.host.configs().tree(ConfigRoot::Game).clone();
    for root in IS_KIND_OF_ROOTS {
        let entry = tree.root().get(root).get(class);
        if entry.is_class() {
            return entry_is_kind_of(&entry, class, base);
        }
    }
    false
}

/// Whether the class `entry` (named `class`) is `base` or inherits from it.
fn entry_is_kind_of(entry: &ConfigRef<'_>, class: &str, base: &str) -> bool {
    if class.eq_ignore_ascii_case(base) {
        return true;
    }
    entry
        .bases()
        .iter()
        .any(|b| b.name().eq_ignore_ascii_case(base))
}

fn mission_value<H: ConfigHost>(ctx: &Ctx<'_, H>, key: &str) -> Option<Value> {
    let tree = ctx.host.configs().tree(ConfigRoot::Mission).clone();
    let entry = tree.root().get(key);
    if entry.is_null() || entry.is_class() {
        return None;
    }
    if entry.is_number() {
        Some(Value::Number(entry.number()))
    } else if entry.is_text() {
        Some(Value::from(entry.text()))
    } else {
        Some(Value::array(entry.array().iter().map(to_sqf)))
    }
}
