//! Global resolution: facts of all files to the symbol graph (`docs/spec.md` section 6,
//! resolution order steps 1 to 8). Pure; recomputed from cached facts on every change (D6).

pub mod derive;
mod js;
pub mod modules;
mod python;

use std::collections::{BTreeMap, HashSet};

use crate::domain::hash::FastMap as HashMap;

use rayon::prelude::*;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::{self, Binding, Facts, Ref, RefKind, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::{
    Confidence, Edge, EdgeKind, EnvSite, External, FieldList, Graph, GraphFile, GraphSymbol, NONE,
};
use crate::domain::role::Role;
use modules::ModuleMap;

/// Bumped whenever resolution or derived data change; a mismatch re-derives the index.
pub const RESOLVE_VERSION: u32 = 7;

/// Maximum implementations a dispatched call fans out to (resolution step 6).
const MAX_DISPATCH: usize = 10;

pub struct Input<'a> {
    pub files: &'a [FileRecord],
    /// Facts parallel to `files`; `None` for files indexed by path only.
    pub facts: &'a [Option<Facts>],
}

/// Indexes over all symbols, shared by every file's resolution.
struct Index<'a> {
    input: &'a Input<'a>,
    modules: ModuleMap,
    /// First global symbol id of each file.
    base: Vec<u32>,
    symbols: Vec<GraphSymbol>,
    /// Top-level, non-method symbols per (module, name).
    by_module: HashMap<(u32, &'a str), Vec<u32>>,
    /// Methods per (method scope, owner type name, method name).
    methods: HashMap<(u32, &'a str, &'a str), Vec<u32>>,
    /// Method scope id per module (modules sharing a crate share one).
    scope_ids: Vec<u32>,
    /// Declared Go package name per module.
    package_names: Vec<Option<String>>,
    /// Required module paths of all `go.mod` files, longest first.
    go_requires: Vec<String>,
    implementation_pairs: Vec<(u32, u32, Confidence, u32)>,
    /// Import environment per file, for resolving names written in other files.
    file_imports: Vec<Imports>,
    js_tables: js::JsTables,
    /// Root-relative path -> file index.
    path_index: HashMap<&'a str, u32>,
    /// Types per (module, name).
    types: HashMap<(u32, &'a str), Vec<u32>>,
    /// Every symbol per name, for the unique-name fallback.
    by_name: HashMap<&'a str, Vec<u32>>,
    /// Declared result type of callables (bindings named "").
    returns: HashMap<u32, &'a str>,
    /// Field types per (type symbol, field name).
    field_types: HashMap<(u32, &'a str), &'a str>,
    /// Implementations of abstract types and supertypes of concrete ones.
    implementations: HashMap<u32, Vec<u32>>,
    supertypes: HashMap<u32, Vec<u32>>,
}

/// Where an import or path leads.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    Module(u32),
    Symbols(Vec<u32>),
    External(String),
}

/// Per-file import environment.
#[derive(Default)]
struct Imports {
    /// Local name -> target (Go package aliases, Rust `use` leaves).
    names: HashMap<String, Target>,
    /// Modules whose names are all visible (Rust globs).
    globs: Vec<u32>,
    /// Modules the file imports (fallback scope of step 7).
    modules: Vec<u32>,
    externals: Vec<String>,
    /// Files the file imports (file -> file `imports` edges).
    files: Vec<(u32, u32)>,
}

struct Resolved {
    edges: Vec<Edge>,
    /// Modules this file imports (module import graph).
    imported_modules: Vec<u32>,
    ambiguous: u32,
    externals: Vec<String>,
}

pub fn build(input: &Input) -> Graph {
    let mut index = Index::new(input);
    let imports: Vec<Imports> = (0..input.files.len())
        .into_par_iter()
        .map(|file| index.imports(file as u32))
        .collect();
    index.file_imports = imports;
    index.link_implementations();
    let resolved: Vec<Resolved> = (0..input.files.len())
        .into_par_iter()
        .map(|file| index.resolve_file(file as u32, &index.file_imports[file]))
        .collect();
    index.into_graph(resolved)
}

impl<'a> Index<'a> {
    fn new(input: &'a Input<'a>) -> Self {
        let modules = modules::assign(input.files, input.facts);
        let mut base = Vec::with_capacity(input.files.len());
        let mut symbols = Vec::new();
        for (file, facts) in input.facts.iter().enumerate() {
            base.push(symbols.len() as u32);
            let Some(facts) = facts else { continue };
            let module = &modules.modules[modules.file_module[file] as usize];
            let start = symbols.len() as u32;
            for symbol in &facts.symbols {
                let parent = symbol.parent.map_or(NONE, |parent| start + parent);
                let mut path = Vec::new();
                let mut current = symbol.parent;
                while let Some(index) = current {
                    let enclosing = &facts.symbols[index as usize];
                    path.push(enclosing.name.as_str());
                    current = enclosing.parent;
                }
                if path.is_empty() && !symbol.owner.is_empty() {
                    path.push(symbol.owner.as_str());
                }
                path.reverse();
                let mut qualified = module.name.clone();
                for part in path.iter().chain(std::iter::once(&symbol.name.as_str())) {
                    if !qualified.is_empty() && qualified != "." {
                        qualified.push_str(module.separator);
                    } else {
                        qualified.clear();
                    }
                    qualified.push_str(part);
                }
                let role = input.files[file].role;
                let mut flags = symbol.flags;
                if role == Role::Test {
                    flags |= facts::FLAG_TEST;
                }
                if role == Role::Gen {
                    flags |= facts::FLAG_GENERATED;
                }
                symbols.push(GraphSymbol {
                    file: file as u32,
                    kind: symbol.kind(),
                    name: symbol.name.clone(),
                    qualified,
                    parent,
                    line: symbol.line,
                    end_line: symbol.end_line,
                    signature: symbol.signature.clone(),
                    doc: symbol.doc.clone(),
                    exported: symbol.exported,
                    flags,
                    rank: 0.0,
                });
            }
        }
        let mut scope_names: HashMap<&str, u32> = HashMap::default();
        let scope_ids: Vec<u32> = modules
            .modules
            .iter()
            .map(|module| {
                let next = scope_names.len() as u32;
                *scope_names
                    .entry(module.method_scope.as_str())
                    .or_insert(next)
            })
            .collect();
        let mut package_names: Vec<Option<String>> = vec![None; modules.modules.len()];
        for (file, facts) in input.facts.iter().enumerate() {
            let module = modules.file_module[file] as usize;
            if let Some(package) = facts.as_ref().and_then(|facts| facts.package.as_ref())
                && (package_names[module].is_none()
                    || package_names[module]
                        .as_ref()
                        .is_some_and(|p| p.ends_with("_test")))
            {
                package_names[module] = Some(package.clone());
            }
        }
        let mut go_requires: Vec<String> = input
            .facts
            .iter()
            .filter_map(|facts| facts.as_ref()?.manifest.as_ref())
            .flat_map(|manifest| manifest.deps.iter().map(|(name, _)| name.clone()))
            .collect();
        go_requires.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        go_requires.dedup();
        let mut index = Self {
            input,
            modules,
            base,
            symbols,
            scope_ids,
            package_names,
            go_requires,
            implementation_pairs: Vec::new(),
            file_imports: Vec::new(),
            js_tables: js::tables(input),
            path_index: input
                .files
                .iter()
                .enumerate()
                .map(|(file, record)| (record.path.as_str(), file as u32))
                .collect(),
            by_module: HashMap::default(),
            methods: HashMap::default(),
            types: HashMap::default(),
            by_name: HashMap::default(),
            returns: HashMap::default(),
            field_types: HashMap::default(),
            implementations: HashMap::default(),
            supertypes: HashMap::default(),
        };
        index.build_tables();
        index
    }

    fn build_tables(&mut self) {
        let input = self.input;
        for (file, facts) in input.facts.iter().enumerate() {
            let Some(facts) = facts else { continue };
            let module = self.modules.file_module[file];
            let scope = self.scope_ids[module as usize];
            let base = self.base[file];
            for (local, symbol) in facts.symbols.iter().enumerate() {
                let id = base + local as u32;
                let kind = symbol.kind();
                let name = symbol.name.as_str();
                self.by_name.entry(name).or_default().push(id);
                let owner = if !symbol.owner.is_empty() {
                    Some(symbol.owner.as_str())
                } else {
                    symbol
                        .parent
                        .map(|parent| &facts.symbols[parent as usize])
                        .filter(|parent| parent.kind().is_type())
                        .map(|parent| parent.name.as_str())
                };
                match owner {
                    Some(owner)
                        if kind == SymKind::Method
                            || kind.is_callable()
                            || kind == SymKind::Const =>
                    {
                        self.methods
                            .entry((scope, owner_base(owner), name))
                            .or_default()
                            .push(id);
                    }
                    _ => {
                        if symbol.parent.is_none() {
                            self.by_module.entry((module, name)).or_default().push(id);
                        }
                        if kind.is_type() {
                            self.types.entry((module, name)).or_default().push(id);
                        }
                    }
                }
            }
            for binding in &facts.bindings {
                if binding.name.is_empty()
                    && let Some(scope) = binding.scope
                {
                    self.returns
                        .insert(base + scope, binding.type_name.as_str());
                }
            }
            for fields in &facts.fields {
                for (name, kind) in fields.names.iter().zip(&fields.types) {
                    if !kind.is_empty() {
                        self.field_types
                            .insert((base + fields.owner, name.as_str()), kind.as_str());
                    }
                }
            }
        }
    }

    fn facts(&self, file: u32) -> Option<&'a Facts> {
        self.input.facts[file as usize].as_ref()
    }

    fn file_type(&self, file: u32) -> Option<FileType> {
        self.input.files[file as usize].file_type
    }

    fn module_of(&self, file: u32) -> u32 {
        self.modules.file_module[file as usize]
    }

    fn is_test(&self, symbol: u32) -> bool {
        self.symbols[symbol as usize].flags & facts::FLAG_TEST != 0
    }

    // ---------------------------------------------------------------- imports

    fn imports(&self, file: u32) -> Imports {
        let mut imports = Imports::default();
        let Some(facts) = self.facts(file) else {
            return imports;
        };
        match self.file_type(file) {
            Some(FileType::Go) => self.go_imports(file, facts, &mut imports),
            Some(FileType::Rust) => self.rust_imports(file, facts, &mut imports),
            Some(FileType::Python) => self.python_imports(file, facts, &mut imports),
            Some(FileType::Bash) => self.path_imports(file, facts, &mut imports),
            Some(
                FileType::TypeScript
                | FileType::JavaScript
                | FileType::Vue
                | FileType::Svelte
                | FileType::Astro,
            ) => self.js_imports(file, facts, &mut imports),
            _ => {}
        }
        imports
    }

    fn go_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let target = self.modules.get(&import.spec);
            let local = import.alias.clone().unwrap_or_else(|| match target {
                Some(module) => self.go_package_name(module),
                None => import
                    .spec
                    .rsplit('/')
                    .next()
                    .unwrap_or(&import.spec)
                    .to_owned(),
            });
            if local == "_" {
                continue;
            }
            match target {
                Some(module) => {
                    if local == "." {
                        imports.globs.push(module);
                    } else {
                        imports.names.insert(local, Target::Module(module));
                    }
                    imports.modules.push(module);
                }
                None => {
                    let external = self.go_external(&import.spec);
                    imports
                        .names
                        .insert(local, Target::External(external.clone()));
                    if let Some(external) = external.strip_prefix("ext:") {
                        imports.externals.push(external.to_owned());
                    }
                }
            }
        }
        let _ = file;
    }

    /// Literal relative imports (Bash `source`): relative to the file, then to the root. The
    /// sourced file's names become visible like a glob import.
    fn path_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let dir = modules::parent_dir(&self.input.files[file as usize].path);
        for import in &facts.imports {
            let spec = import.spec.trim_start_matches("./");
            let candidates = [
                if dir.is_empty() {
                    spec.to_owned()
                } else {
                    format!("{dir}/{spec}")
                },
                spec.to_owned(),
            ];
            let Some(target) = candidates
                .iter()
                .find_map(|path| self.modules.get(&format!("file:{path}")))
            else {
                continue;
            };
            imports.globs.push(target);
            imports.modules.push(target);
            for (other, &module) in self.modules.file_module.iter().enumerate() {
                if module == target {
                    imports.files.push((file, other as u32));
                }
            }
        }
    }

    /// Package name of a Go module: the declared `package` of its non-test files.
    fn go_package_name(&self, module: u32) -> String {
        self.package_names[module as usize]
            .clone()
            .unwrap_or_else(|| {
                let key = &self.modules.modules[module as usize].key;
                key.rsplit('/').next().unwrap_or(key).to_owned()
            })
    }

    /// External dependency name of an unresolved Go import: the required module it belongs to,
    /// or `std:` for the standard library (no dot in the first path element).
    fn go_external(&self, spec: &str) -> String {
        let first = spec.split('/').next().unwrap_or(spec);
        if !first.contains('.') {
            return format!("std:{spec}");
        }
        let required = self
            .go_requires
            .iter()
            .find(|name| spec == name.as_str() || spec.starts_with(&format!("{name}/")));
        format!("ext:{}", required.map_or(spec, String::as_str))
    }

    fn rust_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let segments: Vec<&str> = import.spec.split("::").filter(|s| !s.is_empty()).collect();
            let Some(name) = import.names.first() else {
                continue;
            };
            let base = if segments.is_empty() {
                self.rust_root_segment(file, name)
                    .map(|target| (target, true))
            } else {
                self.rust_path(file, &segments)
                    .map(|target| (target, false))
            };
            let Some((base, is_leaf)) = base else {
                continue;
            };
            let target = if is_leaf {
                Some(base)
            } else if name == "*" {
                if let Target::Module(module) = base {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None
            } else {
                self.rust_member(file, &base, name)
            };
            let Some(target) = target else { continue };
            let local = import.alias.clone().unwrap_or_else(|| name.clone());
            match &target {
                Target::Module(module) => imports.modules.push(*module),
                Target::External(external) => imports.externals.push(external.clone()),
                Target::Symbols(symbols) => {
                    if let Some(&symbol) = symbols.first() {
                        imports
                            .modules
                            .push(self.module_of(self.symbols[symbol as usize].file));
                    }
                }
            }
            imports.names.insert(local, target);
        }
        // `mod child;` declarations make child modules visible by name.
        let module = self.module_of(file);
        let key = self.modules.modules[module as usize].key.clone();
        for symbol in &facts.symbols {
            if symbol.kind() == SymKind::Mod
                && symbol.parent.is_none()
                && symbol.end_line == symbol.line
                && let Some(child) = self.rust_child(&key, &symbol.name)
            {
                imports
                    .names
                    .entry(symbol.name.clone())
                    .or_insert(Target::Module(child));
                for (other, &other_module) in self.modules.file_module.iter().enumerate() {
                    if other_module == child {
                        imports.files.push((file, other as u32));
                    }
                }
            }
        }
    }

    /// A single-segment `use name;`: a workspace crate or an external crate.
    fn rust_root_segment(&self, file: u32, name: &str) -> Option<Target> {
        let _ = file;
        if let Some(module) = self.rust_crate_root(name) {
            return Some(Target::Module(module));
        }
        Some(Target::External(name.to_owned()))
    }

    fn rust_crate_root(&self, crate_name: &str) -> Option<u32> {
        self.modules.get(&format!("{crate_name}::"))
    }

    fn rust_child(&self, key: &str, name: &str) -> Option<u32> {
        let (crate_name, path) = key.split_once("::")?;
        let child = if path.is_empty() || path.contains('/') || path.starts_with("tests::") {
            format!("{crate_name}::{name}")
        } else {
            format!("{crate_name}::{path}::{name}")
        };
        self.modules.get(&child)
    }

    /// Resolves a Rust path prefix (`crate::a::b`, `super::x`, `other_crate::y`).
    fn rust_path(&self, file: u32, segments: &[&str]) -> Option<Target> {
        let module = self.module_of(file);
        let key = self.modules.modules[module as usize].key.as_str();
        let (crate_name, current) = key.split_once("::").unwrap_or((key, ""));
        let current_path: Vec<&str> = if current.contains('/') || current.starts_with("tests::") {
            Vec::new()
        } else {
            current.split("::").filter(|s| !s.is_empty()).collect()
        };
        let mut path: Vec<&str>;
        let mut rest = segments;
        let mut crate_name = crate_name.to_owned();
        match segments.first().copied()? {
            "crate" => {
                path = Vec::new();
                rest = &segments[1..];
            }
            "self" => {
                path = current_path.clone();
                rest = &segments[1..];
            }
            "super" => {
                path = current_path.clone();
                while rest.first() == Some(&"super") {
                    path.pop();
                    rest = &rest[1..];
                }
            }
            first => {
                if self.rust_crate_root(first).is_some() && first != crate_name {
                    crate_name = first.to_owned();
                    path = Vec::new();
                    rest = &segments[1..];
                } else {
                    let mut child = current_path.clone();
                    child.push(first);
                    if self
                        .modules
                        .get(&format!("{crate_name}::{}", child.join("::")))
                        .is_some()
                    {
                        path = current_path.clone();
                    } else if self
                        .modules
                        .get(&format!("{crate_name}::{first}"))
                        .is_some()
                    {
                        path = Vec::new();
                    } else if let Some((types, _)) = self.resolve_type(file, None, None, first) {
                        let mut target = Target::Symbols(types);
                        for segment in &segments[1..] {
                            target = self.rust_member(file, &target, segment)?;
                        }
                        return Some(target);
                    } else {
                        return Some(Target::External(first.to_owned()));
                    }
                }
            }
        }
        let mut target = Target::Module(
            self.modules
                .get(&format!("{crate_name}::{}", path.join("::")))?,
        );
        for segment in rest {
            let mut next_path = path.clone();
            next_path.push(segment);
            if let Some(module) = self
                .modules
                .get(&format!("{crate_name}::{}", next_path.join("::")))
            {
                path = next_path;
                target = Target::Module(module);
            } else {
                target = self.rust_member(file, &target, segment)?;
            }
        }
        Some(target)
    }

    /// Member `name` of a target: a symbol of a module, or an associated item of a type.
    fn rust_member(&self, file: u32, base: &Target, name: &str) -> Option<Target> {
        match base {
            Target::Module(module) => {
                // A child module wins over its `mod name;` declaration symbol.
                let key = &self.modules.modules[*module as usize].key;
                if let Some(child) = self.rust_child(key, name) {
                    return Some(Target::Module(child));
                }
                let found: Vec<u32> = self
                    .module_symbol(*module, name, None)
                    .into_iter()
                    .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Mod)
                    .collect();
                (!found.is_empty()).then_some(Target::Symbols(found))
            }
            Target::Symbols(symbols) => {
                let found: Vec<u32> = symbols
                    .iter()
                    .flat_map(|&symbol| self.members(symbol, name))
                    .collect();
                (!found.is_empty()).then_some(Target::Symbols(found))
            }
            Target::External(external) => {
                let _ = file;
                Some(Target::External(external.clone()))
            }
        }
    }

    // ---------------------------------------------------------------- lookups

    /// Top-level symbols named `name` in a module, optionally restricted by kind.
    fn module_symbol(
        &self,
        module: u32,
        name: &str,
        want: Option<fn(SymKind) -> bool>,
    ) -> Vec<u32> {
        let mut found: Vec<u32> = self
            .by_module
            .get(&(module, name))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&symbol| want.is_none_or(|want| want(self.symbols[symbol as usize].kind)))
            .collect();
        if found.len() > 1 {
            let non_test: Vec<u32> = found
                .iter()
                .copied()
                .filter(|&s| !self.is_test(s))
                .collect();
            if !non_test.is_empty() {
                found = non_test;
            }
        }
        found
    }

    /// Methods and associated items `name` of a type, including those of its supertypes.
    fn members(&self, type_symbol: u32, name: &str) -> Vec<u32> {
        let symbol = &self.symbols[type_symbol as usize];
        let scope = self.scope_ids[self.module_of(symbol.file) as usize];
        let mut found: Vec<u32> = self
            .methods
            .get(&(scope, symbol.name.as_str(), name))
            .into_iter()
            .flatten()
            .copied()
            .collect();
        if found.is_empty() {
            for &supertype in self.supertypes.get(&type_symbol).into_iter().flatten() {
                if supertype != type_symbol {
                    let inherited = self.members_direct(supertype, name);
                    found.extend(inherited);
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    fn members_direct(&self, type_symbol: u32, name: &str) -> Vec<u32> {
        let symbol = &self.symbols[type_symbol as usize];
        let scope = self.scope_ids[self.module_of(symbol.file) as usize];
        self.methods
            .get(&(scope, symbol.name.as_str(), name))
            .cloned()
            .unwrap_or_default()
    }

    /// Resolves a type name as written in `file` (`T`, `pkg.T`, `a::b::T`, `Self`).
    fn resolve_type(
        &self,
        file: u32,
        imports: Option<&Imports>,
        scope: Option<u32>,
        name: &str,
    ) -> Option<(Vec<u32>, Confidence)> {
        let imports = imports.or_else(|| self.file_imports.get(file as usize));
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        if name == "Self" || name == "self" {
            return self
                .owner_type(file, scope)
                .map(|types| (types, Confidence::Exact));
        }
        let module = self.module_of(file);
        if let Some((qualifier, last)) = split_qualified(name) {
            let found = self.qualified_symbols(file, imports, qualifier, last)?;
            let types: Vec<u32> = found
                .into_iter()
                .filter(|&s| self.symbols[s as usize].kind.is_type())
                .collect();
            return (!types.is_empty()).then_some((types, Confidence::Exact));
        }
        if let Some(facts) = self.facts(file) {
            let local: Vec<u32> = facts
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, symbol)| symbol.name == name && symbol.kind().is_type())
                .map(|(local, _)| self.base[file as usize] + local as u32)
                .collect();
            if !local.is_empty() {
                return Some((local, Confidence::Exact));
            }
        }
        if let Some(imports) = imports
            && let Some(Target::Symbols(symbols)) = imports.names.get(name)
        {
            let types: Vec<u32> = symbols
                .iter()
                .copied()
                .filter(|&s| self.symbols[s as usize].kind.is_type())
                .collect();
            if !types.is_empty() {
                return Some((types, Confidence::Exact));
            }
        }
        if let Some(types) = self.types.get(&(module, name)) {
            return Some((types.clone(), Confidence::Exact));
        }
        if let Some(imports) = imports {
            for &glob in &imports.globs {
                if let Some(types) = self.types.get(&(glob, name)) {
                    return Some((types.clone(), Confidence::Exact));
                }
            }
        }
        let candidates: Vec<u32> = self
            .by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&s| self.symbols[s as usize].kind.is_type())
            .collect();
        unique(&candidates, |s| self.is_test(s)).map(|symbol| (vec![symbol], Confidence::Inferred))
    }

    /// Owner type of the method enclosing `scope`.
    fn owner_type(&self, file: u32, scope: Option<u32>) -> Option<Vec<u32>> {
        let facts = self.facts(file)?;
        let mut current = scope;
        while let Some(local) = current {
            let symbol = &facts.symbols[local as usize];
            if !symbol.owner.is_empty() {
                return self
                    .resolve_type(file, None, None, &symbol.owner)
                    .map(|(types, _)| types);
            }
            if symbol.kind().is_type() {
                return Some(vec![self.base[file as usize] + local]);
            }
            current = symbol.parent;
        }
        None
    }

    /// Symbols `name` reached through a qualifier (`pkg`, `a::b`, `Type`).
    fn qualified_symbols(
        &self,
        file: u32,
        imports: Option<&Imports>,
        qualifier: &str,
        name: &str,
    ) -> Option<Vec<u32>> {
        let imports = imports.or_else(|| self.file_imports.get(file as usize));
        if self.file_type(file) == Some(FileType::Python)
            && let Some(found) =
                imports.and_then(|imports| self.python_qualified(imports, qualifier, name))
        {
            return Some(found);
        }
        let rust = self.file_type(file) == Some(FileType::Rust);
        if rust && qualifier.contains("::") {
            let segments: Vec<&str> = qualifier.split("::").collect();
            let base = self.rust_path(file, &segments).or_else(|| {
                imports.and_then(|imports| {
                    let first = imports.names.get(segments[0])?.clone();
                    segments[1..].iter().try_fold(first, |target, segment| {
                        self.rust_member(file, &target, segment)
                    })
                })
            })?;
            return match self.rust_member(file, &base, name)? {
                Target::Symbols(symbols) => Some(symbols),
                _ => None,
            };
        }
        if let Some(imports) = imports
            && let Some(target) = imports.names.get(qualifier)
        {
            return match target {
                Target::Module(module) => {
                    let mut found = self.module_symbol(*module, name, None);
                    if found.is_empty()
                        && self.modules.modules[*module as usize]
                            .key
                            .starts_with("js:")
                    {
                        found = self.js_exported(*module, name);
                    }
                    (!found.is_empty()).then_some(found)
                }
                Target::Symbols(symbols) => {
                    let found: Vec<u32> = symbols
                        .iter()
                        .flat_map(|&s| self.members(s, name))
                        .collect();
                    (!found.is_empty()).then_some(found)
                }
                Target::External(_) => None,
            };
        }
        let (types, _) = self.resolve_type(file, imports, None, qualifier)?;
        let found: Vec<u32> = types.iter().flat_map(|&s| self.members(s, name)).collect();
        (!found.is_empty()).then_some(found)
    }

    // ---------------------------------------------------------------- references

    fn resolve_file(&self, file: u32, imports: &Imports) -> Resolved {
        let mut resolved = Resolved {
            edges: Vec::new(),
            imported_modules: imports.modules.clone(),
            ambiguous: 0,
            externals: imports.externals.clone(),
        };
        let Some(facts) = self.facts(file) else {
            return resolved;
        };
        let base = self.base[file as usize];
        let file_node = self.symbols.len() as u32 + file;
        let file_is_test = self.input.files[file as usize].role == Role::Test;
        for reference in &facts.refs {
            let source = reference.scope.map_or(file_node, |scope| base + scope);
            let outcome = match reference.kind {
                RefKind::Call | RefKind::Type => self.resolve_ref(file, imports, reference),
                RefKind::Inherit => self.resolve_supertype(file, imports, reference),
                RefKind::Path => self.resolve_path(file, &reference.name),
                RefKind::Mention => self.resolve_mention(file, &reference.name),
                _ => Outcome::Unresolved,
            };
            match outcome {
                Outcome::Targets(targets, confidence) => {
                    let source_is_test =
                        file_is_test || (source < file_node && self.is_test(source));
                    let mut kind = match reference.kind {
                        RefKind::Call => EdgeKind::Calls,
                        RefKind::Inherit => EdgeKind::Inherits,
                        RefKind::Path => EdgeKind::References,
                        RefKind::Mention => EdgeKind::Mentions,
                        _ => EdgeKind::Uses,
                    };
                    if kind == EdgeKind::Calls && source_is_test {
                        kind = EdgeKind::Tests;
                    }
                    for (target, target_confidence) in targets {
                        if target == source || self.is_trivial(source, target, kind) {
                            continue;
                        }
                        resolved.edges.push(Edge {
                            source,
                            target,
                            kind,
                            confidence: confidence.max(target_confidence),
                            line: reference.line,
                        });
                    }
                }
                Outcome::External => {}
                Outcome::Ambiguous => resolved.ambiguous += 1,
                Outcome::Unresolved => {}
            }
        }
        for &(from, to) in &imports.files {
            resolved.edges.push(Edge {
                source: self.symbols.len() as u32 + from,
                target: self.symbols.len() as u32 + to,
                kind: EdgeKind::Imports,
                confidence: Confidence::Exact,
                line: 1,
            });
        }
        resolved
    }

    /// A literal relative path: relative to the referring file, then to the root.
    fn resolve_path(&self, file: u32, target: &str) -> Outcome {
        match self.file_by_path(file, target) {
            Some(found) if found != file => exact(self.symbols.len() as u32 + found),
            _ => Outcome::Unresolved,
        }
    }

    fn file_by_path(&self, file: u32, target: &str) -> Option<u32> {
        let dir = modules::parent_dir(&self.input.files[file as usize].path);
        let joined = if dir.is_empty() {
            target.to_owned()
        } else {
            format!("{dir}/{target}")
        };
        [normalize_path(&joined), normalize_path(target)]
            .iter()
            .find_map(|path| self.path_index.get(path.as_str()).copied())
    }

    /// A backticked mention: a path names a file (exact); an identifier or qualified suffix
    /// names the unique symbol carrying it (inferred).
    fn resolve_mention(&self, file: u32, mention: &str) -> Outcome {
        if mention.contains('/') {
            return self.resolve_path(file, mention);
        }
        let normalized = mention.replace("::", ".");
        let name = normalized.rsplit('.').next().unwrap_or(&normalized);
        let candidates: Vec<u32> = self
            .by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&symbol| {
                let qualified = self.symbols[symbol as usize].qualified.replace("::", ".");
                normalized == name
                    || qualified == normalized
                    || qualified.ends_with(&format!(".{normalized}"))
            })
            .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Heading)
            .collect();
        match unique(&candidates, |symbol| self.is_test(symbol)) {
            Some(symbol) => {
                Outcome::Targets(vec![(symbol, Confidence::Inferred)], Confidence::Inferred)
            }
            None if candidates.is_empty() => Outcome::Unresolved,
            None => Outcome::Ambiguous,
        }
    }

    fn caller_is_test(&self, file: u32, scope: Option<u32>) -> bool {
        self.input.files[file as usize].role == Role::Test
            || scope.is_some_and(|scope| self.is_test(self.base[file as usize] + scope))
    }

    /// Edges that carry no information: a file using its own symbols (impl headers) and a
    /// method using its own owner type.
    fn is_trivial(&self, source: u32, target: u32, kind: EdgeKind) -> bool {
        let symbols = self.symbols.len() as u32;
        if source >= symbols {
            return target < symbols && self.symbols[target as usize].file == source - symbols;
        }
        if kind != EdgeKind::Uses {
            return false;
        }
        let method = &self.symbols[source as usize];
        let owner = &self.symbols[target as usize];
        let facts = self.facts(method.file);
        let local = source - self.base[method.file as usize];
        let owner_name = facts
            .and_then(|facts| facts.symbols.get(local as usize))
            .map(|symbol| symbol.owner.as_str())
            .unwrap_or_default();
        method.kind == SymKind::Method
            && owner.kind.is_type()
            && owner_base(owner_name) == owner.name
            || method.parent == target
    }

    fn resolve_supertype(&self, file: u32, imports: &Imports, reference: &Ref) -> Outcome {
        let name = match &reference.qualifier {
            Some(qualifier) if self.file_type(file) == Some(FileType::Rust) => {
                format!("{qualifier}::{}", reference.name)
            }
            Some(qualifier) => format!("{qualifier}.{}", reference.name),
            None => reference.name.clone(),
        };
        match self.resolve_type(file, Some(imports), reference.scope, &name) {
            Some((types, confidence)) if types.len() == 1 => {
                Outcome::Targets(vec![(types[0], confidence)], confidence)
            }
            Some(_) => Outcome::Ambiguous,
            None => Outcome::Unresolved,
        }
    }

    fn resolve_ref(&self, file: u32, imports: &Imports, reference: &Ref) -> Outcome {
        let wants_type = reference.kind == RefKind::Type;
        match reference.qualifier.as_deref() {
            None => self.resolve_plain(file, imports, reference, wants_type),
            Some(qualifier) => self.resolve_qualified(file, imports, reference, qualifier),
        }
    }

    /// Steps 1, 2, 4 and 7 for an unqualified name.
    fn resolve_plain(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        wants_type: bool,
    ) -> Outcome {
        let name = reference.name.as_str();
        let kind_ok = |symbol: u32| {
            let kind = self.symbols[symbol as usize].kind;
            if wants_type {
                kind.is_type()
            } else {
                kind.is_callable() || kind.is_type()
            }
        };
        // 1. Lexical scope: definitions of the file visible from the reference.
        if let Some(facts) = self.facts(file) {
            let base = self.base[file as usize];
            let chain = scope_chain(facts, reference.scope);
            let mut best: Option<(usize, u32)> = None;
            let mut count = 0;
            for (local, symbol) in facts.symbols.iter().enumerate() {
                let id = base + local as u32;
                if symbol.name != name || !kind_ok(id) || symbol.kind() == SymKind::Method {
                    continue;
                }
                let depth = match symbol.parent {
                    None => 0,
                    Some(parent) => match chain.iter().position(|&p| p == parent) {
                        Some(position) => chain.len() - position,
                        None => continue,
                    },
                };
                count += 1;
                if best.is_none_or(|(best_depth, _)| depth > best_depth) {
                    best = Some((depth, id));
                }
            }
            if let Some((_, id)) = best
                && (count == 1 || self.file_type(file) == Some(FileType::Rust))
            {
                return exact(id);
            }
        }
        // 2. Names bound by imports.
        if let Some(target) = imports.names.get(name) {
            return match target {
                Target::Symbols(symbols) => {
                    let wanted: Vec<u32> =
                        symbols.iter().copied().filter(|&s| kind_ok(s)).collect();
                    pick(&wanted, Confidence::Exact)
                }
                Target::Module(_) | Target::External(_) => Outcome::External,
            };
        }
        // 4. Same module unit.
        let module = self.module_of(file);
        let same: Vec<u32> = self
            .module_symbol(module, name, None)
            .into_iter()
            .filter(|&s| kind_ok(s))
            .collect();
        if !same.is_empty() {
            return pick(&same, Confidence::Exact);
        }
        for &glob in &imports.globs {
            let found: Vec<u32> = self
                .module_symbol(glob, name, None)
                .into_iter()
                .filter(|&s| kind_ok(s))
                .collect();
            if !found.is_empty() {
                return pick(&found, Confidence::Exact);
            }
        }
        // 7. Unique name among imported modules, then in the root. Go and Rust only see
        // unqualified names of their own module and imports, so the root is never searched.
        let reach = if has_module_rules(self.file_type(file)) {
            Reach::Imported(module)
        } else {
            Reach::Root
        };
        let caller_is_test = self.caller_is_test(file, reference.scope);
        self.unique_fallback(imports, name, reach, caller_is_test, |s| {
            kind_ok(s) && self.symbols[s as usize].kind != SymKind::Method
        })
    }

    /// Steps 3, 5 and 6 for a qualified reference or member call.
    fn resolve_qualified(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        qualifier: &str,
    ) -> Outcome {
        let name = reference.name.as_str();
        let rust = self.file_type(file) == Some(FileType::Rust);
        let chain: Vec<&str> = qualifier.split('.').map(str::trim).collect();
        let head = chain[0];
        // 3. Qualified name through imports and the module map.
        let plain_head = chain.len() == 1 && is_identifier(head);
        if (plain_head || (rust && qualifier.contains("::")))
            && let Some(found) = self.qualified_symbols(file, Some(imports), qualifier, name)
        {
            return pick_members(self, found, Confidence::Exact);
        }
        if plain_head && let Some(Target::External(_) | Target::Module(_)) = imports.names.get(head)
        {
            return Outcome::External;
        }
        // 5a. Receiver that is itself a call (`Service().run()`, `open(x).read()`): the result
        // type of the call, inferred.
        if let Some(callee) = call_receiver(qualifier) {
            let binding = Binding {
                scope: reference.scope,
                name: String::new(),
                type_name: format!("{callee}()"),
                line: reference.line,
                declared: false,
            };
            if let Some((types, _)) = self.binding_type(file, imports, reference.scope, &binding) {
                let targets: Vec<(u32, Confidence)> = types
                    .iter()
                    .flat_map(|&type_symbol| self.members(type_symbol, name))
                    .map(|member| (member, Confidence::Inferred))
                    .collect();
                if !targets.is_empty() {
                    return Outcome::Targets(targets, Confidence::Inferred);
                }
            }
        }
        // 5. Member call: receiver type from self, bindings or fields.
        if chain.iter().all(|segment| is_identifier(segment)) {
            let receiver =
                self.receiver_type(file, imports, reference.scope, &chain, reference.line);
            if let Receiver::External = receiver {
                return Outcome::External;
            }
            if let Receiver::Known(types, confidence) = receiver {
                let mut targets = Vec::new();
                for &type_symbol in &types {
                    let direct = self.members(type_symbol, name);
                    targets.extend(direct.iter().map(|&member| (member, Confidence::Exact)));
                    // 6. Abstract receiver: every implementation, inferred.
                    if self.symbols[type_symbol as usize].kind.is_abstract() {
                        let implementations = self
                            .implementations
                            .get(&type_symbol)
                            .cloned()
                            .unwrap_or_default();
                        let mut fan_out = Vec::new();
                        let caller_is_test = self.input.files[file as usize].role == Role::Test
                            || reference.scope.is_some_and(|scope| {
                                self.is_test(self.base[file as usize] + scope)
                            });
                        for implementation in implementations {
                            // Test fakes are implementations only for test callers.
                            if self.is_test(implementation) && !caller_is_test {
                                continue;
                            }
                            fan_out.extend(self.members_direct(implementation, name));
                        }
                        if fan_out.len() <= MAX_DISPATCH {
                            targets.extend(
                                fan_out
                                    .into_iter()
                                    .map(|member| (member, Confidence::Inferred)),
                            );
                        }
                    }
                }
                targets.sort_unstable();
                targets.dedup_by_key(|(member, _)| *member);
                if !targets.is_empty() {
                    return Outcome::Targets(targets, confidence);
                }
                return Outcome::Unresolved;
            }
        }
        // 7. Unique method name in the root, unless the name is too common to trust.
        if is_common_member(name) {
            return Outcome::Unresolved;
        }
        // Go method sets come from the types of imported packages; other languages may reach
        // methods of any type in the root.
        // Methods come from types of the own and imported modules; Rust trait methods may live
        // anywhere in the crate, languages without module rules anywhere in the root.
        let reach = if has_module_rules(self.file_type(file))
            && self.file_type(file) != Some(FileType::Rust)
        {
            Reach::Imported(self.module_of(file))
        } else {
            Reach::Root
        };
        let caller_is_test = self.caller_is_test(file, reference.scope);
        self.unique_fallback(imports, name, reach, caller_is_test, |s| {
            self.symbols[s as usize].kind == SymKind::Method
        })
    }

    /// Type of the receiver chain `x`, `self`, `x.field`, `self.field.other`. A receiver whose
    /// binding or field exists but whose type is not in the root is external: no fallback.
    fn receiver_type(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        chain: &[&str],
        line: u32,
    ) -> Receiver {
        let head = chain[0];
        let start = if matches!(head, "self" | "this" | "Self") {
            match self.owner_type(file, scope) {
                Some(types) => Some((types, Confidence::Exact)),
                None => return Receiver::Unknown,
            }
        } else if let Some(binding) = self.binding(file, scope, head, line) {
            match self.binding_type(file, imports, scope, binding) {
                Some(found) => Some(found),
                None if binding.type_name.ends_with("()") => return Receiver::Unknown,
                None => return Receiver::External,
            }
        } else {
            self.resolve_type(file, Some(imports), scope, head)
        };
        let Some((mut types, mut confidence)) = start else {
            return Receiver::Unknown;
        };
        for field in &chain[1..] {
            let Some(&type_symbol) = types.first() else {
                return Receiver::Unknown;
            };
            let Some(&field_type) = self.field_types.get(&(type_symbol, *field)) else {
                return Receiver::Unknown;
            };
            let owner_file = self.symbols[type_symbol as usize].file;
            match self.resolve_type(owner_file, None, None, field_type) {
                Some((next, next_confidence)) => {
                    types = next;
                    confidence = confidence.max(next_confidence);
                }
                None => return Receiver::External,
            }
        }
        Receiver::Known(types, confidence)
    }

    fn binding_type(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        binding: &Binding,
    ) -> Option<(Vec<u32>, Confidence)> {
        // `x = x.next()` and mutually dependent call results recurse through receiver typing;
        // a depth guard ends the chain (the type stays unknown).
        let _guard = DepthGuard::enter()?;
        let confidence = if binding.declared {
            Confidence::Exact
        } else {
            Confidence::Inferred
        };
        if let Some(call) = binding.type_name.strip_suffix("()") {
            // Result of a call: resolve the callee, then its declared result type.
            let reference = match split_qualified(call) {
                Some((qualifier, name)) => Ref {
                    kind: RefKind::Call,
                    name: name.to_owned(),
                    qualifier: Some(qualifier.to_owned()),
                    scope,
                    line: binding.line,
                },
                None => Ref {
                    kind: RefKind::Call,
                    name: call.to_owned(),
                    qualifier: None,
                    scope,
                    line: binding.line,
                },
            };
            let Outcome::Targets(callees, _) = self.resolve_ref(file, imports, &reference) else {
                return None;
            };
            let (callee, _) = *callees.first()?;
            // A call of a type is a constructor: the result is the type itself.
            if self.symbols[callee as usize].kind.is_type() {
                return Some((vec![callee], Confidence::Inferred));
            }
            let result = *self.returns.get(&callee)?;
            let callee_file = self.symbols[callee as usize].file;
            let callee_scope = Some(callee - self.base[callee_file as usize]);
            let (types, _) = self.resolve_type(callee_file, None, callee_scope, result)?;
            return Some((types, Confidence::Inferred));
        }
        let (types, type_confidence) =
            self.resolve_type(file, Some(imports), scope, &binding.type_name)?;
        Some((types, confidence.max(type_confidence)))
    }

    /// Closest binding of `name` visible from `scope` at or before `line`.
    fn binding(&self, file: u32, scope: Option<u32>, name: &str, line: u32) -> Option<&'a Binding> {
        let facts = self.facts(file)?;
        let chain = scope_chain(facts, scope);
        facts
            .bindings
            .iter()
            .filter(|binding| binding.name == name && binding.line <= line)
            .filter(|binding| match binding.scope {
                None => true,
                Some(binding_scope) => chain.contains(&binding_scope),
            })
            .max_by_key(|binding| {
                (
                    binding.scope.map_or(0, |s| {
                        chain.iter().position(|&c| c == s).map_or(0, |p| p + 1)
                    }),
                    binding.line,
                )
            })
    }

    fn unique_fallback(
        &self,
        imports: &Imports,
        name: &str,
        reach: Reach,
        caller_is_test: bool,
        accept: impl Fn(u32) -> bool,
    ) -> Outcome {
        // Test symbols are targets only for test code; definitions nested in a function are
        // invisible outside it.
        let candidates: Vec<u32> = self
            .by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&s| accept(s))
            .filter(|&s| caller_is_test || !self.is_test(s))
            .filter(|&s| {
                let parent = self.symbols[s as usize].parent;
                parent == NONE || self.symbols[parent as usize].kind.is_type()
            })
            .collect();
        if candidates.is_empty() {
            return Outcome::Unresolved;
        }
        let imported: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|&s| {
                let module = self.module_of(self.symbols[s as usize].file);
                imports.modules.contains(&module) || reach == Reach::Imported(module)
            })
            .collect();
        if let Some(symbol) = unique(&imported, |s| self.is_test(s)) {
            return Outcome::Targets(vec![(symbol, Confidence::Inferred)], Confidence::Inferred);
        }
        if let Reach::Imported(_) = reach {
            return if imported.is_empty() {
                Outcome::Unresolved
            } else {
                Outcome::Ambiguous
            };
        }
        match unique(&candidates, |s| self.is_test(s)) {
            Some(symbol) => {
                Outcome::Targets(vec![(symbol, Confidence::Inferred)], Confidence::Inferred)
            }
            None => Outcome::Ambiguous,
        }
    }

    // ---------------------------------------------------------------- implementations

    /// Explicit `implements` (Rust `impl Trait for Type`), declared supertypes, and Go method
    /// sets satisfying interfaces (inferred).
    fn link_implementations(&mut self) {
        let (pairs, inherited) = self.implementation_candidates();
        self.store_implementations(pairs, inherited);
    }

    /// Implementation pairs (emitted as `implements` edges) and declared supertypes (`inherits`
    /// edges come from the references themselves); both feed member lookup and dispatch.
    fn implementation_candidates(&self) -> (Vec<Pair>, Vec<Pair>) {
        let imports = &self.file_imports;
        let mut inherited: Vec<(u32, u32, Confidence, u32)> = Vec::new();
        let mut pairs: Vec<(u32, u32, Confidence, u32)> = Vec::new();
        for (file, facts) in self.input.facts.iter().enumerate() {
            let Some(facts) = facts else { continue };
            let file = file as u32;
            for reference in &facts.refs {
                if reference.kind != RefKind::Implement {
                    continue;
                }
                // `class C implements I` names the implementing type by scope; Rust
                // `impl I for T` by qualifier.
                let Some(qualifier) = &reference.qualifier else {
                    if let Some(scope) = reference.scope
                        && let Outcome::Targets(targets, confidence) =
                            self.resolve_supertype(file, &imports[file as usize], reference)
                    {
                        for (target, _) in targets {
                            pairs.push((
                                self.base[file as usize] + scope,
                                target,
                                confidence,
                                reference.line,
                            ));
                        }
                    }
                    continue;
                };
                let Some((types, _)) =
                    self.resolve_type(file, Some(&imports[file as usize]), None, qualifier)
                else {
                    continue;
                };
                let Some((traits, confidence)) =
                    self.resolve_type(file, Some(&imports[file as usize]), None, &reference.name)
                else {
                    continue;
                };
                for &type_symbol in &types {
                    for &trait_symbol in &traits {
                        pairs.push((type_symbol, trait_symbol, confidence, reference.line));
                    }
                }
            }
            for reference in &facts.refs {
                if reference.kind != RefKind::Inherit {
                    continue;
                }
                let Some(scope) = reference.scope else {
                    continue;
                };
                if let Outcome::Targets(targets, confidence) =
                    self.resolve_supertype(file, &imports[file as usize], reference)
                {
                    for (target, _) in targets {
                        inherited.push((
                            self.base[file as usize] + scope,
                            target,
                            confidence,
                            reference.line,
                        ));
                    }
                }
            }
        }
        pairs.extend(self.go_structural_implementations());
        (pairs, inherited)
    }

    fn store_implementations(
        &mut self,
        pairs: Vec<(u32, u32, Confidence, u32)>,
        inherited: Vec<(u32, u32, Confidence, u32)>,
    ) {
        for &(type_symbol, trait_symbol, _, _) in pairs.iter().chain(&inherited) {
            self.implementations
                .entry(trait_symbol)
                .or_default()
                .push(type_symbol);
            self.supertypes
                .entry(type_symbol)
                .or_default()
                .push(trait_symbol);
        }
        self.implementation_pairs = pairs;
    }

    fn go_structural_implementations(&self) -> Vec<(u32, u32, Confidence, u32)> {
        let mut interfaces: Vec<(u32, Vec<(&str, usize)>)> = Vec::new();
        let mut concrete: Vec<u32> = Vec::new();
        for (id, symbol) in self.symbols.iter().enumerate() {
            if self.file_type(symbol.file) != Some(FileType::Go) {
                continue;
            }
            match symbol.kind {
                SymKind::Iface => {
                    let methods: Vec<(&str, usize)> = self
                        .symbols
                        .iter()
                        .filter(|child| child.parent == id as u32 && child.kind == SymKind::Method)
                        .map(|child| (child.name.as_str(), param_count(&child.signature)))
                        .collect();
                    if !methods.is_empty() {
                        interfaces.push((id as u32, methods));
                    }
                }
                SymKind::Struct | SymKind::Alias => concrete.push(id as u32),
                _ => {}
            }
        }
        // Method names with parameter counts per (scope, owner), computed once.
        let mut by_owner: HashMap<(u32, &str), HashMap<&str, usize>> = HashMap::default();
        for (&(scope, owner, name), methods) in &self.methods {
            for &method in methods {
                let params = param_count_method(&self.symbols[method as usize].signature);
                by_owner
                    .entry((scope, owner))
                    .or_default()
                    .insert(name, params);
            }
        }
        let empty = HashMap::default();
        let method_sets: Vec<&HashMap<&str, usize>> = concrete
            .iter()
            .map(|&type_symbol| {
                let symbol = &self.symbols[type_symbol as usize];
                let scope = self.scope_ids[self.module_of(symbol.file) as usize];
                by_owner
                    .get(&(scope, symbol.name.as_str()))
                    .unwrap_or(&empty)
            })
            .collect();
        let mut module_imports: HashSet<(u32, u32)> = HashSet::new();
        for (file, imports) in self.file_imports.iter().enumerate() {
            let source = self.modules.file_module[file];
            for &target in &imports.modules {
                module_imports.insert((source, target));
            }
        }
        let mut pairs = Vec::new();
        for (position, &type_symbol) in concrete.iter().enumerate() {
            let set = method_sets[position];
            if set.is_empty() {
                continue;
            }
            let type_module = self.module_of(self.symbols[type_symbol as usize].file);
            for (interface, methods) in &interfaces {
                let interface_symbol = &self.symbols[*interface as usize];
                let interface_module = self.module_of(interface_symbol.file);
                // A test type implements only test interfaces; an unexported interface of another
                // package can only receive types its package imports.
                if self.is_test(type_symbol) && !self.is_test(*interface) {
                    continue;
                }
                if !interface_symbol.exported
                    && interface_module != type_module
                    && !module_imports.contains(&(interface_module, type_module))
                {
                    continue;
                }
                if methods
                    .iter()
                    .all(|(name, params)| set.get(name) == Some(params))
                {
                    let line = self.symbols[type_symbol as usize].line;
                    pairs.push((type_symbol, *interface, Confidence::Inferred, line));
                }
            }
        }
        pairs
    }

    // ---------------------------------------------------------------- graph

    fn into_graph(mut self, resolved: Vec<Resolved>) -> Graph {
        let members = self.member_pairs();
        let symbol_count = self.symbols.len() as u32;
        let symbols = std::mem::take(&mut self.symbols);
        let mut graph = Graph {
            symbols,
            members,
            ..Graph::default()
        };
        for (file, record) in self.input.files.iter().enumerate() {
            let facts = self.input.facts[file].as_ref();
            graph.files.push(GraphFile {
                path: record.path.clone(),
                role: record.role,
                file_type: record.file_type,
                module: self.modules.file_module[file],
                lines: facts.map_or(0, |facts| facts.lines),
                first_symbol: self.base[file],
                symbol_count: facts.map_or(0, |facts| facts.symbols.len() as u32),
                ambiguous: 0,
                entry: facts.is_some_and(|facts| facts.entry),
                doc: facts.map(|facts| facts.doc.clone()).unwrap_or_default(),
            });
        }
        let mut edges: Vec<Edge> = Vec::new();
        let mut externals: BTreeMap<(u32, String), u32> = BTreeMap::new();
        let mut module_imports: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for (file, outcome) in resolved.into_iter().enumerate() {
            graph.files[file].ambiguous = outcome.ambiguous;
            let source = self.modules.file_module[file];
            let file_is_test = self.input.files[file].role == Role::Test;
            for &target in &outcome.imported_modules {
                if target != source && !file_is_test {
                    *module_imports.entry((source, target)).or_default() += 1;
                }
            }
            edges.extend(outcome.edges);
            let module = self.modules.file_module[file];
            let mut seen = HashSet::new();
            for external in outcome.externals {
                if seen.insert(external.clone()) && !is_std_external(&external) {
                    *externals.entry((module, external)).or_default() += 1;
                }
            }
        }
        for &(type_symbol, trait_symbol, confidence, line) in &self.implementation_pairs {
            edges.push(Edge {
                source: type_symbol,
                target: trait_symbol,
                kind: EdgeKind::Implements,
                confidence,
                line,
            });
        }
        graph.edges = dedupe_edges(edges);
        graph.module_edges = module_imports
            .into_iter()
            .map(
                |((source, target), count)| crate::domain::graph::ModuleEdge {
                    source,
                    target,
                    count,
                },
            )
            .collect();
        graph.externals = externals
            .into_iter()
            .map(|((module, name), uses)| External { module, name, uses })
            .collect();
        for (file, facts) in self.input.facts.iter().enumerate() {
            let Some(facts) = facts else { continue };
            let base = self.base[file];
            for env in &facts.envs {
                graph.envs.push(EnvSite {
                    name: env.name.clone(),
                    node: env
                        .scope
                        .map_or(symbol_count + file as u32, |scope| base + scope),
                    line: env.line,
                    source: 0,
                });
            }
            for def in &facts.env_defs {
                graph.envs.push(EnvSite {
                    name: def.name.clone(),
                    node: symbol_count + file as u32,
                    line: def.line,
                    source: def.source as u8,
                });
            }
            for fields in &facts.fields {
                graph.fields.push(FieldList {
                    symbol: base + fields.owner,
                    names: fields.names.clone(),
                });
            }
        }
        graph.envs.sort();
        derive::finish(&mut graph, &self.modules);
        graph
    }
}

impl Index<'_> {
    /// Methods of each type: nested definitions plus methods declared with an owner (Go
    /// receivers, Rust `impl` blocks) in the type's method scope.
    fn member_pairs(&self) -> Vec<(u32, u32)> {
        let mut types_by_scope: HashMap<(u32, &str), Vec<u32>> = HashMap::default();
        for (id, symbol) in self.symbols.iter().enumerate() {
            if symbol.kind.is_type() {
                let scope = self.scope_ids[self.module_of(symbol.file) as usize];
                types_by_scope
                    .entry((scope, symbol.name.as_str()))
                    .or_default()
                    .push(id as u32);
            }
        }
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for (&(scope, owner, _), methods) in &self.methods {
            for &type_symbol in types_by_scope.get(&(scope, owner)).into_iter().flatten() {
                pairs.extend(methods.iter().map(|&method| (type_symbol, method)));
            }
        }
        for (id, symbol) in self.symbols.iter().enumerate() {
            if symbol.parent != NONE && self.symbols[symbol.parent as usize].kind.is_type() {
                pairs.push((symbol.parent, id as u32));
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        pairs
    }
}

/// Edges unique per (source, target, kind); exact wins over inferred, first line kept.
fn dedupe_edges(mut edges: Vec<Edge>) -> Vec<Edge> {
    edges.sort_unstable_by_key(|edge| {
        (
            edge.source,
            edge.target,
            edge.kind,
            edge.confidence,
            edge.line,
        )
    });
    edges.dedup_by(|next, kept| {
        next.source == kept.source && next.target == kept.target && next.kind == kept.kind
    });
    edges
}

/// Languages whose unqualified names come only from their module and its imports.
fn has_module_rules(file_type: Option<FileType>) -> bool {
    matches!(
        file_type,
        Some(
            FileType::Go
                | FileType::Rust
                | FileType::Python
                | FileType::TypeScript
                | FileType::JavaScript
                | FileType::Vue
                | FileType::Svelte
                | FileType::Astro
                | FileType::Bash
        )
    )
}

/// Limits the nesting of binding and receiver typing per thread.
struct DepthGuard;

const MAX_TYPING_DEPTH: u32 = 8;

thread_local! {
    static TYPING_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

impl DepthGuard {
    fn enter() -> Option<Self> {
        TYPING_DEPTH.with(|depth| {
            if depth.get() >= MAX_TYPING_DEPTH {
                None
            } else {
                depth.set(depth.get() + 1);
                Some(Self)
            }
        })
    }
}

impl Drop for DepthGuard {
    fn drop(&mut self) {
        TYPING_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// (type, supertype, confidence, line).
type Pair = (u32, u32, Confidence, u32);

/// How far the unique-name fallback (step 7) may look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// Imported modules and the given own module only.
    Imported(u32),
    Root,
}

enum Receiver {
    Known(Vec<u32>, Confidence),
    External,
    Unknown,
}

enum Outcome {
    Targets(Vec<(u32, Confidence)>, Confidence),
    External,
    Ambiguous,
    Unresolved,
}

fn exact(symbol: u32) -> Outcome {
    Outcome::Targets(vec![(symbol, Confidence::Exact)], Confidence::Exact)
}

fn pick(candidates: &[u32], confidence: Confidence) -> Outcome {
    match candidates {
        [] => Outcome::Unresolved,
        [only] => Outcome::Targets(vec![(*only, confidence)], confidence),
        _ => Outcome::Ambiguous,
    }
}

fn pick_members(index: &Index, found: Vec<u32>, confidence: Confidence) -> Outcome {
    let mut found = found;
    found.sort_unstable();
    found.dedup();
    if found.len() > 1 {
        let non_test: Vec<u32> = found
            .iter()
            .copied()
            .filter(|&s| !index.is_test(s))
            .collect();
        if non_test.len() == 1 {
            return Outcome::Targets(vec![(non_test[0], confidence)], confidence);
        }
    }
    pick(&found, confidence)
}

/// The single candidate, preferring non-test symbols when several exist.
fn unique(candidates: &[u32], is_test: impl Fn(u32) -> bool) -> Option<u32> {
    match candidates {
        [] => None,
        [only] => Some(*only),
        _ => {
            let non_test: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&s| !is_test(s))
                .collect();
            (non_test.len() == 1).then(|| non_test[0])
        }
    }
}

/// Local indices of `scope` and its enclosing symbols, innermost first.
fn scope_chain(facts: &Facts, scope: Option<u32>) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut current = scope;
    while let Some(local) = current {
        chain.push(local);
        current = facts
            .symbols
            .get(local as usize)
            .and_then(|symbol| symbol.parent);
    }
    chain
}

/// Splits `pkg.Name` or `a::b::Name` at the last separator.
fn split_qualified(name: &str) -> Option<(&str, &str)> {
    if let Some((qualifier, last)) = name.rsplit_once("::") {
        return Some((qualifier, last));
    }
    name.rsplit_once('.')
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
        && !text.starts_with(|ch: char| ch.is_ascii_digit())
}

/// Parameter count of a signature's first parenthesized list.
fn param_count(signature: &str) -> usize {
    let Some(open) = signature.find('(') else {
        return 0;
    };
    count_list(&signature[open + 1..])
}

/// Parameter count of a Go method signature: the list after the receiver.
fn param_count_method(signature: &str) -> usize {
    let Some(open) = signature.find('(') else {
        return 0;
    };
    let rest = &signature[open + 1..];
    let Some(close) = rest.find(')') else {
        return 0;
    };
    let after = &rest[close + 1..];
    let Some(next) = after.find('(') else {
        return 0;
    };
    count_list(&after[next + 1..])
}

fn count_list(text: &str) -> usize {
    let mut depth = 0usize;
    let mut count = 0;
    let mut any = false;
    for ch in text.chars() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' if depth == 0 => break,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => count += 1,
            ch if !ch.is_whitespace() => any = true,
            _ => {}
        }
    }
    if any { count + 1 } else { 0 }
}

/// Resolves `.` and `..` segments of a root-relative path.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// The callee of a receiver expression that is a single call: `Service()`, `pkg.open(x)`.
fn call_receiver(qualifier: &str) -> Option<&str> {
    let qualifier = qualifier.trim();
    let open = qualifier.find('(')?;
    let callee = &qualifier[..open];
    let rest = &qualifier[open..];
    let balanced_once = rest.ends_with(')')
        && rest[1..rest.len() - 1].matches('(').count()
            == rest[1..rest.len() - 1].matches(')').count();
    (balanced_once
        && !callee.is_empty()
        && callee
            .split(['.', ':'])
            .filter(|part| !part.is_empty())
            .all(is_identifier)
        && !rest[1..rest.len() - 1].contains(")."))
    .then_some(callee)
}

/// Standard libraries: external, but not worth listing as dependencies.
fn is_std_external(name: &str) -> bool {
    name.starts_with("std:")
        || matches!(
            name,
            "std"
                | "core"
                | "alloc"
                | "os"
                | "sys"
                | "re"
                | "json"
                | "time"
                | "datetime"
                | "typing"
                | "pathlib"
                | "collections"
                | "itertools"
                | "functools"
                | "subprocess"
                | "logging"
                | "argparse"
                | "dataclasses"
                | "enum"
                | "abc"
                | "io"
                | "math"
                | "random"
                | "shutil"
                | "tempfile"
                | "unittest"
                | "uuid"
                | "hashlib"
                | "base64"
                | "copy"
                | "contextlib"
                | "asyncio"
                | "threading"
                | "socket"
                | "struct"
                | "string"
                | "textwrap"
                | "glob"
                | "inspect"
                | "traceback"
                | "warnings"
                | "pickle"
                | "csv"
                | "urllib"
                | "http"
                | "__future__"
                | "types"
                | "operator"
                | "platform"
                | "signal"
                | "queue"
        )
}

/// Strips generic arguments and pointer marks from an owner name.
fn owner_base(owner: &str) -> &str {
    let owner = owner.trim_start_matches(['*', '&']);
    owner.split(['<', '[']).next().unwrap_or(owner)
}

/// Member names too common across standard libraries to resolve by uniqueness alone.
fn is_common_member(name: &str) -> bool {
    matches!(
        name,
        "get"
            | "set"
            | "len"
            | "size"
            | "push"
            | "pop"
            | "insert"
            | "remove"
            | "append"
            | "add"
            | "map"
            | "filter"
            | "iter"
            | "into_iter"
            | "iter_mut"
            | "collect"
            | "unwrap"
            | "expect"
            | "clone"
            | "to_string"
            | "to_owned"
            | "format"
            | "join"
            | "split"
            | "trim"
            | "contains"
            | "keys"
            | "values"
            | "items"
            | "update"
            | "send"
            | "write"
            | "read"
            | "close"
            | "open"
            | "next"
            | "find"
            | "lock"
            | "run"
            | "start"
            | "stop"
            | "call"
            | "apply"
            | "new"
            | "default"
            | "from"
            | "into"
            | "as_ref"
            | "borrow"
            | "parse"
            | "print"
            | "log"
            | "info"
            | "debug"
            | "error"
            | "warn"
            | "emit"
            | "on"
            | "then"
            | "catch"
            | "resolve"
            | "reject"
            | "toString"
            | "equals"
            | "hashCode"
            | "length"
            | "forEach"
            | "some"
            | "every"
            | "reduce"
            | "slice"
            | "indexOf"
            | "includes"
            | "has"
            | "delete"
            | "clear"
            | "copy"
            | "sort"
            | "reverse"
            | "extend"
            | "lower"
            | "upper"
            | "strip"
            | "encode"
            | "decode"
            | "is_empty"
            | "is_some"
            | "is_none"
            | "ok"
            | "err"
            | "and_then"
            | "or_else"
            | "unwrap_or"
            | "unwrap_or_default"
            | "unwrap_or_else"
            | "map_err"
            | "ok_or"
            | "ok_or_else"
            | "as_str"
            | "to_vec"
            | "entry"
            | "or_insert"
            | "flush"
            | "exec"
            | "query"
            | "fetch"
            | "save"
            | "load"
            | "close_all"
            | "String"
            | "Error"
            | "Errorf"
            | "Printf"
            | "Sprintf"
            | "Println"
            | "Fatal"
            | "Fatalf"
            | "Close"
            | "Write"
            | "Read"
            | "Get"
            | "Set"
            | "Len"
            | "Add"
            | "Do"
            | "Run"
            | "Lock"
            | "Unlock"
            | "Wait"
            | "Done"
            | "Err"
            | "Value"
            | "Next"
            | "Scan"
            // Iterator, slice, map and string adaptors of standard libraries.
            | "skip" | "take" | "rev" | "chain" | "zip" | "enumerate" | "flat_map" | "filter_map"
            | "flatten" | "peekable" | "step_by" | "last" | "first" | "count" | "sum" | "min"
            | "max" | "any" | "all" | "position" | "fold" | "cloned" | "copied" | "nth"
            | "windows" | "chunks" | "sort_by" | "sort_by_key" | "sort_unstable" | "dedup"
            | "retain" | "truncate" | "drain" | "starts_with" | "ends_with" | "replace" | "lines"
            | "chars" | "bytes" | "trim_start" | "trim_end" | "to_lowercase" | "to_uppercase"
            | "split_once" | "rsplit" | "find_map" | "take_while" | "skip_while" | "inspect"
            | "for_each" | "max_by_key" | "min_by_key" | "partition" | "unzip" | "as_slice"
            | "as_bytes" | "as_mut" | "get_mut" | "push_str" | "extend_from_slice" | "is_ok"
            | "is_err" | "unwrap_err" | "map_or" | "map_or_else" | "into_inner" | "with_capacity"
            | "capacity" | "reserve" | "binary_search" | "contains_key" | "values_mut"
            | "or_default" | "or_insert_with" | "and_modify" | "elapsed" | "duration_since"
            | "as_secs" | "Sum" | "Equal" | "Contains" | "HasPrefix" | "HasSuffix" | "TrimSpace"
            | "Split" | "Join" | "Fields" | "Index" | "Replace" | "ToLower" | "ToUpper" | "Bytes"
            | "Unmarshal" | "Marshal" | "Encode" | "Decode" | "Seek" | "Stat" | "Name" | "Size"
            | "Mode" | "IsDir" | "Before" | "After" | "Unix" | "Format" | "Parse" | "Sub"
            | "Since" | "Until" | "Truncate" | "Reset" | "Cancel" | "Kind" | "Type" | "Elem"
            | "Field" | "Method" | "Call" | "Match" | "FindString" | "FindStringSubmatch"
            | "MatchString" | "ReplaceAllString" | "Header" | "Body" | "Status" | "Context"
            // Flag sets.
            | "Bool" | "Int" | "Int64" | "Uint" | "Float64" | "Duration" | "StringVar"
            | "BoolVar" | "IntVar" | "DurationVar" | "Var" | "Args" | "Arg" | "NArg"
    )
}

#[cfg(test)]
mod tests;
