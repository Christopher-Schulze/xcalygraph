//! Resolved knowledge graph: files, symbols, edges and derived data (`docs/spec.md` section 6).

use super::facts::{FieldName, SymKind};
use super::filetype::FileType;
use super::role::Role;

/// Node ids: symbols are `0..symbols.len()`, files follow.
pub type NodeId = u32;
pub const NONE: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum EdgeKind {
    Calls = 1,
    Uses,
    Inherits,
    Implements,
    Imports,
    Mentions,
    Tests,
    References,
    Runs,
    Queries,
}

pub const EDGE_KINDS: [EdgeKind; 10] = [
    EdgeKind::Calls,
    EdgeKind::Uses,
    EdgeKind::Inherits,
    EdgeKind::Implements,
    EdgeKind::Imports,
    EdgeKind::Mentions,
    EdgeKind::Tests,
    EdgeKind::References,
    EdgeKind::Runs,
    EdgeKind::Queries,
];

impl EdgeKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        EDGE_KINDS.into_iter().find(|kind| *kind as u8 == value)
    }

    pub const fn id(self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Uses => "uses",
            Self::Inherits => "inherits",
            Self::Implements => "implements",
            Self::Imports => "imports",
            Self::Mentions => "mentions",
            Self::Tests => "tests",
            Self::References => "references",
            Self::Runs => "runs",
            Self::Queries => "queries",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Confidence {
    Exact = 1,
    Inferred = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge {
    pub source: NodeId,
    pub target: NodeId,
    pub kind: EdgeKind,
    pub confidence: Confidence,
    /// Line of the reference in the source's file.
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphFile {
    pub path: String,
    pub role: Role,
    pub file_type: Option<FileType>,
    pub module: u32,
    pub lines: u32,
    /// Symbols of this file: `symbols[first..first + count]`.
    pub first_symbol: u32,
    pub symbol_count: u32,
    /// Unresolved references counted as ambiguous (resolution step 8).
    pub ambiguous: u32,
    pub entry: bool,
    /// Leading file comment (module documentation).
    pub doc: String,
    /// Config key paths and env definition names, for the search index only.
    pub keys: String,
    /// Words of top-level code outside symbols, for the search index only.
    pub terms: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphSymbol {
    pub file: u32,
    pub kind: SymKind,
    pub name: String,
    /// Qualified name: module path, enclosing types, name.
    pub qualified: String,
    pub parent: u32,
    pub line: u32,
    pub end_line: u32,
    pub signature: String,
    pub doc: String,
    /// Body words for search ([`crate::domain::facts::Symbol::terms`]).
    pub terms: String,
    pub exported: bool,
    pub flags: u8,
    pub rank: f32,
}

/// Whether a qualified name (`::` normalized to `.`) ends with the dotted `suffix` at a name
/// boundary: `internal/store.Store.Get` ends with `Store.Get` and `store.Store.Get`.
pub fn has_qualified_suffix(qualified: &str, suffix: &str) -> bool {
    qualified
        .strip_suffix(suffix)
        .is_some_and(|head| head.is_empty() || head.ends_with(['.', '/']))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// Display name: package directory, crate module path or file.
    pub name: String,
    pub files: u32,
    pub symbols: u32,
    /// Longest-path level on the condensed module DAG.
    pub layer: u32,
    /// Strongly connected component id; modules sharing one form a cycle.
    pub cycle: u32,
}

/// Aggregated module dependency (module graph).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleEdge {
    pub source: u32,
    pub target: u32,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct External {
    pub module: u32,
    pub name: String,
    pub uses: u32,
}

/// Environment variable read or definition site.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EnvSite {
    pub name: String,
    pub node: NodeId,
    pub line: u32,
    /// 0 for reads; the `EnvSource` discriminant for definitions.
    pub source: u8,
}

/// Symbol field names for cards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldList {
    pub symbol: u32,
    pub names: Vec<FieldName>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Graph {
    pub files: Vec<GraphFile>,
    pub symbols: Vec<GraphSymbol>,
    pub edges: Vec<Edge>,
    pub modules: Vec<Module>,
    pub module_edges: Vec<ModuleEdge>,
    pub externals: Vec<External>,
    pub envs: Vec<EnvSite>,
    pub fields: Vec<FieldList>,
    /// (type, member) pairs: methods and associated items of each type, sorted.
    pub members: Vec<(u32, u32)>,
    /// Doc references to missing files or members: (referring node, line, reference text).
    pub stale: Vec<(NodeId, u32, String)>,
    /// Names of references that stayed ambiguous or unresolved although symbols carry them,
    /// sorted: such symbols are never reported dead.
    pub ambiguous_names: Vec<String>,
}

impl Graph {
    pub fn node_count(&self) -> usize {
        self.symbols.len() + self.files.len()
    }
}
