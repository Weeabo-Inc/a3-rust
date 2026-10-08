//! Glue between the VM and the preprocessor ([`a3_preproc`]).
//!
//! - [`preprocess_with_host`] runs a script file through the preprocessor, reading the file and
//!   its `#include`s with [`Host::load_file`]. It backs the default [`Host::preprocess_file`],
//!   and so `preprocessFile`, `preprocessFileLineNumbers`, `execVM` and `compileScript`.
//! - [`SqfEvaluator`] runs config `__EXEC` / `__EVAL` on the VM, in `parsingNamespace`.

use std::cell::RefCell;

use a3_preproc::{
    EvalValue, Evaluator, IncludeError, IncludeResolver, Preprocessor, ResolvedInclude,
    join_virtual_path,
};

use crate::host::Host;
use crate::symbol::Sym;
use crate::value::{Namespace, Value};
use crate::vm::Vm;

/// An [`IncludeResolver`] that reads files through [`Host::load_file`].
///
/// Paths that start with `\` are absolute in the VFS. A relative include in a file whose own
/// path is relative (a mission script such as `scripts\init.sqf`) stays relative, so the host can
/// look it up in the mission folder first, as the engine does.
pub struct HostResolver<'a, H: Host + ?Sized> {
    host: RefCell<&'a mut H>,
}

impl<'a, H: Host + ?Sized> HostResolver<'a, H> {
    /// A resolver over `host`.
    pub fn new(host: &'a mut H) -> Self {
        Self {
            host: RefCell::new(host),
        }
    }

    fn path(current_file: &str, include: &str) -> String {
        let joined = join_virtual_path(current_file, include);
        let relative = !current_file.starts_with(['\\', '/']) && !include.starts_with(['\\', '/']);
        if relative {
            joined.trim_start_matches('\\').to_owned()
        } else {
            joined
        }
    }
}

impl<H: Host + ?Sized> IncludeResolver for HostResolver<'_, H> {
    fn resolve(&self, current_file: &str, include: &str) -> Result<ResolvedInclude, IncludeError> {
        let path = Self::path(current_file, include);
        let source = self
            .host
            .borrow_mut()
            .load_file(&path)
            .map_err(|_| IncludeError::NotFound(path.clone()))?;
        Ok(ResolvedInclude { path, source })
    }
}

/// Loads `path` through `host` and preprocesses it as SQF. With `line_numbers`, the result
/// carries `#line` directives (`preprocessFileLineNumbers`). Preprocessor warnings go to
/// [`Host::diag_log`]; an error is returned as its message.
pub fn preprocess_with_host<H: Host + ?Sized>(
    host: &mut H,
    path: &str,
    line_numbers: bool,
) -> Result<String, String> {
    let source = host.load_file(path)?;
    let (result, warnings) = {
        let resolver = HostResolver::new(host);
        let mut pp = Preprocessor::new(&resolver);
        match pp.preprocess_str(path, &source) {
            Ok(out) => {
                let text = if line_numbers {
                    out.with_line_directives()
                } else {
                    out.text
                };
                (Ok(text), out.warnings)
            }
            Err(e) => (Err(e.to_string()), Vec::new()),
        }
    };
    for warning in warnings {
        host.diag_log(&format!("Warning: preprocessor: {warning}"));
    }
    result
}

/// Runs config `__EXEC` / `__EVAL` as SQF on a [`Vm`], as the engine does: in
/// `parsingNamespace`, with the private variables of one call visible to the next.
///
/// Private variables are kept in `parsingNamespace` under their `_` names. Results other than
/// numbers and strings become their `str` text (Booleans `"true"`/`"false"`).
pub struct SqfEvaluator<'a, H: Host> {
    vm: &'a mut Vm<H>,
}

impl<'a, H: Host> SqfEvaluator<'a, H> {
    /// An evaluator running on `vm`.
    pub fn new(vm: &'a mut Vm<H>) -> Self {
        Self { vm }
    }

    fn run(&mut self, name: &str, code: &str) -> Result<Value, String> {
        let code = self
            .vm
            .compile_file(name, code)
            .map_err(|e| e.message.clone())?;
        let locals: Vec<(Sym, Value)> = self
            .vm
            .namespace(Namespace::Parsing)
            .iter()
            .filter(|(name, _)| name.as_str().starts_with('_'))
            .map(|(name, value)| (name, value.clone()))
            .collect();
        let (result, locals) = self.vm.call_with_locals(&code, Namespace::Parsing, locals);
        let parsing = self.vm.namespace_mut(Namespace::Parsing);
        for (name, value) in locals {
            parsing.set(name, value);
        }
        result.map_err(|e| e.error.to_string())
    }
}

impl<H: Host> Evaluator for SqfEvaluator<'_, H> {
    fn exec(&mut self, code: &str) -> Result<(), String> {
        self.run("__EXEC", code).map(|_| ())
    }

    fn eval(&mut self, expression: &str) -> Result<EvalValue, String> {
        Ok(match self.run("__EVAL", expression)? {
            Value::Number(n) => EvalValue::Number(f64::from(n)),
            Value::String(s) => EvalValue::String(s.to_string()),
            other => EvalValue::String(other.to_sqf_string()),
        })
    }
}
