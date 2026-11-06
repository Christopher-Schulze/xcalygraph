//! TypeScript and JavaScript module rules (`docs/spec.md` section 6): relative specifiers with
//! extension and index probing, `tsconfig.json` and `jsconfig.json` `paths` and `baseUrl`,
//! workspace packages from `package.json`, the SvelteKit `$lib` alias, external packages
//! otherwise.

use std::collections::{HashMap, HashSet};

use super::modules::parent_dir;
use super::{Imports, Index, Input, Target};
use crate::domain::facts::{Facts, Manifest, SymKind, TypeScriptProjectConfig};
use crate::domain::filetype::FileType;
use crate::domain::role;
use crate::extract::manifest::is_project_config;

/// Extensions a specifier may omit or name with a `.js` stand-in.
const STRIP: [&str; 14] = [
    ".d.ts", ".tsx", ".ts", ".mts", ".cts", ".jsx", ".js", ".mjs", ".cjs", ".vue", ".svelte",
    ".astro", ".json", ".node",
];

/// Runtime files have physical module identities. TypeScript implementation/declaration
/// grouping retains its separately verified pairing contract.
pub fn module_key(path: &str) -> String {
    let path = if [".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"]
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
    /// Project config by root-relative path.
    configs: HashMap<String, JsConfig>,
    /// Directory -> `tsconfig.json`, else `jsconfig.json` of that directory.
    config_of_dir: HashMap<String, String>,
    /// Directories holding a `svelte.config.*`.
    svelte_dirs: HashSet<String>,
    /// (package name, root-relative entry as declared, package directory).
    packages: Vec<(String, String, String)>,
}

/// One project config with its `extends` chain applied.
#[derive(Debug, Default, Clone)]
struct JsConfig {
    aliases: Vec<JsAlias>,
    /// Root-relative `baseUrl`.
    base_dir: Option<String>,
    references: Vec<String>,
    scope: Option<(String, TypeScriptProjectConfig)>,
}

/// A `paths` entry: `prefix*suffix`, or the exact name without a star, and its targets in order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct JsAlias {
    prefix: String,
    suffix: String,
    wildcard: bool,
    targets: Vec<String>,
}

impl JsAlias {
    fn matched<'a>(&self, spec: &'a str) -> Option<&'a str> {
        if !self.wildcard {
            return (spec == self.prefix).then_some("");
        }
        spec.strip_prefix(self.prefix.as_str())?
            .strip_suffix(self.suffix.as_str())
    }
}

/// What one config file declares before `extends`.
#[derive(Debug, Default, Clone)]
struct RawConfig {
    dir: String,
    extends: Vec<String>,
    references: Vec<String>,
    /// Target templates; `None` without a `paths` key.
    paths: Option<Vec<(String, Vec<String>)>>,
    /// Directory of the config that declared `paths`.
    paths_dir: String,
    base_url: Option<String>,
    files: Option<Vec<String>>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

/// `extends` and `references` name a file, or a directory or stem of one; package configs outside
/// the tree have no entry here.
fn config_path(raw: &HashMap<String, RawConfig>, dir: &str, target: &str) -> Option<String> {
    if !target.starts_with('.') && !target.starts_with('/') {
        return None;
    }
    let joined = normalize(&join(dir, target));
    [
        joined.clone(),
        format!("{joined}.json"),
        join(&joined, "tsconfig.json"),
    ]
    .into_iter()
    .find(|path| raw.contains_key(path))
}

fn effective(
    raw: &HashMap<String, RawConfig>,
    path: &str,
    seen: &mut Vec<String>,
) -> Option<RawConfig> {
    if seen.iter().any(|visited| visited == path) {
        return None;
    }
    seen.push(path.to_owned());
    let own = raw.get(path)?;
    let mut merged = RawConfig {
        dir: own.dir.clone(),
        extends: own.extends.clone(),
        references: own.references.clone(),
        ..RawConfig::default()
    };
    // Later `extends` entries override earlier ones, the config itself overrides them all.
    // Relative values stay relative to the config that declares them.
    let mut layers: Vec<RawConfig> = own
        .extends
        .iter()
        .filter_map(|target| config_path(raw, &own.dir, target))
        .filter_map(|parent| effective(raw, &parent, seen))
        .collect();
    let mut rooted = own.clone();
    rooted.paths_dir.clone_from(&own.dir);
    rooted.base_url = own
        .base_url
        .as_ref()
        .map(|base| normalize(&join(&own.dir, base)));
    layers.push(rooted);
    for layer in layers {
        if layer.paths.is_some() {
            merged.paths.clone_from(&layer.paths);
            merged.paths_dir.clone_from(&layer.paths_dir);
        }
        if let Some(base) = &layer.base_url {
            merged.base_url = Some(base.clone());
        }
        for (slot, value) in [
            (&mut merged.files, &layer.files),
            (&mut merged.include, &layer.include),
            (&mut merged.exclude, &layer.exclude),
        ] {
            if value.is_some() {
                slot.clone_from(value);
            }
        }
    }
    seen.pop();
    Some(merged)
}

fn raw_config(path: &str, manifest: &Manifest) -> RawConfig {
    let dir = parent_dir(path).to_owned();
    let mut config = RawConfig {
        dir,
        base_url: manifest.source_roots.first().cloned(),
        ..RawConfig::default()
    };
    for link in &manifest.project_links {
        let Some((key, value)) = link.split_once('=') else {
            continue;
        };
        match (key, value) {
            ("extends", _) => config.extends.push(value.to_owned()),
            ("reference", _) => config.references.push(value.to_owned()),
            ("has", "paths") => config.paths = Some(Vec::new()),
            ("has", "files") => config.files = Some(Vec::new()),
            ("has", "include") => config.include = Some(Vec::new()),
            ("has", "exclude") => config.exclude = Some(Vec::new()),
            ("file", _) => config.files.get_or_insert_default().push(value.to_owned()),
            ("include", _) => config
                .include
                .get_or_insert_default()
                .push(value.to_owned()),
            ("exclude", _) => config
                .exclude
                .get_or_insert_default()
                .push(value.to_owned()),
            _ => {}
        }
    }
    if let Some(paths) = &mut config.paths {
        for entry in &manifest.include_dirs {
            let Some((alias, target)) = entry.split_once('=') else {
                continue;
            };
            match paths.iter_mut().find(|(name, _)| name == alias) {
                Some((_, targets)) => targets.push(target.to_owned()),
                None => paths.push((alias.to_owned(), vec![target.to_owned()])),
            }
        }
    }
    config
}

/// Physical files behind the existing module lookup, before declaration grouping.
struct JsImportTarget {
    value_file: u32,
    type_file: u32,
}

pub fn tables(input: &Input) -> JsTables {
    let mut tables = JsTables::default();
    let mut raw = HashMap::new();
    for (record, facts) in input.files.iter().zip(input.facts) {
        // SvelteKit's default `$lib` alias; the generated tsconfig declaring it is not checked in.
        let name = role::file_name(&record.path);
        if name.starts_with("svelte.config.") {
            tables
                .svelte_dirs
                .insert(parent_dir(&record.path).to_owned());
        }
        let Some(manifest) = facts.as_ref().and_then(|facts| facts.manifest.as_ref()) else {
            continue;
        };
        let dir = parent_dir(&record.path);
        if is_project_config(name) {
            raw.insert(record.path.clone(), raw_config(&record.path, manifest));
            if name == "tsconfig.json"
                || (name == "jsconfig.json" && !tables.config_of_dir.contains_key(dir))
            {
                tables
                    .config_of_dir
                    .insert(dir.to_owned(), record.path.clone());
            }
        } else if name == "package.json"
            && let Some(package) = &manifest.module
        {
            let entry = manifest.source_roots.first().map_or_else(
                || join(dir, "src/index"),
                |main| normalize(&join(dir, main)),
            );
            tables
                .packages
                .push((package.clone(), entry, dir.to_owned()));
        }
    }
    for path in raw.keys() {
        let Some(config) = effective(&raw, path, &mut Vec::new()) else {
            continue;
        };
        tables.configs.insert(path.clone(), resolved(&raw, &config));
    }
    tables
}

/// Targets of `paths` are relative to the `baseUrl` in effect, else to the config declaring them.
fn resolved(raw: &HashMap<String, RawConfig>, config: &RawConfig) -> JsConfig {
    let base = config.base_url.as_deref().unwrap_or(&config.paths_dir);
    let aliases = config
        .paths
        .iter()
        .flatten()
        .map(|(alias, targets)| {
            let (prefix, suffix, wildcard) = match alias.split_once('*') {
                Some((prefix, suffix)) => (prefix.to_owned(), suffix.to_owned(), true),
                None => (alias.clone(), String::new(), false),
            };
            JsAlias {
                prefix,
                suffix,
                wildcard,
                targets: targets
                    .iter()
                    .map(|target| normalize(&join(base, target)))
                    .collect(),
            }
        })
        .collect();
    JsConfig {
        aliases,
        base_dir: config.base_url.clone(),
        references: config
            .references
            .iter()
            .filter_map(|target| config_path(raw, &config.dir, target))
            .collect(),
        scope: Some((
            config.dir.clone(),
            TypeScriptProjectConfig {
                files: config.files.clone(),
                include: config.include.clone(),
                exclude: config.exclude.clone(),
                out_dir: None,
                module_detection: None,
                module: None,
                jsx: None,
            },
        )),
    }
}

#[cfg(feature = "lang-typescript")]
fn project_holds(root: &str, scope: &TypeScriptProjectConfig, path: &str) -> bool {
    super::typescript::project_includes_file(root, scope, path) == Some(true)
}

#[cfg(not(feature = "lang-typescript"))]
fn project_holds(_: &str, _: &TypeScriptProjectConfig, _: &str) -> bool {
    false
}

impl JsTables {
    /// The config answering for `path`: the nearest ancestor config; with `references`, the
    /// referenced or own project that holds the file. `None` without a config, or when several
    /// projects with different aliases hold the file: then no alias choice is proven.
    fn config_for(&self, path: &str) -> Option<&JsConfig> {
        let mut dir = parent_dir(path);
        let nearest = loop {
            if let Some(config) = self.config_of_dir.get(dir) {
                break self.configs.get(config)?;
            }
            if dir.is_empty() {
                return None;
            }
            dir = parent_dir(dir);
        };
        if nearest.references.is_empty() {
            return Some(nearest);
        }
        let holds = |config: &&JsConfig| {
            config
                .scope
                .as_ref()
                .is_some_and(|(root, scope)| project_holds(root, scope, path))
        };
        let mut holding = nearest
            .references
            .iter()
            .filter_map(|reference| self.configs.get(reference))
            .chain(std::iter::once(nearest))
            .filter(holds);
        let Some(first) = holding.next() else {
            return Some(nearest);
        };
        holding
            .all(|other| other.aliases == first.aliases && other.base_dir == first.base_dir)
            .then_some(first)
    }

    /// Candidate paths of a non-relative specifier, as `tsc` selects them: the exact alias, else the
    /// wildcard with the longest prefix, and then only its targets, in order. Without a match the
    /// `baseUrl` applies. Nothing leaves the config of the importing file.
    fn alias_bases(&self, path: &str, spec: &str, bases: &mut Vec<String>) {
        let config = self.config_for(path);
        let aliases = config.map_or(&[][..], |config| &config.aliases[..]);
        let chosen = aliases
            .iter()
            .find(|alias| !alias.wildcard && alias.matched(spec).is_some())
            .or_else(|| {
                aliases
                    .iter()
                    .filter(|alias| alias.wildcard && alias.matched(spec).is_some())
                    .max_by_key(|alias| alias.prefix.len())
            });
        if let Some(alias) = chosen {
            let star = alias.matched(spec).unwrap_or_default();
            bases.extend(
                alias
                    .targets
                    .iter()
                    .map(|target| normalize(&target.replacen('*', star, 1))),
            );
            return;
        }
        if let Some(rest) = spec.strip_prefix("$lib/") {
            let mut dir = parent_dir(path);
            loop {
                if self.svelte_dirs.contains(dir) {
                    bases.push(normalize(&join(&join(dir, "src/lib"), rest)));
                    return;
                }
                if dir.is_empty() {
                    break;
                }
                dir = parent_dir(dir);
            }
        }
        if let Some(base) = config.and_then(|config| config.base_dir.as_deref()) {
            bases.push(normalize(&join(base, spec)));
        }
    }
}

impl Index<'_> {
    pub(super) fn js_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let typed_source = self.file_type(file) != Some(FileType::JavaScript);
        let only_types = |symbols: &[u32]| {
            typed_source
                && !symbols.is_empty()
                && symbols.iter().all(|&symbol| {
                    matches!(
                        self.symbols[symbol as usize].kind,
                        SymKind::Iface | SymKind::Alias
                    )
                })
        };
        // An empty spec is a local `export { a as b }`, which binds nothing in this file.
        for import in facts
            .imports
            .iter()
            .filter(|import| !import.spec.is_empty())
        {
            let Some(selected) = self.js_import_target(file, &import.spec) else {
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
            let target = self.module_of(selected.type_file);
            imports.modules.push(target);
            let mut type_dependency = import.type_only;
            let mut value_dependency = !import.type_only && import.names.is_empty();
            // `const util = require("./util")`: the alias names the whole module.
            if import.names.is_empty()
                && let Some(alias) = &import.alias
            {
                imports.names.insert(alias.clone(), Target::Module(target));
            }
            for name in &import.names {
                let local = import.alias.clone().unwrap_or_else(|| name.clone());
                let mut imported_type_only = false;
                match name.as_str() {
                    "" => {}
                    "*" if import.alias.is_none() => imports.globs.push(target),
                    "*" => {
                        imports.names.insert(local, Target::Module(target));
                    }
                    "default" => {
                        let found = self.default_export(target, &local);
                        imported_type_only =
                            found.as_ref().is_some_and(|symbols| only_types(symbols));
                        let bound = found.map_or(Target::Module(target), Target::Symbols);
                        imports.names.insert(local, bound);
                    }
                    name => {
                        let found = self.exported(target, name);
                        imported_type_only = only_types(&found);
                        if !found.is_empty() {
                            imports.names.insert(local, Target::Symbols(found));
                        }
                    }
                }
                if !import.type_only {
                    type_dependency |= imported_type_only;
                    value_dependency |= !imported_type_only;
                }
            }
            // TS value imports use the selected declaration for typing as well as
            // the emitted value file. A plain require call has no ES import binding.
            let require_binding = import.names.is_empty() && import.alias.is_some();
            type_dependency |= typed_source
                && value_dependency
                && !require_binding
                && selected.type_file != selected.value_file;
            if type_dependency {
                imports.files.push((file, selected.type_file));
            }
            if value_dependency && (!type_dependency || selected.value_file != selected.type_file) {
                imports.files.push((file, selected.value_file));
            }
        }
    }

    pub(super) fn js_module(&self, file: u32, spec: &str) -> Option<u32> {
        self.js_import_target(file, spec)
            .map(|target| self.module_of(target.type_file))
    }

    fn js_import_target(&self, file: u32, spec: &str) -> Option<JsImportTarget> {
        let dir = parent_dir(&self.input.files[file as usize].path);
        let typescript = self.file_type(file) != Some(FileType::JavaScript);
        let tables = &self.js_tables;
        let mut bases: Vec<String> = Vec::new();
        if spec.starts_with('.') {
            bases.push(normalize(&join(dir, spec)));
        } else {
            tables.alias_bases(&self.input.files[file as usize].path, spec, &mut bases);
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
            .then_some(JsImportTarget {
                value_file: target,
                type_file: target,
            })
        };
        let probe = |stem: &str, extensions: &[&str]| {
            let mut path = String::with_capacity(stem.len() + 12);
            path.push_str(stem);
            extensions.iter().find_map(|extension| {
                path.truncate(stem.len());
                path.push_str(extension);
                let mut selected = module_at(&path)?;
                // A declaration selected by extension substitution supplies types;
                // an existing runtime sibling supplies the value dependency. Explicit
                // declaration paths reach module_at directly and retain their identity.
                let runtime = match *extension {
                    ".d.ts" => &[".js", ".jsx"][..],
                    ".d.mts" => &[".mjs"][..],
                    ".d.cts" => &[".cjs"][..],
                    _ => &[],
                };
                for extension in runtime {
                    path.truncate(stem.len());
                    path.push_str(extension);
                    if let Some(found) = module_at(&path) {
                        selected.value_file = found.value_file;
                        break;
                    }
                }
                Some(selected)
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
        self.exported_at(module, name, 0, true)
    }

    pub(super) fn js_exported(&self, module: u32, name: &str) -> Vec<u32> {
        self.exported_at(module, name, 0, true)
    }

    fn exported_at(&self, module: u32, name: &str, depth: usize, declarations: bool) -> Vec<u32> {
        // TypeScript namespaces are module symbols that importers name.
        let own: Vec<u32> = self.module_symbol(module, name, None);
        if !own.is_empty() {
            return own;
        }
        if depth >= 8 {
            return if declarations {
                self.js_declaration_module(module)
                    .map_or_else(Vec::new, |declaration| {
                        self.module_symbol(declaration, name, None)
                    })
            } else {
                own
            };
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
                let target = if import.spec.is_empty() {
                    Some(module)
                } else {
                    self.js_module(file as u32, &import.spec)
                };
                let Some(target) = target else {
                    continue;
                };
                let original = import
                    .names
                    .iter()
                    .find(|imported| import.alias.as_deref().unwrap_or(imported) == name)
                    .map_or(name, String::as_str);
                let found = self.exported_at(target, original, depth + 1, declarations);
                if !found.is_empty() {
                    return found;
                }
            }
        }
        declarations
            .then(|| self.js_declaration_module(module))
            .flatten()
            .map_or_else(Vec::new, |declaration| {
                // Declaration context is not another re-export hop.
                self.exported_at(declaration, name, depth, false)
            })
    }

    /// Declaration context for a concrete runtime file, never another runtime sibling.
    pub(super) fn js_declaration_module(&self, module: u32) -> Option<u32> {
        let key = self.modules.modules[module as usize].key.as_str();
        let path = key.strip_prefix("js:")?;
        let (stem, declaration) = if let Some(stem) = path.strip_suffix(".js") {
            self.modules.get(key.strip_suffix(".js")?)?;
            (stem, ".d.ts")
        } else if let Some(stem) = path.strip_suffix(".jsx") {
            self.modules.get(key.strip_suffix(".jsx")?)?;
            (stem, ".d.ts")
        } else if let Some(stem) = path.strip_suffix(".mjs") {
            (stem, ".d.mts")
        } else {
            (path.strip_suffix(".cjs")?, ".d.cts")
        };
        let mut path = String::with_capacity(stem.len() + declaration.len());
        path.push_str(stem);
        path.push_str(declaration);
        let &file = self.path_index.get(path.as_str())?;
        let record = &self.input.files[file as usize];
        (record.role.is_extracted() && record.file_type == Some(FileType::TypeScript))
            .then(|| self.module_of(file))
    }

    /// Prefer a concrete default candidate before consulting its declaration context.
    fn default_export(&self, module: u32, local: &str) -> Option<Vec<u32>> {
        let named = self.exported_at(module, local, 0, false);
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
        match candidates.len() {
            1 => Some(candidates),
            0 => self
                .js_declaration_module(module)
                .and_then(|declaration| self.default_export(declaration, local)),
            _ => None,
        }
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
    #[cfg(feature = "lang-typescript")]
    fn runtime_symbols_do_not_merge_with_siblings_but_keep_api_context() -> std::io::Result<()> {
        use crate::domain::graph::EdgeKind;
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("javascript-runtime-symbols")?;
        root.write(
            "api.js",
            "export function execute() {}\nfunction factory() { return { contract() {} }; }\nconst { contract } = factory();\nexport { contract };\n",
        )?;
        root.write("api.jsx", "export function execute() {}\n")?;
        root.write(
            "api.d.ts",
            "export declare function execute(): void;\nexport declare function contract(): void;\n",
        )?;
        root.write("api.mjs", "export function execute() {}\n")?;
        root.write("api.cjs", "exports.execute = function execute() {};\n")?;
        root.write("default.js", "export default class Runtime {}\n")?;
        root.write("default.d.ts", "export default class Declared {}\n")?;
        root.write("view.js", "export class Failure { serialize() {} }\n")?;
        root.write(
            "view.d.ts",
            "export declare class Failure { serialize(): string; }\nexport declare class Headers { serialize(): string; }\n",
        )?;
        root.write(
            "barrel.js",
            "export { execute as forwarded } from './api.js';\n",
        )?;
        for depth in 0..=8 {
            let target = if depth == 0 {
                "api.js".to_owned()
            } else {
                format!("deep-{}.js", depth - 1)
            };
            root.write(
                &format!("deep-{depth}.js"),
                &format!("export * from './{target}';\n"),
            )?;
        }
        for (name, source) in [
            (
                "client-direct.js",
                "import { execute } from './api.js';\nexecute();\n",
            ),
            (
                "client-namespace.js",
                "import * as api from './api.js';\napi.execute();\n",
            ),
            (
                "client-forwarded.js",
                "import { forwarded } from './barrel.js';\nforwarded();\n",
            ),
            (
                "client-contract.js",
                "import * as api from './api.js';\napi.contract();\n",
            ),
            (
                "client-default.js",
                "import Declared from './default.js';\nnew Declared();\n",
            ),
            (
                "client-jsx.js",
                "import { execute } from './api.jsx';\nexecute();\n",
            ),
            (
                "client-unknown.js",
                "import { Failure } from './view.js';\nresponse.serialize();\n",
            ),
            (
                "client-deep.js",
                "import { contract } from './deep-7.js';\ncontract();\n",
            ),
            (
                "client-too-deep.js",
                "import { contract } from './deep-8.js';\ncontract();\n",
            ),
        ] {
            root.write(name, source)?;
        }
        let graph = super::super::tests::graph_of(root.path())?;
        let mut calls: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .filter(|edge| {
                edge.source as usize >= graph.symbols.len()
                    && graph.files[edge.source as usize - graph.symbols.len()]
                        .path
                        .starts_with("client-")
            })
            .map(|edge| {
                let target = &graph.symbols[edge.target as usize];
                (
                    graph.files[target.file as usize].path.as_str(),
                    target.name.as_str(),
                )
            })
            .collect();
        calls.sort_unstable();
        assert_eq!(
            calls,
            [
                ("api.d.ts", "contract"),
                ("api.d.ts", "contract"),
                ("api.js", "execute"),
                ("api.js", "execute"),
                ("api.js", "execute"),
                ("api.jsx", "execute"),
                ("default.js", "Runtime"),
            ]
        );
        let clients: Vec<_> = graph
            .files
            .iter()
            .filter(|file| file.path.starts_with("client-"))
            .map(|file| file.module)
            .collect();
        let declarations: Vec<_> = graph
            .files
            .iter()
            .filter(|file| file.path.ends_with(".d.ts"))
            .map(|file| file.module)
            .collect();
        assert!(
            !graph
                .module_edges
                .iter()
                .any(|edge| clients.contains(&edge.source) && declarations.contains(&edge.target))
        );
        Ok(())
    }

    #[cfg(feature = "lang-typescript")]
    #[test]
    fn typescript_value_import_can_be_named_type() -> std::io::Result<()> {
        use crate::domain::graph::EdgeKind;
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("typescript-import-named-type")?;
        root.write("api.ts", "export function type(): void {}\n")?;
        root.write(
            "entry.ts",
            "import { type as named } from './api.js';\nnamed();\n",
        )?;
        root.write(
            "barrel.ts",
            "export { type /* alias */ as named } from './api.js';\n",
        )?;
        root.write(
            "consumer.ts",
            "import { named as forwarded } from './barrel.js';\nforwarded();\n",
        )?;
        let graph = super::super::tests::graph_of(root.path())?;
        let targets: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Calls)
            .filter_map(|edge| graph.symbols.get(edge.target as usize))
            .map(|symbol| {
                (
                    graph.files[symbol.file as usize].path.as_str(),
                    symbol.name.as_str(),
                )
            })
            .collect();
        assert_eq!(targets, [("api.ts", "type"), ("api.ts", "type")]);
        Ok(())
    }

    #[test]
    #[cfg(feature = "lang-typescript")]
    fn imports_link_runtime_and_type_files_by_their_declared_role() -> std::io::Result<()> {
        use crate::domain::graph::EdgeKind;
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("javascript-import-roles")?;
        root.write(
            "api.js",
            "export function execute() {}\nexport function type() {}\n",
        )?;
        root.write(
            "api.d.ts",
            "export declare function execute(): void;\nexport declare function type(): void;\nexport interface Api {}\nexport type ApiAlias = Api;\n",
        )?;
        root.write(
            "runtime.js",
            "import { execute } from './api.js';\nexecute();\n",
        )?;
        root.write(
            "value.ts",
            "import { execute } from './api.js';\nexecute();\n",
        )?;
        root.write(
            "types.ts",
            "import type { Api } from './api.js';\nlet value: Api;\n",
        )?;
        root.write(
            "mixed.ts",
            "import { execute, type Api } from './api.js';\nlet value: Api;\nexecute();\n",
        )?;
        root.write(
            "barrel.ts",
            "export type { Api } from './api.js';\nexport { execute } from './api.js';\n",
        )?;
        for (name, source) in [
            (
                "named-type.ts",
                "import { type as named } from './api.js';\nnamed();\n",
            ),
            (
                "namespace-types.ts",
                "import type * as api from './api.js';\nlet value: api.Api;\n",
            ),
            (
                "namespace-values.ts",
                "import * as api from './api.js';\napi.execute();\n",
            ),
            (
                "empty.ts",
                "import type {} from './api.js'; import {} from './api.js';\n",
            ),
            (
                "decl-path.ts",
                "import type { Api } from './api.d.ts';\nlet value: Api;\n",
            ),
            (
                "only.ts",
                "import { Only } from './only-api';\nlet value: Only;\n",
            ),
            (
                "implicit-types.ts",
                "import { Api } from './api.js';\nlet value: Api;\n",
            ),
            (
                "implicit-alias.ts",
                "import { ApiAlias } from './api.js';\nlet value: ApiAlias;\n",
            ),
            (
                "implicit-mixed.ts",
                "import { execute, Api } from './api.js';\nlet value: Api;\nexecute();\n",
            ),
        ] {
            root.write(name, source)?;
        }
        root.write("only-api.d.ts", "export interface Only {}\n")?;
        let graph = super::super::tests::graph_of(root.path())?;
        let mut imports: Vec<_> = graph
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
        imports.sort_unstable();
        assert_eq!(
            imports,
            [
                ("barrel.ts", "api.d.ts"),
                ("barrel.ts", "api.js"),
                ("decl-path.ts", "api.d.ts"),
                ("empty.ts", "api.d.ts"),
                ("empty.ts", "api.js"),
                ("implicit-alias.ts", "api.d.ts"),
                ("implicit-mixed.ts", "api.d.ts"),
                ("implicit-mixed.ts", "api.js"),
                ("implicit-types.ts", "api.d.ts"),
                ("mixed.ts", "api.d.ts"),
                ("mixed.ts", "api.js"),
                ("named-type.ts", "api.d.ts"),
                ("named-type.ts", "api.js"),
                ("namespace-types.ts", "api.d.ts"),
                ("namespace-values.ts", "api.d.ts"),
                ("namespace-values.ts", "api.js"),
                ("only.ts", "only-api.d.ts"),
                ("runtime.js", "api.js"),
                ("types.ts", "api.d.ts"),
                ("value.ts", "api.d.ts"),
                ("value.ts", "api.js"),
            ]
        );
        Ok(())
    }

    #[test]
    fn javascript_commonjs_extensions_must_be_named() -> std::io::Result<()> {
        use crate::domain::graph::EdgeKind;
        use crate::infra::testdir::TestDir;

        let root = TestDir::new("javascript-commonjs-extensions")?;
        root.write("helper.cjs", "exports.work = function work() {};\n")?;
        root.write("helper.mjs", "export function work() {}\n")?;
        root.write("helper.d.cts", "export declare function work(): void;\n")?;
        root.write("helper.d.mts", "export declare function work(): void;\n")?;
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
