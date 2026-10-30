//! Query commands over the mapped derived index (`docs/spec.md` section 7).

pub mod ctx;
pub mod find;
pub mod impact;
pub mod map;
pub mod names;
pub mod output;
pub mod path;
pub mod show;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::app::error::AppError;
use crate::cli::Options;
use crate::domain::facts::{FLAG_GENERATED, FLAG_TEST, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::role::Role;
use crate::infra::store::derived::{Derived, EdgeRecord, FileRecord, SymbolRecord};
use output::{Value, Writer};

/// Restrictions shared by every query command (`-p`, `-l`, `-k`, `--exact`, `--no-tests`).
pub struct Filter {
    paths: Option<Gitignore>,
    langs: Vec<FileType>,
    kinds: Vec<SymKind>,
    pub exact: bool,
    no_tests: bool,
}

impl Filter {
    pub fn new(options: &Options) -> Result<Self, AppError> {
        let paths = if options.paths.is_empty() {
            None
        } else {
            let mut builder = GitignoreBuilder::new("");
            for glob in &options.paths {
                builder.add_line(None, glob).map_err(|error| {
                    AppError::Invalid(format!("invalid --path '{glob}': {error}"))
                })?;
            }
            Some(
                builder
                    .build()
                    .map_err(|error| AppError::Invalid(format!("invalid --path: {error}")))?,
            )
        };
        Ok(Self {
            paths,
            langs: options.langs.clone(),
            kinds: options.kinds.clone(),
            exact: options.exact,
            no_tests: options.no_tests,
        })
    }

    pub fn file(&self, index: &Index, file: &FileRecord) -> bool {
        if self.no_tests && matches!(file.role(), Role::Test | Role::Gen | Role::Fixture) {
            return false;
        }
        if !self.langs.is_empty()
            && !file
                .file_type()
                .is_some_and(|file_type| self.langs.contains(&file_type))
        {
            return false;
        }
        match &self.paths {
            Some(paths) => paths
                .matched_path_or_any_parents(index.string(file.path), false)
                .is_ignore(),
            None => true,
        }
    }

    pub fn symbol(&self, index: &Index, symbol: &SymbolRecord) -> bool {
        if self.no_tests && symbol.flags & (FLAG_TEST | FLAG_GENERATED) != 0 {
            return false;
        }
        if !self.kinds.is_empty() && !self.kinds.contains(&symbol.kind()) {
            return false;
        }
        index
            .derived
            .files()
            .get(symbol.file as usize)
            .is_some_and(|file| self.file(index, file))
    }

    /// Whether a node passes the filter: symbols by symbol rules, files by file rules and only
    /// when no kind filter is set.
    pub fn node(&self, index: &Index, node: u32) -> bool {
        match index.node(node) {
            Node::Symbol(symbol) => self.symbol(index, symbol),
            Node::File(file) => self.kinds.is_empty() && self.file(index, file),
        }
    }

    pub fn edge(&self, edge: &EdgeRecord) -> bool {
        !self.exact || edge.is_exact()
    }
}

pub enum Node<'a> {
    Symbol(&'a SymbolRecord),
    File(&'a FileRecord),
}

/// The opened index.
pub struct Index {
    pub derived: Derived,
}

impl Index {
    pub fn string(&self, id: u32) -> &str {
        self.derived.string(id)
    }

    pub fn symbol_count(&self) -> u32 {
        self.derived.symbols().len() as u32
    }

    pub fn node(&self, node: u32) -> Node<'_> {
        let symbols = self.derived.symbols();
        match symbols.get(node as usize) {
            Some(symbol) => Node::Symbol(symbol),
            None => Node::File(
                &self.derived.files()[(node as usize - symbols.len())
                    .min(self.derived.files().len().saturating_sub(1))],
            ),
        }
    }

    /// Index of the file a node belongs to.
    pub fn file_index_of(&self, node: u32) -> u32 {
        match self.node(node) {
            Node::Symbol(symbol) => symbol.file,
            Node::File(_) => node - self.symbol_count(),
        }
    }

    pub fn file_of(&self, node: u32) -> &FileRecord {
        match self.node(node) {
            Node::Symbol(symbol) => &self.derived.files()[symbol.file as usize],
            Node::File(file) => file,
        }
    }

    pub fn path_of(&self, node: u32) -> &str {
        self.string(self.file_of(node).path)
    }

    /// Role weight of a node's file (section 7.2).
    pub fn role_weight(&self, node: u32) -> f32 {
        let weight = self.file_of(node).role().weight();
        match self.node(node) {
            Node::Symbol(symbol) if symbol.flags & FLAG_TEST != 0 => {
                weight.min(Role::Test.weight())
            }
            _ => weight,
        }
    }

    /// Short display name: qualified name of a symbol, path of a file.
    pub fn label(&self, node: u32) -> &str {
        match self.node(node) {
            Node::Symbol(symbol) => self.string(symbol.qualified),
            Node::File(file) => self.string(file.path),
        }
    }

    pub fn line_of(&self, node: u32) -> u32 {
        match self.node(node) {
            Node::Symbol(symbol) => symbol.line,
            Node::File(_) => 1,
        }
    }

    /// Signature, else name: the one-line form of a symbol.
    pub fn summary(&self, symbol: &SymbolRecord) -> &str {
        let signature = self.string(symbol.signature);
        if signature.is_empty() {
            self.string(symbol.name)
        } else {
            signature
        }
    }

    /// One symbol as a grouped text line and JSON record.
    pub fn write_symbol(
        &self,
        writer: &mut Writer,
        node: u32,
        score: Option<f32>,
        with_doc: bool,
    ) -> bool {
        let Node::Symbol(symbol) = self.node(node) else {
            let path = self.path_of(node);
            let mut record = vec![("t", Value::Str("file")), ("path", Value::Str(path))];
            if let Some(score) = score {
                record.push(("score", Value::Float(score)));
            }
            return writer.file(path, &record);
        };
        let path = self.path_of(node);
        let summary = self.summary(symbol);
        let doc = if with_doc {
            self.string(symbol.doc)
        } else {
            ""
        };
        // The signature carries the kind; symbols without one show kind and name.
        let mut text = if self.string(symbol.signature).is_empty() {
            format!("{} {} {}", symbol.line, symbol.kind().id(), summary)
        } else {
            format!("{} {}", symbol.line, summary)
        };
        if !doc.is_empty() {
            text.push_str(" // ");
            text.push_str(doc);
        }
        let mut record = vec![
            ("t", Value::Str("sym")),
            ("path", Value::Str(path)),
            ("line", Value::Int(u64::from(symbol.line))),
            ("end", Value::Int(u64::from(symbol.end_line))),
            ("kind", Value::Str(symbol.kind().id())),
            ("name", Value::Str(self.string(symbol.qualified))),
            ("sig", Value::Str(self.string(symbol.signature))),
            ("doc", Value::Str(doc)),
        ];
        if let Some(score) = score {
            record.push(("score", Value::Float(score)));
        }
        writer.item(Some(path), &text, &record)
    }
}

/// Opens the derived index or reports a corrupt index as a runtime error.
pub fn open(store: &crate::infra::store::Store) -> Result<Index, AppError> {
    let container = store
        .read_container("derived", crate::infra::store::container::Kind::Derived)
        .map_err(|error| {
            AppError::Invalid(format!(
                "index unreadable ({error}); run 'xcalygraph index'"
            ))
        })?;
    let derived = Derived::open(container)
        .ok_or_else(|| AppError::Invalid("index corrupt; run 'xcalygraph index'".to_owned()))?;
    Ok(Index { derived })
}
