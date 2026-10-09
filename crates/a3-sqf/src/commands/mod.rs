//! The world-independent script commands.
//!
//! Each submodule registers one area. Argument types are checked by the
//! registry before a command runs, so implementations match on the value
//! variants their signature allows.

mod array;
mod compare;
mod control;
mod diag;
mod env;
mod extra;
mod fsm;
mod hashmap;
mod json;
mod logic;
mod math;
mod misc;
mod object;
mod regex;
mod string;
mod text;
mod vars;

use crate::code::Code;
use crate::error::SqfError;
use crate::host::Host;
use crate::registry::Registry;
use crate::types::{Type, TypeSet};
use crate::value::{Array, Value};

pub use control::{ExitScope, Loop};

/// Registers every core command of this crate.
pub fn register_core<H: Host>(r: &mut Registry<H>) {
    math::register(r);
    logic::register(r);
    compare::register(r);
    control::register(r);
    vars::register(r);
    misc::register(r);
    string::register(r);
    array::register(r);
    hashmap::register(r);
    regex::register(r);
    extra::register(r);
    diag::register(r);
    text::register(r);
    json::register(r);
    object::register(r);
    env::register(r);
    fsm::register(r);
}

pub(crate) const NUM: TypeSet = TypeSet::NUMBER;
pub(crate) const BOOL: TypeSet = TypeSet::of(Type::Bool);
pub(crate) const STR: TypeSet = TypeSet::of(Type::String);
pub(crate) const ARR: TypeSet = TypeSet::of(Type::Array);
pub(crate) const CODE: TypeSet = TypeSet::of(Type::Code);
pub(crate) const HASH: TypeSet = TypeSet::of(Type::HashMap);
pub(crate) const NS: TypeSet = TypeSet::of(Type::Namespace);
pub(crate) const SIDE: TypeSet = TypeSet::of(Type::Side);
pub(crate) const SCRIPT: TypeSet = TypeSet::of(Type::Script);
pub(crate) const ANY: TypeSet = TypeSet::ANYTHING;
pub(crate) const NOTHING: TypeSet = TypeSet::of(Type::Nothing);

pub(crate) fn num(v: &Value) -> f32 {
    match v {
        Value::Number(n) => *n,
        _ => 0.0,
    }
}

pub(crate) fn boolean(v: &Value) -> bool {
    matches!(v, Value::Bool(true))
}

pub(crate) fn string(v: &Value) -> &str {
    match v {
        Value::String(s) => s,
        _ => "",
    }
}

pub(crate) fn array(v: &Value) -> Array {
    match v {
        Value::Array(a) => a.clone(),
        _ => Array::new(),
    }
}

pub(crate) fn expect_num(v: &Value) -> Result<f32, SqfError> {
    match v {
        Value::Number(n) => Ok(*n),
        _ => Err(SqfError::type_error(v, NUM)),
    }
}

pub(crate) fn expect_str(v: &Value) -> Result<&str, SqfError> {
    match v {
        Value::String(s) => Ok(s),
        _ => Err(SqfError::type_error(v, STR)),
    }
}

pub(crate) fn expect_code(v: &Value) -> Result<Code, SqfError> {
    match v {
        Value::Code(c) => Ok(c.clone()),
        _ => Err(SqfError::type_error(v, CODE)),
    }
}

/// An index argument. The engine converts with the CPU's default rounding
/// (`cvtss2si`, round half to even): `select 0.5` is element 0, `select
/// 1.5` element 2, `select 0.6` element 1.
pub(crate) fn index(n: f32) -> i64 {
    n.round_ties_even() as i64
}
