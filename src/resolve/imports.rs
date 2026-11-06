//! Import and module rules of the smaller languages (Zig, Lua, R, Elixir, Erlang, Dart, Swift, Ruby, OCaml).

use super::Ids;
use super::*;

impl<'a> Index<'a> {
    /// `require` and `require_relative`: `.rb` files relative to the file, the root and `lib/`;
    /// gems outside the root are external.
    pub(super) fn ruby_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
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

    /// Source directory (`R/`) of the local R package named `name` (`DESCRIPTION` `Package:`).
    pub(super) fn r_package_dir(&self, name: &str) -> Option<String> {
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

    /// `source("x.R")` relative to the file or the root; `library(pkg)` of a local package
    /// makes its `R/` functions visible, other packages are external.
    pub(super) fn r_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
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
                None => imports.externals.push(import.spec.to_string()),
            }
        }
    }

    /// `open M` / `include M`: the file module `M` of the root becomes visible; library modules
    /// are external.
    pub(super) fn ocaml_imports(&self, facts: &Facts, imports: &mut Imports) {
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
    pub(super) fn elixir_modules(&self, name: &str, file: u32) -> Ids {
        let mut candidates: Ids = Ids::new();
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
        let exact: Ids = candidates
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
    pub(super) fn elixir_full_name(&self, symbol: u32) -> String {
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

    /// `alias A.B` binds `B`; `import A.B` and `use A.B` make the module's functions visible.
    /// Modules outside the root are external.
    pub(super) fn elixir_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
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
                // `import Plug.Conn`: the module's functions are callable unqualified.
                if import.names.is_empty() {
                    for &module in &modules {
                        self.import_members(file, module, imports);
                    }
                }
            }
        }
    }

    /// `-include("x.hrl")` (relative, `include/`), `-import(m, [...])`: `m`'s functions visible.
    pub(super) fn erlang_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
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
    pub(super) fn lua_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
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
                        .insert(alias.to_string(), Target::External(head.clone()));
                }
                imports.externals.push(head);
                continue;
            };
            imports.files.push((file, target));
            let module = self.module_of(target);
            imports.modules.push(module);
            if let Some(alias) = &import.alias {
                imports
                    .names
                    .insert(alias.to_string(), Target::Module(module));
            }
        }
        // Members of required modules re-bound to locals; other tables are no imports.
        for import in members {
            let (Some(alias), Some(member)) = (&import.alias, import.names.first()) else {
                continue;
            };
            match imports.names.get(import.spec.as_str()) {
                Some(Target::Module(module)) => {
                    let found = self.module_symbol(*module, member, None);
                    if !found.is_empty() {
                        imports
                            .names
                            .insert(alias.to_string(), Target::Symbols(found));
                    }
                }
                Some(Target::External(name)) => {
                    let name = name.clone();
                    imports
                        .names
                        .insert(alias.to_string(), Target::External(name));
                }
                _ => {}
            }
        }
    }

    /// Root file of a named module `b.addModule("name", ...)` in a `build.zig` of the root; the
    /// first definition in path order wins.
    pub(super) fn zig_named_module(&self, name: &str) -> Option<u32> {
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
    pub(super) fn zig_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        // Whole modules first, then declarations re-binding one of their members.
        let (members, modules): (Vec<&facts::Import>, Vec<&facts::Import>) = facts
            .imports
            .iter()
            .filter(|import| !import.reexport)
            .partition(|import| !import.names.is_empty());
        for import in modules.into_iter().chain(members) {
            let target = if let Some(bound) = imports
                .names
                .get(import.spec.as_str())
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
                            imports.externals.push(import.spec.to_string());
                        }
                        Target::External(import.spec.to_string())
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
                    let found: Ids = types
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
                imports.names.insert(alias.to_string(), target);
            }
        }
    }

    /// `import 'package:app/src/x.dart'` (the package's `lib/`), relative imports; imported files'
    /// names become visible, with the files they `export` and their `part`s. A `part of` file
    /// sees its library and the library's imports. `dart:` libraries are the platform, other
    /// packages external.
    pub(super) fn dart_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
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
    pub(super) fn dart_visible(&self, target: u32, imports: &mut Imports, visited: &mut Vec<u32>) {
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

    /// `import Module`: a SwiftPM target of the root becomes visible; platform modules are
    /// dropped, other modules are external dependencies.
    pub(super) fn swift_imports(&self, facts: &Facts, imports: &mut Imports) {
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
                    imports.externals.push(import.spec.to_string());
                }
                None => {}
            }
        }
    }
}
