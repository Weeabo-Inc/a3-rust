//! Boolean logic. `a && {b}` and `a || {b}` evaluate the code only when
//! needed (lazy evaluation).

use super::*;
use crate::vm::{Flow, Invoke};

pub(super) fn register<H: Host>(r: &mut Registry<H>) {
    r.nular("true", BOOL, |_| Ok(Value::Bool(true)));
    r.nular("false", BOOL, |_| Ok(Value::Bool(false)));
    r.nular("nil", ANY, |_| Ok(Value::Nil));
    for name in ["!", "not"] {
        r.unary(name, BOOL, BOOL, |_, a| Ok(Value::Bool(!boolean(&a))));
    }
    for name in ["&&", "and"] {
        r.binary(name, BOOL, BOOL, BOOL, |_, a, b| {
            Ok(Value::Bool(boolean(&a) && boolean(&b)))
        });
        r.binary_flow(name, BOOL, CODE, BOOL, |_, a, b| {
            if !boolean(&a) {
                return Ok(Flow::Value(Value::Bool(false)));
            }
            Ok(Flow::Call(Invoke::new(expect_code(&b)?)))
        });
    }
    for name in ["||", "or"] {
        r.binary(name, BOOL, BOOL, BOOL, |_, a, b| {
            Ok(Value::Bool(boolean(&a) || boolean(&b)))
        });
        r.binary_flow(name, BOOL, CODE, BOOL, |_, a, b| {
            if boolean(&a) {
                return Ok(Flow::Value(Value::Bool(true)));
            }
            Ok(Flow::Call(Invoke::new(expect_code(&b)?)))
        });
    }
}
