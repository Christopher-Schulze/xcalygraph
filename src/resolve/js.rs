//! TypeScript and JavaScript module rules (`docs/spec.md` section 6): relative specifiers with
//! extension and index probing, `tsconfig.json` and `jsconfig.json` `paths` and `baseUrl`,
//! workspace packages from `package.json`, external packages otherwise.

use super::modules::parent_dir;
use super::{Imports, Index, Input, Target};
use crate::domain::facts::{Facts, SymKind};
use crate::domain::role;

/// Extensions a specifier may omit or name with a `.js` stand-in.
const STRIP: [&str; 14] = [
    ".d.ts", ".tsx", ".ts", ".mts", ".cts", ".jsx", ".js", ".mjs", ".cjs", ".vue", ".svelte",
    ".astro", ".json", ".node",
];

/// Module key of a file: `js:` plus its path without extension.
pub fn module_key(path: &str) -> String {
    format!("js:{}", strip_extension(path))
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
    /// (package name, root-relative entry without extension).
    packages: Vec<(String, String)>,
}

pub fn tables(input: &Input) -> JsTables {
    let mut tables = JsTables::default();
    for (record, facts) in input.files.iter().zip(input.facts) {
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
                    tables
                        .packages
                        .push((name.clone(), strip_extension(&entry).to_owned()));
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
        let dir = parent_dir(&self.input.files[file as usize].path).to_owned();
        for import in &facts.imports {
            let Some(target) = self.js_module(&dir, &import.spec) else {
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

    fn js_module(&self, dir: &str, spec: &str) -> Option<u32> {
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
            for (name, entry) in &tables.packages {
                if spec == name {
                    bases.push(entry.clone());
                } else if let Some(rest) = spec.strip_prefix(&format!("{name}/")) {
                    let package_dir = parent_dir(entry);
                    let package_dir = package_dir.strip_suffix("/src").unwrap_or(package_dir);
                    bases.push(normalize(&join(package_dir, rest)));
                }
            }
        }
        bases.iter().find_map(|base| {
            let base = strip_extension(base);
            self.modules
                .get(&format!("js:{base}"))
                .or_else(|| self.modules.get(&format!("js:{base}/index")))
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
        let own: Vec<u32> = self
            .module_symbol(module, name, None)
            .into_iter()
            .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Mod)
            .collect();
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
            let dir = parent_dir(&self.input.files[file].path).to_owned();
            for import in facts.imports.iter().filter(|import| import.reexport) {
                let forwards = import.names.iter().any(|imported| {
                    let exposed = import.alias.as_deref().unwrap_or(imported);
                    exposed == name || (imported == "*" && import.alias.is_none())
                });
                if !forwards {
                    continue;
                }
                let Some(target) = self.js_module(&dir, &import.spec) else {
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
}
