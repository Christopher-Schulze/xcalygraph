//! Query commands over the mapped derived index (`docs/spec.md` section 7).

pub mod audit;
pub mod ctx;
pub mod find;
pub mod impact;
pub mod map;
pub mod names;
pub mod output;
pub mod path;
pub mod show;

#[cfg(test)]
mod benchmark;

use std::borrow::Cow;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::app::error::AppError;
use crate::cli::Options;
use crate::domain::facts::{Coverage, FLAG_GENERATED, FLAG_TEST, SymKind};
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

/// Languages that can reference each other's symbols, for scoping a coverage note.
fn same_language_family(a: FileType, b: FileType) -> bool {
    use FileType::*;
    let family = |file_type| match file_type {
        JavaScript | TypeScript | Vue | Svelte | Astro => 1,
        C | Cpp | ObjC => 2,
        Java | Kotlin | Scala | Groovy => 3,
        _ => 0,
    };
    a == b || (family(a) != 0 && family(a) == family(b))
}

/// One line on what "nothing refers to this" means for a node without incoming edges: proven up
/// to dynamic use, or unproven because analysis or name resolution stayed incomplete.
pub(super) fn absence_note(index: &Index, filter: &Filter, node: u32) -> String {
    let file_type = index.file_of(node).file_type();
    if let Some(note) = coverage_note(index, filter, Coverage::ParseErrors, file_type) {
        return note;
    }
    let Node::Symbol(symbol) = index.node(node) else {
        return "! no static reference in the repository; complete analysis".to_owned();
    };
    let name = index.string(symbol.name);
    if index.derived.is_ambiguous_name(name) {
        return format!(
            "! no resolved reference; ambiguous references named {name} remain, absence unproven"
        );
    }
    if symbol.exported != 0 {
        return "! no static reference in the repository; complete analysis, exported".to_owned();
    }
    "! no static reference in the repository; complete analysis".to_owned()
}

/// One bounded line naming the files, among those passing `filter` and at least `minimum` severe,
/// that were not fully analyzed: most severe first, deterministic. With `language`, only files
/// that could reference symbols of that language count. `None` when no such file exists, so
/// healthy outputs do not grow.
pub(super) fn coverage_note(
    index: &Index,
    filter: &Filter,
    minimum: Coverage,
    language: Option<FileType>,
) -> Option<String> {
    const SHOWN_PATHS: usize = 3;
    let mut total = 0usize;
    let mut incomplete: Vec<(Coverage, &str)> = Vec::new();
    for file in index.derived.files() {
        if !filter.file(index, file)
            || language.is_some_and(|language| {
                !file
                    .file_type()
                    .is_some_and(|file_type| same_language_family(file_type, language))
            })
        {
            continue;
        }
        total += 1;
        if file.coverage() >= minimum.max(Coverage::ParseErrors) {
            incomplete.push((file.coverage(), index.string(file.path)));
        }
    }
    if incomplete.is_empty() {
        return None;
    }
    incomplete.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let mut classes: Vec<(Coverage, usize)> = Vec::new();
    for (coverage, _) in &incomplete {
        match classes.last_mut() {
            Some((class, count)) if class == coverage => *count += 1,
            _ => classes.push((*coverage, 1)),
        }
    }
    let classes = classes
        .iter()
        .map(|(class, count)| format!("{count} {}", class.label()))
        .collect::<Vec<_>>()
        .join(", ");
    let paths = incomplete
        .iter()
        .take(SHOWN_PATHS)
        .map(|(_, path)| *path)
        .collect::<Vec<_>>()
        .join(" ");
    let more = incomplete.len().saturating_sub(SHOWN_PATHS);
    let more = if more > 0 {
        format!(" +{more}")
    } else {
        String::new()
    };
    Some(format!(
        "! coverage: {} of {total} files incomplete ({classes}): {paths}{more}; absence unproven",
        incomplete.len()
    ))
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

    /// File rules for a path absent from the index (a file deleted since REV).
    pub fn removed_file(&self, path: &str, file_type: Option<FileType>, role: Role) -> bool {
        if self.no_tests && matches!(role, Role::Test | Role::Gen | Role::Fixture) {
            return false;
        }
        if !self.langs.is_empty()
            && !file_type.is_some_and(|file_type| self.langs.contains(&file_type))
        {
            return false;
        }
        match &self.paths {
            Some(paths) => paths.matched_path_or_any_parents(path, false).is_ignore(),
            None => true,
        }
    }

    /// Symbol rules for a definition parsed from a revision rather than the current index.
    pub fn removed_symbol(&self, kind: SymKind, flags: u8) -> bool {
        (!self.no_tests || flags & (FLAG_TEST | FLAG_GENERATED) == 0)
            && (self.kinds.is_empty() || self.kinds.contains(&kind))
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

    /// Whether a node may appear in an answer or on a path: symbols by symbol rules, files by
    /// file rules. Unlike `node`, a kind filter does not exclude files, so a file NAME, a
    /// file-level fact or an import hop stays usable under `-k`.
    pub fn includes(&self, index: &Index, node: u32) -> bool {
        match index.node(node) {
            Node::Symbol(symbol) => self.symbol(index, symbol),
            Node::File(file) => self.file(index, file),
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
    pub fn label(&self, node: u32) -> Cow<'_, str> {
        match self.node(node) {
            Node::Symbol(symbol) => self.qualified(symbol),
            Node::File(file) => Cow::Borrowed(self.string(file.path)),
        }
    }

    /// Name for a text line that also prints the symbol's `path:line`: that handle identifies it,
    /// so the module part of the qualified name only repeats the path (`Store.Get`).
    pub fn short_label(&self, node: u32) -> Cow<'_, str> {
        let Node::Symbol(symbol) = self.node(node) else {
            return self.label(node);
        };
        let module = self
            .derived
            .modules()
            .get(self.file_of(node).module as usize)
            .map_or("", |module| self.string(module.name));
        let qualified = self.qualified(symbol);
        match qualified.strip_prefix(module) {
            Some(rest) if !module.is_empty() => {
                let rest = rest.trim_start_matches(['.', ':', '/', '\\']);
                if rest.is_empty() {
                    qualified
                } else {
                    Cow::Owned(rest.to_owned())
                }
            }
            _ => qualified,
        }
    }

    /// Qualified name of a symbol: its stored prefix and its name.
    pub fn qualified(&self, symbol: &SymbolRecord) -> Cow<'_, str> {
        let prefix = self.string(symbol.prefix);
        let name = self.string(symbol.name);
        match (symbol.whole != 0, prefix.is_empty()) {
            (true, _) => Cow::Borrowed(prefix),
            (false, true) => Cow::Borrowed(name),
            (false, false) => Cow::Owned(format!("{prefix}{name}")),
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
        let qualified = self.qualified(symbol);
        let mut record = vec![
            ("t", Value::Str("sym")),
            ("path", Value::Str(path)),
            ("line", Value::Int(u64::from(symbol.line))),
            ("end", Value::Int(u64::from(symbol.end_line))),
            ("kind", Value::Str(symbol.kind().id())),
            ("name", Value::Str(&qualified)),
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
        .map_err(|error| match error {
            crate::infra::store::container::OpenError::Io(error) => {
                AppError::IndexIo("index unreadable".to_owned(), error)
            }
            error => AppError::Invalid(format!(
                "index unreadable ({error}); run 'xcalygraph index'"
            )),
        })?;
    let derived = Derived::open(container)
        .ok_or_else(|| AppError::Invalid("index corrupt; run 'xcalygraph index'".to_owned()))?;
    Ok(Index { derived })
}
