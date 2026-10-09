//! Damage: the damage model of an Entity type, an Entity's damage state, and destruction.
//!
//! Mirrors the original, whose formulas are in `docs/re/sim-ballistics.md` §5–§7 and whose
//! behaviour is in `docs/re/sim-damage.md`:
//!
//! - a hit carries total damage and per-hit-point damage; each value either replaces or adds to
//!   what is stored (`FUN_141026a00`; the `hit_info+0x9` flag);
//! - the Object's [`DamageHandler`] sees the values a hit would leave and returns the values to
//!   store instead (`HandleDamage`, event 30);
//! - a hit point with a `depends` expression is recomputed from the hit points it reads when one
//!   of them changes;
//! - an Entity is destroyed when its total damage reaches 1, or when a fatal hit point of its
//!   class is depleted; a destroyed Object whose type has a `simulation = "ruin"` entry in
//!   `DestructionEffects` becomes its ruin;
//! - damage is applied by the Object's local owner.
//!
//! The World records [`WorldEvent::Dammaged`] and [`WorldEvent::Killed`] for scripts; the
//! mission-framework event handler registry that will dispatch them (and register the SQF
//! `HandleDamage` handler) is future work, so the seam is a trait a host installs.

use std::fmt;
use std::sync::Arc;

use a3_config::ConfigRef;

use crate::{
    EntityClass, EntityId, EntityType, ObjectRef, SimulationClass, StaticKey, World, WorldEvent,
};

/// Total damage is capped at this when stored, as the original does (`FUN_14102cb00`).
const TOTAL_CAP: f32 = 1000.0;

/// A change below this is not stored (the original's `1e-6` dead band).
const TOTAL_DEAD_BAND: f32 = 1e-6;

/// How a hit's values combine with the stored ones (`hit_info+0x9`, `docs/re/sim-damage.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DamageMode {
    /// Store the given values (`0`): a script's `setDamage`/`setHit`/`setHitIndex`.
    #[default]
    Replace,
    /// Add the given values to the stored ones (`1`): a ballistic hit.
    Add,
}

/// Where a hit comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DamageOrigin {
    /// Engine damage (ballistics, explosions, collisions). `allowDamage` gates it and a
    /// [`DamageHandler`] sees it.
    #[default]
    Engine,
    /// A script's direct set (`setDamage`, `setHit`, `setHitIndex`). It bypasses `allowDamage`
    /// and the handler, as the commands do (`docs/re/sim-damage.md` §6).
    Scripted,
}

/// What a `HandleDamage` call is about: the `_context` argument (`docs/re/sim-damage.md` §5).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageContext {
    /// 0: the total damage.
    TotalDamage = 0,
    /// 1: one hit point.
    HitPoint = 1,
    /// 2: the hit point hit last (the original's `LastHitPoint`; not produced yet).
    LastHitPoint = 2,
    /// 3: a fake head hit (the original's `FakeHeadHit`; not produced yet).
    FakeHeadHit = 3,
    /// 4: the total before bleeding (the original's `TotalDamageBeforeBleeding`; not produced yet).
    TotalDamageBeforeBleeding = 4,
}

impl DamageContext {
    /// The `_context` number the original passes.
    pub fn as_int(self) -> u32 {
        self as u32
    }
}

/// A parsed `depends` expression: one hit point's damage derived from other hit points and from
/// `Total` (`CfgVehicles >> HitPoints >> <name> >> depends`, `docs/re/sim-damage.md` §3).
///
/// Operators are `max`, `min`, `+`, `-`, `*`, `/`, parentheses and unary minus. `max`/`min` bind
/// loosest, then `+`/`-`, then `*`/`/`; the shipped configs parenthesise every mixed expression,
/// so the order only decides malformed input. An expression that does not parse is treated as no
/// dependency (the original logs a warning instead).
#[derive(Debug, Clone, PartialEq)]
pub struct Depends {
    source: String,
    expr: Expr,
    names: Vec<String>,
}

impl Depends {
    /// Parses a `depends` value, or `None` for an empty one and for `"0"` (which a class uses to
    /// clear a dependency it inherited).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() || text == "0" {
            return None;
        }
        let tokens = tokenize(text)?;
        let mut parser = Parser { tokens, pos: 0 };
        let expr = parser.expr()?;
        if parser.pos != parser.tokens.len() {
            return None;
        }
        let mut names = Vec::new();
        expr.names(&mut names);
        Some(Depends {
            source: text.to_owned(),
            expr,
            names,
        })
    }

    /// The expression as written in config.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The hit point names the expression reads, in order of first use; `Total` is one of them
    /// when the expression reads the total damage.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Evaluates the expression, `lookup` resolving a name to a damage value.
    pub fn eval(&self, lookup: impl Fn(&str) -> f32) -> f32 {
        self.expr.eval(&lookup)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Expr {
    Number(f32),
    Name(String),
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Max(Box<Expr>, Box<Expr>),
    Min(Box<Expr>, Box<Expr>),
}

impl Expr {
    fn eval(&self, lookup: &impl Fn(&str) -> f32) -> f32 {
        match self {
            Expr::Number(n) => *n,
            Expr::Name(name) => lookup(name),
            Expr::Neg(a) => -a.eval(lookup),
            Expr::Add(a, b) => a.eval(lookup) + b.eval(lookup),
            Expr::Sub(a, b) => a.eval(lookup) - b.eval(lookup),
            Expr::Mul(a, b) => a.eval(lookup) * b.eval(lookup),
            Expr::Div(a, b) => a.eval(lookup) / b.eval(lookup),
            Expr::Max(a, b) => a.eval(lookup).max(b.eval(lookup)),
            Expr::Min(a, b) => a.eval(lookup).min(b.eval(lookup)),
        }
    }

    fn names(&self, out: &mut Vec<String>) {
        fn add(out: &mut Vec<String>, name: &str) {
            if !out.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                out.push(name.to_owned());
            }
        }
        match self {
            Expr::Number(_) => {}
            Expr::Name(name) => add(out, name),
            Expr::Neg(a) => a.names(out),
            Expr::Add(a, b) | Expr::Sub(a, b) | Expr::Mul(a, b) | Expr::Div(a, b) => {
                a.names(out);
                b.names(out);
            }
            Expr::Max(a, b) | Expr::Min(a, b) => {
                a.names(out);
                b.names(out);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f32),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
}

/// Splits a `depends` expression; `None` on a character no expression contains.
fn tokenize(text: &str) -> Option<Vec<Token>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'+' => {
                out.push(Token::Plus);
                i += 1;
            }
            b'-' => {
                out.push(Token::Minus);
                i += 1;
            }
            b'*' => {
                out.push(Token::Star);
                i += 1;
            }
            b'/' => {
                out.push(Token::Slash);
                i += 1;
            }
            b'(' => {
                out.push(Token::LParen);
                i += 1;
            }
            b')' => {
                out.push(Token::RParen);
                i += 1;
            }
            _ if c.is_ascii_digit() || c == b'.' => {
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                out.push(Token::Number(text[start..i].parse().ok()?));
            }
            _ if c.is_ascii_alphanumeric() || c == b'_' || c == b'#' => {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'#')
                {
                    i += 1;
                }
                out.push(Token::Name(text[start..i].to_owned()));
            }
            _ => return None,
        }
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// `max`/`min` chains, loosest.
    fn expr(&mut self) -> Option<Expr> {
        let mut left = self.term()?;
        loop {
            match self.peek() {
                Some(Token::Name(name)) if name.eq_ignore_ascii_case("max") => {
                    self.pos += 1;
                    let right = self.term()?;
                    left = Expr::Max(Box::new(left), Box::new(right));
                }
                Some(Token::Name(name)) if name.eq_ignore_ascii_case("min") => {
                    self.pos += 1;
                    let right = self.term()?;
                    left = Expr::Min(Box::new(left), Box::new(right));
                }
                _ => return Some(left),
            }
        }
    }

    fn term(&mut self) -> Option<Expr> {
        let mut left = self.factor()?;
        loop {
            if self.eat(&Token::Plus) {
                let right = self.factor()?;
                left = Expr::Add(Box::new(left), Box::new(right));
            } else if self.eat(&Token::Minus) {
                let right = self.factor()?;
                left = Expr::Sub(Box::new(left), Box::new(right));
            } else {
                return Some(left);
            }
        }
    }

    fn factor(&mut self) -> Option<Expr> {
        let mut left = self.unary()?;
        loop {
            if self.eat(&Token::Star) {
                let right = self.unary()?;
                left = Expr::Mul(Box::new(left), Box::new(right));
            } else if self.eat(&Token::Slash) {
                let right = self.unary()?;
                left = Expr::Div(Box::new(left), Box::new(right));
            } else {
                return Some(left);
            }
        }
    }

    fn unary(&mut self) -> Option<Expr> {
        if self.eat(&Token::Minus) {
            return Some(Expr::Neg(Box::new(self.unary()?)));
        }
        self.eat(&Token::Plus);
        self.atom()
    }

    fn atom(&mut self) -> Option<Expr> {
        match self.bump()? {
            Token::Number(n) => Some(Expr::Number(n)),
            Token::Name(name) => Some(Expr::Name(name)),
            Token::LParen => {
                let expr = self.expr()?;
                self.eat(&Token::RParen).then_some(expr)
            }
            _ => None,
        }
    }
}

/// One hit point of a type: a `CfgVehicles >> <type> >> HitPoints` class.
///
/// A hit point is addressed by its config name (`HitHead`) in `setHitPointDamage`, and by its
/// model selection (`head`) in `setHit`; its index is its place in the merged `HitPoints` class
/// (`getHitIndex`, `getAllHitPointsDamage`).
#[derive(Debug, Clone, PartialEq)]
pub struct HitPoint {
    /// The config class name.
    pub name: String,
    /// The model selection (`name` in config), defaulting to the class name.
    pub selection: String,
    /// `armor`, as written: the effective armor is `armor` times the type's when it is not
    /// negative, and `-armor` when it is (`sim-ballistics.md` §7.1).
    pub armor: f32,
    /// `radius`, −1 (no point cloud) by default (`sim-ballistics.md` §7.2).
    pub radius: f32,
    /// `passThrough`, 1 by default: how much of the hit passes on to the total.
    pub pass_through: f32,
    /// `explosionShielding`, 1 by default.
    pub explosion_shielding: f32,
    /// `minimalHit`, 0.01 by default.
    pub minimal_hit: f32,
    /// `visual`: the injury visual of the selection, "" when none.
    pub visual: String,
    /// `material`, −1 when unset.
    pub material: i32,
    /// `depends`: how this hit point's damage derives from others.
    pub depends: Option<Depends>,
}

/// A hit point whose depletion destroys the Object (`docs/re/sim-damage.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FatalHitPoint {
    /// The hit point's index in the type's hit points.
    pub index: usize,
    /// The damage at which the Object is destroyed.
    pub threshold: f32,
}

/// What a destroyed Object leaves behind: a `simulation = "ruin"` entry of its type's
/// `DestructionEffects` (`docs/re/sim-damage.md` §7.2). The class is named `Land_` plus the
/// model's file stem, which is what the original resolves the model path to.
#[derive(Debug, Clone, PartialEq)]
pub struct Ruin {
    /// The `type` entry: a model path.
    pub model: String,
}

impl Ruin {
    /// The `CfgVehicles` class name of the ruin model
    /// (`\A3\…\House_Big_01_V1_ruins_F.p3d` → `Land_House_Big_01_V1_ruins_F`).
    pub fn class_name(&self) -> String {
        let file = self.model.rsplit(['\\', '/']).next().unwrap_or(&self.model);
        let stem = file
            .strip_suffix(".p3d")
            .or_else(|| file.strip_suffix(".P3D"))
            .unwrap_or(file);
        if stem.to_ascii_lowercase().starts_with("land_") {
            stem.to_owned()
        } else {
            format!("Land_{stem}")
        }
    }
}

/// `destrType`: how a destroyed Object behaves (`docs/re/sim-damage.md` §7.4). Parsed for
/// consumers (the destruction animation); the damage code only uses [`DamageModel::ruins`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DestructionType {
    /// `DestructNo`: the Object stays as it is.
    No,
    /// `DestructDefault`: the Object stays and its `DestructionEffects` run.
    #[default]
    Default,
    /// `DestructTree`/`DestructBush`: the model falls over.
    Tree,
    /// `DestructWall`: falls away from the hit.
    Wall,
    /// `DestructBuilding`: collapses into its ruin.
    Building,
    /// `DestructTent`: collapses.
    Tent,
    /// `DestructEngine`: burns.
    Engine,
    /// `DestructWreck`: becomes a wreck.
    Wreck,
    /// `DestructColumn`: leans and falls (new in 2.22).
    Column,
    /// Any other value, as written (a mod's own, or a number).
    Other(String),
}

impl DestructionType {
    /// Parses a `destrType` value, case-insensitively; an empty one is
    /// [`Default`](DestructionType::Default).
    pub fn from_config(text: &str) -> Self {
        match text.to_ascii_lowercase().as_str() {
            "" => DestructionType::Default,
            "0" | "destructno" => DestructionType::No,
            "destructdefault" => DestructionType::Default,
            "destructtree" | "destructbush" => DestructionType::Tree,
            "destructwall" => DestructionType::Wall,
            "destructbuilding" => DestructionType::Building,
            "destructtent" => DestructionType::Tent,
            "destructengine" => DestructionType::Engine,
            "destructwreck" => DestructionType::Wreck,
            "destructcolumn" => DestructionType::Column,
            _ => DestructionType::Other(text.to_owned()),
        }
    }
}

/// The hit points of a `Person`: a head or body hit is fatal (`FUN_1407442e0`).
const MAN_FATAL_HIT_POINTS: &[(&str, f32)] = &[("HitHead", 1.0), ("HitBody", 1.0)];

/// The hit points that destroy their Object when depleted (wiki, medium: HitHull for tracked
/// vehicles, HitFuel for wheeled ones).
const FATAL_HIT_POINTS: &[(&str, f32)] = &[("HitHull", 1.0), ("HitFuel", 1.0)];

/// The damage data of one Entity type: what `CfgVehicles` says about taking damage.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageModel {
    hit_points: Vec<HitPoint>,
    fatal_hit_points: Vec<FatalHitPoint>,
    armor: f32,
    armor_structural: f32,
    explosion_shielding: f32,
    min_total_damage_threshold: f32,
    impact_damage_multiplier: f32,
    destruction: DestructionType,
    replace_damaged: Option<String>,
    replace_damaged_limit: f32,
    replace_damaged_hit_points: Vec<String>,
    ruins: Vec<Ruin>,
}

impl Default for DamageModel {
    fn default() -> Self {
        Self::new()
    }
}

impl DamageModel {
    /// The defaults of a type with no config behind it: no hit points, no ruins.
    pub fn new() -> Self {
        Self {
            hit_points: Vec::new(),
            fatal_hit_points: Vec::new(),
            // The `armor` default documented for `CfgVehicles` is 30; the rest are the config
            // loaders' defaults (`sim-ballistics.md` §7.1) or the shipped values where the
            // default is untraced (`explosionShielding`, `impactDamageMultiplier`).
            armor: 30.0,
            armor_structural: 1.0,
            explosion_shielding: 1.0,
            min_total_damage_threshold: 0.001,
            impact_damage_multiplier: 1.0,
            destruction: DestructionType::Default,
            replace_damaged: None,
            replace_damaged_limit: 0.9,
            replace_damaged_hit_points: Vec::new(),
            ruins: Vec::new(),
        }
    }

    /// The model of a config class (`CfgVehicles >> <type>`).
    pub fn from_config(cfg: &ConfigRef<'_>, class: SimulationClass) -> Self {
        let hit_points = parse_hit_points(cfg);
        let fatal_hit_points = fatal_hit_points(class, &hit_points);
        Self {
            hit_points,
            fatal_hit_points,
            armor: number(cfg, "armor", 30.0),
            armor_structural: number(cfg, "armorStructural", 1.0),
            explosion_shielding: number(cfg, "explosionShielding", 1.0),
            min_total_damage_threshold: number(cfg, "minTotalDamageThreshold", 0.001),
            impact_damage_multiplier: number(cfg, "impactDamageMultiplier", 1.0),
            destruction: DestructionType::from_config(&cfg.get("destrType").text()),
            replace_damaged: match cfg.get("replaceDamaged").text() {
                text if text.is_empty() => None,
                text => Some(text),
            },
            replace_damaged_limit: number(cfg, "replaceDamagedLimit", 0.9),
            replace_damaged_hit_points: cfg
                .get("replaceDamagedHitpoints")
                .array()
                .into_iter()
                .filter_map(|v| match v {
                    a3_config::Value::String(s) | a3_config::Value::Expression(s) => Some(s),
                    _ => None,
                })
                .collect(),
            ruins: parse_ruins(cfg),
        }
    }

    pub fn hit_points(&self) -> &[HitPoint] {
        &self.hit_points
    }

    /// The hit points whose depletion destroys the Object.
    pub fn fatal_hit_points(&self) -> &[FatalHitPoint] {
        &self.fatal_hit_points
    }

    /// The type-level `armor`.
    pub fn armor(&self) -> f32 {
        self.armor
    }

    /// The effective armor of a hit point: `armor ×` the type's when not negative, `-armor` when
    /// it is (`sim-ballistics.md` §7.1).
    pub fn armor_of(&self, index: usize) -> f32 {
        let armor = self.hit_points.get(index).map_or(0.0, |p| p.armor);
        if armor < 0.0 {
            -armor
        } else {
            armor * self.armor
        }
    }

    /// `armorStructural`.
    pub fn armor_structural(&self) -> f32 {
        self.armor_structural
    }

    /// `explosionShielding`.
    pub fn explosion_shielding(&self) -> f32 {
        self.explosion_shielding
    }

    /// `minTotalDamageThreshold`: totals below this are ignored (`sim-ballistics.md` §6).
    pub fn min_total_damage_threshold(&self) -> f32 {
        self.min_total_damage_threshold
    }

    /// `impactDamageMultiplier`.
    pub fn impact_damage_multiplier(&self) -> f32 {
        self.impact_damage_multiplier
    }

    /// `destrType`.
    pub fn destruction(&self) -> &DestructionType {
        &self.destruction
    }

    /// `replaceDamaged`: the class the type is swapped for while heavily damaged
    /// (`sim-damage.md` §7.3); not implemented yet.
    pub fn replace_damaged(&self) -> Option<&str> {
        self.replace_damaged.as_deref()
    }

    /// `replaceDamagedLimit`.
    pub fn replace_damaged_limit(&self) -> f32 {
        self.replace_damaged_limit
    }

    /// `replaceDamagedHitpoints[]`.
    pub fn replace_damaged_hit_points(&self) -> &[String] {
        &self.replace_damaged_hit_points
    }

    /// The ruin Objects the type leaves behind when destroyed.
    pub fn ruins(&self) -> &[Ruin] {
        &self.ruins
    }

    /// The index of the hit point with this config name, case-insensitively.
    pub fn hit_point_index(&self, name: &str) -> Option<usize> {
        self.hit_points
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// The index of the hit point whose model selection this is, case-insensitively.
    pub fn hit_point_index_of_selection(&self, selection: &str) -> Option<usize> {
        self.hit_points
            .iter()
            .position(|p| p.selection.eq_ignore_ascii_case(selection))
    }
}

/// A config number, or `default` when the entry is missing or not a number.
fn number(cfg: &ConfigRef<'_>, name: &str, default: f32) -> f32 {
    let entry = cfg.get(name);
    if entry.is_null() {
        return default;
    }
    let value = entry.number();
    if value.is_finite() { value } else { default }
}

fn parse_hit_points(cfg: &ConfigRef<'_>) -> Vec<HitPoint> {
    let hit_points = cfg.get("HitPoints");
    if !hit_points.is_class() {
        return Vec::new();
    }
    hit_points
        .entries_with_inherited()
        .into_iter()
        .filter(|entry| entry.is_class())
        .map(|entry| {
            let name = entry.name().to_owned();
            let selection = match entry.get("name").text() {
                text if text.is_empty() => name.clone(),
                text => text,
            };
            HitPoint {
                name,
                selection,
                // A hit point's own `armor` is always written in the shipped configs; 1 keeps
                // its effective armor equal to the type's.
                armor: number(&entry, "armor", 1.0),
                radius: number(&entry, "radius", -1.0),
                pass_through: number(&entry, "passThrough", 1.0),
                explosion_shielding: number(&entry, "explosionShielding", 1.0),
                minimal_hit: number(&entry, "minimalHit", 0.01),
                visual: entry.get("visual").text(),
                material: entry.get("material").number() as i32,
                depends: Depends::parse(&entry.get("depends").text()),
            }
        })
        .collect()
}

fn parse_ruins(cfg: &ConfigRef<'_>) -> Vec<Ruin> {
    let effects = cfg.get("DestructionEffects");
    if !effects.is_class() {
        return Vec::new();
    }
    effects
        .entries_with_inherited()
        .into_iter()
        .filter(|entry| entry.is_class())
        .filter(|entry| entry.get("simulation").text().eq_ignore_ascii_case("ruin"))
        .filter_map(|entry| match entry.get("type").text() {
            model if model.is_empty() => None,
            model => Some(Ruin { model }),
        })
        .collect()
}

fn fatal_hit_points(class: SimulationClass, hit_points: &[HitPoint]) -> Vec<FatalHitPoint> {
    let table = if class.engine_class().is_kind_of(EntityClass::Person) {
        MAN_FATAL_HIT_POINTS
    } else {
        FATAL_HIT_POINTS
    };
    table
        .iter()
        .filter_map(|(name, threshold)| {
            let index = hit_points
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(name))?;
            Some(FatalHitPoint {
                index,
                threshold: *threshold,
            })
        })
        .collect()
}

/// The damage values of one Entity: the total (`Object+0xf0`) and one value per hit point
/// (`Object+0x260`).
#[derive(Debug, Clone, PartialEq)]
pub struct DamageState {
    total: f32,
    hit_points: Vec<f32>,
}

impl DamageState {
    /// An intact state for a type: one zero per hit point of its model.
    pub fn new(model: &DamageModel) -> Self {
        Self {
            total: 0.0,
            hit_points: vec![0.0; model.hit_points().len()],
        }
    }

    /// The total damage as scripts read it (`damage`): the stored value clamped to 0..1.
    pub fn total(&self) -> f32 {
        self.total.clamp(0.0, 1.0)
    }

    /// The stored total, up to the original's cap of 1000.
    pub fn stored_total(&self) -> f32 {
        self.total
    }

    pub fn hit_points(&self) -> &[f32] {
        &self.hit_points
    }

    /// One hit point's damage; 0 for an index the type does not have.
    pub fn hit_point(&self, index: usize) -> f32 {
        self.hit_points.get(index).copied().unwrap_or(0.0)
    }

    /// Whether the Object is destroyed: the total reached 1, or a fatal hit point of its type is
    /// depleted (`docs/re/sim-damage.md` §2.1). A value derived from the state, so `setDamage 0`
    /// restores a destroyed Object (the original also keeps a flag a class rule sets).
    pub fn is_destroyed(&self, model: &DamageModel) -> bool {
        self.total() >= 1.0
            || model
                .fatal_hit_points()
                .iter()
                .any(|fatal| self.hit_point(fatal.index) >= fatal.threshold)
    }

    /// Stores the total damage the way the original's `SetDamage` does (`FUN_14102cb00`): changes
    /// below `1e-6` are ignored, 0 or less stores 0, and the value is capped at 1000. Returns
    /// whether the stored value changed.
    pub fn set_total(&mut self, damage: f32) -> bool {
        if damage.is_nan() {
            return false;
        }
        let value = if damage <= 0.0 {
            0.0
        } else {
            damage.min(TOTAL_CAP)
        };
        if (value - self.total).abs() < TOTAL_DEAD_BAND {
            return false;
        }
        self.total = value;
        true
    }

    /// Adds to the total damage (the ballistic path). Returns whether the stored value changed.
    pub fn add_total(&mut self, damage: f32) -> bool {
        self.set_total(self.total + damage)
    }

    /// Stores one hit point's damage, clamped to 0..1. Returns whether the value changed.
    pub fn set_hit_point(&mut self, index: usize, damage: f32) -> bool {
        if damage.is_nan() || index >= self.hit_points.len() {
            return false;
        }
        let value = damage.clamp(0.0, 1.0);
        if value == self.hit_points[index] {
            return false;
        }
        self.hit_points[index] = value;
        true
    }

    /// Adds to one hit point's damage. Returns whether the value changed.
    pub fn add_hit_point(&mut self, index: usize, damage: f32) -> bool {
        self.set_hit_point(index, self.hit_point(index) + damage)
    }

    /// Recomputes the hit points a `depends` expression derives, after [`changed`](Self::set_total)
    /// hit points and, when `total_changed`, the total:
    ///
    /// - a dependent hit point is recomputed when one of the names it reads changed (or, for
    ///   `Total`, when the total did) and the change did not write it directly — a direct write
    ///   wins, so `setHitPointDamage` on a dependent hit point sticks;
    /// - recomputation runs in hit point order, so a chain of dependencies propagates: the
    ///   dominant hit point has to be declared before its dependant (`docs/re/sim-damage.md` §3);
    /// - a value that is not finite is left alone.
    pub fn recompute_depends(
        &mut self,
        model: &DamageModel,
        changed: &[usize],
        total_changed: bool,
    ) {
        let mut touched = changed.to_vec();
        for (index, point) in model.hit_points().iter().enumerate() {
            let Some(depends) = &point.depends else {
                continue;
            };
            if touched.contains(&index) {
                continue;
            }
            let sourced = depends.names().iter().any(|name| {
                if name.eq_ignore_ascii_case("Total") {
                    total_changed
                } else {
                    model
                        .hit_point_index(name)
                        .is_some_and(|i| touched.contains(&i))
                }
            });
            if !sourced {
                continue;
            }
            let value = depends.eval(|name| {
                if name.eq_ignore_ascii_case("Total") {
                    self.total()
                } else {
                    model
                        .hit_point_index(name)
                        .map_or(0.0, |i| self.hit_point(i))
                }
            });
            if value.is_finite() {
                self.set_hit_point(index, value);
            }
            touched.push(index);
        }
    }
}

/// A hit on an Object: what a projectile or an explosion computed (`sim-ballistics.md` §7.2), or
/// what a script asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageHit {
    /// Total damage; `None` leaves the total alone (`setHitPointDamage`, `setHit`).
    pub total: Option<f32>,
    /// Per-hit-point damage, by hit point index.
    pub hit_points: Vec<(usize, f32)>,
    /// Whether the values replace or add to the stored ones.
    pub mode: DamageMode,
    /// Where the hit comes from.
    pub origin: DamageOrigin,
    /// The Object that caused it (`_source`/`_killer`), when known.
    pub source: Option<EntityId>,
    /// The Object that instigated it (`_instigator`), when known.
    pub instigator: Option<EntityId>,
    /// Whether destruction effects (the ruin) run when this hit destroys the Object.
    pub use_effects: bool,
}

impl Default for DamageHit {
    fn default() -> Self {
        Self {
            total: None,
            hit_points: Vec::new(),
            mode: DamageMode::Replace,
            origin: DamageOrigin::Engine,
            source: None,
            instigator: None,
            use_effects: true,
        }
    }
}

impl DamageHit {
    /// A hit that replaces the total damage (`setDamage`).
    pub fn total(damage: f32) -> Self {
        Self {
            total: Some(damage),
            ..Self::default()
        }
    }

    /// A hit that adds to the total damage (a ballistic hit that carries only a total).
    pub fn added(damage: f32) -> Self {
        Self {
            total: Some(damage),
            mode: DamageMode::Add,
            ..Self::default()
        }
    }

    /// The same hit with per-hit-point damage, added at a hit point index.
    pub fn at(mut self, index: usize, damage: f32) -> Self {
        self.hit_points.push((index, damage));
        self
    }

    /// The same hit with per-hit-point damage by config name, leaving the hit alone when the type
    /// has no such hit point.
    pub fn at_name(mut self, index: Option<usize>, damage: f32) -> Self {
        if let Some(index) = index {
            self.hit_points.push((index, damage));
        }
        self
    }

    /// The Object that caused the hit (`_killer`/`_source`).
    pub fn caused_by(mut self, source: EntityId) -> Self {
        self.source = Some(source);
        self
    }

    /// The Object that instigated the hit (`_instigator`).
    pub fn instigated_by(mut self, instigator: EntityId) -> Self {
        self.instigator = Some(instigator);
        self
    }

    /// A script's direct set: `allowDamage` and a [`DamageHandler`] do not apply.
    pub fn scripted(mut self) -> Self {
        self.origin = DamageOrigin::Scripted;
        self
    }

    /// Without destruction effects (`setDamage [d, false]`): the Object is destroyed but does not
    /// become its ruin.
    pub fn without_effects(mut self) -> Self {
        self.use_effects = false;
        self
    }

    /// The same hit with its values added instead of replaced.
    pub fn adding(mut self) -> Self {
        self.mode = DamageMode::Add;
        self
    }
}

/// What applying a hit did.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageOutcome {
    /// The total damage after the hit, as `damage` reads it (0..1).
    pub damage: f32,
    /// The hit points whose damage changed, with their new value.
    pub hit_points: Vec<(usize, f32)>,
    /// Whether this hit destroyed the Object.
    pub destroyed: bool,
    /// Whether the Object has become its ruin: its type and model are now the ruin's
    /// (`docs/re/sim-damage.md` §7.2).
    pub ruined: bool,
}

/// The `HandleDamage` seam (`docs/re/sim-damage.md` §5): every engine hit is offered to the
/// handler with the values it would leave, and the handler returns the values to store. The SQF
/// event handler registry that will dispatch it comes with the mission framework; until then a
/// host may install one for gameplay rules.
pub trait DamageHandler: fmt::Debug + Send {
    /// Sees the World read-only: the handler cannot damage anything itself, and the damage path
    /// is not re-entered.
    fn handle_damage(&mut self, world: &World, request: &DamageRequest) -> DamageReply;
}

/// One `HandleDamage` call.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    pub entity: EntityId,
    /// The total damage the hit would leave (0..1).
    pub damage: f32,
    /// The hit points the hit would change, with the values it would leave.
    pub hit_points: Vec<(usize, f32)>,
    pub source: Option<EntityId>,
    pub instigator: Option<EntityId>,
    /// What the call is about.
    pub context: DamageContext,
}

/// What a [`DamageHandler`] returns: the values to store instead, absolutely, as the event's
/// return value is. A [`default`](DamageReply::default) reply stores what the hit computed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DamageReply {
    /// The total damage to store instead.
    pub damage: Option<f32>,
    /// Hit points to store instead, by index; ones not listed keep what the hit left.
    pub hit_points: Vec<(usize, f32)>,
}

impl DamageReply {
    /// The reply that stores what the hit computed.
    pub fn allow() -> Self {
        Self::default()
    }
}

/// `getAllHitPointsDamage`: `[hitpointNames, selectionNames, damageValues]`, all three in hit
/// point order.
#[derive(Debug, Clone, PartialEq)]
pub struct AllHitPointsDamage {
    pub hit_points: Vec<String>,
    pub selections: Vec<String>,
    pub damage: Vec<f32>,
}

impl World {
    /// Installs the `HandleDamage` handler, replacing the previous one.
    pub fn set_damage_handler(&mut self, handler: Option<Box<dyn DamageHandler>>) {
        self.damage_handler = handler;
    }

    /// Applies a hit to an Object. A Static object is promoted first: a hit on terrain
    /// vegetation or on a building is what turns it into an Entity. Damage is applied on the
    /// machine that owns the Object (the original's local argument rule); for a remote Object
    /// this returns `None` — its owner applies the hit and the state arrives over the network
    /// (#131). Returns `None` too for an Object that does not exist or is being deleted, and for
    /// engine damage on a destroyed one.
    pub fn apply_damage(&mut self, object: ObjectRef, hit: DamageHit) -> Option<DamageOutcome> {
        let id = match object {
            ObjectRef::Entity(id) => id,
            ObjectRef::Static(key) => self.promote_static_with_model_type(key).ok()?,
        };
        self.apply_damage_to(id, hit)
    }

    /// [`apply_damage`](Self::apply_damage) on an Entity ID.
    pub fn apply_damage_to(&mut self, entity: EntityId, hit: DamageHit) -> Option<DamageOutcome> {
        let entity_type = self
            .entity(entity)
            .filter(|e| !e.is_deleted() && e.is_local())
            .map(|e| e.entity_type().clone())?;
        if hit.origin == DamageOrigin::Engine && !self.entity(entity)?.damage_allowed() {
            return None;
        }
        let model = entity_type.damage();

        let before = self.entity(entity)?.damage_state().clone();
        if hit.origin == DamageOrigin::Engine && before.is_destroyed(model) {
            return None;
        }

        let mut next = before.clone();
        let mut written = Vec::new();
        let mut total_changed = false;
        if let Some(total) = hit.total {
            total_changed = match hit.mode {
                DamageMode::Add => next.add_total(total),
                DamageMode::Replace => next.set_total(total),
            };
        }
        for &(index, value) in &hit.hit_points {
            let changed = match hit.mode {
                DamageMode::Add => next.add_hit_point(index, value),
                DamageMode::Replace => next.set_hit_point(index, value),
            };
            if changed && !written.contains(&index) {
                written.push(index);
            }
        }
        next.recompute_depends(model, &written, total_changed);

        // HandleDamage. The handler is taken out of the World so it sees it read-only; a panic
        // in it loses the handler rather than the World.
        if hit.origin == DamageOrigin::Engine && self.damage_handler.is_some() {
            let changed = changed_hit_points(&before, &next);
            let request = DamageRequest {
                entity,
                damage: next.total(),
                hit_points: changed.clone(),
                source: hit.source,
                instigator: hit.instigator,
                context: if hit.hit_points.is_empty() {
                    DamageContext::TotalDamage
                } else {
                    DamageContext::HitPoint
                },
            };
            let mut handler = self.damage_handler.take().expect("checked above");
            let reply = handler.handle_damage(self, &request);
            self.damage_handler = Some(handler);

            let reply_total = reply.damage;
            let reply_hit_points = reply.hit_points;
            if let Some(damage) = reply_total {
                next.set_total(damage);
            }
            let mut written = Vec::new();
            for (index, value) in &reply_hit_points {
                if next.set_hit_point(*index, *value) {
                    written.push(*index);
                }
            }
            next.recompute_depends(model, &written, reply_total.is_some());
        }

        let changed = changed_hit_points(&before, &next);
        let destroyed = next.is_destroyed(model);
        let was_destroyed = before.is_destroyed(model);
        // An engine hit always damages something the event handler can name; a hit that carries
        // only a total leaves the hit points alone, so it is reported as a total change (`None`).
        let total_only = changed.is_empty() && hit.origin == DamageOrigin::Engine;
        let total_delta = next.total() - before.total();

        if let Some(e) = self.entity_mut(entity) {
            e.set_damage_state(next);
        }

        for &(index, value) in &changed {
            self.record(WorldEvent::Dammaged {
                entity,
                hit_point: Some(index),
                damage: value - before.hit_point(index),
                source: hit.source,
            });
        }
        if total_only && total_delta != 0.0 {
            self.record(WorldEvent::Dammaged {
                entity,
                hit_point: None,
                damage: total_delta,
                source: hit.source,
            });
        }

        let mut ruined = false;
        if destroyed && !was_destroyed {
            self.record(WorldEvent::Killed {
                entity,
                killer: hit.source,
                instigator: hit.instigator,
                use_effects: hit.use_effects,
            });
            if hit.use_effects {
                ruined = self.apply_ruin(entity);
            }
        }

        Some(DamageOutcome {
            damage: self.entity(entity)?.damage(),
            hit_points: changed,
            destroyed: destroyed && !was_destroyed,
            ruined,
        })
    }

    /// Replaces a destroyed Entity's type and model with the ruin its type names
    /// (`docs/re/sim-damage.md` §7.2), dropping the Static object record the promoted Object had:
    /// the terrain object is gone and the Entity a script holds is the ruin. Returns whether a
    /// ruin was applied; a type without a `simulation = "ruin"` entry keeps the destroyed Entity.
    ///
    /// The ruin's class is the one the World's model type resolver gives for the entry's `type`
    /// (a model path), as the original's model-path registry does; without a resolver, and for a
    /// ruin class the config does not have, the type is the plain one named `Land_` + the model's
    /// file stem with the destroyed Entity's class.
    fn apply_ruin(&mut self, entity: EntityId) -> bool {
        let Some(entity_type) = self.entity(entity).map(|e| e.entity_type().clone()) else {
            return false;
        };
        let Some(ruin) = entity_type.damage().ruins().first().cloned() else {
            return false;
        };
        let resolved = self
            .model_type_resolver
            .as_mut()
            .and_then(|resolver| resolver.resolve(&ruin.model))
            .filter(|ty| !ty.model().is_empty());
        let ruin_type = resolved.unwrap_or_else(|| {
            Arc::new(EntityType::new(ruin.class_name(), entity_type.class()).with_model(ruin.model))
        });
        if let Some(key) = self
            .entity(entity)
            .and_then(|e| e.network_id())
            .and_then(StaticKey::from_network_id)
        {
            self.forget_static(key);
        }
        match self.entity_mut(entity) {
            Some(e) => {
                e.set_entity_type(ruin_type);
                // The ruin's model has its own hit points; the totals it is left with keep it
                // destroyed (`DamageState::is_destroyed`). A script can still `setDamage` it.
                let total = e.damage();
                let mut state = DamageState::new(e.entity_type().damage());
                state.set_total(total);
                e.set_damage_state(state);
                true
            }
            None => false,
        }
    }

    /// Total damage as `damage`/`getDammage` read it; 0 for an Object that is not an Entity (a
    /// Static object is intact until it is promoted).
    pub fn damage_of(&self, object: ObjectRef) -> Option<f32> {
        match object {
            ObjectRef::Entity(id) => self.entity(id).map(|e| e.damage()),
            ObjectRef::Static(_) => Some(0.0),
        }
    }

    /// One hit point's damage by config name, as `getHitPointDamage` reads it; `None` for an
    /// Object without that hit point.
    pub fn hit_point_damage(&self, object: ObjectRef, name: &str) -> Option<f32> {
        let ObjectRef::Entity(id) = object else {
            return None;
        };
        let entity = self.entity(id)?;
        let index = entity.entity_type().damage().hit_point_index(name)?;
        Some(entity.hit_point_damage(index))
    }

    /// One hit point's damage by model selection, as `getHit` reads it.
    pub fn hit_point_damage_of_selection(&self, object: ObjectRef, selection: &str) -> Option<f32> {
        let ObjectRef::Entity(id) = object else {
            return None;
        };
        let entity = self.entity(id)?;
        let index = entity
            .entity_type()
            .damage()
            .hit_point_index_of_selection(selection)?;
        Some(entity.hit_point_damage(index))
    }

    /// One hit point's damage by index, as `getHitIndex` reads it.
    pub fn hit_point_damage_of_index(&self, object: ObjectRef, index: usize) -> Option<f32> {
        let ObjectRef::Entity(id) = object else {
            return None;
        };
        Some(self.entity(id)?.hit_point_damage(index))
    }

    /// Every hit point's damage, as `getAllHitPointsDamage` reads it; `None` for an Object that
    /// is not an Entity. An Entity whose type has no hit points gives empty arrays.
    pub fn all_hit_points_damage(&self, object: ObjectRef) -> Option<AllHitPointsDamage> {
        let ObjectRef::Entity(id) = object else {
            return None;
        };
        let entity = self.entity(id)?;
        let model = entity.entity_type().damage();
        Some(AllHitPointsDamage {
            hit_points: model.hit_points().iter().map(|p| p.name.clone()).collect(),
            selections: model
                .hit_points()
                .iter()
                .map(|p| p.selection.clone())
                .collect(),
            damage: entity.damage_state().hit_points().to_vec(),
        })
    }

    /// Whether the Object takes engine damage (`isDamageAllowed`; the command also requires the
    /// Object to be local).
    pub fn damage_allowed(&self, object: ObjectRef) -> Option<bool> {
        match object {
            ObjectRef::Entity(id) => self.entity(id).map(|e| e.damage_allowed()),
            // `allowDamage` does not apply to terrain vegetation.
            ObjectRef::Static(_) => Some(false),
        }
    }

    /// Sets whether the Object takes engine damage (`allowDamage`). Returns `false` when the
    /// Object is not an Entity: the command does not work on a Static object.
    pub fn set_damage_allowed(&mut self, object: ObjectRef, allowed: bool) -> bool {
        let ObjectRef::Entity(id) = object else {
            return false;
        };
        match self.entity_mut(id) {
            Some(e) if e.is_local() => {
                e.set_damage_allowed(allowed);
                true
            }
            _ => false,
        }
    }
}

/// The hit points whose damage differs between two states, with their new value.
fn changed_hit_points(before: &DamageState, after: &DamageState) -> Vec<(usize, f32)> {
    (0..after.hit_points().len())
        .filter(|&index| after.hit_point(index) != before.hit_point(index))
        .map(|index| (index, after.hit_point(index)))
        .collect()
}
