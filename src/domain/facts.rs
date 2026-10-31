//! Per-file facts produced by extraction (`docs/spec.md` section 5).

/// Symbol kinds (section 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum SymKind {
    Mod = 1,
    Struct,
    Class,
    Enum,
    Iface,
    Trait,
    Alias,
    Fn,
    Method,
    Const,
    Var,
    Macro,
    Heading,
    Table,
    View,
    Task,
    Svc,
    Op,
}

pub const SYM_KINDS: [SymKind; 18] = [
    SymKind::Mod,
    SymKind::Struct,
    SymKind::Class,
    SymKind::Enum,
    SymKind::Iface,
    SymKind::Trait,
    SymKind::Alias,
    SymKind::Fn,
    SymKind::Method,
    SymKind::Const,
    SymKind::Var,
    SymKind::Macro,
    SymKind::Heading,
    SymKind::Table,
    SymKind::View,
    SymKind::Task,
    SymKind::Svc,
    SymKind::Op,
];

impl SymKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        SYM_KINDS.into_iter().find(|kind| *kind as u8 == value)
    }

    pub fn from_id(id: &str) -> Option<Self> {
        SYM_KINDS.into_iter().find(|kind| kind.id() == id)
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::Struct => "struct",
            Self::Class => "class",
            Self::Enum => "enum",
            Self::Iface => "iface",
            Self::Trait => "trait",
            Self::Alias => "alias",
            Self::Fn => "fn",
            Self::Method => "method",
            Self::Const => "const",
            Self::Var => "var",
            Self::Macro => "macro",
            Self::Heading => "heading",
            Self::Table => "table",
            Self::View => "view",
            Self::Task => "task",
            Self::Svc => "svc",
            Self::Op => "op",
        }
    }

    /// Types that own methods and take part in member-call resolution.
    pub const fn is_type(self) -> bool {
        matches!(
            self,
            Self::Struct | Self::Class | Self::Enum | Self::Iface | Self::Trait | Self::Alias
        )
    }

    /// Abstract types whose methods dispatch to implementations (resolution step 6).
    pub const fn is_abstract(self) -> bool {
        matches!(self, Self::Iface | Self::Trait)
    }

    pub const fn is_callable(self) -> bool {
        matches!(self, Self::Fn | Self::Method | Self::Macro)
    }
}

pub const FLAG_TEST: u8 = 1;
pub const FLAG_ENTRY: u8 = 2;
pub const FLAG_GENERATED: u8 = 4;
/// A declaration without body (C prototype, C++ in-class method declaration): references prefer
/// the definition.
pub const FLAG_DECLARATION: u8 = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Symbol {
    pub kind: Option<SymKind>,
    pub name: String,
    /// Index of the enclosing symbol in the same file.
    pub parent: Option<u32>,
    /// Type a method belongs to when declared outside the type (Go receiver, Rust `impl`).
    pub owner: String,
    pub line: u32,
    pub end_line: u32,
    /// One line, whitespace-normalized, body removed, max 160 chars.
    pub signature: String,
    /// First sentence of the attached doc comment, max 120 chars.
    pub doc: String,
    /// Distinct search tokens of the body outside nested symbols (identifiers, strings,
    /// comments, markup text), normalized, space-separated, max [`BODY_TERMS`]; search only.
    pub terms: String,
    pub exported: bool,
    pub flags: u8,
}

/// Words kept per symbol body for search.
pub const BODY_TERMS: usize = 64;

impl Symbol {
    pub fn kind(&self) -> SymKind {
        self.kind.unwrap_or(SymKind::Fn)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Import {
    /// Module specifier as written (path, package, namespace).
    pub spec: String,
    pub alias: Option<String>,
    /// Explicitly imported names; empty for whole-module imports.
    pub names: Vec<String>,
    pub line: u32,
    /// `pub use` and similar re-exports.
    pub reexport: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum RefKind {
    Call = 1,
    Type,
    Inherit,
    Implement,
    /// Task or script running a command, script, entrypoint or task (`runs`).
    Run,
    /// SQL object named in a string literal with an SQL keyword (`queries`).
    Query,
    /// Literal relative path to a file of the root (`references`).
    Path,
    /// Backticked identifier in a document (`mentions`).
    Mention,
    /// API operation id matched to its handler (contract `implements`).
    Contract,
    /// Identifier used as a value (callback argument, returned or assigned function, constant).
    Value,
}

pub const REF_KINDS: [RefKind; 10] = [
    RefKind::Call,
    RefKind::Type,
    RefKind::Inherit,
    RefKind::Implement,
    RefKind::Run,
    RefKind::Query,
    RefKind::Path,
    RefKind::Mention,
    RefKind::Contract,
    RefKind::Value,
];

impl RefKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        REF_KINDS.into_iter().find(|kind| *kind as u8 == value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub kind: RefKind,
    /// Identifier, or the member of a member chain.
    pub name: String,
    /// Receiver or qualifier of a member chain (`x` in `x.m()`, `pkg` in `pkg.F`).
    pub qualifier: Option<String>,
    /// Enclosing symbol index in the same file.
    pub scope: Option<u32>,
    pub line: u32,
    /// Argument count of a call (overload selection); [`NO_ARITY`] otherwise.
    pub arity: u8,
}

/// Arity of references that are no calls or whose arguments were not counted.
pub const NO_ARITY: u8 = u8::MAX;

/// Declared or constructed type of a name in a scope, for receiver typing (resolution step 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub scope: Option<u32>,
    pub name: String,
    pub type_name: String,
    pub line: u32,
    /// True for declared types; false for factory or constructor results.
    pub declared: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRead {
    pub name: String,
    pub scope: Option<u32>,
    pub line: u32,
}

/// Field names of a type: card content only, no graph nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields {
    pub owner: u32,
    pub names: Vec<String>,
    /// Declared type per field (empty when unknown), parallel to `names`.
    pub types: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EnvSource {
    EnvExample = 1,
    Compose,
    Dockerfile,
    Ci,
    Kubernetes,
}

impl EnvSource {
    pub fn from_u8(value: u8) -> Option<Self> {
        [
            Self::EnvExample,
            Self::Compose,
            Self::Dockerfile,
            Self::Ci,
            Self::Kubernetes,
        ]
        .into_iter()
        .find(|source| *source as u8 == value)
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::EnvExample => ".env.example",
            Self::Compose => "compose",
            Self::Dockerfile => "dockerfile",
            Self::Ci => "ci",
            Self::Kubernetes => "k8s",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvDef {
    pub name: String,
    pub source: EnvSource,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigKey {
    /// Dotted key path, max depth 6; values are never stored.
    pub path: String,
    pub kind: ValueKind,
    pub line: u32,
}

/// Kind of a config value; the value itself is never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ValueKind {
    Object = 1,
    Array,
    String,
    Number,
    Bool,
    Null,
}

impl ValueKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        [
            Self::Object,
            Self::Array,
            Self::String,
            Self::Number,
            Self::Bool,
            Self::Null,
        ]
        .into_iter()
        .find(|kind| *kind as u8 == value)
    }

    #[cfg(test)]
    pub fn id(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Null => "null",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSegment {
    pub heading: String,
    /// Page, sheet or line where the segment starts.
    pub locator: u32,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetMeta {
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    pub module: Option<String>,
    /// Dependency name and version requirement.
    pub deps: Vec<(String, String)>,
    pub members: Vec<String>,
    pub source_roots: Vec<String>,
    pub include_dirs: Vec<String>,
    pub entrypoints: Vec<String>,
}

/// Everything extracted from one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Declared package, namespace or module name (Go package, Java package, C# namespace).
    pub package: Option<String>,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub refs: Vec<Ref>,
    pub bindings: Vec<Binding>,
    pub envs: Vec<EnvRead>,
    pub fields: Vec<Fields>,
    pub env_defs: Vec<EnvDef>,
    pub config_keys: Vec<ConfigKey>,
    pub texts: Vec<TextSegment>,
    pub asset: Option<AssetMeta>,
    pub manifest: Option<Manifest>,
    /// Number of lines of the file.
    pub lines: u32,
    /// Leading comment of the file (module or package documentation), whitespace-normalized,
    /// max 400 chars.
    pub doc: String,
    /// Distinct words of top-level code outside every symbol, as [`Symbol::terms`].
    pub terms: String,
    /// The file is an executable entrypoint (shebang, `__main__` block).
    pub entry: bool,
}

/// Truncates to at most `max` characters, appending `…` when shortened.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// Collapses whitespace runs to one space and trims.
pub fn normalize_space(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// First sentence of a comment text, max 120 chars.
pub fn first_sentence(text: &str) -> String {
    let text = normalize_space(text);
    let end = text
        .char_indices()
        .find(|&(index, ch)| {
            ch == '.'
                && text[index + 1..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map_or(text.len(), |(index, _)| index + 1);
    clip(&text[..end], 120)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_ids_round_trip() {
        for kind in SYM_KINDS {
            assert_eq!(SymKind::from_id(kind.id()), Some(kind));
            assert_eq!(SymKind::from_u8(kind as u8), Some(kind));
        }
        for kind in REF_KINDS {
            assert_eq!(RefKind::from_u8(kind as u8), Some(kind));
        }
        assert_eq!(EnvSource::from_u8(EnvSource::Ci as u8), Some(EnvSource::Ci));
    }

    #[test]
    fn text_helpers_normalize_clip_and_cut_sentences() {
        assert_eq!(normalize_space("  fn  a(\n  b )  "), "fn a( b )");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
        assert_eq!(first_sentence("Parses input. Then more."), "Parses input.");
        assert_eq!(
            first_sentence("Uses v1.2 format\nfor x"),
            "Uses v1.2 format for x"
        );
    }
}
