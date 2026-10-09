//! The command registry: implementations of script commands, keyed by the
//! [`CommandTable`] ids the compiler emits.
//!
//! The registry is open: the core commands live in this crate
//! ([`crate::commands::register_core`]); world, UI and config commands are
//! registered by their own crates against the same [`Registry<H>`]. A
//! command may have several overloads per form; the VM picks the first whose
//! argument types match. Registering an implementation also declares its
//! signature in the table, so commands unknown to `data/commands.tsv` work
//! too.

use crate::error::SqfError;
use crate::host::Host;
use crate::table::{CommandId, CommandTable, Form, Signature};
use crate::types::TypeSet;
use crate::value::Value;
use crate::vm::{Ctx, Flow};

/// A nular command returning a value.
pub type NularFn<H> = fn(&mut Ctx<'_, H>) -> Result<Value, SqfError>;
/// A unary command returning a value.
pub type UnaryFn<H> = fn(&mut Ctx<'_, H>, Value) -> Result<Value, SqfError>;
/// A binary command returning a value.
pub type BinaryFn<H> = fn(&mut Ctx<'_, H>, Value, Value) -> Result<Value, SqfError>;
/// A nular command that controls flow.
pub type NularFlowFn<H> = fn(&mut Ctx<'_, H>) -> Result<Flow<H>, SqfError>;
/// A unary command that controls flow.
pub type UnaryFlowFn<H> = fn(&mut Ctx<'_, H>, Value) -> Result<Flow<H>, SqfError>;
/// A binary command that controls flow.
pub type BinaryFlowFn<H> = fn(&mut Ctx<'_, H>, Value, Value) -> Result<Flow<H>, SqfError>;

pub(crate) enum NularImpl<H: Host> {
    Value(NularFn<H>),
    Flow(NularFlowFn<H>),
}

pub(crate) enum UnaryImpl<H: Host> {
    Value(UnaryFn<H>),
    Flow(UnaryFlowFn<H>),
}

pub(crate) enum BinaryImpl<H: Host> {
    Value(BinaryFn<H>),
    Flow(BinaryFlowFn<H>),
}

pub(crate) struct UnaryOverload<H: Host> {
    pub right: TypeSet,
    pub imp: UnaryImpl<H>,
}

pub(crate) struct BinaryOverload<H: Host> {
    pub left: TypeSet,
    pub right: TypeSet,
    pub imp: BinaryImpl<H>,
}

/// Command implementations for a VM with host `H`.
pub struct Registry<H: Host> {
    table: CommandTable,
    nular: Vec<Option<NularImpl<H>>>,
    unary: Vec<Vec<UnaryOverload<H>>>,
    binary: Vec<Vec<BinaryOverload<H>>>,
}

impl<H: Host> Default for Registry<H> {
    fn default() -> Self {
        Registry::new(CommandTable::builtin())
    }
}

impl<H: Host> Registry<H> {
    /// A registry with no implementations over `table`.
    pub fn new(table: CommandTable) -> Registry<H> {
        let mut r = Registry {
            table,
            nular: Vec::new(),
            unary: Vec::new(),
            binary: Vec::new(),
        };
        r.grow();
        r
    }

    /// The builtin table with every core command of this crate registered.
    pub fn with_core() -> Registry<H> {
        let mut r = Registry::default();
        crate::commands::register_core(&mut r);
        r
    }

    /// `table` (e.g. the builtin table extended with the engine's full command list) with every
    /// core command of this crate registered. Commands of `table` without an implementation
    /// parse and fail at run time.
    pub fn with_core_table(table: CommandTable) -> Registry<H> {
        let mut r = Registry::new(table);
        crate::commands::register_core(&mut r);
        r
    }

    fn grow(&mut self) {
        let n = self.table.len();
        self.nular.resize_with(n, || None);
        self.unary.resize_with(n, Vec::new);
        self.binary.resize_with(n, Vec::new);
    }

    fn declare(&mut self, name: &str, form: Form, sig: Signature) -> usize {
        let id = self.table.declare(name, form, sig);
        self.grow();
        id.0 as usize
    }

    /// The signature table (used by the compiler).
    pub fn table(&self) -> &CommandTable {
        &self.table
    }

    /// Registers a nular command.
    pub fn nular(&mut self, name: &str, ret: impl Into<TypeSet>, f: NularFn<H>) {
        let i = self.declare(name, Form::Nular, Signature::nular(ret.into()));
        self.nular[i] = Some(NularImpl::Value(f));
    }

    /// Registers a nular command that controls flow.
    pub fn nular_flow(&mut self, name: &str, ret: impl Into<TypeSet>, f: NularFlowFn<H>) {
        let i = self.declare(name, Form::Nular, Signature::nular(ret.into()));
        self.nular[i] = Some(NularImpl::Flow(f));
    }

    /// Registers a unary overload accepting `right`.
    pub fn unary(
        &mut self,
        name: &str,
        right: impl Into<TypeSet>,
        ret: impl Into<TypeSet>,
        f: UnaryFn<H>,
    ) {
        let right = right.into();
        let i = self.declare(name, Form::Unary, Signature::unary(right, ret.into()));
        self.unary[i].push(UnaryOverload {
            right,
            imp: UnaryImpl::Value(f),
        });
    }

    /// Registers a unary overload that controls flow.
    pub fn unary_flow(
        &mut self,
        name: &str,
        right: impl Into<TypeSet>,
        ret: impl Into<TypeSet>,
        f: UnaryFlowFn<H>,
    ) {
        let right = right.into();
        let i = self.declare(name, Form::Unary, Signature::unary(right, ret.into()));
        self.unary[i].push(UnaryOverload {
            right,
            imp: UnaryImpl::Flow(f),
        });
    }

    /// Registers a binary overload accepting `left` and `right`.
    pub fn binary(
        &mut self,
        name: &str,
        left: impl Into<TypeSet>,
        right: impl Into<TypeSet>,
        ret: impl Into<TypeSet>,
        f: BinaryFn<H>,
    ) {
        let (left, right) = (left.into(), right.into());
        let i = self.declare(
            name,
            Form::Binary,
            Signature::binary(left, right, ret.into()),
        );
        self.binary[i].push(BinaryOverload {
            left,
            right,
            imp: BinaryImpl::Value(f),
        });
    }

    /// Registers a binary overload that controls flow.
    pub fn binary_flow(
        &mut self,
        name: &str,
        left: impl Into<TypeSet>,
        right: impl Into<TypeSet>,
        ret: impl Into<TypeSet>,
        f: BinaryFlowFn<H>,
    ) {
        let (left, right) = (left.into(), right.into());
        let i = self.declare(
            name,
            Form::Binary,
            Signature::binary(left, right, ret.into()),
        );
        self.binary[i].push(BinaryOverload {
            left,
            right,
            imp: BinaryImpl::Flow(f),
        });
    }

    /// Whether `name` has an implementation of `form`.
    pub fn is_implemented(&self, name: &str, form: Form) -> bool {
        let Some(id) = self.table.lookup(name) else {
            return false;
        };
        let i = id.0 as usize;
        match form {
            Form::Nular => self.nular.get(i).is_some_and(Option::is_some),
            Form::Unary => self.unary.get(i).is_some_and(|v| !v.is_empty()),
            Form::Binary => self.binary.get(i).is_some_and(|v| !v.is_empty()),
        }
    }

    /// The argument types of the implemented overloads of `name` in `form`, in registration
    /// order (`left` empty for unary, both empty for nular). Empty when `name` has no
    /// implementation of that form. Return types are not tracked (`ret` is empty).
    pub fn overloads(&self, name: &str, form: Form) -> Vec<Signature> {
        let Some(id) = self.table.lookup(name) else {
            return Vec::new();
        };
        let i = id.0 as usize;
        match form {
            Form::Nular => self
                .nular
                .get(i)
                .and_then(Option::as_ref)
                .map(|_| Signature::nular(TypeSet::EMPTY))
                .into_iter()
                .collect(),
            Form::Unary => self.unary[i]
                .iter()
                .map(|o| Signature::unary(o.right, TypeSet::EMPTY))
                .collect(),
            Form::Binary => self.binary[i]
                .iter()
                .map(|o| Signature::binary(o.left, o.right, TypeSet::EMPTY))
                .collect(),
        }
    }

    /// Implementation coverage: `(implemented, declared)` overload counts
    /// per form, where an implemented name counts all its table overloads.
    pub fn coverage(&self) -> Coverage {
        let mut c = Coverage::default();
        for (id, info) in self.table.iter() {
            let i = id.0 as usize;
            if info.nular.is_some() {
                c.nular.1 += 1;
                if self.nular[i].is_some() {
                    c.nular.0 += 1;
                }
            }
            if !info.unary.is_empty() {
                c.unary.1 += 1;
                if !self.unary[i].is_empty() {
                    c.unary.0 += 1;
                }
            }
            if !info.binary.is_empty() {
                c.binary.1 += 1;
                if !self.binary[i].is_empty() {
                    c.binary.0 += 1;
                }
            }
        }
        c
    }

    /// Names (with form) declared in the table but not implemented.
    pub fn unimplemented(&self) -> Vec<(String, Form)> {
        let mut out = Vec::new();
        for (id, info) in self.table.iter() {
            let i = id.0 as usize;
            if info.nular.is_some() && self.nular[i].is_none() {
                out.push((info.name.clone(), Form::Nular));
            }
            if !info.unary.is_empty() && self.unary[i].is_empty() {
                out.push((info.name.clone(), Form::Unary));
            }
            if !info.binary.is_empty() && self.binary[i].is_empty() {
                out.push((info.name.clone(), Form::Binary));
            }
        }
        out
    }

    pub(crate) fn nular_impl(&self, id: CommandId) -> Option<&NularImpl<H>> {
        self.nular.get(id.0 as usize).and_then(Option::as_ref)
    }

    /// Picks the unary overload for `arg`, or the error to raise. `None`
    /// means the argument is `nil` and no overload takes `nil`: the engine
    /// then skips the command and its result is `nil`.
    pub(crate) fn unary_impl(
        &self,
        id: CommandId,
        arg: &Value,
    ) -> Result<Option<&UnaryImpl<H>>, SqfError> {
        let overloads = &self.unary[id.0 as usize];
        let ty = arg.ty();
        if let Some(o) = overloads.iter().find(|o| o.right.contains(ty)) {
            return Ok(Some(&o.imp));
        }
        if overloads.is_empty() {
            return Err(SqfError::Unimplemented(self.table.get(id).name.clone()));
        }
        if arg.is_nil() {
            return Ok(None);
        }
        let expected = overloads
            .iter()
            .fold(TypeSet::EMPTY, |acc, o| acc.union(o.right));
        Err(SqfError::Type { got: ty, expected })
    }

    /// Picks the binary overload for the arguments, or the error to raise.
    /// `None` means an argument is `nil` and no overload takes it (the
    /// result is `nil`).
    pub(crate) fn binary_impl(
        &self,
        id: CommandId,
        left: &Value,
        right: &Value,
    ) -> Result<Option<&BinaryImpl<H>>, SqfError> {
        let overloads = &self.binary[id.0 as usize];
        let (lt, rt) = (left.ty(), right.ty());
        if let Some(o) = overloads
            .iter()
            .find(|o| o.left.contains(lt) && o.right.contains(rt))
        {
            return Ok(Some(&o.imp));
        }
        if overloads.is_empty() {
            return Err(SqfError::Unimplemented(self.table.get(id).name.clone()));
        }
        if left.is_nil() || right.is_nil() {
            return Ok(None);
        }
        let left_ok: Vec<_> = overloads.iter().filter(|o| o.left.contains(lt)).collect();
        if left_ok.is_empty() {
            let expected = overloads
                .iter()
                .fold(TypeSet::EMPTY, |acc, o| acc.union(o.left));
            return Err(SqfError::Type { got: lt, expected });
        }
        let expected = left_ok
            .iter()
            .fold(TypeSet::EMPTY, |acc, o| acc.union(o.right));
        Err(SqfError::Type { got: rt, expected })
    }
}

/// Implementation coverage per form: `(implemented, declared)` names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub nular: (usize, usize),
    pub unary: (usize, usize),
    pub binary: (usize, usize),
}
