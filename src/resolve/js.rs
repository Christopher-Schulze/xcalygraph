//! TypeScript and JavaScript module rules (`docs/spec.md` section 6): relative specifiers with
//! extension and index probing, `tsconfig.json` and `jsconfig.json` `paths` and `baseUrl`,
//! workspace packages from `package.json`, the SvelteKit `$lib` alias, external packages
//! otherwise.

use super::modules::parent_dir;
use super::{Imports, Index, Input, Target};
use crate::domain::facts::{Facts, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::role;

/// Extensions a specifier may omit or name with a `.js` stand-in.
const STRIP: [&str; 14] = [
    ".d.ts", ".tsx", ".ts", ".mts", ".cts", ".jsx", ".js", ".mjs", ".cjs", ".vue", ".svelte",
    ".astro", ".json", ".node",
];

/// Explicit module-system extensions never collapse into an extensionless path. Other
/// implementation/declaration grouping retains its existing contract.
pub fn module_key(path: &str) -> String {
    let path = if [".mjs", ".cjs", ".mts", ".cts"]
        .iter()
        .any(|extension| path.ends_with(extension))
    {
        path
    } else {
        strip_extension(path)
    };
    format!("js:{path}")
}

pub fn strip_extension(path: &str) -> &str {
    STRIP
        .iter()
        .find_map(|extension| path.strip_suffix(extension))
        .unwrap_or(path)
}

/// Path aliases and workspace packages, computed once per resolution.
#[derive(Debug, Default)]
pub struct JsTables {
    /// (alias prefix, root-relative target prefix), longest alias first.
    aliases: Vec<(String, String)>,
    /// Root-relative `baseUrl` directories.
    base_dirs: Vec<String>,
    /// (package name, root-relative entry as declared, package directory).
    packages: Vec<(String, String, String)>,
}

pub fn tables(input: &Input) -> JsTables {
    let mut tables = JsTables::default();
    for (record, facts) in input.files.iter().zip(input.facts) {
        // SvelteKit's default `$lib` alias; the generated tsconfig declaring it is not checked in.
        if role::file_name(&record.path).starts_with("svelte.config.") {
            let dir = parent_dir(&record.path);
            tables
                .aliases
                .push(("$lib/".to_owned(), normalize(&join(dir, "src/lib"))));
        }
        let Some(manifest) = facts.as_ref().and_then(|facts| facts.manifest.as_ref()) else {
            continue;
        };
        let dir = parent_dir(&record.path);
        match role::file_name(&record.path) {
            "tsconfig.json" | "jsconfig.json" => {
                let base = manifest
                    .source_roots
                    .first()
                    .map_or(dir.to_owned(), |base| normalize(&join(dir, base)));
                for entry in &manifest.include_dirs {
                    if let Some((alias, target)) = entry.split_once('=') {
                        tables.aliases.push((
                            alias.trim_end_matches('*').to_owned(),
                            normalize(&join(&base, target.trim_end_matches('*'))),
                        ));
                    }
                }
                if !manifest.source_roots.is_empty() {
                    tables.base_dirs.push(base);
                }
            }
            "package.json" => {
                if let Some(name) = &manifest.module {
                    let entry = manifest.source_roots.first().map_or_else(
                        || join(dir, "src/index"),
                        |main| normalize(&join(dir, main)),
                    );
                    tables.packages.push((name.clone(), entry, dir.to_owned()));
                }
            }
            _ => {}
        }
    }
    tables
        .aliases
        .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.cmp(b)));
    tables
}

impl Index<'_> {
    pub(super) fn js_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let Some(target) = self.js_module(file, &import.spec) else {
                let package = package_name(&import.spec);
                if !package.is_empty() {
                    imports.externals.push(package.clone());
                    for name in &import.names {
                        let local = import.alias.clone().unwrap_or_else(|| name.clone());
                        if !local.is_empty() && local != "*" {
                            imports
                                .names
                                .insert(local, Target::External(package.clone()));
                        }
                    }
                }
                continue;
            };
            imports.modules.push(target);
            for (other, &module) in self.modules.file_module.iter().enumerate() {
                if module == target {
                    imports.files.push((file, other as u32));
                }
            }
            // `const util = require("./util")`: the alias names the whole module.
            if import.names.is_empty()
                && let Some(alias) = &import.alias
            {
                imports.names.insert(alias.clone(), Target::Module(target));
            }
            for name in &import.names {
                let local = import.alias.clone().unwrap_or_else(|| name.clone());
                match name.as_str() {
                    "" => {}
                    "*" if import.alias.is_none() => imports.globs.push(target),
                    "*" => {
                        imports.names.insert(local, Target::Module(target));
                    }
                    "default" => {
                        let bound = self
                            .default_export(target, &local)
                            .map_or(Target::Module(target), Target::Symbols);
                        imports.names.insert(local, bound);
                    }
                    name => {
                        let found = self.exported(target, name);
                        if !found.is_empty() {
                            imports.names.insert(local, Target::Symbols(found));
                        }
                    }
                }
            }
        }
    }

    pub(super) fn js_module(&self, file: u32, spec: &str) -> Option<u32> {
        let dir = parent_dir(&self.input.files[file as usize].path);
        let typescript = self.file_type(file) != Some(FileType::JavaScript);
        let tables = &self.js_tables;
        let mut bases: Vec<String> = Vec::new();
        if spec.starts_with('.') {
            bases.push(normalize(&join(dir, spec)));
        } else {
            for (alias, target) in &tables.aliases {
                if let Some(rest) = spec.strip_prefix(alias.as_str()) {
                    bases.push(normalize(&join(target, rest)));
                }
            }
            for base in &tables.base_dirs {
                bases.push(normalize(&join(base, spec)));
            }
            for (name, entry, package_root) in &tables.packages {
                if spec == name {
                    bases.push(entry.clone());
                    bases.push(normalize(&join(package_root, "src/index")));
                } else if let Some(rest) = spec.strip_prefix(&format!("{name}/")) {
                    let package_dir = parent_dir(entry);
                    let package_dir = package_dir.strip_suffix("/src").unwrap_or(package_dir);
                    bases.push(normalize(&join(package_dir, rest)));
                    // Subpath exports point at build output (`dist/utils/*.js`) that the tree
                    // does not hold; the sources live under the package's `src/`.
                    bases.push(normalize(&join(package_root, &format!("src/{rest}"))));
                    bases.push(normalize(&join(package_root, rest)));
                }
            }
        }
        let module_at = |path: &str| {
            let &target = self.path_index.get(path)?;
            let record = &self.input.files[target as usize];
            (record.role.is_extracted()
                && matches!(
                    record.file_type,
                    Some(
                        FileType::JavaScript
                            | FileType::TypeScript
                            | FileType::Vue
                            | FileType::Svelte
                            | FileType::Astro
                    )
                ))
            .then(|| self.module_of(target))
        };
        let probe = |stem: &str, extensions: &[&str]| {
            let mut path = String::with_capacity(stem.len() + 12);
            path.push_str(stem);
            extensions.iter().find_map(|extension| {
                path.truncate(stem.len());
                path.push_str(extension);
                module_at(&path)
            })
        };
        bases.iter().find_map(|base| {
            // Probe concrete files while retaining TypeScript's source/declaration substitution.
            // Module-system extensions are never guessed by the extensionless route below.
            for (extension, typed) in [
                (".js", &[".ts", ".tsx", ".d.ts"][..]),
                (".mjs", &[".mts", ".d.mts"][..]),
                (".cjs", &[".cts", ".d.cts"][..]),
            ] {
                if let Some(stem) = base.strip_suffix(extension) {
                    return if typescript {
                        probe(stem, typed).or_else(|| module_at(base))
                    } else {
                        module_at(base).or_else(|| {
                            // Source trees may hold the implementation before compilation.
                            probe(stem, &typed[..typed.len() - 1])
                        })
                    };
                }
            }
            if strip_extension(base) != base {
                return module_at(base);
            }
            let extensions = if typescript {
                &[
                    ".ts", ".tsx", ".d.ts", ".js", ".jsx", ".vue", ".svelte", ".astro",
                ][..]
            } else {
                &[".js", ".jsx", ".ts", ".tsx", ".vue", ".svelte", ".astro"][..]
            };
            module_at(base)
                .or_else(|| probe(base, extensions))
                .or_else(|| probe(&format!("{base}/index"), extensions))
        })
    }

    /// Exported symbols `name` of a module, following `export { name } from` and
    /// `export * from` re-exports (depth max 8).
    fn exported(&self, module: u32, name: &str) -> Vec<u32> {
        self.exported_at(module, name, 0)
    }

    pub(super) fn js_exported(&self, module: u32, name: &str) -> Vec<u32> {
        self.exported_at(module, name, 0)
    }

    fn exported_at(&self, module: u32, name: &str, depth: usize) -> Vec<u32> {
        // TypeScript namespaces are module symbols that importers name.
        let own: Vec<u32> = self.module_symbol(module, name, None);
        if !own.is_empty() || depth >= 8 {
            return own;
        }
        for (file, &file_module) in self.modules.file_module.iter().enumerate() {
            if file_module != module {
                continue;
            }
            let Some(facts) = self.input.facts[file].as_ref() else {
                continue;
            };
            for import in facts.imports.iter().filter(|import| import.reexport) {
                let forwards = import.names.iter().any(|imported| {
                    let exposed = import.alias.as_deref().unwrap_or(imported);
                    exposed == name || (imported == "*" && import.alias.is_none())
                });
                if !forwards {
                    continue;
                }
                let Some(target) = self.js_module(file as u32, &import.spec) else {
                    continue;
                };
                let original = import
                    .names
                    .iter()
                    .find(|imported| import.alias.as_deref().unwrap_or(imported) == name)
                    .map_or(name, String::as_str);
                let found = self.exported_at(target, original, depth + 1);
                if !found.is_empty() {
                    return found;
                }
            }
        }
        Vec::new()
    }

    /// The default export: a symbol named like the local binding, else the module's only
    /// exported class or function.
    fn default_export(&self, module: u32, local: &str) -> Option<Vec<u32>> {
        let named = self.exported(module, local);
        if !named.is_empty() {
            return Some(named);
        }
        let candidates: Vec<u32> = self
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| {
                symbol.parent == u32::MAX
                    && symbol.exported
                    && matches!(symbol.kind, SymKind::Class | SymKind::Fn)
                    && self.module_of(symbol.file) == module
            })
            .map(|(id, _)| id as u32)
            .collect();
        (candidates.len() == 1).then_some(candidates)
    }
}

/// External package of a bare specifier: `@scope/name` or the first segment.
fn package_name(spec: &str) -> String {
    if spec.starts_with('.') || spec.starts_with('/') {
        return String::new();
    }
    let mut parts = spec.split('/');
    match (parts.next(), parts.next()) {
        (Some(scope), Some(name)) if scope.starts_with('@') => format!("{scope}/{name}"),
        (Some(first), _) => first.trim_start_matches("node:").to_owned(),
        _ => String::new(),
    }
}

fn join(dir: &str, path: &str) -> String {
    if dir.is_empty() || path.starts_with('/') {
        path.trim_start_matches('/').to_owned()
    } else {
        format!("{dir}/{path}")
    }
}

/// Resolves `.` and `..` segments; never climbs above the root.
fn normalize(path: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specifiers_normalize_and_packages_are_named() {
        assert_eq!(normalize("src/app/../lib/./x"), "src/lib/x");
        assert_eq!(strip_extension("src/a.d.ts"), "src/a");
        assert_eq!(strip_extension("src/a.service.ts"), "src/a.service");
        assert_eq!(package_name("@scope/pkg/sub"), "@scope/pkg");
        assert_eq!(package_name("react-dom/client"), "react-dom");
        assert_eq!(package_name("node:fs"), "fs");
        assert_eq!(package_name("./x"), "");
    }

    #[test]
    fn module_keys_drop_extensions() {
        assert_eq!(module_key("web/src/index.tsx"), "js:web/src/index");
    }

    #[test]
    fn javascript_commonjs_extensions_must_be_named() -> std::io::Result<()> {
        use crate::domain::graph::EdgeKind;
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("javascript-commonjs-extensions")?;
        root.write("helper.cjs", "exports.work = function work() {};\n")?;
        root.write("helper.mjs", "export function work() {}\n")?;
        root.write(
            "omitted.cjs",
            "const helper = require('./helper');\nhelper.work();\n",
        )?;
        root.write(
            "explicit.cjs",
            "const helper = require('./helper.cjs');\nhelper.work();\n",
        )?;
        root.write("esm.mjs", "import { work } from './helper.mjs';\nwork();\n")?;
        let graph = super::super::tests::graph_of(root.path())?;
        let imports: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Imports)
            .map(|edge| {
                (
                    graph.files[edge.source as usize - graph.symbols.len()]
                        .path
                        .as_str(),
                    graph.files[edge.target as usize - graph.symbols.len()]
                        .path
                        .as_str(),
                )
            })
            .collect();
        assert_eq!(
            imports,
            [("esm.mjs", "helper.mjs"), ("explicit.cjs", "helper.cjs")]
        );
        let callers: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .map(|edge| {
                let source = if let Some(symbol) = graph.symbols.get(edge.source as usize) {
                    symbol.file as usize
                } else {
                    edge.source as usize - graph.symbols.len()
                };
                graph.files[source].path.as_str()
            })
            .collect();
        assert_eq!(callers, ["esm.mjs", "explicit.cjs"]);
        Ok(())
    }
}
