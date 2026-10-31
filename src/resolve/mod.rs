//! Global resolution: facts of all files to the symbol graph (`docs/spec.md` section 6,
//! resolution order steps 1 to 8). Pure; recomputed from cached facts on every change (D6).

mod cfamily;
pub mod derive;
mod dotnet;
mod js;
mod jvm;
pub mod modules;
mod python;

use std::collections::BTreeMap;

use crate::domain::hash::FastMap as HashMap;
use crate::domain::hash::FastSet as HashSet;

use rayon::prelude::*;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::{self, Binding, FLAG_DECLARATION, Facts, Ref, RefKind, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::{
    self, Confidence, Edge, EdgeKind, EnvSite, External, FieldList, Graph, GraphFile, GraphSymbol,
    NONE,
};
use crate::domain::role::Role;
use modules::ModuleMap;

/// Bumped whenever resolution or derived data change; a mismatch re-derives the index.
pub const RESOLVE_VERSION: u32 = 42;

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
    /// File name -> files carrying it.
    name_index: HashMap<&'a str, Vec<u32>>,
    /// Types per (module, name).
    types: HashMap<(u32, &'a str), Vec<u32>>,
    /// Every symbol per name, for the unique-name fallback.
    by_name: HashMap<&'a str, Vec<u32>>,
    /// Declared result type of callables (bindings named "").
    returns: HashMap<u32, &'a str>,
    /// Field types per (type symbol, field name).
    field_types: HashMap<(u32, &'a str), &'a str>,
    /// Every (type symbol, field name), typed or not.
    field_names: HashSet<(u32, String)>,
    /// Implementations of abstract types and supertypes of concrete ones.
    implementations: HashMap<u32, Vec<u32>>,
    supertypes: HashMap<u32, Vec<u32>>,
    /// .NET `global using` directives with their file: visible in every file of the language.
    global_usings: Vec<(u32, &'a facts::Import)>,
    /// C-family include directories (root-relative) and direct includes per file.
    include_dirs: Vec<String>,
    includes: Vec<Vec<u32>>,
    /// SQL tables and views per lowercase name (SQL identifiers ignore case).
    tables: HashMap<String, Vec<u32>>,
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
    ambiguous_names: Vec<String>,
    stale: Vec<(u32, u32, String)>,
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
                // The owner of an out-of-line member (`Type::name` inside a namespace) is its
                // innermost qualifier.
                if !symbol.owner.is_empty() && path.first() != Some(&symbol.owner.as_str()) {
                    path.insert(0, symbol.owner.as_str());
                }
                path.reverse();
                // Headings are found by title; their file path locates them.
                if symbol.kind() == SymKind::Heading {
                    let role = input.files[file].role;
                    symbols.push(GraphSymbol {
                        file: file as u32,
                        kind: SymKind::Heading,
                        name: symbol.name.clone(),
                        qualified: symbol.name.clone(),
                        parent,
                        line: symbol.line,
                        end_line: symbol.end_line,
                        signature: String::new(),
                        doc: String::new(),
                        exported: true,
                        flags: if role == Role::Test {
                            facts::FLAG_TEST
                        } else {
                            0
                        },
                        rank: 0.0,
                    });
                    continue;
                }
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
            name_index: {
                let mut names: HashMap<&str, Vec<u32>> = HashMap::default();
                for (file, record) in input.files.iter().enumerate() {
                    names
                        .entry(role_file_name(&record.path))
                        .or_default()
                        .push(file as u32);
                }
                names
            },
            by_module: HashMap::default(),
            methods: HashMap::default(),
            types: HashMap::default(),
            by_name: HashMap::default(),
            returns: HashMap::default(),
            field_types: HashMap::default(),
            field_names: HashSet::default(),
            implementations: HashMap::default(),
            supertypes: HashMap::default(),
            global_usings: Vec::new(),
            include_dirs: cfamily::include_dirs(input.files, input.facts),
            includes: Vec::new(),
            tables: HashMap::default(),
        };
        index.build_tables();
        index.includes = (0..input.files.len() as u32)
            .map(|file| match index.facts(file) {
                Some(facts) if cfamily::is_cfamily(index.file_type(file)) => facts
                    .imports
                    .iter()
                    .filter_map(|import| index.include_target(file, &import.spec))
                    .collect(),
                _ => Vec::new(),
            })
            .collect();
        index
    }

    fn build_tables(&mut self) {
        let input = self.input;
        for (file, facts) in input.facts.iter().enumerate() {
            let Some(facts) = facts else { continue };
            let record_type = input.files[file].file_type;
            if jvm::is_dotnet(input.files[file].file_type) {
                self.global_usings.extend(
                    facts
                        .imports
                        .iter()
                        .filter(|import| import.reexport)
                        .map(|import| (file as u32, import)),
                );
            }
            let module = self.modules.file_module[file];
            let scope = self.scope_ids[module as usize];
            let base = self.base[file];
            for (local, symbol) in facts.symbols.iter().enumerate() {
                let id = base + local as u32;
                let kind = symbol.kind();
                let name = symbol.name.as_str();
                self.by_name.entry(name).or_default().push(id);
                if matches!(kind, SymKind::Table | SymKind::View) {
                    self.tables
                        .entry(name.to_ascii_lowercase())
                        .or_default()
                        .push(id);
                }
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
                        // A Julia `module` block wrapping the file is the module unit itself.
                        let in_file_module = symbol.parent.is_some_and(|parent| {
                            let parent = &facts.symbols[parent as usize];
                            parent.kind() == SymKind::Mod
                                && parent.parent.is_none()
                                && matches!(record_type, Some(FileType::Julia | FileType::Fortran))
                        });
                        // C++ namespaces span files: a function inside `namespace a { ... }`
                        // is a module-level name of its file like one outside any namespace.
                        let in_namespaces = record_type == Some(FileType::Cpp) && {
                            let mut parent = symbol.parent;
                            let mut only_namespaces = parent.is_some();
                            while let Some(index) = parent {
                                let enclosing = &facts.symbols[index as usize];
                                if enclosing.kind() != SymKind::Mod {
                                    only_namespaces = false;
                                    break;
                                }
                                parent = enclosing.parent;
                            }
                            only_namespaces
                        };
                        if symbol.parent.is_none() || in_file_module || in_namespaces {
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
                for name in &fields.names {
                    self.field_names.insert((base + fields.owner, name.clone()));
                }
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
            file_type if jvm::is_jvm(file_type) || file_type == Some(FileType::Php) => {
                self.jvm_imports(file, facts, &mut imports)
            }
            Some(FileType::CSharp | FileType::VbNet) => {
                self.dotnet_imports(file, facts, &mut imports)
            }
            file_type if cfamily::is_cfamily(file_type) => {
                self.cfamily_imports(file, facts, &mut imports)
            }
            Some(FileType::Swift) => self.swift_imports(facts, &mut imports),
            Some(FileType::Terraform) => self.terraform_imports(file, facts, &mut imports),
            Some(FileType::Ruby) => self.ruby_imports(file, facts, &mut imports),
            Some(FileType::Dart) => self.dart_imports(file, facts, &mut imports),
            Some(FileType::Elixir) => self.elixir_imports(file, facts, &mut imports),
            Some(FileType::Erlang) => self.erlang_imports(file, facts, &mut imports),
            Some(FileType::Lua) => self.lua_imports(file, facts, &mut imports),
            Some(FileType::Zig) => self.zig_imports(file, facts, &mut imports),
            Some(FileType::Julia) => self.julia_imports(file, facts, &mut imports),
            Some(FileType::R) => self.r_imports(file, facts, &mut imports),
            Some(FileType::OCaml) => self.ocaml_imports(facts, &mut imports),
            Some(FileType::Fortran) => self.fortran_imports(file, facts, &mut imports),
            Some(FileType::Pascal) => self.pascal_imports(file, facts, &mut imports),
            Some(FileType::Lisp) => self.lisp_imports(file, facts, &mut imports),
            Some(FileType::Solidity) => self.solidity_imports(file, facts, &mut imports),
            Some(FileType::Verilog | FileType::PowerShell | FileType::Robot | FileType::Dm) => {
                self.script_imports(file, facts, &mut imports)
            }
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

    /// `require` and `require_relative`: `.rb` files relative to the file, the root and `lib/`;
    /// gems outside the root are external.
    fn ruby_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let spec = import.spec.trim_end_matches(".rb");
            let target = [format!("{spec}.rb"), format!("lib/{spec}.rb")]
                .iter()
                .find_map(|candidate| self.file_by_path(file, candidate));
            match target {
                Some(target) if target != file => imports.files.push((file, target)),
                Some(_) => {}
                None if !spec.starts_with('.') => {
                    imports
                        .externals
                        .push(spec.split('/').next().unwrap_or(spec).to_owned());
                }
                None => {}
            }
        }
    }

    /// Names a language resolves globally: Verilog modules and functions, DM top-level procs
    /// (exact when unique in the root), Robot keywords by normalized name (case, spaces and
    /// underscores ignored) in the file, its resources and its Python libraries.
    fn global_name(&self, file: u32, imports: &Imports, reference: &Ref) -> Option<Outcome> {
        let file_type = self.file_type(file)?;
        if reference.kind != RefKind::Call {
            return None;
        }
        let candidates: Vec<u32> = match file_type {
            FileType::Verilog | FileType::Dm => self
                .by_name
                .get(reference.name.as_str())
                .into_iter()
                .flatten()
                .copied()
                .filter(|&symbol| {
                    let symbol = &self.symbols[symbol as usize];
                    self.file_type(symbol.file) == Some(file_type)
                        && symbol.parent == NONE
                        && (symbol.kind.is_callable() || symbol.kind == SymKind::Class)
                })
                .collect(),
            FileType::Robot => {
                let wanted = robot_key(&reference.name);
                let visible: Vec<u32> = std::iter::once(file)
                    .chain(
                        imports
                            .files
                            .iter()
                            .filter(|(from, _)| *from == file)
                            .map(|(_, to)| *to),
                    )
                    .collect();
                visible
                    .iter()
                    .flat_map(|&visible_file| {
                        let base = self.base[visible_file as usize];
                        let count = self
                            .facts(visible_file)
                            .map_or(0, |facts| facts.symbols.len());
                        (base..base + count as u32).filter(|&symbol| {
                            let symbol = &self.symbols[symbol as usize];
                            symbol.kind.is_callable() && robot_key(&symbol.name) == wanted
                        })
                    })
                    .collect()
            }
            _ => return None,
        };
        match candidates.as_slice() {
            [] if file_type == FileType::Robot => Some(Outcome::Unresolved),
            [] => None,
            [only] => Some(exact(*only)),
            several => Some(pick(several, Confidence::Inferred)),
        }
    }

    /// Solidity imports: relative paths, remappings (`remappings.txt`, `foundry.toml`), then
    /// `lib/` and `node_modules/`; the imported file's contracts become visible.
    fn solidity_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let remappings: Vec<(&str, String)> = self
            .input
            .files
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                let name = record.path.rsplit('/').next().unwrap_or(&record.path);
                matches!(name, "remappings.txt" | "foundry.toml")
            })
            .filter_map(|(index, record)| {
                let manifest = self.facts(index as u32)?.manifest.as_ref()?;
                let dir = modules::parent_dir(&record.path);
                Some(manifest.deps.iter().map(move |(prefix, target)| {
                    let target = if dir.is_empty() {
                        target.clone()
                    } else {
                        format!("{dir}/{target}")
                    };
                    (prefix.as_str(), target)
                }))
            })
            .flatten()
            .collect();
        for import in &facts.imports {
            let spec = import.spec.as_str();
            let remapped = remappings
                .iter()
                .filter(|(prefix, _)| spec.starts_with(prefix))
                .max_by_key(|(prefix, _)| prefix.len())
                .map(|(prefix, target)| format!("{target}{}", &spec[prefix.len()..]));
            let target = remapped
                .and_then(|path| self.path_index.get(normalize_path(&path).as_str()).copied())
                .or_else(|| self.file_by_path(file, spec))
                .or_else(|| {
                    ["lib/", "node_modules/"].iter().find_map(|base| {
                        self.path_index
                            .get(format!("{base}{spec}").as_str())
                            .copied()
                    })
                })
                .filter(|&target| target != file);
            match target {
                Some(target) => {
                    imports.files.push((file, target));
                    let module = self.module_of(target);
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None if !spec.starts_with('.') => {
                    imports
                        .externals
                        .push(spec.split('/').next().unwrap_or(spec).to_owned());
                }
                None => {}
            }
        }
    }

    /// Path imports of Verilog `include`s, PowerShell modules and dot-sourced scripts, Robot
    /// resources and libraries, DM `#include`s: relative to the file, then the root; PowerShell
    /// module names and Verilog packages by name anywhere in the root.
    fn script_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let file_type = self.file_type(file);
        for import in &facts.imports {
            let spec = import
                .spec
                .trim_matches(['"', '\'', '<', '>'])
                .trim_start_matches("$PSScriptRoot/")
                .trim_start_matches("$PSScriptRoot\\")
                .replace('\\', "/");
            let mut target = self.file_by_path(file, &spec);
            if target.is_none() && file_type == Some(FileType::PowerShell) && !spec.contains('/') {
                // `Import-Module Store` -> a `Store.psm1` or `Store.psd1` of the root.
                target = self
                    .input
                    .files
                    .iter()
                    .position(|record| {
                        let name = record.path.rsplit('/').next().unwrap_or(&record.path);
                        name.eq_ignore_ascii_case(&format!("{spec}.psm1"))
                            || name.eq_ignore_ascii_case(&format!("{spec}.psd1"))
                    })
                    .map(|index| index as u32);
            }
            if target.is_none() && file_type == Some(FileType::Verilog) {
                // `import pkg::*`: the file declaring package `pkg`.
                target = self
                    .module_symbols(&spec, FileType::Verilog)
                    .first()
                    .map(|&package| self.symbols[package as usize].file);
            }
            match target.filter(|&target| target != file) {
                Some(target) => {
                    imports.files.push((file, target));
                    let module = self.module_of(target);
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None if !spec.contains('/') && !spec.contains('.') => imports.externals.push(spec),
                None => {}
            }
        }
    }

    /// `use m` makes the procedures of module `m` visible; `include` files likewise.
    fn fortran_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let spec = import.spec.trim_matches(['"', '\'']);
            if spec.contains('.') {
                if let Some(target) = self
                    .file_by_path(file, spec)
                    .filter(|&target| target != file)
                {
                    imports.files.push((file, target));
                    let module = self.module_of(target);
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                continue;
            }
            let modules = self.module_symbols(spec, FileType::Fortran);
            if modules.is_empty() {
                imports.externals.push(spec.to_owned());
                continue;
            }
            for &module in &modules {
                let target = self.symbols[module as usize].file;
                if target != file && !imports.files.contains(&(file, target)) {
                    imports.files.push((file, target));
                }
                let target_module = self.module_of(target);
                imports.globs.push(target_module);
                imports.modules.push(target_module);
            }
        }
    }

    /// `uses A, B`: units of the root become visible (case-insensitive); a form imports its
    /// unit by file name; other units are the runtime library or packages.
    fn pascal_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let unit = import.spec.trim_end_matches(".pas");
            let module = self
                .modules
                .get(&format!("pas:{}", unit.to_ascii_lowercase()));
            match module {
                Some(module) => {
                    if let Some(target) = self.modules.file_module.iter().position(|&m| m == module)
                    {
                        let target = target as u32;
                        if target != file {
                            imports.files.push((file, target));
                        }
                    }
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None => imports.externals.push(unit.to_owned()),
            }
        }
    }

    /// `(:use :x)` of the `defpackage` defining the file's package (in any file) makes package
    /// `x` visible (`:cl` is the language); ASDF `(:file "x")` components import `x.lisp`
    /// relative to the system file.
    fn lisp_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let package = facts.package.as_deref().map(modules::lisp_package);
        let mut uses: Vec<&facts::Import> = Vec::new();
        if let Some(package) = &package {
            for (other, record) in self.input.files.iter().enumerate() {
                if record.file_type != Some(FileType::Lisp) {
                    continue;
                }
                let Some(other_facts) = self.facts(other as u32) else {
                    continue;
                };
                uses.extend(other_facts.imports.iter().filter(|import| {
                    import.alias.as_deref().map(modules::lisp_package).as_ref() == Some(package)
                }));
            }
        }
        for import in facts
            .imports
            .iter()
            .filter(|import| import.alias.is_none())
            .chain(uses)
        {
            let spec = import.spec.trim_matches('"');
            if import.spec.starts_with('"')
                || self.input.files[file as usize].path.ends_with(".asd")
            {
                let path = format!("{spec}.lisp");
                if let Some(target) = self
                    .file_by_path(file, &path)
                    .filter(|&target| target != file)
                {
                    imports.files.push((file, target));
                }
                continue;
            }
            let package = modules::lisp_package(spec);
            if matches!(package.as_str(), "cl" | "common-lisp") {
                continue;
            }
            match self.modules.get(&format!("lisp:{package}")) {
                Some(module) => {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None => imports.externals.push(package),
            }
        }
    }

    /// `module` symbols of a language named `name` (Julia, OCaml nested modules).
    fn module_symbols(&self, name: &str, file_type: FileType) -> Vec<u32> {
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&symbol| {
                let symbol = &self.symbols[symbol as usize];
                symbol.kind == SymKind::Mod && self.file_type(symbol.file) == Some(file_type)
            })
            .collect()
    }

    /// Julia `Util.check`, OCaml `Store.find` and `Repo.find` (file or nested module), R
    /// `pkg::f`: members of the named module; modules outside the root are external.
    fn module_qualified(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        qualifier: &str,
    ) -> Option<Outcome> {
        let file_type = self.file_type(file)?;
        if !matches!(
            file_type,
            FileType::Julia | FileType::OCaml | FileType::R | FileType::Pascal | FileType::Lisp
        ) || imports.names.contains_key(qualifier)
        {
            return None;
        }
        let name = reference.name.as_str();
        let last = qualifier.rsplit('.').next().unwrap_or(qualifier);
        let mut found: Vec<u32> = self
            .module_symbols(last, file_type)
            .iter()
            .flat_map(|&module| {
                self.by_name
                    .get(name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(move |&member| self.symbols[member as usize].parent == module)
            })
            .collect();
        let module_key = match file_type {
            FileType::OCaml => Some(format!("ocaml:{last}")),
            FileType::Pascal => Some(format!("pas:{}", last.to_ascii_lowercase())),
            FileType::Lisp => Some(format!("lisp:{}", modules::lisp_package(qualifier))),
            FileType::R => self
                .r_package_dir(qualifier)
                .map(|dir| format!("dir:{dir}")),
            _ => None,
        };
        if found.is_empty()
            && let Some(module) = module_key.and_then(|key| self.modules.get(&key))
        {
            found = self.module_symbol(module, name, None);
        }
        if found.is_empty() {
            // `List.map`, `Base.println`, `dplyr::filter`: outside the root; a lowercase Julia
            // or OCaml qualifier is a value whose field is called.
            let module_like = match file_type {
                FileType::R | FileType::Lisp => true,
                // `FRows.Load`: a typed receiver, resolved by the member rules.
                FileType::Pascal => false,
                _ => qualifier.starts_with(|ch: char| ch.is_ascii_uppercase()),
            };
            return module_like.then_some(Outcome::External);
        }
        Some(pick_members(
            self,
            found,
            Confidence::Exact,
            reference.arity,
        ))
    }

    /// Source directory (`R/`) of the local R package named `name` (`DESCRIPTION` `Package:`).
    fn r_package_dir(&self, name: &str) -> Option<String> {
        self.input
            .files
            .iter()
            .enumerate()
            .find_map(|(index, record)| {
                let is_description =
                    record.path == "DESCRIPTION" || record.path.ends_with("/DESCRIPTION");
                if !is_description {
                    return None;
                }
                let manifest = self.facts(index as u32)?.manifest.as_ref()?;
                (manifest.module.as_deref() == Some(name)).then(|| {
                    let dir = modules::parent_dir(&record.path);
                    if dir.is_empty() {
                        "R".to_owned()
                    } else {
                        format!("{dir}/R")
                    }
                })
            })
    }

    /// `include("x.jl")` makes the file's names visible; `using`/`import` of a module of the
    /// root makes its names visible and binds its name; other modules are external.
    fn julia_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            if import.spec.ends_with(".jl") {
                if let Some(target) = self
                    .file_by_path(file, &import.spec)
                    .filter(|&target| target != file)
                {
                    imports.files.push((file, target));
                    let module = self.module_of(target);
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                continue;
            }
            let path = import.spec.trim_start_matches('.');
            let last = path.rsplit('.').next().unwrap_or(path);
            let modules = self.module_symbols(last, FileType::Julia);
            if modules.is_empty() {
                imports
                    .externals
                    .push(path.split('.').next().unwrap_or(path).to_owned());
                continue;
            }
            for &module in &modules {
                let target = self.symbols[module as usize].file;
                if target != file && !imports.files.contains(&(file, target)) {
                    imports.files.push((file, target));
                }
                let target_module = self.module_of(target);
                imports.globs.push(target_module);
                imports.modules.push(target_module);
            }
            imports
                .names
                .insert(last.to_owned(), Target::Symbols(modules));
        }
    }

    /// `source("x.R")` relative to the file or the root; `library(pkg)` of a local package
    /// makes its `R/` functions visible, other packages are external.
    fn r_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        // Tests, vignettes and scripts of a package run with its namespace loaded (testthat).
        let path = &self.input.files[file as usize].path;
        let mut dir = modules::parent_dir(path);
        loop {
            let description = if dir.is_empty() {
                "DESCRIPTION".to_owned()
            } else {
                format!("{dir}/DESCRIPTION")
            };
            if self.path_index.contains_key(description.as_str()) {
                let source = if dir.is_empty() {
                    "R".to_owned()
                } else {
                    format!("{dir}/R")
                };
                if let Some(module) = self.modules.get(&format!("dir:{source}"))
                    && module != self.module_of(file)
                {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                break;
            }
            if dir.is_empty() {
                break;
            }
            dir = modules::parent_dir(dir);
        }
        for import in &facts.imports {
            if import.spec.to_ascii_lowercase().ends_with(".r") {
                if let Some(target) = self
                    .file_by_path(file, &import.spec)
                    .filter(|&target| target != file)
                {
                    imports.files.push((file, target));
                    let module = self.module_of(target);
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                continue;
            }
            let local = self
                .r_package_dir(&import.spec)
                .and_then(|dir| self.modules.get(&format!("dir:{dir}")));
            match local {
                Some(module) => {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None => imports.externals.push(import.spec.clone()),
            }
        }
    }

    /// `open M` / `include M`: the file module `M` of the root becomes visible; library modules
    /// are external.
    fn ocaml_imports(&self, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let head = import.spec.split('.').next().unwrap_or(&import.spec);
            match self.modules.get(&format!("ocaml:{head}")) {
                Some(module) => {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None if self.module_symbols(head, FileType::OCaml).is_empty() => {
                    imports.externals.push(head.to_owned());
                }
                None => {}
            }
        }
    }

    /// Elixir modules of the root named `name` (`Store.Repo`). Nested `defmodule`s carry the
    /// enclosing modules' names; inside `file` a nested module is also reachable by its own
    /// name (Elixir's implicit alias).
    fn elixir_modules(&self, name: &str, file: u32) -> Vec<u32> {
        let mut candidates: Vec<u32> = Vec::new();
        let mut suffix = name;
        loop {
            candidates.extend(self.by_name.get(suffix).into_iter().flatten().copied());
            match suffix.split_once('.') {
                Some((_, rest)) => suffix = rest,
                None => break,
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        let modules = |symbol: &u32| {
            let symbol = &self.symbols[*symbol as usize];
            symbol.kind.is_type() && self.file_type(symbol.file) == Some(FileType::Elixir)
        };
        let exact: Vec<u32> = candidates
            .iter()
            .filter(|symbol| modules(symbol) && self.elixir_full_name(**symbol) == name)
            .copied()
            .collect();
        if !exact.is_empty() {
            return exact;
        }
        candidates
            .into_iter()
            .filter(|symbol| {
                modules(symbol)
                    && self.symbols[*symbol as usize].file == file
                    && self.symbols[*symbol as usize].name == name
            })
            .collect()
    }

    /// `Outer.Inner` for a `defmodule Inner` nested in `defmodule Outer`.
    fn elixir_full_name(&self, symbol: u32) -> String {
        let mut name = self.symbols[symbol as usize].name.to_string();
        let mut parent = self.symbols[symbol as usize].parent;
        while parent != NONE {
            let outer = &self.symbols[parent as usize];
            if outer.kind.is_type() {
                name = format!("{}.{name}", outer.name);
            }
            parent = outer.parent;
        }
        name
    }

    /// Elixir `Store.Repo.get` and Erlang `store_repo:get`: the module by its full name; a module
    /// outside the root (`Map`, `GenServer`, `maps`, `gen_server`) is external.
    fn beam_qualified(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        qualifier: &str,
    ) -> Option<Outcome> {
        match self.file_type(file) {
            // `:binary.match`: an Erlang module.
            Some(FileType::Elixir) if qualifier.starts_with(':') => Some(Outcome::External),
            Some(FileType::Elixir) if qualifier.starts_with(|ch: char| ch.is_ascii_uppercase()) => {
                if imports.names.contains_key(qualifier) {
                    return None;
                }
                let modules = self.elixir_modules(qualifier, file);
                if modules.is_empty() {
                    return Some(Outcome::External);
                }
                let members: Vec<u32> = modules
                    .iter()
                    .flat_map(|&m| self.members(m, &reference.name))
                    .collect();
                Some(pick_members(
                    self,
                    members,
                    Confidence::Exact,
                    reference.arity,
                ))
            }
            Some(FileType::Erlang) if qualifier.starts_with(|ch: char| ch.is_ascii_lowercase()) => {
                let Some(module) = self.modules.get(&format!("erl:{qualifier}")) else {
                    return Some(Outcome::External);
                };
                let found = self.module_symbol(module, &reference.name, None);
                Some(pick_members(
                    self,
                    found,
                    Confidence::Exact,
                    reference.arity,
                ))
            }
            _ => None,
        }
    }

    /// `alias A.B` binds `B`; `import A.B` and `use A.B` make the module's functions visible.
    /// Modules outside the root are external.
    fn elixir_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let spec = import.spec.as_str();
            let modules = self.elixir_modules(spec, file);
            if modules.is_empty() {
                imports
                    .externals
                    .push(spec.split('.').next().unwrap_or(spec).to_owned());
                continue;
            }
            for &module in &modules {
                let target = self.symbols[module as usize].file;
                if target != file && !imports.files.contains(&(file, target)) {
                    imports.files.push((file, target));
                }
                imports.modules.push(self.module_of(target));
            }
            if import.alias.is_some() {
                let local = spec.rsplit('.').next().unwrap_or(spec).to_owned();
                imports.names.insert(local, Target::Symbols(modules));
            } else {
                imports.globs.extend(
                    modules
                        .iter()
                        .map(|&module| self.module_of(self.symbols[module as usize].file)),
                );
            }
        }
    }

    /// `-include("x.hrl")` (relative, `include/`), `-import(m, [...])`: `m`'s functions visible.
    fn erlang_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let spec = import.spec.as_str();
            if spec.ends_with(".hrl") || spec.ends_with(".erl") {
                let name = spec.trim_start_matches("./");
                let target = [
                    name.to_owned(),
                    format!("../include/{name}"),
                    format!("include/{name}"),
                ]
                .iter()
                .find_map(|candidate| self.file_by_path(file, candidate));
                if let Some(target) = target.filter(|&target| target != file) {
                    imports.files.push((file, target));
                }
                continue;
            }
            match self.modules.get(&format!("erl:{spec}")) {
                Some(module) => {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None => imports.externals.push(spec.to_owned()),
            }
        }
    }

    /// `local m = require("a.b")`: `a/b.lua` or `a/b/init.lua` under the root, `lua/` or `src/`;
    /// the binding names the module. Other modules are external rocks.
    fn lua_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let (members, modules): (Vec<&facts::Import>, Vec<&facts::Import>) = facts
            .imports
            .iter()
            .partition(|import| !import.names.is_empty());
        for import in modules {
            // `.toc` file lists name files relative to the manifest.
            if import.spec.ends_with(".lua") || import.spec.ends_with(".xml") {
                if let Some(target) = self.file_by_path(file, &import.spec) {
                    imports.files.push((file, target));
                }
                continue;
            }
            let path = import.spec.replace('.', "/");
            let target = ["", "lua/", "src/", "lib/"].iter().find_map(|base| {
                [
                    format!("{base}{path}.lua"),
                    format!("{base}{path}/init.lua"),
                ]
                .iter()
                .find_map(|candidate| self.path_index.get(candidate.as_str()).copied())
            });
            let Some(target) = target.filter(|&target| target != file) else {
                let head = import
                    .spec
                    .split('.')
                    .next()
                    .unwrap_or(&import.spec)
                    .to_owned();
                if let Some(alias) = &import.alias {
                    imports
                        .names
                        .insert(alias.clone(), Target::External(head.clone()));
                }
                imports.externals.push(head);
                continue;
            };
            imports.files.push((file, target));
            let module = self.module_of(target);
            imports.modules.push(module);
            if let Some(alias) = &import.alias {
                imports.names.insert(alias.clone(), Target::Module(module));
            }
        }
        // Members of required modules re-bound to locals; other tables are no imports.
        for import in members {
            let (Some(alias), Some(member)) = (&import.alias, import.names.first()) else {
                continue;
            };
            match imports.names.get(&import.spec) {
                Some(Target::Module(module)) => {
                    let found = self.module_symbol(*module, member, None);
                    if !found.is_empty() {
                        imports.names.insert(alias.clone(), Target::Symbols(found));
                    }
                }
                Some(Target::External(name)) => {
                    let name = name.clone();
                    imports.names.insert(alias.clone(), Target::External(name));
                }
                _ => {}
            }
        }
    }

    /// Root file of a named module `b.addModule("name", ...)` in a `build.zig` of the root; the
    /// first definition in path order wins.
    fn zig_named_module(&self, name: &str) -> Option<u32> {
        self.input
            .files
            .iter()
            .enumerate()
            .find_map(|(index, record)| {
                if !(record.path == "build.zig" || record.path.ends_with("/build.zig")) {
                    return None;
                }
                let import = self
                    .facts(index as u32)?
                    .imports
                    .iter()
                    .find(|import| import.reexport && import.alias.as_deref() == Some(name))?;
                self.file_by_path(index as u32, &import.spec)
            })
    }

    /// `const x = @import("x.zig")`: a file relative to the importer binds `x`; named modules of
    /// `build.zig` resolve to their root file; `std`, `builtin` and packages are external.
    fn zig_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        // Whole modules first, then declarations re-binding one of their members.
        let (members, modules): (Vec<&facts::Import>, Vec<&facts::Import>) = facts
            .imports
            .iter()
            .filter(|import| !import.reexport)
            .partition(|import| !import.names.is_empty());
        for import in modules.into_iter().chain(members) {
            let target = if let Some(bound) = imports
                .names
                .get(&import.spec)
                .filter(|_| !import.names.is_empty())
            {
                bound.clone()
            } else {
                let file_target = if import.spec.ends_with(".zig") {
                    self.file_by_path(file, &import.spec)
                } else if matches!(import.spec.as_str(), "std" | "builtin" | "root") {
                    None
                } else {
                    self.zig_named_module(&import.spec)
                }
                .filter(|&target| target != file);
                match file_target {
                    Some(target) => {
                        imports.files.push((file, target));
                        let module = self.module_of(target);
                        imports.modules.push(module);
                        Target::Module(module)
                    }
                    // `const x = local.y;` of a non-import: not an import.
                    None if !import.names.is_empty()
                        && !import.spec.ends_with(".zig")
                        && !matches!(import.spec.as_str(), "std" | "builtin" | "root") =>
                    {
                        continue;
                    }
                    None => {
                        if !matches!(import.spec.as_str(), "std" | "builtin" | "root") {
                            imports.externals.push(import.spec.clone());
                        }
                        Target::External(import.spec.clone())
                    }
                }
            };
            let target = match (&target, import.names.first()) {
                (Target::Module(module), Some(member)) => {
                    let found = self.module_symbol(*module, member, None);
                    if found.is_empty() {
                        continue;
                    }
                    Target::Symbols(found)
                }
                (Target::Symbols(types), Some(member)) => {
                    let found: Vec<u32> = types
                        .iter()
                        .flat_map(|&t| self.members(t, member))
                        .collect();
                    if found.is_empty() {
                        continue;
                    }
                    Target::Symbols(found)
                }
                _ => target,
            };
            if let Some(alias) = &import.alias {
                imports.names.insert(alias.clone(), target);
            }
        }
    }

    /// `import 'package:app/src/x.dart'` (the package's `lib/`), relative imports; imported files'
    /// names become visible, with the files they `export` and their `part`s. A `part of` file
    /// sees its library and the library's imports. `dart:` libraries are the platform, other
    /// packages external.
    fn dart_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let mut visited = vec![file];
        for import in &facts.imports {
            let Some(target) = self.dart_target(file, &import.spec, imports) else {
                continue;
            };
            imports.files.push((file, target));
            self.dart_visible(target, imports, &mut visited);
            // `part of`: the library's own imports apply to the part.
            let library = self.facts(target);
            let is_part_of = library.is_some_and(|library| {
                library.imports.iter().any(|own| {
                    own.reexport
                        && self.dart_target(target, &own.spec, &mut Imports::default())
                            == Some(file)
                })
            });
            if is_part_of && let Some(library) = library {
                for own in library.imports.iter().filter(|own| !own.reexport) {
                    if let Some(other) = self.dart_target(target, &own.spec, imports) {
                        self.dart_visible(other, imports, &mut visited);
                    }
                }
            }
        }
    }

    /// A library's names and, transitively, those it re-exports become visible.
    fn dart_visible(&self, target: u32, imports: &mut Imports, visited: &mut Vec<u32>) {
        if visited.contains(&target) || visited.len() > 64 {
            return;
        }
        visited.push(target);
        let module = self.module_of(target);
        imports.globs.push(module);
        imports.modules.push(module);
        let Some(facts) = self.facts(target) else {
            return;
        };
        for import in facts.imports.iter().filter(|import| import.reexport) {
            if let Some(next) = self.dart_target(target, &import.spec, &mut Imports::default()) {
                self.dart_visible(next, imports, visited);
            }
        }
    }

    /// File of a Dart URI relative to `file`; unknown packages are recorded as external.
    fn dart_target(&self, file: u32, spec: &str, imports: &mut Imports) -> Option<u32> {
        if spec.starts_with("dart:") {
            return None;
        }
        let target = match spec.strip_prefix("package:") {
            Some(rest) => {
                let (package, path) = rest.split_once('/').unwrap_or((rest, ""));
                // The package root from the `pubspec.yaml` naming it, else a unique `lib/` match.
                let named = self
                    .input
                    .files
                    .iter()
                    .enumerate()
                    .find_map(|(index, record)| {
                        let is_pubspec =
                            record.path == "pubspec.yaml" || record.path.ends_with("/pubspec.yaml");
                        let manifest = is_pubspec
                            .then(|| self.facts(index as u32)?.manifest.as_ref())
                            .flatten()?;
                        (manifest.module.as_deref() == Some(package)).then(|| {
                            let dir = modules::parent_dir(&record.path);
                            if dir.is_empty() {
                                format!("lib/{path}")
                            } else {
                                format!("{dir}/lib/{path}")
                            }
                        })
                    });
                if let Some(found) =
                    named.and_then(|named| self.path_index.get(named.as_str()).copied())
                {
                    return Some(found).filter(|&found| found != file);
                }
                let wanted = format!("lib/{path}");
                let found: Vec<u32> = self
                    .input
                    .files
                    .iter()
                    .enumerate()
                    .filter(|(_, record)| {
                        record.path == wanted || record.path.ends_with(&format!("/{wanted}"))
                    })
                    .map(|(index, _)| index as u32)
                    .collect();
                match found.as_slice() {
                    [only] => Some(*only),
                    _ => {
                        imports.externals.push(package.to_owned());
                        None
                    }
                }
            }
            None => self.file_by_path(file, spec),
        };
        target.filter(|&target| target != file)
    }

    /// Local `module` sources: the module directory's files are imported.
    fn terraform_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let dir = modules::parent_dir(&self.input.files[file as usize].path);
        for import in &facts.imports {
            let target = normalize_path(&if dir.is_empty() {
                import.spec.clone()
            } else {
                format!("{dir}/{}", import.spec)
            });
            let Some(module) = self.modules.get(&format!("dir:{target}")) else {
                continue;
            };
            imports.modules.push(module);
            for (other, &other_module) in self.modules.file_module.iter().enumerate() {
                if other_module == module && other as u32 != file {
                    imports.files.push((file, other as u32));
                }
            }
        }
    }

    /// `import Module`: a SwiftPM target of the root becomes visible; platform modules are
    /// dropped, other modules are external dependencies.
    fn swift_imports(&self, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            match self.modules.get(&format!("swift:{}", import.spec)) {
                Some(module) => {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None if !matches!(
                    import.spec.as_str(),
                    "Foundation"
                        | "UIKit"
                        | "AppKit"
                        | "SwiftUI"
                        | "Combine"
                        | "XCTest"
                        | "Testing"
                        | "os"
                        | "Darwin"
                        | "Dispatch"
                        | "CoreData"
                        | "CoreGraphics"
                        | "Observation"
                        | "PackageDescription"
                ) =>
                {
                    imports.externals.push(import.spec.clone());
                }
                None => {}
            }
        }
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
        let mut found = self.members_direct(type_symbol, name);
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
        if symbol.kind == SymKind::Mod {
            // Module blocks (Julia, OCaml): every symbol declared in the body.
            return self
                .by_name
                .get(name)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&member| self.symbols[member as usize].parent == type_symbol)
                .collect();
        }
        if jvm::is_jvm(self.file_type(symbol.file)) {
            return self.declared_members(type_symbol, name);
        }
        if jvm::is_dotnet(self.file_type(symbol.file)) {
            // Partial types: every part of the same name and nesting contributes members.
            let module = self.module_of(symbol.file);
            let parts = self
                .types
                .get(&(module, symbol.name.as_str()))
                .cloned()
                .unwrap_or_default();
            let mut found: Vec<u32> = parts
                .iter()
                .filter(|&&part| self.symbols[part as usize].parent == NONE || part == type_symbol)
                .flat_map(|&part| self.declared_members(part, name))
                .collect();
            if found.is_empty() {
                found = self.declared_members(type_symbol, name);
            }
            return found;
        }
        let scope = self.scope_ids[self.module_of(symbol.file) as usize];
        self.methods
            .get(&(scope, symbol.name.as_str(), name))
            .cloned()
            .unwrap_or_default()
    }

    /// Members `name` declared in the body of `type_symbol`: JVM types of one package may share a
    /// simple name (nested `Builder` classes), so the owner name alone is ambiguous.
    fn declared_members(&self, type_symbol: u32, name: &str) -> Vec<u32> {
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&member| {
                let symbol = &self.symbols[member as usize];
                symbol.parent == type_symbol
                    && (symbol.kind.is_callable() || symbol.kind == SymKind::Const)
            })
            .collect()
    }

    /// Resolves a type name as written in `file` (`T`, `pkg.T`, `a::b::T`, `Self`).
    fn resolve_type(
        &self,
        file: u32,
        imports: Option<&Imports>,
        scope: Option<u32>,
        name: &str,
    ) -> Option<(Vec<u32>, Confidence)> {
        // Forward declarations (Pascal `TCore = class;`, C `struct s;` prototypes) stand for the
        // definition next to them.
        self.resolve_type_candidates(file, imports, scope, name)
            .map(|(mut types, confidence)| {
                let declared = |s: &u32| self.symbols[*s as usize].flags & FLAG_DECLARATION != 0;
                if types.iter().any(declared) && !types.iter().all(declared) {
                    types.retain(|s| !declared(s));
                }
                (types, confidence)
            })
    }

    fn resolve_type_candidates(
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
            let found = self
                .qualified_symbols(file, imports, qualifier, last)
                .unwrap_or_default();
            let mut types: Vec<u32> = found
                .into_iter()
                .filter(|&s| self.symbols[s as usize].kind.is_type())
                .collect();
            if types.is_empty() {
                // Nested type `Outer.Inner`.
                types = self.nested_types(file, imports, scope, qualifier, last);
            }
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
        // JVM types come only from the package, imports and `java.lang`: no root-wide guess.
        if jvm::is_class_based(self.file_type(file)) {
            return None;
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

    /// Types `name` declared inside the type `outer`.
    fn nested_types(
        &self,
        file: u32,
        imports: Option<&Imports>,
        scope: Option<u32>,
        outer: &str,
        name: &str,
    ) -> Vec<u32> {
        let Some((outers, _)) = self.resolve_type(file, imports, scope, outer) else {
            return Vec::new();
        };
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&s| {
                let symbol = &self.symbols[s as usize];
                symbol.kind.is_type() && outers.contains(&symbol.parent)
            })
            .collect()
    }

    /// Names of the namespaces enclosing a symbol, outermost first.
    fn namespaces(&self, symbol: u32) -> Vec<&str> {
        let mut names = Vec::new();
        let mut current = self.symbols[symbol as usize].parent;
        while current != NONE {
            let enclosing = &self.symbols[current as usize];
            if enclosing.kind == SymKind::Mod {
                names.push(enclosing.name.as_str());
            }
            current = enclosing.parent;
        }
        names.reverse();
        names
    }

    /// Every part of the Objective-C classes in `types`: the `@interface`, class extensions,
    /// categories and the `@implementation` are separate symbols of one class.
    fn objc_class_parts(&self, types: &[u32]) -> Vec<u32> {
        let mut parts: Vec<u32> = types.to_vec();
        for &type_symbol in types {
            let name = self.symbols[type_symbol as usize].name.as_str();
            parts.extend(
                self.by_name
                    .get(name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&part| {
                        let symbol = &self.symbols[part as usize];
                        matches!(symbol.kind, SymKind::Class | SymKind::Iface)
                            && self.file_type(symbol.file) == Some(FileType::ObjC)
                    }),
            );
        }
        parts.sort_unstable();
        parts.dedup();
        parts
    }

    /// Owner type of the method enclosing `scope`.
    fn owner_type(&self, file: u32, scope: Option<u32>) -> Option<Vec<u32>> {
        let facts = self.facts(file)?;
        let mut current = scope;
        while let Some(local) = current {
            let symbol = &facts.symbols[local as usize];
            // The enclosing type itself when it carries the owner name (Elixir `Store.Repo`
            // would otherwise resolve as a qualified name).
            if let Some(parent) = symbol.parent
                && facts.symbols[parent as usize].kind().is_type()
                && facts.symbols[parent as usize].name == symbol.owner
            {
                return Some(vec![self.base[file as usize] + parent]);
            }
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
        if matches!(self.file_type(file), Some(FileType::Cpp | FileType::Ruby))
            && let Some(found) = self.cpp_qualified(qualifier, name)
        {
            return Some(found);
        }
        if self.file_type(file) == Some(FileType::Python)
            && let Some(found) =
                imports.and_then(|imports| self.python_qualified(imports, qualifier, name))
        {
            return Some(found);
        }
        if jvm::is_class_based(self.file_type(file))
            && imports.is_none_or(|imports| !imports.names.contains_key(qualifier))
            && let Some(module) = self
                .modules
                .get(&jvm::namespace_key(self.file_type(file), qualifier))
        {
            // A package of the root (`com.example.Type`, Scala `os.read`).
            let found = self.module_symbol(module, name, None);
            return (!found.is_empty()).then_some(found);
        }
        if jvm::is_class_based(self.file_type(file)) && qualifier.contains('.') {
            // Fully qualified name outside the root.
            let head = qualifier.split('.').next().unwrap_or_default();
            if head.starts_with(|ch: char| ch.is_ascii_lowercase())
                && imports.is_none_or(|imports| !imports.names.contains_key(head))
                && self
                    .facts(file)
                    .is_some_and(|facts| !facts.bindings.iter().any(|binding| binding.name == head))
            {
                return None;
            }
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
            ambiguous_names: Vec::new(),
            stale: Vec::new(),
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
                // A value reference only counts when it certainly names a symbol: no fallback,
                // no ambiguity count (most identifiers are locals).
                RefKind::Value => match self.resolve_value(file, imports, reference) {
                    Outcome::Targets(targets, confidence) => Outcome::Targets(targets, confidence),
                    Outcome::External => Outcome::External,
                    // Never counted as ambiguity (most value identifiers are locals), but the
                    // name stays uncertain for dead-code checks.
                    Outcome::Ambiguous | Outcome::Unresolved => Outcome::Unresolved,
                },
                RefKind::Inherit => self.resolve_supertype(file, imports, reference),
                RefKind::Path => self.resolve_path(file, &reference.name),
                RefKind::Mention => self.resolve_mention(file, &reference.name),
                RefKind::Run => self.resolve_run(file, reference),
                RefKind::Query => self.resolve_table(file, &reference.name),
                _ => Outcome::Unresolved,
            };
            match outcome {
                Outcome::Targets(targets, confidence) => {
                    let source_is_test =
                        file_is_test || (source < file_node && self.is_test(source));
                    let mut kind = match reference.kind {
                        RefKind::Call => EdgeKind::Calls,
                        RefKind::Value => EdgeKind::Uses,
                        RefKind::Inherit => EdgeKind::Inherits,
                        RefKind::Path => EdgeKind::References,
                        RefKind::Mention => EdgeKind::Mentions,
                        RefKind::Run => EdgeKind::Runs,
                        RefKind::Query => EdgeKind::Queries,
                        _ => EdgeKind::Uses,
                    };
                    if kind == EdgeKind::Calls && source_is_test {
                        kind = EdgeKind::Tests;
                    }
                    // C-family declarations stand for their definitions.
                    let targets: Vec<(u32, Confidence)> = targets
                        .into_iter()
                        .map(|(target, target_confidence)| {
                            (
                                self.definition_of(target).unwrap_or(target),
                                target_confidence,
                            )
                        })
                        .collect();
                    for (target, target_confidence) in targets {
                        if target == source || self.is_trivial(source, target, kind) {
                            continue;
                        }
                        // One base list names the superclass and the interfaces (Pascal
                        // `class(TBase, IRoute)`, Kotlin `: Base(), Route`): a class
                        // naming an interface implements it.
                        let kind = if kind == EdgeKind::Inherits
                            && source < file_node
                            && !matches!(self.symbols[source as usize].kind, SymKind::Iface)
                            && self.symbols[target as usize].kind == SymKind::Iface
                        {
                            EdgeKind::Implements
                        } else if kind == EdgeKind::Uses
                            && reference.kind == RefKind::Value
                            && self.file_type(file) == Some(FileType::Pascal)
                            && self.symbols[target as usize].kind.is_callable()
                        {
                            // Pascal calls parameterless routines without parentheses.
                            if source_is_test {
                                EdgeKind::Tests
                            } else {
                                EdgeKind::Calls
                            }
                        } else {
                            kind
                        };
                        // Julia `Base.length(q::Queue)` extends a generic function: a call of
                        // `length` reaches it only for arguments of that type.
                        let dispatch = self.file_type(file) == Some(FileType::Julia)
                            && reference.qualifier.is_none()
                            && self.symbols[target as usize]
                                .signature
                                .contains(&format!("Base.{}(", reference.name));
                        // C++: an unqualified name reaches a namespace that encloses the
                        // reference; another namespace needs a `using` the graph does not track.
                        let foreign_namespace = self.file_type(file) == Some(FileType::Cpp)
                            && reference.qualifier.is_none()
                            && source < file_node
                            && !self
                                .namespaces(source)
                                .starts_with(&self.namespaces(target));
                        let target_confidence = if dispatch || foreign_namespace {
                            Confidence::Inferred
                        } else {
                            target_confidence
                        };
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
                Outcome::Ambiguous => {
                    resolved.ambiguous += 1;
                    resolved.ambiguous_names.push(reference.name.clone());
                }
                Outcome::Unresolved => {
                    // Literal paths in code may name generated or remote files; only documents
                    // promise that their references exist.
                    let from_doc = self.input.files[file as usize].role == Role::Doc;
                    if (from_doc || reference.kind != RefKind::Path) && self.is_stale(reference) {
                        resolved
                            .stale
                            .push((source, reference.line, reference.name.clone()));
                    }
                    // An unresolved reference to a name some symbol carries may target it.
                    if matches!(reference.kind, RefKind::Call | RefKind::Value)
                        && self.by_name.contains_key(reference.name.as_str())
                    {
                        resolved.ambiguous_names.push(reference.name.clone());
                    }
                }
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
    /// Contract `implements` edges (inferred): code functions and methods whose normalized name
    /// equals an OpenAPI `operationId`, a Protobuf rpc or a GraphQL root field (`Query`,
    /// `Mutation`, `Subscription`; also `resolve<Field>`). Several matches narrow to handlers whose
    /// owner names the service; otherwise no edge.
    fn contract_edges(&self) -> Vec<Edge> {
        let normalize = |name: &str| name.to_ascii_lowercase().replace(['_', '-'], "");
        let contract_file = |file: u32| {
            matches!(
                self.file_type(file),
                Some(FileType::Proto | FileType::GraphQl)
            )
        };
        let mut handlers: HashMap<String, Vec<u32>> = HashMap::default();
        for (id, symbol) in self.symbols.iter().enumerate() {
            let code = self
                .file_type(symbol.file)
                .is_some_and(|file_type| file_type.is_code())
                && self.file_type(symbol.file) != Some(FileType::Sql)
                && self.input.files[symbol.file as usize].role == Role::Src;
            if code && matches!(symbol.kind, SymKind::Fn | SymKind::Method) {
                let key = normalize(&symbol.name);
                let key = key
                    .strip_prefix("resolve")
                    .filter(|rest| !rest.is_empty())
                    .map_or(key.clone(), str::to_owned);
                handlers.entry(key).or_default().push(id as u32);
            }
        }
        let mut edges = Vec::new();
        for (id, contract) in self.symbols.iter().enumerate() {
            let parent = (contract.parent != NONE).then(|| &self.symbols[contract.parent as usize]);
            let service = match (contract.kind, self.file_type(contract.file)) {
                (SymKind::Op, _) if !contract.name.contains(' ') => String::new(),
                (SymKind::Method, Some(FileType::Proto)) => parent
                    .map(|service| service.name.clone())
                    .unwrap_or_default(),
                (SymKind::Method, Some(FileType::GraphQl))
                    if parent.is_some_and(|root| {
                        matches!(root.name.as_str(), "Query" | "Mutation" | "Subscription")
                    }) =>
                {
                    String::new()
                }
                _ => continue,
            };
            let Some(candidates) = handlers.get(&normalize(&contract.name)) else {
                continue;
            };
            let candidates: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&handler| !contract_file(self.symbols[handler as usize].file))
                .collect();
            let chosen: Vec<u32> = if candidates.len() == 1 {
                candidates
            } else if !service.is_empty() {
                let wanted = normalize(&service);
                let owned: Vec<u32> = candidates
                    .into_iter()
                    .filter(|&handler| normalize(&self.owner_name(handler)).contains(&wanted))
                    .collect();
                if owned.len() == 1 { owned } else { Vec::new() }
            } else {
                Vec::new()
            };
            for handler in chosen {
                edges.push(Edge {
                    source: handler,
                    target: id as u32,
                    kind: EdgeKind::Implements,
                    confidence: Confidence::Inferred,
                    line: self.symbols[handler as usize].line,
                });
            }
        }
        edges
    }

    /// What a task runs: a script path (a directory means its `main.go`, `main.rs`, `__main__.py`
    /// or `index` file), a task of the same file (`needs`, prerequisites, `depends_on`), or a
    /// Makefile target (`make t`) or package script (`npm run t`) elsewhere, nearest first.
    fn resolve_run(&self, file: u32, reference: &Ref) -> Outcome {
        let name = reference.name.as_str();
        let task_kind = |symbol: u32| {
            matches!(
                self.symbols[symbol as usize].kind,
                SymKind::Task | SymKind::Svc
            )
        };
        match reference.qualifier.as_deref() {
            Some(runner @ ("make" | "npm")) => {
                let manifest = if runner == "make" {
                    "Makefile"
                } else {
                    "package.json"
                };
                let candidates: Vec<u32> = self
                    .by_name
                    .get(name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&symbol| {
                        task_kind(symbol)
                            && self.input.files[self.symbols[symbol as usize].file as usize]
                                .path
                                .rsplit('/')
                                .next()
                                .is_some_and(|file_name| {
                                    file_name == manifest
                                        || (runner == "make" && file_name.ends_with(".mk"))
                                })
                    })
                    .collect();
                let dir = modules::parent_dir(&self.input.files[file as usize].path);
                let nearest = candidates.iter().copied().max_by_key(|&symbol| {
                    let other = modules::parent_dir(
                        &self.input.files[self.symbols[symbol as usize].file as usize].path,
                    );
                    let shared = dir
                        .split('/')
                        .zip(other.split('/'))
                        .take_while(|(a, b)| a == b)
                        .count();
                    (shared, std::cmp::Reverse(other.len()))
                });
                match nearest {
                    Some(symbol) if candidates.len() == 1 => exact(symbol),
                    Some(symbol) => {
                        Outcome::Targets(vec![(symbol, Confidence::Inferred)], Confidence::Inferred)
                    }
                    None => Outcome::External,
                }
            }
            _ if name.contains('/') || name.contains('.') => {
                let path = name.trim_start_matches("./");
                for candidate in [
                    path.to_owned(),
                    format!("{path}/main.go"),
                    format!("{path}/main.rs"),
                    format!("{path}/__main__.py"),
                    format!("{path}/index.ts"),
                    format!("{path}/index.js"),
                ] {
                    if let Some(found) = self.file_by_path(file, &candidate)
                        && found != file
                    {
                        return exact(self.symbols.len() as u32 + found);
                    }
                }
                Outcome::Unresolved
            }
            _ => {
                let base = self.base[file as usize];
                let local: Vec<u32> = self
                    .facts(file)
                    .map(|facts| {
                        facts
                            .symbols
                            .iter()
                            .enumerate()
                            .filter(|(_, symbol)| symbol.name == name)
                            .map(|(position, _)| base + position as u32)
                            .filter(|&symbol| task_kind(symbol))
                            .collect()
                    })
                    .unwrap_or_default();
                pick(&local, Confidence::Exact)
            }
        }
    }

    /// An SQL table or view named in a query: the definitions nearest to the referring file
    /// (longest shared directory prefix); one nearest is exact, several (migrations recreating a
    /// table) are inferred.
    fn resolve_table(&self, file: u32, name: &str) -> Outcome {
        let Some(tables) = self.tables.get(&name.to_ascii_lowercase()) else {
            return Outcome::External;
        };
        let dir = modules::parent_dir(&self.input.files[file as usize].path);
        let shared = |table: u32| {
            let other = modules::parent_dir(
                &self.input.files[self.symbols[table as usize].file as usize].path,
            );
            dir.split('/')
                .zip(other.split('/'))
                .take_while(|(a, b)| a == b)
                .count()
        };
        let best = tables
            .iter()
            .map(|&table| shared(table))
            .max()
            .unwrap_or_default();
        let nearest: Vec<u32> = tables
            .iter()
            .copied()
            .filter(|&table| shared(table) == best)
            .collect();
        match nearest.as_slice() {
            [only] => exact(*only),
            several => Outcome::Targets(
                several
                    .iter()
                    .take(MAX_DISPATCH)
                    .map(|&table| (table, Confidence::Inferred))
                    .collect(),
                Confidence::Inferred,
            ),
        }
    }

    fn resolve_path(&self, file: u32, target: &str) -> Outcome {
        let target = strip_line_suffix(target);
        let found = self.file_by_path(file, target).or_else(|| {
            // Web roots (`/img/a.png` served from `public/`) and the `@/` source alias.
            let (bases, rest): (&[&str], &str) = if let Some(rest) = target.strip_prefix("@/") {
                (&["src/"], rest)
            } else {
                (
                    &["public/", "static/", "assets/", ""],
                    target.strip_prefix('/')?,
                )
            };
            bases.iter().find_map(|base| {
                self.path_index
                    .get(format!("{base}{rest}").as_str())
                    .copied()
            })
        });
        if let Some(found) = found.filter(|&found| found != file) {
            return exact(self.symbols.len() as u32 + found);
        }
        // Structural languages name copybooks, partials and includes by file name from search
        // paths the tree does not declare: a unique file of that name, inferred.
        let structural = self.file_type(file).is_some_and(|file_type| {
            file_type.class() == crate::domain::filetype::Class::Structural
        });
        if structural && !target.contains("://") {
            let name = target.rsplit('/').next().unwrap_or(target);
            // COBOL copybooks differ in case between `COPY` and the file name.
            let mut matches: Vec<u32> = [name.to_owned(), name.to_lowercase(), name.to_uppercase()]
                .iter()
                .flat_map(|candidate| {
                    self.name_index
                        .get(candidate.as_str())
                        .into_iter()
                        .flatten()
                })
                .copied()
                .collect();
            matches.sort_unstable();
            matches.dedup();
            if let [only] = matches.as_slice()
                && *only != file
            {
                let node = self.symbols.len() as u32 + only;
                return Outcome::Targets(vec![(node, Confidence::Inferred)], Confidence::Inferred);
            }
        }
        Outcome::Unresolved
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
        let mention = strip_line_suffix(mention);
        // A qualified name of the index (`internal/platform.Worker`, `store::Store::open`).
        let normalized = mention.replace("::", ".");
        if normalized.contains('.')
            && let Some(name) = normalized.rsplit('.').next()
        {
            let exact: Vec<u32> = self
                .by_name
                .get(name)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&symbol| {
                    self.symbols[symbol as usize].qualified.replace("::", ".") == normalized
                })
                .collect();
            if let [only] = exact.as_slice() {
                return Outcome::Targets(vec![(*only, Confidence::Exact)], Confidence::Exact);
            }
        }
        // `path:name` and `path::name`: a symbol of that file.
        for separator in ["::", ":"] {
            if let Some((path, name)) = mention.rsplit_once(separator)
                && path.contains('.')
                && let Some(target_file) = self.file_by_path(file, path)
            {
                let record = &self.input.facts[target_file as usize];
                let base = self.base[target_file as usize];
                let found = record.as_ref().and_then(|facts| {
                    facts
                        .symbols
                        .iter()
                        .position(|symbol| symbol.name == name)
                        .map(|local| base + local as u32)
                });
                return match found {
                    Some(symbol) => {
                        Outcome::Targets(vec![(symbol, Confidence::Exact)], Confidence::Exact)
                    }
                    None => Outcome::Targets(
                        vec![(self.symbols.len() as u32 + target_file, Confidence::Exact)],
                        Confidence::Exact,
                    ),
                };
            }
        }
        if mention.contains('/') {
            return self.resolve_path(file, mention);
        }
        // A file name (`go.mod`, `cli.rs`): the unique file carrying it.
        let named_files: Vec<u32> = self.name_index.get(mention).cloned().unwrap_or_default();
        if mention.contains('.') && !named_files.is_empty() {
            return match named_files.as_slice() {
                [only] if *only != file => Outcome::Targets(
                    vec![(self.symbols.len() as u32 + *only, Confidence::Inferred)],
                    Confidence::Inferred,
                ),
                [_] => Outcome::External,
                _ => Outcome::External,
            };
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
                    || graph::has_qualified_suffix(&qualified, &normalized)
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

    /// A doc reference that certainly points at nothing: a link or path mention with an
    /// extension or directory that matches no file, or a qualified mention whose head names a
    /// symbol or module of the root while the whole name matches nothing.
    fn is_stale(&self, reference: &Ref) -> bool {
        let name = strip_line_suffix(&reference.name);
        // URLs, absolute or home paths, globs and ranges are no repository references.
        if name.contains("://")
            || name.starts_with(['/', '~', '$'])
            || name.contains("..")
            || name.contains(['*', '<', '>', '{', ' '])
        {
            return false;
        }
        match reference.kind {
            RefKind::Path => true,
            RefKind::Mention if name.contains('/') => name
                .rsplit('/')
                .next()
                .is_some_and(|last| last.contains('.')),
            RefKind::Mention => {
                let normalized = name.replace("::", ".");
                let Some((head, last)) = normalized.split_once('.') else {
                    return false;
                };
                // `go.mod`, `build.gradle`: a file name, not a member.
                if last.len() <= 6
                    && last
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
                {
                    return false;
                }
                // Stale only when the head names a type of the root that has neither a member
                // nor a field of that name.
                let member = last.split('.').next().unwrap_or(last);
                let types: Vec<u32> = self
                    .by_name
                    .get(head)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&symbol| self.symbols[symbol as usize].kind.is_type())
                    .collect();
                !types.is_empty()
                    && types.iter().all(|&type_symbol| {
                        self.members(type_symbol, member).is_empty()
                            && !self.field_types.contains_key(&(type_symbol, member))
                            && !self.field_names.contains(&(type_symbol, member.to_owned()))
                    })
            }
            _ => false,
        }
    }

    /// Same-kind symbols in different files whose names carry a build constraint suffix
    /// (`_darwin`, `_linux`, `_windows`, `_unix`, `_other`, `_js`, `_wasm`, architectures).
    fn build_variants(&self, symbols: &[u32]) -> bool {
        let constrained = |symbol: u32| {
            let path = &self.input.files[self.symbols[symbol as usize].file as usize].path;
            let stem = path
                .rsplit('/')
                .next()
                .unwrap_or(path)
                .trim_end_matches(".go");
            stem.rsplit('_').next().is_some_and(|suffix| {
                matches!(
                    suffix,
                    "darwin"
                        | "linux"
                        | "windows"
                        | "unix"
                        | "other"
                        | "freebsd"
                        | "openbsd"
                        | "netbsd"
                        | "js"
                        | "wasm"
                        | "amd64"
                        | "arm64"
                        | "386"
                        | "arm"
                        | "ios"
                        | "android"
                        | "plan9"
                        | "solaris"
                        | "illumos"
                        | "nounix"
                        | "nonunix"
                        | "stub"
                        | "nocgo"
                        | "cgo"
                )
            })
        };
        let kind = self.symbols[symbols[0] as usize].kind;
        symbols
            .iter()
            .all(|&symbol| constrained(symbol) && self.symbols[symbol as usize].kind == kind)
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
            Some((types, confidence)) if reference.arity != facts::NO_ARITY => {
                // Generic overloads by type parameter count (`IRequest`, `IRequest<T>`).
                let matching: Vec<u32> = types
                    .into_iter()
                    .filter(|&candidate| {
                        let symbol = &self.symbols[candidate as usize];
                        type_parameter_count(&symbol.signature, &symbol.name)
                            == usize::from(reference.arity)
                    })
                    .collect();
                match matching.as_slice() {
                    [only] => Outcome::Targets(vec![(*only, confidence)], confidence),
                    _ => Outcome::Ambiguous,
                }
            }
            Some(_) => Outcome::Ambiguous,
            None => Outcome::Unresolved,
        }
    }

    fn resolve_ref(&self, file: u32, imports: &Imports, reference: &Ref) -> Outcome {
        let wants_type = reference.kind == RefKind::Type;
        if wants_type
            && reference.qualifier.is_none()
            && self.file_type(file) == Some(FileType::Elixir)
        {
            // Module names are global: `Store.Repo`, or the last segment bound by `alias`.
            let modules = match imports.names.get(&reference.name) {
                Some(Target::Symbols(symbols)) => symbols.clone(),
                _ => self.elixir_modules(&reference.name, file),
            };
            return if modules.is_empty() {
                Outcome::External
            } else {
                pick_members(self, modules, Confidence::Exact, facts::NO_ARITY)
            };
        }
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
        if let Some(outcome) = self.global_name(file, imports, reference) {
            return outcome;
        }
        // Lisp lists also spell lambda lists, bindings and slots: only functions are called.
        let lisp_call =
            reference.kind == RefKind::Call && self.file_type(file) == Some(FileType::Lisp);
        let kind_ok = |symbol: u32| {
            let kind = self.symbols[symbol as usize].kind;
            if wants_type {
                kind.is_type()
            } else if lisp_call {
                kind.is_callable() || kind == SymKind::Macro
            } else {
                kind.is_callable()
                    || kind.is_type()
                    || (reference.kind == RefKind::Value
                        && matches!(kind, SymKind::Const | SymKind::Var))
            }
        };
        // 0. JVM and .NET class bodies: methods and constants of the enclosing class and its
        // supertypes need no `this`.
        if !wants_type && jvm::has_implicit_this(self.file_type(file)) {
            let members: Vec<u32> = self
                .enclosing_members(file, reference.scope, name)
                .into_iter()
                .filter(|&member| {
                    let symbol = &self.symbols[member as usize];
                    let kind = symbol.kind;
                    // `new T()` names the class, not its constructor.
                    let constructor = symbol.parent != NONE
                        && self.symbols[symbol.parent as usize].name == symbol.name;
                    if constructor {
                        false
                    } else if reference.kind == RefKind::Value {
                        // C# and VB.NET method groups (`Register(Handle)`) name methods as values;
                        // a bare Pascal method name in an expression calls it (`X := Count`).
                        kind == SymKind::Const
                            || ((jvm::is_dotnet(self.file_type(file))
                                || self.file_type(file) == Some(FileType::Pascal))
                                && kind.is_callable())
                    } else {
                        kind.is_callable()
                    }
                })
                .collect();
            if !members.is_empty() {
                return pick_members(self, members, Confidence::Exact, reference.arity);
            }
        }
        // 1. Lexical scope: definitions of the file visible from the reference.
        if let Some(facts) = self.facts(file) {
            let base = self.base[file as usize];
            let chain = scope_chain(facts, reference.scope);
            let mut best: Option<(usize, u32)> = None;
            let mut count = 0;
            let mut found: Vec<(usize, u32)> = Vec::new();
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
                found.push((depth, id));
                if best.is_none_or(|(best_depth, _)| depth > best_depth) {
                    best = Some((depth, id));
                }
            }
            // Forward declarations and prototypes announce a definition of the same file.
            let declared = |id: u32| self.symbols[id as usize].flags & FLAG_DECLARATION != 0;
            if found.iter().any(|&(_, id)| declared(id))
                && !found.iter().all(|&(_, id)| declared(id))
            {
                found.retain(|&(_, id)| !declared(id));
                count = found.len();
                best = found.iter().copied().fold(None, |best, (depth, id)| {
                    if best.is_none_or(|(best_depth, _)| depth > best_depth) {
                        Some((depth, id))
                    } else {
                        best
                    }
                });
            }
            if let Some((best_depth, id)) = best {
                if count == 1 {
                    return exact(id);
                }
                // Rust: same-named items at the same depth are `cfg` variants; each is a target.
                if self.file_type(file) == Some(FileType::Rust) {
                    let variants: Vec<(u32, Confidence)> = found
                        .iter()
                        .filter(|(depth, _)| *depth == best_depth)
                        .map(|&(_, id)| (id, Confidence::Exact))
                        .collect();
                    return Outcome::Targets(variants, Confidence::Exact);
                }
                // Objective-C: `@interface`, class extensions, categories and `@implementation`
                // are parts of one class; the first part stands for it.
                if self.file_type(file) == Some(FileType::ObjC)
                    && found.iter().all(|&(_, part)| {
                        let symbol = &self.symbols[part as usize];
                        symbol.kind.is_type() && symbol.name == name
                    })
                    && let Some(&(_, first)) = found.iter().min_by_key(|&&(_, part)| part)
                {
                    return exact(first);
                }
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
        // 4. Same module unit; Go build-tag variants (`x_darwin.go`, `x_other.go`) define the
        // same name in several files and are all targets.
        let module = self.module_of(file);
        let same: Vec<u32> = self
            .module_symbol(module, name, None)
            .into_iter()
            .filter(|&s| kind_ok(s))
            .collect();
        if !same.is_empty() {
            if same.len() > 1
                && self.file_type(file) == Some(FileType::Go)
                && self.build_variants(&same)
            {
                return Outcome::Targets(
                    same.iter().map(|&s| (s, Confidence::Exact)).collect(),
                    Confidence::Exact,
                );
            }
            if jvm::is_class_based(self.file_type(file)) || has_declarations(self.file_type(file)) {
                return pick_members(self, same, Confidence::Exact, reference.arity);
            }
            return pick(&same, Confidence::Exact);
        }
        for &glob in &imports.globs {
            let found: Vec<u32> = self
                .module_symbol(glob, name, None)
                .into_iter()
                .filter(|&s| kind_ok(s))
                .collect();
            if !found.is_empty() {
                if jvm::is_class_based(self.file_type(file))
                    || has_declarations(self.file_type(file))
                {
                    return pick_members(self, found, Confidence::Exact, reference.arity);
                }
                return pick(&found, Confidence::Exact);
            }
        }
        // JVM names come from the file, the enclosing class, the package and imports only;
        // nested types of other classes are not visible unqualified.
        if jvm::is_class_based(self.file_type(file)) {
            return Outcome::Unresolved;
        }
        // 7. Unique name among imported modules, then in the root. Go and Rust only see
        // unqualified names of their own module and imports, so the root is never searched.
        let reach = if has_module_rules(self.file_type(file)) {
            Reach::Imported(module)
        } else {
            Reach::Root
        };
        let caller_is_test = self.caller_is_test(file, reference.scope);
        // Structural templates and markup call methods of their code-behind (XAML handlers).
        let structural = self.file_type(file).is_some_and(|file_type| {
            file_type.class() == crate::domain::filetype::Class::Structural
        });
        self.unique_fallback(imports, name, reach, caller_is_test, |s| {
            kind_ok(s) && (structural || self.symbols[s as usize].kind != SymKind::Method)
        })
    }

    /// A value reference: a local definition, an imported or same-module symbol, or a qualified
    /// name; never a local variable (those shadow nothing in the symbol table).
    fn resolve_value(&self, file: u32, imports: &Imports, reference: &Ref) -> Outcome {
        // Terraform addresses (`var.x`, `module.m`, `aws_s3_bucket.logs`) name blocks of the
        // module directory.
        if self.file_type(file) == Some(FileType::Terraform) {
            let found = self.module_symbol(self.module_of(file), &reference.name, None);
            return pick(&found, Confidence::Exact);
        }
        let local_binding = reference.qualifier.is_none()
            && self
                .binding(file, reference.scope, &reference.name, reference.line)
                .is_some();
        if local_binding {
            return Outcome::External;
        }
        let outcome = match reference.qualifier.as_deref() {
            Some(qualifier) => {
                match self.qualified_symbols(file, Some(imports), qualifier, &reference.name) {
                    Some(found) => pick_members(self, found, Confidence::Exact, reference.arity),
                    // `Type.MEMBER` whose member is no symbol (a field, an enum entry outside
                    // the root): the type itself is used.
                    None if is_identifier(qualifier)
                        && qualifier.starts_with(|ch: char| ch.is_uppercase()) =>
                    {
                        match self.resolve_type(file, Some(imports), reference.scope, qualifier) {
                            Some((types, Confidence::Exact)) => pick(&types, Confidence::Exact),
                            _ => Outcome::Unresolved,
                        }
                    }
                    None => Outcome::Unresolved,
                }
            }
            None => self.resolve_plain(file, imports, reference, false),
        };
        match outcome {
            Outcome::Targets(targets, Confidence::Exact) => {
                Outcome::Targets(targets, Confidence::Exact)
            }
            _ => Outcome::Unresolved,
        }
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
        if reference.kind == RefKind::Type && jvm::is_class_based(self.file_type(file)) {
            // `Outer.Inner`, `pkg.Type`, `Map.Entry`: a type or nothing, never a member.
            return match self.resolve_type(
                file,
                Some(imports),
                reference.scope,
                &format!("{qualifier}.{name}"),
            ) {
                Some((types, confidence)) => pick_members(self, types, confidence, facts::NO_ARITY),
                None => Outcome::Unresolved,
            };
        }
        if let Some(outcome) = self.beam_qualified(file, imports, reference, qualifier) {
            return outcome;
        }
        if let Some(outcome) = self.module_qualified(file, imports, reference, qualifier) {
            return outcome;
        }
        // `a->b` (PHP, C++) and `a?->b` chain like `a.b`.
        let dotted = qualifier.replace("?->", ".").replace("->", ".");
        let chain: Vec<&str> = dotted.split('.').map(str::trim).collect();
        let head = chain[0];
        // 3. Qualified name through imports and the module map; a local binding of the head
        // shadows an import of the same name.
        // Rust qualifiers here are paths (`store::load`), which locals cannot shadow.
        let shadowed = !rust
            && self
                .binding(file, reference.scope, head, reference.line)
                .is_some();
        let plain_head = chain.len() == 1 && is_identifier(head) && !shadowed;
        if (plain_head || (rust && qualifier.contains("::")))
            && let Some(found) = self.qualified_symbols(file, Some(imports), qualifier, name)
        {
            return pick_members(self, found, Confidence::Exact, reference.arity);
        }
        if plain_head && let Some(Target::External(_) | Target::Module(_)) = imports.names.get(head)
        {
            return Outcome::External;
        }
        // `std.heap.ArenaAllocator.init`: a chain through an external import stays external.
        if !shadowed
            && is_identifier(head)
            && let Some(Target::External(_)) = imports.names.get(head)
        {
            return Outcome::External;
        }
        // Objective-C `[[Type alloc] init...]` and `[Type new]`: the receiver is an instance of
        // `Type`; a type outside the root ends resolution.
        if self.file_type(file) == Some(FileType::ObjC)
            && let Some(class) = qualifier
                .strip_prefix('[')
                .and_then(|inner| {
                    inner
                        .strip_suffix(" alloc]")
                        .or_else(|| inner.strip_suffix(" new]"))
                })
                .filter(|class| is_identifier(class))
        {
            return match self.resolve_type(file, Some(imports), reference.scope, class) {
                Some((types, _)) => {
                    let members: Vec<u32> =
                        types.iter().flat_map(|&t| self.members(t, name)).collect();
                    if members.is_empty() {
                        Outcome::External
                    } else {
                        pick_members(self, members, Confidence::Exact, reference.arity)
                    }
                }
                None => Outcome::External,
            };
        }
        // 5a. Receiver that is itself a call (`Service().run()`, `open(x).read()`): the result
        // type of the call, inferred.
        if let Some(callee) = call_receiver(qualifier) {
            if self.call_result_is_external(file, imports, reference, callee) {
                return self
                    .extension(file, imports, reference.scope, name, None)
                    .unwrap_or(Outcome::External);
            }
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
            // Declared type of a single receiver variable: selects among extension overloads.
            let receiver_name = (chain.len() == 1)
                .then(|| self.binding(file, reference.scope, head, reference.line))
                .flatten()
                .filter(|binding| binding.declared)
                .map(|binding| binding.type_name.as_str());
            if let Receiver::External = receiver {
                return self
                    .extension(file, imports, reference.scope, name, receiver_name)
                    .unwrap_or(Outcome::External);
            }
            if let Receiver::Known(types, confidence) = receiver {
                let mut targets = Vec::new();
                for &type_symbol in &types {
                    let (direct, overload) =
                        self.overloads(&self.members(type_symbol, name), reference.arity);
                    targets.extend(direct.iter().map(|&member| (member, overload)));
                    // 6. Abstract receiver or C++ virtual member: every implementation, inferred.
                    let virtual_member = self.file_type(self.symbols[type_symbol as usize].file)
                        == Some(FileType::Cpp)
                        && self.members(type_symbol, name).iter().any(|&member| {
                            self.symbols[member as usize].signature.contains("virtual")
                        });
                    if self.symbols[type_symbol as usize].kind.is_abstract() || virtual_member {
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
                let owner = types
                    .first()
                    .map(|&owner| self.symbols[owner as usize].name.as_str());
                return self
                    .extension(file, imports, reference.scope, name, owner)
                    .unwrap_or(Outcome::Unresolved);
            }
        }
        // `Type.f()` on a type outside the root (`Character.isUpperCase`) is a static call, not
        // an extension call.
        let static_call = chain.len() == 1
            && head.starts_with(|ch: char| ch.is_uppercase())
            && self
                .binding(file, reference.scope, head, reference.line)
                .is_none();
        if !static_call
            && let Some(extension) = self.extension(file, imports, reference.scope, name, None)
        {
            return extension;
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

    /// Overload selection by argument count: the single candidate accepting `arity` is exact;
    /// otherwise the accepting candidates (all when none accepts) are inferred, since argument
    /// types decide, which names alone cannot.
    fn overloads(&self, candidates: &[u32], arity: u8) -> (Vec<u32>, Confidence) {
        // C-family declarations next to their definitions: the definitions count.
        let declaration = |s: &u32| self.symbols[*s as usize].flags & FLAG_DECLARATION != 0;
        let defined: Vec<u32>;
        let candidates =
            if candidates.iter().any(declaration) && !candidates.iter().all(declaration) {
                defined = candidates
                    .iter()
                    .copied()
                    .filter(|s| !declaration(s))
                    .collect();
                defined.as_slice()
            } else {
                candidates
            };
        if candidates.len() <= 1 {
            return (candidates.to_vec(), Confidence::Exact);
        }
        let accepting: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|&candidate| {
                let symbol = &self.symbols[candidate as usize];
                arity != facts::NO_ARITY
                    && parameter_count(&symbol.signature, &symbol.name).is_none_or(
                        |(required, total, variadic)| {
                            let arity = usize::from(arity);
                            (required..=total).contains(&arity)
                                || (variadic && arity + 1 >= required)
                        },
                    )
            })
            .collect();
        match accepting.as_slice() {
            [only] => (vec![*only], Confidence::Exact),
            [] => (candidates.to_vec(), Confidence::Inferred),
            _ => (accepting, Confidence::Inferred),
        }
    }

    /// Whether the receiver call `callee(...)` leads outside the root: the callee itself is
    /// external, or it resolves to a function whose declared result type is not in the root.
    fn call_result_is_external(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        callee: &str,
    ) -> bool {
        let callee_ref = match split_qualified(callee) {
            Some((qualifier, name)) => Ref {
                kind: RefKind::Call,
                name: name.to_owned(),
                qualifier: Some(qualifier.to_owned()),
                scope: reference.scope,
                line: reference.line,
                arity: facts::NO_ARITY,
            },
            None => Ref {
                kind: RefKind::Call,
                name: callee.to_owned(),
                qualifier: None,
                scope: reference.scope,
                line: reference.line,
                arity: facts::NO_ARITY,
            },
        };
        let Some(_guard) = DepthGuard::enter() else {
            return false;
        };
        match self.resolve_ref(file, imports, &callee_ref) {
            Outcome::External => true,
            Outcome::Targets(callees, _) => callees.iter().all(|&(callee, _)| {
                if self.symbols[callee as usize].kind.is_type() {
                    return false;
                }
                let Some(&result) = self.returns.get(&callee) else {
                    return false;
                };
                let callee_file = self.symbols[callee as usize].file;
                let callee_scope = Some(callee - self.base[callee_file as usize]);
                self.resolve_type(callee_file, None, callee_scope, result)
                    .is_none()
            }),
            _ => false,
        }
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
        let objc = self.file_type(file) == Some(FileType::ObjC);
        let start = if matches!(head, "self" | "this" | "Self" | "$this") {
            match self.owner_type(file, scope) {
                Some(types) if objc => Some((self.objc_class_parts(&types), Confidence::Exact)),
                Some(types) => Some((types, Confidence::Exact)),
                None => return Receiver::Unknown,
            }
        } else if objc && head == "super" {
            // `[super m]`: the superclass named by any part of the enclosing class.
            let Some(types) = self.owner_type(file, scope) else {
                return Receiver::Unknown;
            };
            let mut supers: Vec<u32> = self
                .objc_class_parts(&types)
                .iter()
                .flat_map(|part| self.supertypes.get(part).into_iter().flatten().copied())
                .collect();
            supers.sort_unstable();
            supers.dedup();
            if supers.is_empty() {
                return Receiver::External;
            }
            Some((self.objc_class_parts(&supers), Confidence::Exact))
        } else if let Some(binding) = self.binding(file, scope, head, line) {
            match self.binding_type(file, imports, scope, binding) {
                Some(found) => Some(found),
                None if binding.type_name.ends_with("()") => return Receiver::Unknown,
                None => return Receiver::External,
            }
        } else if let Some(field) = jvm::has_implicit_this(self.file_type(file))
            .then(|| self.enclosing_field_type(file, scope, head))
            .flatten()
        {
            match field {
                Some(found) => Some(found),
                None => return Receiver::External,
            }
        } else {
            self.resolve_type(file, Some(imports), scope, head)
        };
        let Some((mut types, mut confidence)) = start else {
            return Receiver::Unknown;
        };
        for field in &chain[1..] {
            if types.is_empty() {
                return Receiver::Unknown;
            }
            // Objective-C declares properties in the interface or a class extension, not in
            // the implementation the method belongs to: any part of the class carries them.
            let parts = if objc {
                self.objc_class_parts(&types)
            } else {
                types[..1].to_vec()
            };
            let Some((type_symbol, field_type)) = parts.iter().find_map(|&part| {
                self.field_types
                    .get(&(part, *field))
                    .map(|&field_type| (part, field_type))
            }) else {
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
        if let Some(collection) = binding.type_name.strip_prefix("range:") {
            // The element of a ranged collection: the collection binding's (element) type.
            let source = self.binding(file, scope, collection, binding.line)?;
            if std::ptr::eq(source, binding) {
                return None;
            }
            return self
                .binding_type(file, imports, scope, source)
                .map(|(types, _)| (types, Confidence::Inferred));
        }
        if let Some(call) = binding.type_name.strip_suffix("()") {
            // Result of a call: resolve the callee, then its declared result type.
            let reference = match split_qualified(call) {
                Some((qualifier, name)) => Ref {
                    kind: RefKind::Call,
                    name: name.to_owned(),
                    qualifier: Some(qualifier.to_owned()),
                    scope,
                    line: binding.line,
                    arity: facts::NO_ARITY,
                },
                None => Ref {
                    kind: RefKind::Call,
                    name: call.to_owned(),
                    qualifier: None,
                    scope,
                    line: binding.line,
                    arity: facts::NO_ARITY,
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
        let mut module_imports: HashSet<(u32, u32)> = HashSet::default();
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
        // Before the symbols move into the graph.
        let contract_edges = self.contract_edges();
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
                keys: facts
                    .map(|facts| {
                        let keys = facts.config_keys.iter().map(|key| key.path.as_str());
                        let envs = facts.env_defs.iter().map(|def| def.name.as_str());
                        let texts = facts.texts.iter().map(|segment| segment.text.as_str());
                        keys.chain(envs).chain(texts).collect::<Vec<_>>().join(" ")
                    })
                    .unwrap_or_default(),
            });
        }
        let mut edges: Vec<Edge> = Vec::new();
        let mut externals: BTreeMap<(u32, String), u32> = BTreeMap::new();
        let mut module_imports: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for (file, outcome) in resolved.into_iter().enumerate() {
            graph.files[file].ambiguous = outcome.ambiguous;
            graph.stale.extend(outcome.stale);
            graph.ambiguous_names.extend(outcome.ambiguous_names);
            let source = self.modules.file_module[file];
            let file_is_test = self.input.files[file].role == Role::Test;
            for &target in &outcome.imported_modules {
                if target != source && !file_is_test {
                    *module_imports.entry((source, target)).or_default() += 1;
                }
            }
            edges.extend(outcome.edges);
            let module = self.modules.file_module[file];
            let mut seen = HashSet::default();
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
        edges.extend(contract_edges);
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
        graph.ambiguous_names.sort_unstable();
        graph.ambiguous_names.dedup();
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
    edges.par_sort_unstable_by_key(|edge| {
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
                | FileType::Java
                | FileType::Kotlin
                | FileType::Scala
                | FileType::Groovy
                | FileType::CSharp
                | FileType::VbNet
                | FileType::Swift
                | FileType::Terraform
                | FileType::Php
                | FileType::Dart
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

/// Languages whose modules hold declarations next to definitions (OCaml `.mli` and `.ml`, Lisp
/// `defgeneric` and `defmethod`, Pascal forward class declarations and interface sections).
fn has_declarations(file_type: Option<FileType>) -> bool {
    matches!(
        file_type,
        Some(FileType::OCaml | FileType::Lisp | FileType::Pascal)
    )
}

fn pick_members(index: &Index, found: Vec<u32>, confidence: Confidence, arity: u8) -> Outcome {
    let mut found = found;
    found.sort_unstable();
    found.dedup();
    // C-family: a declaration next to its definition (in-class prototype and out-of-line body).
    if found.len() > 1 {
        let declaration = |s: &u32| index.symbols[*s as usize].flags & FLAG_DECLARATION != 0;
        if found.iter().any(declaration) && !found.iter().all(declaration) {
            found.retain(|s| !declaration(s));
        }
    }
    // .NET partial types: the parts of one type collapse to its first part.
    if found.len() > 1 {
        let first = &index.symbols[found[0] as usize];
        let partial = first.kind.is_type()
            && jvm::is_dotnet(index.file_type(first.file))
            && found.iter().all(|&s| {
                let symbol = &index.symbols[s as usize];
                symbol.name == first.name
                    && symbol.kind.is_type()
                    && index.module_of(symbol.file) == index.module_of(first.file)
            });
        if partial {
            found.truncate(1);
        }
    }
    if found.len() > 1 {
        let non_test: Vec<u32> = found
            .iter()
            .copied()
            .filter(|&s| !index.is_test(s))
            .collect();
        if non_test.len() == 1 {
            return Outcome::Targets(vec![(non_test[0], confidence)], confidence);
        }
        // Overloads of one owner (parent type, or the `Type::`/extension owner of top-level
        // members): the one taking the argument count, else every candidate inferred.
        let first = &index.symbols[found[0] as usize];
        let owner = |s: u32| (index.symbols[s as usize].parent, index.owner_name(s));
        let first_owner = owner(found[0]);
        if (first.parent != NONE || !first_owner.1.is_empty())
            && found
                .iter()
                .all(|&s| index.symbols[s as usize].name == first.name && owner(s) == first_owner)
        {
            let (targets, overload) = index.overloads(&found, arity);
            return Outcome::Targets(
                targets
                    .iter()
                    .map(|&s| (s, overload.max(confidence)))
                    .collect(),
                overload.max(confidence),
            );
        }
    }
    pick(&found, confidence)
}

/// Parameter counts of the declaration `name(...)` in a signature: required (without default
/// values), total, and whether it is variadic; `None` when the list cannot be found (clipped or
/// no parentheses).
/// Type parameters declared right after `name` in a signature: `interface IRequest<out T>` has
/// one, `class Store` none.
fn type_parameter_count(signature: &str, name: &str) -> usize {
    let bytes = signature.as_bytes();
    let Some(start) = signature
        .match_indices(name)
        .map(|(position, _)| position)
        .find(|&position| {
            let before = position == 0
                || !(bytes[position - 1].is_ascii_alphanumeric() || bytes[position - 1] == b'_');
            let after = bytes.get(position + name.len()).copied();
            before && !after.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
    else {
        return 0;
    };
    let Some(rest) = signature[start + name.len()..].strip_prefix('<') else {
        return 0;
    };
    let mut depth = 0i32;
    let mut count = 1;
    for ch in rest.chars() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' if depth == 0 => return count,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

fn parameter_count(signature: &str, name: &str) -> Option<(usize, usize, bool)> {
    // Objective-C `- (T)name:(A)a with:(B)b`: one parameter per selector colon.
    if let Some(rest) = signature
        .strip_prefix('-')
        .or_else(|| signature.strip_prefix('+'))
        && rest.trim_start().starts_with('(')
    {
        let mut depth = 0i32;
        let colons = rest
            .chars()
            .filter(|&ch| {
                match ch {
                    '(' | '[' | '{' | '<' => depth += 1,
                    ')' | ']' | '}' | '>' => depth -= 1,
                    _ => {}
                }
                ch == ':' && depth == 0
            })
            .count();
        return Some((colons, colons, false));
    }
    let bytes = signature.as_bytes();
    let word = |index: usize| {
        bytes
            .get(index)
            .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
    };
    let start = signature
        .match_indices(name)
        .map(|(position, _)| position)
        .find(|&position| position == 0 || !word(position - 1))?;
    let mut rest = signature[start + name.len()..].trim_start();
    if rest.starts_with('<') {
        let mut depth = 0;
        let close = rest.char_indices().find_map(|(index, ch)| {
            match ch {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(index)
        })?;
        rest = rest[close + 1..].trim_start();
    }
    let inner = rest.strip_prefix('(')?;
    let (mut depth, mut commas, mut end) = (0i32, 0, None);
    let mut defaults = 0;
    let mut in_default = false;
    let mut previous = ' ';
    for (index, ch) in inner.char_indices() {
        // `->` and `=>` in function types are no closing brackets.
        let arrow = ch == '>' && matches!(previous, '-' | '=');
        previous = ch;
        if arrow {
            continue;
        }
        match ch {
            '(' | '<' | '[' | '{' => depth += 1,
            ')' if depth == 0 => {
                end = Some(index);
                break;
            }
            ')' | '>' | ']' | '}' => depth -= 1,
            // Pascal separates parameter groups with `;` (`A, B: Integer; C: string`).
            ',' | ';' if depth == 0 => {
                commas += 1;
                in_default = false;
            }
            '=' if depth == 0 && !in_default => {
                defaults += 1;
                in_default = true;
            }
            _ => {}
        }
    }
    let list = &inner[..end?];
    let count: usize = if list.trim().is_empty() {
        0
    } else {
        commas + 1
    };
    let variadic = list.contains("...") || list.contains("vararg ") || list.contains("params ");
    Some((count.saturating_sub(defaults), count, variadic))
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

fn role_file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `path:12` and `path:12-30` cite lines of a file; the file is the reference.
fn strip_line_suffix(text: &str) -> &str {
    match text.rsplit_once(':') {
        Some((path, lines))
            if !path.is_empty()
                && !lines.is_empty()
                && lines.chars().all(|ch| ch.is_ascii_digit() || ch == '-') =>
        {
            path
        }
        _ => text,
    }
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
    // `(new Store())->open()`, PHP 8.4 `new Store()->open()`: the constructed type.
    let qualifier = qualifier
        .strip_prefix("(new ")
        .and_then(|inner| inner.strip_suffix(')'))
        .or_else(|| qualifier.strip_prefix("new "))
        .map_or(qualifier, str::trim);
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

/// Robot Framework keyword identity: case, spaces and underscores are insignificant
/// (`Open Store`, `open_store`).
fn robot_key(name: &str) -> String {
    name.chars()
        .filter(|ch| !matches!(ch, ' ' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests;
