//! SQF value types, mirroring the engine's `GameType` set.
//!
//! Every [`Value`](crate::Value) has exactly one [`Type`]. Command signatures
//! describe which types they accept with a [`TypeSet`] (a bit mask), as the
//! engine does.

use std::fmt;

/// The type of one SQF value.
///
/// The `type_name` of each variant is the string the `typeName` command
/// returns; `display_name` is the name used in engine error messages
/// ("Error Type String, expected Number").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Type {
    /// `nil`: an undefined variable or the value of `nil`.
    Any = 0,
    /// The result of a command that returns nothing (e.g. `hint`).
    Nothing,
    Bool,
    /// A finite (or infinite) number. NaN numbers report [`Type::NaN`].
    Number,
    /// A number that is NaN. Stored like a number; reported separately by
    /// `typeName`.
    NaN,
    String,
    Array,
    HashMap,
    Code,
    Namespace,
    Side,
    Script,
    Config,
    Object,
    Group,
    Control,
    Display,
    Location,
    Task,
    TeamMember,
    DiaryRecord,
    Text,
    NetObject,
    Target,
    SubGroup,
    /// Engine-internal position/orientation types that appear in a few
    /// command signatures.
    Vector,
    Trans,
    Orient,
    /// Intermediate result of `if cond`.
    If,
    /// Intermediate result of `while {cond}`.
    While,
    /// Intermediate result of `for ...`.
    For,
    /// Intermediate result of `switch value`.
    Switch,
    /// Intermediate result of `with namespace`.
    With,
    /// Intermediate result of `try {code}`.
    Exception,
}

impl Type {
    /// Every type, in declaration order.
    pub const ALL: [Type; 34] = [
        Type::Any,
        Type::Nothing,
        Type::Bool,
        Type::Number,
        Type::NaN,
        Type::String,
        Type::Array,
        Type::HashMap,
        Type::Code,
        Type::Namespace,
        Type::Side,
        Type::Script,
        Type::Config,
        Type::Object,
        Type::Group,
        Type::Control,
        Type::Display,
        Type::Location,
        Type::Task,
        Type::TeamMember,
        Type::DiaryRecord,
        Type::Text,
        Type::NetObject,
        Type::Target,
        Type::SubGroup,
        Type::Vector,
        Type::Trans,
        Type::Orient,
        Type::If,
        Type::While,
        Type::For,
        Type::Switch,
        Type::With,
        Type::Exception,
    ];

    /// The string `typeName` returns for a value of this type.
    pub fn type_name(self) -> &'static str {
        match self {
            Type::Any => "ANY",
            Type::Nothing => "NOTHING",
            Type::Bool => "BOOL",
            Type::Number => "SCALAR",
            Type::NaN => "NaN",
            Type::String => "STRING",
            Type::Array => "ARRAY",
            Type::HashMap => "HASHMAP",
            Type::Code => "CODE",
            Type::Namespace => "NAMESPACE",
            Type::Side => "SIDE",
            Type::Script => "SCRIPT",
            Type::Config => "CONFIG",
            Type::Object => "OBJECT",
            Type::Group => "GROUP",
            Type::Control => "CONTROL",
            Type::Display => "DISPLAY",
            Type::Location => "LOCATION",
            Type::Task => "TASK",
            Type::TeamMember => "TEAM_MEMBER",
            Type::DiaryRecord => "DIARY_RECORD",
            Type::Text => "TEXT",
            Type::NetObject => "NetObject",
            Type::Target => "TARGET",
            Type::SubGroup => "SUBGROUP",
            Type::Vector => "VECTOR",
            Type::Trans => "TRANS",
            Type::Orient => "ORIENT",
            Type::If => "IF",
            Type::While => "WHILE",
            Type::For => "FOR",
            Type::Switch => "SWITCH",
            Type::With => "WITH",
            Type::Exception => "EXCEPTION",
        }
    }

    /// The name used for this type in engine error messages.
    ///
    /// _(uncertain: confirm the exact spelling of the rarer types against
    /// the binary.)_
    pub fn display_name(self) -> &'static str {
        match self {
            Type::Any => "Any",
            Type::Nothing => "Nothing",
            Type::Bool => "Bool",
            Type::Number => "Number",
            Type::NaN => "Not a Number",
            Type::String => "String",
            Type::Array => "Array",
            Type::HashMap => "HashMap",
            Type::Code => "Code",
            Type::Namespace => "Namespace",
            Type::Side => "Side",
            Type::Script => "Script",
            Type::Config => "Config entry",
            Type::Object => "Object",
            Type::Group => "Group",
            Type::Control => "Control",
            Type::Display => "Display",
            Type::Location => "Location",
            Type::Task => "Task",
            Type::TeamMember => "Team member",
            Type::DiaryRecord => "Diary record",
            Type::Text => "Text",
            Type::NetObject => "Network Object",
            Type::Target => "Target",
            Type::SubGroup => "Sub-group",
            Type::Vector => "Vector",
            Type::Trans => "Transformation",
            Type::Orient => "Orientation",
            Type::If => "If Type",
            Type::While => "While Type",
            Type::For => "For Type",
            Type::Switch => "Switch Type",
            Type::With => "With Type",
            Type::Exception => "Exception Type",
        }
    }

    /// Parses a type token as used in the command signature table: either a
    /// `typeName` string (`SCALAR`, `STRING`, ...) or a variant name.
    /// Case-insensitive.
    pub fn from_token(token: &str) -> Option<Type> {
        let t = token.trim();
        Type::ALL
            .iter()
            .copied()
            .find(|ty| {
                ty.type_name().eq_ignore_ascii_case(t) || format!("{ty:?}").eq_ignore_ascii_case(t)
            })
            .or_else(|| match t.to_ascii_uppercase().as_str() {
                "NUMBER" => Some(Type::Number),
                "BOOLEAN" => Some(Type::Bool),
                "NIL" => Some(Type::Any),
                "STRUCTUREDTEXT" | "STRUCTURED_TEXT" => Some(Type::Text),
                "TEAMMEMBER" => Some(Type::TeamMember),
                "DIARYRECORD" => Some(Type::DiaryRecord),
                "EXCEPTIONHANDLING" => Some(Type::Exception),
                _ => None,
            })
    }

    const fn bit(self) -> u64 {
        1u64 << (self as u8)
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// A set of [`Type`]s, used for command argument and return signatures.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TypeSet(u64);

impl TypeSet {
    /// No types at all.
    pub const EMPTY: TypeSet = TypeSet(0);
    /// Every type, including `nil` and nothing. Used by commands such as
    /// `str`, `isNil` and `typeName` that accept anything.
    pub const ANYTHING: TypeSet = TypeSet((1u64 << Type::ALL.len()) - 1);
    /// Numbers, including NaN.
    pub const NUMBER: TypeSet = TypeSet(Type::Number.bit() | Type::NaN.bit());

    /// The set holding only `ty`. A number type also admits NaN.
    pub const fn of(ty: Type) -> TypeSet {
        match ty {
            Type::Number => TypeSet::NUMBER,
            _ => TypeSet(ty.bit()),
        }
    }

    /// Whether `ty` is a member.
    pub const fn contains(self, ty: Type) -> bool {
        self.0 & ty.bit() != 0
    }

    /// The union of two sets.
    pub const fn union(self, other: TypeSet) -> TypeSet {
        TypeSet(self.0 | other.0)
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The member types in declaration order.
    pub fn iter(self) -> impl Iterator<Item = Type> {
        Type::ALL.into_iter().filter(move |t| self.contains(*t))
    }

    /// Parses a comma- or `|`-separated list of type tokens. `ANY` (or
    /// `ANYTHING`) means every type. Unknown tokens are reported as `Err`.
    pub fn parse(list: &str) -> Result<TypeSet, String> {
        let mut set = TypeSet::EMPTY;
        for tok in list
            .split([',', '|'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            if tok.eq_ignore_ascii_case("ANY") || tok.eq_ignore_ascii_case("ANYTHING") {
                set = TypeSet::ANYTHING;
                continue;
            }
            match Type::from_token(tok) {
                Some(t) => set = set.union(TypeSet::of(t)),
                None => return Err(tok.to_string()),
            }
        }
        Ok(set)
    }

    /// Formats the set the way engine error messages list expected types:
    /// display names joined with `,`.
    pub fn display_list(self) -> String {
        if self == TypeSet::ANYTHING {
            return "Anything".to_string();
        }
        self.iter()
            .map(Type::display_name)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Formats the set as signature-table tokens (`typeName` strings joined
    /// with `,`), or `ANY` for every type.
    pub fn to_tokens(self) -> String {
        if self == TypeSet::ANYTHING {
            return "ANY".to_string();
        }
        let mut out = Vec::new();
        for t in self.iter() {
            // NaN is implied by SCALAR.
            if t == Type::NaN && self.contains(Type::Number) {
                continue;
            }
            out.push(t.type_name());
        }
        out.join(",")
    }
}

impl From<Type> for TypeSet {
    fn from(t: Type) -> Self {
        TypeSet::of(t)
    }
}

impl std::ops::BitOr for TypeSet {
    type Output = TypeSet;
    fn bitor(self, rhs: TypeSet) -> TypeSet {
        self.union(rhs)
    }
}

impl std::ops::BitOr<Type> for TypeSet {
    type Output = TypeSet;
    fn bitor(self, rhs: Type) -> TypeSet {
        self.union(TypeSet::of(rhs))
    }
}

impl std::ops::BitOr for Type {
    type Output = TypeSet;
    fn bitor(self, rhs: Type) -> TypeSet {
        TypeSet::of(self).union(TypeSet::of(rhs))
    }
}

impl fmt::Debug for TypeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypeSet({})", self.to_tokens())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_match_type_name_command() {
        assert_eq!(Type::Number.type_name(), "SCALAR");
        assert_eq!(Type::Bool.type_name(), "BOOL");
        assert_eq!(Type::HashMap.type_name(), "HASHMAP");
        assert_eq!(Type::TeamMember.type_name(), "TEAM_MEMBER");
    }

    #[test]
    fn signature_tokens_round_trip() {
        let set = TypeSet::parse("SCALAR, STRING").unwrap();
        assert!(set.contains(Type::Number));
        assert!(set.contains(Type::NaN));
        assert!(set.contains(Type::String));
        assert!(!set.contains(Type::Array));
        assert_eq!(set.to_tokens(), "SCALAR,STRING");
        assert_eq!(TypeSet::parse("ANY").unwrap(), TypeSet::ANYTHING);
        assert!(TypeSet::parse("BOGUS").is_err());
    }

    #[test]
    fn expected_list_uses_display_names() {
        let set = Type::Number | Type::Array;
        assert_eq!(set.display_list(), "Number,Not a Number,Array");
    }
}
