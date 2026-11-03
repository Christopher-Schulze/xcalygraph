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
    Option,
}

pub const SYM_KINDS: [SymKind; 19] = [
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
    SymKind::Option,
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
            Self::Option => "option",
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
/// A TypeScript interface whose complete relative module-augmentation evidence was captured.
pub const FLAG_TYPESCRIPT_AUGMENTATION: u8 = 16;
/// A Python method whose body consists only of an optional docstring and `raise NotImplementedError`.
pub const FLAG_PYTHON_ABSTRACT_STUB: u8 = 32;
/// A Rust item directly gated to the Unix target family (`unix`, a Unix `target_os`).
pub const FLAG_RUST_CFG_UNIX: u8 = 64;
/// A Rust item directly gated outside the Unix family (`not(unix)`, `windows`, wasm).
pub const FLAG_RUST_CFG_NOT_UNIX: u8 = 128;

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

/// Internal qualifier tag for Rust `#[path] mod` facts. The payload is two UTF-8 byte-length
/// prefixed fields: the declared logical module suffix and the inline path context.
pub const RUST_MODULE_PATH_REF_PREFIX: &str = "xcalygraph:rust-module-path:";

pub fn rust_module_path_ref_qualifier(module_path: &str, inline_path: &str) -> String {
    let mut qualifier = String::with_capacity(
        RUST_MODULE_PATH_REF_PREFIX.len()
            + module_path.len()
            + inline_path.len()
            + 2 * usize::BITS as usize / 3,
    );
    qualifier.push_str(RUST_MODULE_PATH_REF_PREFIX);
    push_length_prefixed(&mut qualifier, module_path);
    push_length_prefixed(&mut qualifier, inline_path);
    qualifier
}

pub fn rust_module_path_ref_context(qualifier: &str) -> Option<(&str, &str)> {
    let payload = qualifier.strip_prefix(RUST_MODULE_PATH_REF_PREFIX)?;
    let (module_path, payload) = take_length_prefixed(payload)?;
    let (inline_path, payload) = take_length_prefixed(payload)?;
    payload.is_empty().then_some((module_path, inline_path))
}

fn push_length_prefixed(target: &mut String, value: &str) {
    target.push_str(&value.len().to_string());
    target.push(':');
    target.push_str(value);
}

fn take_length_prefixed(source: &str) -> Option<(&str, &str)> {
    let (length, payload) = source.split_once(':')?;
    let length = length.parse::<usize>().ok()?;
    if length > payload.len() || !payload.is_char_boundary(length) {
        return None;
    }
    Some(payload.split_at(length))
}

/// Prefix of a complete TypeScript callable header identity stored on a symbol fact.
#[cfg(feature = "lang-typescript")]
pub const TYPESCRIPT_CALLABLE_IDENTITY_PREFIX: &str = "typescript_callable:v1:";

/// Prefix of a TypeScript union of source-resolved field type names.
#[cfg(feature = "lang-typescript")]
pub const TYPESCRIPT_UNION_FIELD_TYPE_PREFIX: &str = "typescript_union:v1:";

/// Prefix of a TypeScript intersection of source-resolved field type names.
#[cfg(feature = "lang-typescript")]
pub const TYPESCRIPT_INTERSECTION_FIELD_TYPE_PREFIX: &str = "typescript_intersection:v1:";

/// Kind of local fact with a complete source-backed identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum SourceIdentityKind {
    Symbol = 1,
    Reference,
    /// Full TypeScript interface header, not a claim that declarations are merge-compatible.
    InterfaceHeader,
    /// Complete supported TypeScript interface member syntax for compatibility checks.
    InterfaceMembers,
    /// Verified same-file group, identified by its first local symbol index.
    InterfaceGroup,
    /// Source-proven concrete field types for generic bases of an interface group.
    InterfaceGenericFields,
    /// File-level TypeScript parsing and module-augmentation coverage.
    TypeScriptScope,
    /// Relative module specifier of a source-proven TypeScript interface augmentation.
    TypeScriptAugmentation,
    /// Export forms that do not directly mark the underlying declaration.
    TypeScriptComplexExports,
    /// TypeScript source-level module syntax, indexed by file at zero.
    TypeScriptModuleSyntax,
    /// Parsed project scope options from a `tsconfig.json`, indexed by file at zero.
    TypeScriptProject,
    /// File-level completeness of source-proven `declare global` evidence.
    TypeScriptGlobalAugmentationScope,
    /// Interface declaration inside a source-proven `declare global` block.
    TypeScriptGlobalAugmentation,
    /// `package.json` type setting relevant to TypeScript's Node module detection.
    TypeScriptPackageType,
}

impl SourceIdentityKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            value if value == Self::Symbol as u8 => Some(Self::Symbol),
            value if value == Self::Reference as u8 => Some(Self::Reference),
            value if value == Self::InterfaceHeader as u8 => Some(Self::InterfaceHeader),
            value if value == Self::InterfaceMembers as u8 => Some(Self::InterfaceMembers),
            value if value == Self::InterfaceGroup as u8 => Some(Self::InterfaceGroup),
            value if value == Self::InterfaceGenericFields as u8 => {
                Some(Self::InterfaceGenericFields)
            }
            value if value == Self::TypeScriptScope as u8 => Some(Self::TypeScriptScope),
            value if value == Self::TypeScriptAugmentation as u8 => {
                Some(Self::TypeScriptAugmentation)
            }
            value if value == Self::TypeScriptComplexExports as u8 => {
                Some(Self::TypeScriptComplexExports)
            }
            value if value == Self::TypeScriptModuleSyntax as u8 => {
                Some(Self::TypeScriptModuleSyntax)
            }
            value if value == Self::TypeScriptProject as u8 => Some(Self::TypeScriptProject),
            value if value == Self::TypeScriptGlobalAugmentationScope as u8 => {
                Some(Self::TypeScriptGlobalAugmentationScope)
            }
            value if value == Self::TypeScriptGlobalAugmentation as u8 => {
                Some(Self::TypeScriptGlobalAugmentation)
            }
            value if value == Self::TypeScriptPackageType as u8 => {
                Some(Self::TypeScriptPackageType)
            }
            _ => None,
        }
    }
}

/// Source-backed `tsconfig.json` options needed to prove script membership and module scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeScriptProjectConfig {
    pub files: Option<Vec<String>>,
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub out_dir: Option<String>,
    pub module_detection: Option<String>,
    pub module: Option<String>,
    pub jsx: Option<String>,
}

/// Stable identity for a supported TypeScript project configuration.
pub fn typescript_project_identity(config: &TypeScriptProjectConfig) -> String {
    let mut identity = String::from("typescript_project:v1:");
    push_optional_strings(&mut identity, config.files.as_deref());
    push_optional_strings(&mut identity, config.include.as_deref());
    push_optional_strings(&mut identity, config.exclude.as_deref());
    push_optional_string(&mut identity, config.out_dir.as_deref());
    push_optional_string(&mut identity, config.module_detection.as_deref());
    push_optional_string(&mut identity, config.module.as_deref());
    push_optional_string(&mut identity, config.jsx.as_deref());
    identity
}

/// Decodes a project identity, rejecting unsupported or malformed settings.
pub fn typescript_project_config(identity: &str) -> Option<TypeScriptProjectConfig> {
    if identity == "unproven" {
        return None;
    }
    let mut rest = identity.strip_prefix("typescript_project:v1:")?;
    let (files, next) = take_optional_strings(rest)?;
    rest = next;
    let (include, next) = take_optional_strings(rest)?;
    rest = next;
    let (exclude, next) = take_optional_strings(rest)?;
    rest = next;
    let (out_dir, next) = take_optional_string(rest)?;
    rest = next;
    let (module_detection, next) = take_optional_string(rest)?;
    rest = next;
    let (module, next) = take_optional_string(rest)?;
    let (jsx, rest) = take_optional_string(next)?;
    if !rest.is_empty()
        || module_detection
            .as_deref()
            .is_some_and(|value| !matches!(value, "auto" | "legacy" | "force"))
        || module.as_deref().is_some_and(|value| {
            !matches!(
                value,
                "commonjs"
                    | "es6"
                    | "es2015"
                    | "es2020"
                    | "es2022"
                    | "esnext"
                    | "node16"
                    | "node18"
                    | "node20"
                    | "nodenext"
                    | "preserve"
            )
        })
        || jsx.as_deref().is_some_and(|value| {
            !matches!(
                value,
                "preserve" | "react-native" | "react-jsx" | "react-jsxdev" | "react"
            )
        })
        || out_dir
            .as_deref()
            .is_some_and(|value| !valid_typescript_project_path(value))
        || files
            .iter()
            .chain(include.iter())
            .chain(exclude.iter())
            .flatten()
            .any(|value| !valid_typescript_project_path(value))
    {
        return None;
    }
    Some(TypeScriptProjectConfig {
        files,
        include,
        exclude,
        out_dir,
        module_detection,
        module,
        jsx,
    })
}

pub(crate) fn valid_typescript_project_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.as_bytes().get(1).is_some_and(|byte| *byte == b':')
        && !value.contains('\\')
        && !value.chars().any(char::is_control)
}

/// Decodes a source-backed `package.json` type setting used by TypeScript module detection.
pub fn typescript_package_type(identity: &str) -> Option<&str> {
    match identity {
        "typescript_package:v1:module" => Some("module"),
        "typescript_package:v1:commonjs" => Some("commonjs"),
        "typescript_package:v1:absent" => Some("absent"),
        _ => None,
    }
}

fn push_optional_strings(target: &mut String, values: Option<&[String]>) {
    let Some(values) = values else {
        target.push('0');
        return;
    };
    target.push('1');
    push_length_prefixed(target, &values.len().to_string());
    for value in values {
        push_length_prefixed(target, value);
    }
}

fn take_optional_strings(source: &str) -> Option<(Option<Vec<String>>, &str)> {
    match source.as_bytes().first().copied()? {
        b'0' => Some((None, &source[1..])),
        b'1' => {
            let (count, mut rest) = take_length_prefixed(&source[1..])?;
            let count = count.parse::<usize>().ok()?;
            if count > rest.len() / 2 {
                return None;
            }
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                let (value, tail) = take_length_prefixed(rest)?;
                if !valid_typescript_project_path(value) {
                    return None;
                }
                values.push(value.to_owned());
                rest = tail;
            }
            Some((Some(values), rest))
        }
        _ => None,
    }
}

fn push_optional_string(target: &mut String, value: Option<&str>) {
    match value {
        Some(value) => {
            target.push('1');
            push_length_prefixed(target, value);
        }
        None => target.push('0'),
    }
}

fn take_optional_string(source: &str) -> Option<(Option<String>, &str)> {
    match source.as_bytes().first().copied()? {
        b'0' => Some((None, &source[1..])),
        b'1' => {
            let (value, rest) = take_length_prefixed(&source[1..])?;
            if value.is_empty() || value.chars().any(char::is_control) {
                return None;
            }
            Some((Some(value.to_owned()), rest))
        }
        _ => None,
    }
}

/// Encode `(base symbol, field name, concrete type)` triples in sorted, unambiguous form.
#[cfg(any(feature = "lang-typescript", test))]
pub fn interface_generic_fields_identity(fields: &[(u32, &str, &str)]) -> String {
    let mut identity = String::from("generic_fields:");
    for &(base, name, concrete) in fields {
        push_length_prefixed(&mut identity, &base.to_string());
        push_length_prefixed(&mut identity, name);
        push_length_prefixed(&mut identity, concrete);
    }
    identity
}

/// Decode generic-field evidence, rejecting malformed, duplicate or unsorted entries.
pub fn interface_generic_fields(identity: &str) -> Option<Vec<(u32, &str, &str)>> {
    let mut rest = identity.strip_prefix("generic_fields:")?;
    let mut fields = Vec::new();
    let mut previous = None;
    while !rest.is_empty() {
        let (base, tail) = take_length_prefixed(rest)?;
        let base = base.parse::<u32>().ok()?;
        let (name, tail) = take_length_prefixed(tail)?;
        let (concrete, tail) = take_length_prefixed(tail)?;
        if name.is_empty() || concrete.is_empty() || previous.is_some_and(|key| key >= (base, name))
        {
            return None;
        }
        fields.push((base, name, concrete));
        previous = Some((base, name));
        rest = tail;
    }
    (!fields.is_empty()).then_some(fields)
}

/// Sparse exact source identity, indexed into the local symbol or reference vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    pub kind: SourceIdentityKind,
    pub index: u32,
    pub identity: String,
}

/// Declared or constructed type of a name in a scope, for receiver typing (resolution step 5).
/// The type is empty when Rust retains only the local name for lexical call shadowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub scope: Option<u32>,
    pub name: String,
    pub type_name: String,
    pub line: u32,
    /// Last line in the lexical block, when the extractor records one.
    pub scope_end_line: Option<u32>,
    /// True for declared types; false for factory or constructor results.
    pub declared: bool,
}

/// Simple Julia variable argument of a call, indexed into [`Facts::refs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JuliaCallArgument {
    pub reference: u32,
    pub position: u8,
    pub binding: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvRead {
    pub name: String,
    pub scope: Option<u32>,
    pub line: u32,
}

/// A field key, including JavaScript strings that cannot be represented as UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldName {
    Text(String),
    Utf16(Vec<u16>),
}

impl FieldName {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Utf16(_) => None,
        }
    }

    /// Unambiguous token in a comma-separated field list, before transport escaping.
    pub fn token(&self) -> String {
        use std::fmt::Write as _;

        if let Self::Text(value) = self
            && !value.is_empty()
            && !value
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, ',' | '"' | '\\'))
        {
            return value.clone();
        }
        let units: Vec<u16> = match self {
            Self::Text(value) => value.encode_utf16().collect(),
            Self::Utf16(value) => value.clone(),
        };
        let mut result = String::from("\"");
        for item in char::decode_utf16(units) {
            match item {
                Ok('"') => result.push_str("\\\""),
                Ok('\\') => result.push_str("\\\\"),
                Ok(ch) if (ch as u32) < 0x20 => {
                    let _ = write!(result, "\\u{:04x}", ch as u32);
                }
                Ok(ch) => result.push(ch),
                Err(error) => {
                    let _ = write!(result, "\\u{:04x}", error.unpaired_surrogate());
                }
            }
        }
        result.push('"');
        result
    }
}

/// Field names of a type: card content only, no graph nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fields {
    pub owner: u32,
    pub names: Vec<FieldName>,
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
    /// Cargo crate roots with library identity retained for module resolution.
    pub rust_target_roots: Vec<RustTargetRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustTargetRoot {
    /// Path relative to `Cargo.toml`.
    pub path: String,
    pub is_library: bool,
}

/// One source-backed Go embed pattern, before filesystem resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoEmbed {
    pub symbol: u32,
    pub line: u32,
    pub pattern: Vec<u8>,
}

/// Everything extracted from one file.
/// How completely a file was analyzed; ordered by how much of the file's facts may be missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Coverage {
    #[default]
    Complete = 0,
    /// The parser recovered from syntax errors; facts outside the error regions stand.
    ParseErrors = 1,
    /// The query match limit cut extraction short; later matches are missing.
    MatchLimit = 2,
    /// A deep language without an available grammar or query in this build.
    NoGrammar = 3,
    /// The parser failed or panicked; the file contributes no facts.
    Failed = 4,
    /// The source could not be read; the file contributes no facts.
    Unreadable = 5,
}

impl Coverage {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Complete),
            1 => Some(Self::ParseErrors),
            2 => Some(Self::MatchLimit),
            3 => Some(Self::NoGrammar),
            4 => Some(Self::Failed),
            5 => Some(Self::Unreadable),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::ParseErrors => "parse errors",
            Self::MatchLimit => "match limit",
            Self::NoGrammar => "no grammar",
            Self::Failed => "failed",
            Self::Unreadable => "unreadable",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// Declared package, namespace or module name (Go package, Java package, C# namespace).
    pub package: Option<String>,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
    pub refs: Vec<Ref>,
    pub go_embeds: Vec<GoEmbed>,
    /// Unqualified builtins retained so cross-file Julia Base extensions can be dispatched.
    pub julia_builtin_calls: Vec<u32>,
    /// Sparse simple-variable arguments for Julia calls, sorted by reference and position.
    pub julia_call_arguments: Vec<JuliaCallArgument>,
    /// Exact language-level callable identities, stored only for labeled declarations/calls.
    pub source_identities: Option<Box<[SourceIdentity]>>,
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
    pub coverage: Coverage,
}

impl Facts {
    pub fn is_julia_builtin_call(&self, reference: u32) -> bool {
        self.julia_builtin_calls.binary_search(&reference).is_ok()
    }

    pub fn call_arguments_for(&self, reference: u32) -> &[JuliaCallArgument] {
        let start = self
            .julia_call_arguments
            .partition_point(|argument| argument.reference < reference);
        let end = start
            + self.julia_call_arguments[start..]
                .partition_point(|argument| argument.reference == reference);
        &self.julia_call_arguments[start..end]
    }

    pub fn source_identity(&self, kind: SourceIdentityKind, index: u32) -> Option<&str> {
        let identities = self.source_identities.as_deref()?;
        let position = identities
            .binary_search_by_key(&(kind, index), |item| (item.kind, item.index))
            .ok()?;
        Some(identities[position].identity.as_str())
    }
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
    fn field_tokens_distinguish_separators_empty_and_surrogates() {
        assert_eq!(FieldName::Text("a".to_owned()).token(), "a");
        assert_eq!(FieldName::Text("a,b".to_owned()).token(), "\"a,b\"");
        assert_eq!(FieldName::Text(String::new()).token(), "\"\"");
        assert_eq!(FieldName::Utf16(vec![0xd800]).token(), "\"\\ud800\"");
        assert_eq!(
            FieldName::Text("\\ud800".to_owned()).token(),
            "\"\\\\ud800\""
        );
        assert_eq!(FieldName::Text("a\nb".to_owned()).token(), "\"a\\u000ab\"");
    }

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

    #[test]
    fn rust_module_path_qualifiers_round_trip_paths_without_delimiter_assumptions() {
        let qualifier = rust_module_path_ref_qualifier("inventory::r#type", "thread_files/naïve");
        assert_eq!(
            rust_module_path_ref_context(&qualifier),
            Some(("inventory::r#type", "thread_files/naïve"))
        );
        assert_eq!(rust_module_path_ref_context("receiver"), None);
        assert_eq!(
            rust_module_path_ref_context("xcalygraph:rust-module-path:99:x"),
            None
        );
    }
}
