//! The command signature table: every script command's name, forms and
//! argument types.
//!
//! The parser needs to know, for every identifier, whether it is a nular,
//! unary or binary command (or several), and each binary command's
//! precedence. The table is loaded from `data/commands.tsv` (derived from the
//! game's command table and the community wiki), so every command parses even
//! before it has an implementation. Hosts may [`declare`](CommandTable::declare)
//! further commands.

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use crate::types::TypeSet;

/// Index of a command name in a [`CommandTable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommandId(pub u32);

/// The syntactic form of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Form {
    /// No arguments: `time`.
    Nular,
    /// One argument on the right: `count _arr`.
    Unary,
    /// Arguments on both sides: `_a select 1`.
    Binary,
}

impl Form {
    fn parse(s: &str) -> Option<Form> {
        match s.trim().to_ascii_lowercase().as_str() {
            "nular" | "nullary" | "n" => Some(Form::Nular),
            "unary" | "u" => Some(Form::Unary),
            "binary" | "b" => Some(Form::Binary),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Form::Nular => "nular",
            Form::Unary => "unary",
            Form::Binary => "binary",
        }
    }
}

/// One overload of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Signature {
    /// Left argument types (binary only, otherwise empty).
    pub left: TypeSet,
    /// Right argument types (unary and binary, otherwise empty).
    pub right: TypeSet,
    /// Return types.
    pub ret: TypeSet,
}

impl Signature {
    pub const fn nular(ret: TypeSet) -> Signature {
        Signature {
            left: TypeSet::EMPTY,
            right: TypeSet::EMPTY,
            ret,
        }
    }

    pub const fn unary(right: TypeSet, ret: TypeSet) -> Signature {
        Signature {
            left: TypeSet::EMPTY,
            right,
            ret,
        }
    }

    pub const fn binary(left: TypeSet, right: TypeSet, ret: TypeSet) -> Signature {
        Signature { left, right, ret }
    }
}

/// Everything the table knows about one command name.
#[derive(Clone, Debug)]
pub struct CommandInfo {
    /// Canonical spelling (`createHashMapFromArray`, `==`).
    pub name: String,
    pub nular: Option<Signature>,
    pub unary: Vec<Signature>,
    pub binary: Vec<Signature>,
    /// Binary precedence; higher binds tighter. See [`binary_precedence`].
    pub precedence: u8,
}

impl CommandInfo {
    pub fn has_form(&self, form: Form) -> bool {
        match form {
            Form::Nular => self.nular.is_some(),
            Form::Unary => !self.unary.is_empty(),
            Form::Binary => !self.binary.is_empty(),
        }
    }
}

/// Binary operator precedence levels (higher binds tighter), as listed on
/// the community wiki's "Operators" page and in the engine's operator
/// table:
///
/// | level | operators                               |
/// |-------|-----------------------------------------|
/// | 9     | `#`                                     |
/// | 8     | `^`                                     |
/// | 7     | `* / % mod atan2`                       |
/// | 6     | `+ - min max`                           |
/// | 5     | `else`                                  |
/// | 4     | every other binary command              |
/// | 3     | `== != > < >= <= >>`                    |
/// | 2     | `&& and`                                |
/// | 1     | `\|\| or`                               |
///
/// Unary commands bind tighter than any binary command; all binary levels
/// associate to the left.
pub fn binary_precedence(name: &str) -> u8 {
    match name.to_ascii_lowercase().as_str() {
        "#" => 9,
        "^" => 8,
        "*" | "/" | "%" | "mod" | "atan2" => 7,
        "+" | "-" | "min" | "max" => 6,
        "else" => 5,
        "==" | "!=" | ">" | "<" | ">=" | "<=" | ">>" => 3,
        "&&" | "and" => 2,
        "||" | "or" => 1,
        _ => 4,
    }
}

/// An error in the signature table text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("commands table line {line}: {message}")]
pub struct TableError {
    pub line: usize,
    pub message: String,
}

/// The set of known commands.
#[derive(Clone, Default)]
pub struct CommandTable {
    by_name: HashMap<String, CommandId>,
    commands: Vec<CommandInfo>,
}

/// The signature table shipped with the crate.
pub const BUILTIN_TABLE_TSV: &str = include_str!("../data/commands.tsv");

impl CommandTable {
    /// An empty table.
    pub fn new() -> CommandTable {
        CommandTable::default()
    }

    /// The table parsed from the shipped `data/commands.tsv`.
    pub fn builtin() -> CommandTable {
        static BUILTIN: OnceLock<CommandTable> = OnceLock::new();
        BUILTIN
            .get_or_init(|| {
                CommandTable::from_tsv(BUILTIN_TABLE_TSV).expect("shipped commands.tsv is valid")
            })
            .clone()
    }

    /// Parses a table in the `commands.tsv` format: one overload per line,
    /// tab-separated `name`, `form` (`nular`/`unary`/`binary`), `left`
    /// types, `right` types, `return` types. Types are `typeName` tokens
    /// separated by `,`; `ANY` means every type. Lines starting with `# `
    /// (hash, space) and blank lines are ignored.
    pub fn from_tsv(text: &str) -> Result<CommandTable, TableError> {
        let mut table = CommandTable::new();
        for (i, raw) in text.lines().enumerate() {
            let line_no = i + 1;
            let line = raw.trim_end_matches('\r');
            // `# ` starts a comment; the `#` command itself is `#<TAB>binary...`.
            if line.trim().is_empty() || line.starts_with("# ") {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            let err = |message: String| TableError {
                line: line_no,
                message,
            };
            if cols.len() < 5 {
                return Err(err(format!("expected 5 columns, found {}", cols.len())));
            }
            let form =
                Form::parse(cols[1]).ok_or_else(|| err(format!("bad form {:?}", cols[1])))?;
            let parse = |s: &str| TypeSet::parse(s).map_err(|t| err(format!("unknown type {t:?}")));
            let sig = Signature {
                left: parse(cols[2])?,
                right: parse(cols[3])?,
                ret: parse(cols[4])?,
            };
            table.declare(cols[0], form, sig);
        }
        Ok(table)
    }

    /// Serialises the table in the format [`from_tsv`](Self::from_tsv)
    /// reads, sorted by name and form.
    pub fn to_tsv(&self) -> String {
        let mut rows = Vec::new();
        for info in &self.commands {
            let mut push = |form: Form, s: &Signature| {
                rows.push(format!(
                    "{}\t{}\t{}\t{}\t{}",
                    info.name,
                    form.as_str(),
                    s.left.to_tokens_or_empty(),
                    s.right.to_tokens_or_empty(),
                    s.ret.to_tokens_or_empty()
                ));
            };
            if let Some(s) = &info.nular {
                push(Form::Nular, s);
            }
            for s in &info.unary {
                push(Form::Unary, s);
            }
            for s in &info.binary {
                push(Form::Binary, s);
            }
        }
        rows.sort_by_key(|r| r.to_ascii_lowercase());
        let mut out = String::from("# name\tform\tleft\tright\treturn\n");
        for r in rows {
            out.push_str(&r);
            out.push('\n');
        }
        out
    }

    /// Adds an overload, creating the command if the name is new. Returns the
    /// command's id. An identical overload is not added twice.
    pub fn declare(&mut self, name: &str, form: Form, sig: Signature) -> CommandId {
        let id = self.intern(name);
        let info = &mut self.commands[id.0 as usize];
        match form {
            Form::Nular => {
                info.nular.get_or_insert(sig);
            }
            Form::Unary => {
                if !info.unary.contains(&sig) {
                    info.unary.push(sig);
                }
            }
            Form::Binary => {
                if !info.binary.contains(&sig) {
                    info.binary.push(sig);
                }
            }
        }
        id
    }

    /// The id of `name`, creating an entry with no forms if it is new.
    pub fn intern(&mut self, name: &str) -> CommandId {
        let key = name.to_ascii_lowercase();
        if let Some(&id) = self.by_name.get(&key) {
            return id;
        }
        let id = CommandId(self.commands.len() as u32);
        self.commands.push(CommandInfo {
            name: name.to_string(),
            nular: None,
            unary: Vec::new(),
            binary: Vec::new(),
            precedence: binary_precedence(name),
        });
        self.by_name.insert(key, id);
        id
    }

    /// Looks a name up, ignoring ASCII case.
    pub fn lookup(&self, name: &str) -> Option<CommandId> {
        if name.bytes().any(|b| b.is_ascii_uppercase()) {
            self.by_name.get(&name.to_ascii_lowercase()).copied()
        } else {
            self.by_name.get(name).copied()
        }
    }

    pub fn get(&self, id: CommandId) -> &CommandInfo {
        &self.commands[id.0 as usize]
    }

    /// Whether `name` is a command with the given form.
    pub fn has(&self, name: &str, form: Form) -> bool {
        self.lookup(name)
            .is_some_and(|id| self.get(id).has_form(form))
    }

    /// Number of distinct command names.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// All commands, indexed by [`CommandId`].
    pub fn iter(&self) -> impl Iterator<Item = (CommandId, &CommandInfo)> {
        self.commands
            .iter()
            .enumerate()
            .map(|(i, c)| (CommandId(i as u32), c))
    }

    /// Number of overloads of each form: `(nular, unary, binary)`.
    pub fn form_counts(&self) -> (usize, usize, usize) {
        self.commands.iter().fold((0, 0, 0), |(n, u, b), c| {
            (
                n + usize::from(c.nular.is_some()),
                u + c.unary.len(),
                b + c.binary.len(),
            )
        })
    }
}

impl fmt::Debug for CommandTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommandTable({} commands)", self.commands.len())
    }
}

impl TypeSet {
    fn to_tokens_or_empty(self) -> String {
        if self.is_empty() {
            String::new()
        } else {
            self.to_tokens()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Type;

    #[test]
    fn tsv_round_trip() {
        let text = "# name\tform\tleft\tright\treturn\n\
                    count\tunary\t\tARRAY,STRING\tSCALAR\n\
                    count\tbinary\tCODE\tARRAY\tSCALAR\n\
                    time\tnular\t\t\tSCALAR\n";
        let table = CommandTable::from_tsv(text).unwrap();
        assert_eq!(table.len(), 2);
        let count = table.lookup("COUNT").unwrap();
        assert!(table.get(count).has_form(Form::Unary));
        assert!(table.get(count).has_form(Form::Binary));
        assert!(!table.get(count).has_form(Form::Nular));
        assert!(table.get(count).unary[0].right.contains(Type::String));
        let again = CommandTable::from_tsv(&table.to_tsv()).unwrap();
        assert_eq!(again.to_tsv(), table.to_tsv());
    }

    #[test]
    fn precedence_levels() {
        assert!(binary_precedence("#") > binary_precedence("^"));
        assert!(binary_precedence("^") > binary_precedence("*"));
        assert!(binary_precedence("mod") > binary_precedence("max"));
        assert!(binary_precedence("+") > binary_precedence("else"));
        assert!(binary_precedence("else") > binary_precedence("select"));
        assert!(binary_precedence("select") > binary_precedence(">>"));
        assert!(binary_precedence("==") > binary_precedence("and"));
        assert!(binary_precedence("AND") > binary_precedence("or"));
    }

    #[test]
    fn builtin_table_knows_core_commands() {
        let t = CommandTable::builtin();
        for (name, form) in [
            ("+", Form::Binary),
            ("-", Form::Unary),
            ("select", Form::Binary),
            ("count", Form::Unary),
            ("count", Form::Binary),
            ("true", Form::Nular),
            ("if", Form::Unary),
            ("then", Form::Binary),
            ("forEach", Form::Binary),
            ("private", Form::Unary),
        ] {
            assert!(t.has(name, form), "{name} {form:?}");
        }
    }
}
