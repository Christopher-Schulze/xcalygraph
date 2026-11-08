//! Module units per language (`docs/spec.md` section 6, module unit column).

use std::collections::{HashMap, HashSet};

use crate::domain::corpus::FileRecord;
use crate::domain::facts::{self, Facts};
use crate::domain::filetype::FileType;
use crate::domain::role::{self, Role};

/// One module unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleInfo {
    /// Resolution key: Go import path, Rust `crate::path`, else the directory.
    pub key: String,
    /// Display name in output and qualified names.
    pub name: String,
    /// Separator of qualified names (`.` or `::`).
    pub separator: &'static str,
    /// Scope of method lookup by owner type: the Cargo target for Rust when identified, else the
    /// crate; for other languages, the module itself.
    pub method_scope: String,
    pub file_type: Option<FileType>,
}

pub struct ModuleMap {
    pub modules: Vec<ModuleInfo>,
    pub file_module: Vec<u32>,
    /// Files of each module in ascending file order; the inverse of `file_module`.
    module_files: Vec<Vec<u32>>,
    pub by_key: HashMap<String, u32>,
    /// Go module paths (from `go.mod`) with their directory, longest first.
    pub go_modules: Vec<(String, String)>,
    /// Rust crates: directory and crate name.
    pub crates: Vec<(String, String)>,
    rust_target_roots: HashMap<String, CargoRootKind>,
    /// Exact file target for each Rust `#[path] mod` reference, keyed by (source file, ref index).
    pub rust_path_targets: HashMap<(u32, u32), u32>,
    /// Module target for each unambiguous Rust `#[path] mod` declaration.
    pub rust_path_modules: HashMap<(u32, u32), u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CargoRootKind {
    Library,
    Target,
    Ambiguous,
}

impl ModuleMap {
    pub fn get(&self, key: &str) -> Option<u32> {
        self.by_key.get(key).copied()
    }

    /// Files of `module` in ascending order.
    pub fn files_of(&self, module: u32) -> &[u32] {
        self.module_files
            .get(module as usize)
            .map_or(&[], Vec::as_slice)
    }

    /// The explicit Cargo target root outside `src/` whose directory holds `path`.
    pub fn rust_target_root_of(&self, path: &str) -> Option<&str> {
        self.rust_target_roots
            .keys()
            .filter(|root| {
                let directory = parent_dir(root);
                !rust_conventional_source_dir(self, directory)
                    && path.starts_with(&format!("{directory}/"))
            })
            .max_by_key(|root| root.len())
            .map(String::as_str)
    }

    pub fn rust_path_target(&self, file: u32, reference: u32) -> Option<u32> {
        self.rust_path_targets.get(&(file, reference)).copied()
    }

    pub fn rust_path_module(&self, file: u32, reference: u32) -> Option<u32> {
        self.rust_path_modules.get(&(file, reference)).copied()
    }
}

pub fn assign(files: &[FileRecord], facts: &[Option<Box<Facts>>]) -> ModuleMap {
    let mut go_modules: Vec<(String, String)> = Vec::new();
    let mut crates: Vec<(String, String)> = Vec::new();
    let mut rust_target_roots = HashMap::new();
    for (record, facts) in files.iter().zip(facts) {
        let Some(manifest) = facts.as_ref().and_then(|facts| facts.manifest.as_ref()) else {
            continue;
        };
        let dir = parent_dir(&record.path).to_owned();
        match role::file_name(&record.path) {
            "go.mod" => {
                if let Some(module) = &manifest.module {
                    go_modules.push((module.clone(), dir));
                }
            }
            "Cargo.toml" => {
                if let Some(name) = &manifest.module {
                    crates.push((dir, name.replace('-', "_")));
                }
                for root in &manifest.rust_target_roots {
                    let Some(path) = append_root_relative(parent_dir(&record.path), &root.path)
                    else {
                        continue;
                    };
                    let kind = if root.is_library {
                        CargoRootKind::Library
                    } else {
                        CargoRootKind::Target
                    };
                    if rust_target_roots.insert(path.clone(), kind).is_some() {
                        rust_target_roots.insert(path, CargoRootKind::Ambiguous);
                    }
                }
            }
            _ => {}
        }
    }
    go_modules.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.cmp(b)));
    crates.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.cmp(b)));
    let mut map = ModuleMap {
        modules: Vec::new(),
        file_module: Vec::with_capacity(files.len()),
        module_files: Vec::new(),
        by_key: HashMap::new(),
        go_modules,
        crates,
        rust_target_roots,
        rust_path_targets: HashMap::new(),
        rust_path_modules: HashMap::new(),
    };
    let rust_path_refs = rust_module_path_refs(facts);
    let (root_scopes, target_scopes) = rust_target_method_scopes(files, &map);
    let base_infos: Vec<ModuleInfo> = files
        .iter()
        .zip(facts)
        .map(|(record, facts)| module_of(&map, record, facts.as_deref()))
        .collect();
    if rust_path_refs.is_empty() {
        for (record, mut info) in files.iter().zip(base_infos) {
            apply_rust_target_method_scope(record, &map, &root_scopes, &target_scopes, &mut info);
            let id = insert_module(&mut map, info);
            push_file_module(&mut map, id);
        }
        return map;
    }
    map.rust_path_targets =
        resolve_rust_path_targets(files, facts, &map, &base_infos, &rust_path_refs);
    let (mut infos, valid_modules) = rust_module_overrides(
        files,
        &map,
        &base_infos,
        &rust_path_refs,
        &map.rust_path_targets,
    );
    for (record, info) in files.iter().zip(&mut infos) {
        apply_rust_target_method_scope(record, &map, &root_scopes, &target_scopes, info);
    }
    for info in infos {
        let id = insert_module(&mut map, info);
        push_file_module(&mut map, id);
    }
    for reference in &rust_path_refs {
        let (Ok(source), Ok(reference_index)) = (
            u32::try_from(reference.source),
            u32::try_from(reference.reference),
        ) else {
            continue;
        };
        let Some(target) = map.rust_path_target(source, reference_index) else {
            continue;
        };
        let target = target as usize;
        if valid_modules.get(target).copied().unwrap_or(false)
            && base_infos
                .get(target)
                .is_some_and(|info| info.file_type == Some(FileType::Rust))
            && let Some(module) = map.file_module.get(target).copied()
        {
            map.rust_path_modules
                .insert((source, reference_index), module);
        }
    }
    map
}

/// Finds independent Cargo target scopes without treating the library and default binary roots as
/// separate aliases of a shared physical module tree.
fn rust_target_method_scopes(
    files: &[FileRecord],
    map: &ModuleMap,
) -> (
    HashMap<String, Option<String>>,
    HashMap<String, Option<String>>,
) {
    let mut roots = HashMap::new();
    let mut targets = HashMap::new();
    for (path, kind) in &map.rust_target_roots {
        if *kind == CargoRootKind::Target {
            record_rust_target_scope(map, path, &mut roots, &mut targets);
        }
    }
    for record in files {
        if record.file_type != Some(FileType::Rust)
            || map.rust_target_roots.contains_key(&record.path)
        {
            continue;
        }
        let (crate_dir, _) = rust_crate(map, &record.path);
        if !map.crates.iter().any(|(dir, _)| dir == &crate_dir) {
            continue;
        }
        let relative = record
            .path
            .strip_prefix(crate_dir.as_str())
            .unwrap_or(&record.path)
            .trim_start_matches('/');
        if rust_is_crate_root(relative) && !matches!(relative, "lib.rs" | "main.rs") {
            record_rust_target_scope(map, &record.path, &mut roots, &mut targets);
        }
    }
    (roots, targets)
}

fn record_rust_target_scope(
    map: &ModuleMap,
    path: &str,
    roots: &mut HashMap<String, Option<String>>,
    targets: &mut HashMap<String, Option<String>>,
) {
    let (crate_dir, crate_name) = rust_crate(map, path);
    if !map.crates.iter().any(|(dir, _)| dir == &crate_dir) {
        return;
    }
    let relative = path
        .strip_prefix(crate_dir.as_str())
        .unwrap_or(path)
        .trim_start_matches('/');
    let target = rust_target(relative);
    if target.is_empty() {
        return;
    }
    let scope = format!("{crate_name}::target:{path}");
    record_unique_scope(roots, path.to_owned(), scope.clone());
    record_unique_scope(targets, format!("{crate_dir}\0{target}"), scope);
}

fn record_unique_scope(scopes: &mut HashMap<String, Option<String>>, key: String, scope: String) {
    if let Some(existing) = scopes.get_mut(&key) {
        if existing.as_deref() != Some(scope.as_str()) {
            *existing = None;
        }
    } else {
        scopes.insert(key, Some(scope));
    }
}

fn apply_rust_target_method_scope(
    record: &FileRecord,
    map: &ModuleMap,
    roots: &HashMap<String, Option<String>>,
    targets: &HashMap<String, Option<String>>,
    info: &mut ModuleInfo,
) {
    if record.file_type != Some(FileType::Rust) || info.method_scope.starts_with("file:") {
        return;
    }
    match roots.get(&record.path) {
        Some(Some(scope)) => {
            info.method_scope.clone_from(scope);
            return;
        }
        Some(None) => return,
        None => {}
    }
    let (crate_dir, _) = rust_crate(map, &record.path);
    if !map.crates.iter().any(|(dir, _)| dir == &crate_dir) {
        return;
    }
    let relative = record
        .path
        .strip_prefix(crate_dir.as_str())
        .unwrap_or(&record.path)
        .trim_start_matches('/');
    let mut prefix = rust_target(relative);
    let mut found = None;
    loop {
        let key = format!("{crate_dir}\0{prefix}");
        if let Some(scope) = targets.get(&key) {
            let Some(scope) = scope else {
                return;
            };
            if found.is_some() {
                return;
            }
            found = Some(scope);
        }
        let Some((parent, _)) = prefix.rsplit_once("::") else {
            break;
        };
        prefix = parent.to_owned();
    }
    if let Some(scope) = found {
        info.method_scope.clone_from(scope);
    }
}

/// Assign the next file to `module`, keeping both directions of the mapping.
fn push_file_module(map: &mut ModuleMap, module: u32) {
    let file = map.file_module.len() as u32;
    map.file_module.push(module);
    if map.module_files.len() <= module as usize {
        map.module_files.resize_with(module as usize + 1, Vec::new);
    }
    map.module_files[module as usize].push(file);
}

fn insert_module(map: &mut ModuleMap, info: ModuleInfo) -> u32 {
    if let Some(&id) = map.by_key.get(&info.key) {
        return id;
    }
    let id = map.modules.len() as u32;
    map.by_key.insert(info.key.clone(), id);
    map.modules.push(info);
    id
}

#[derive(Clone, Copy)]
struct RustModulePathRef<'a> {
    source: usize,
    reference: usize,
    path: &'a str,
    module_suffix: &'a str,
    inline_path: &'a str,
}

fn rust_module_path_refs(facts: &[Option<Box<Facts>>]) -> Vec<RustModulePathRef<'_>> {
    let mut references = Vec::new();
    for (source, file_facts) in facts.iter().enumerate() {
        let Some(file_facts) = file_facts else {
            continue;
        };
        for (reference, fact) in file_facts.refs.iter().enumerate() {
            if fact.kind != facts::RefKind::Path {
                continue;
            }
            let Some(qualifier) = fact.qualifier.as_deref() else {
                continue;
            };
            let Some((module_suffix, inline_path)) = facts::rust_module_path_ref_context(qualifier)
            else {
                continue;
            };
            references.push(RustModulePathRef {
                source,
                reference,
                path: &fact.name,
                module_suffix,
                inline_path,
            });
        }
    }
    references
}

fn resolve_rust_path_targets(
    files: &[FileRecord],
    facts: &[Option<Box<Facts>>],
    map: &ModuleMap,
    base_infos: &[ModuleInfo],
    references: &[RustModulePathRef<'_>],
) -> HashMap<(u32, u32), u32> {
    if references.is_empty() {
        return HashMap::new();
    }
    let path_index: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(index, record)| (record.path.as_str(), index))
        .collect();
    let mut path_loaded = vec![false; files.len()];
    let mut seen = HashSet::new();
    seen.insert(Vec::<usize>::new());

    for _ in 0..=references.len() {
        let targets =
            resolve_rust_path_targets_once(files, map, references, &path_index, &path_loaded);
        let mut next_loaded = vec![false; files.len()];
        for target in targets.values() {
            let index = *target as usize;
            if base_infos
                .get(index)
                .is_some_and(|info| info.file_type == Some(FileType::Rust))
                && facts.get(index).is_some_and(Option::is_some)
            {
                next_loaded[index] = true;
            }
        }
        if next_loaded == path_loaded {
            return targets;
        }
        let state: Vec<usize> = next_loaded
            .iter()
            .enumerate()
            .filter_map(|(index, loaded)| loaded.then_some(index))
            .collect();
        if !seen.insert(state) {
            return resolve_rust_path_targets_direct(files, references, &path_index);
        }
        path_loaded = next_loaded;
    }
    resolve_rust_path_targets_direct(files, references, &path_index)
}

fn resolve_rust_path_targets_once(
    files: &[FileRecord],
    map: &ModuleMap,
    references: &[RustModulePathRef<'_>],
    path_index: &HashMap<&str, usize>,
    path_loaded: &[bool],
) -> HashMap<(u32, u32), u32> {
    let mut targets = HashMap::new();
    for reference in references {
        let Some(source) = files.get(reference.source) else {
            continue;
        };
        let inline = reference.module_suffix.contains("::");
        let base = if inline {
            let Some(module_dir) = rust_source_module_directory(
                map,
                &source.path,
                path_loaded.get(reference.source).copied().unwrap_or(false),
            ) else {
                continue;
            };
            let Some(inline_dir) = append_root_relative(&module_dir, reference.inline_path) else {
                continue;
            };
            inline_dir
        } else {
            parent_dir(&source.path).to_owned()
        };
        let Some(path) = append_root_relative(&base, reference.path) else {
            continue;
        };
        let Some(target) = path_index.get(path.as_str()) else {
            continue;
        };
        if let (Ok(source), Ok(reference), Ok(target)) = (
            u32::try_from(reference.source),
            u32::try_from(reference.reference),
            u32::try_from(*target),
        ) {
            targets.insert((source, reference), target);
        }
    }
    targets
}

fn resolve_rust_path_targets_direct(
    files: &[FileRecord],
    references: &[RustModulePathRef<'_>],
    path_index: &HashMap<&str, usize>,
) -> HashMap<(u32, u32), u32> {
    let mut targets = HashMap::new();
    for reference in references
        .iter()
        .filter(|reference| !reference.module_suffix.contains("::"))
    {
        let Some(source) = files.get(reference.source) else {
            continue;
        };
        let base = parent_dir(&source.path);
        let Some(path) = append_root_relative(base, reference.path) else {
            continue;
        };
        let Some(target) = path_index.get(path.as_str()) else {
            continue;
        };
        if let (Ok(source), Ok(reference), Ok(target)) = (
            u32::try_from(reference.source),
            u32::try_from(reference.reference),
            u32::try_from(*target),
        ) {
            targets.insert((source, reference), target);
        }
    }
    targets
}

fn rust_source_module_directory(map: &ModuleMap, path: &str, path_loaded: bool) -> Option<String> {
    if path_loaded {
        return Some(parent_dir(path).to_owned());
    }
    let (crate_dir, _) = rust_crate(map, path);
    let relative = path
        .strip_prefix(crate_dir.as_str())
        .unwrap_or(path)
        .trim_start_matches('/');
    let file_name = relative.rsplit('/').next().unwrap_or(relative);
    if file_name == "mod.rs" || rust_is_root(map, path, relative) {
        return Some(parent_dir(path).to_owned());
    }
    let stem = file_name.strip_suffix(".rs")?;
    let stem = stem.strip_prefix("r#").unwrap_or(stem);
    Some(join_path(parent_dir(path), stem))
}

fn rust_is_crate_root(relative: &str) -> bool {
    if matches!(
        relative,
        "lib.rs" | "main.rs" | "build.rs" | "src/lib.rs" | "src/main.rs"
    ) {
        return true;
    }
    let parts: Vec<&str> = relative.split('/').collect();
    let Some(file_name) = parts.last().copied() else {
        return false;
    };
    if !file_name.ends_with(".rs") {
        return false;
    }
    (parts.len() == 3 && parts[0] == "src" && parts[1] == "bin")
        || (parts.len() == 2 && matches!(parts[0], "tests" | "examples" | "benches"))
        || (parts.len() == 4 && parts[0] == "src" && parts[1] == "bin" && file_name == "main.rs")
}

fn rust_is_root(map: &ModuleMap, path: &str, relative: &str) -> bool {
    rust_is_crate_root(relative) || map.rust_target_roots.contains_key(path)
}

fn rust_root_target(map: &ModuleMap, path: &str, relative: &str) -> Option<String> {
    match map.rust_target_roots.get(path) {
        Some(CargoRootKind::Library) => Some(String::new()),
        Some(CargoRootKind::Target) => Some(rust_target(relative)),
        Some(CargoRootKind::Ambiguous) => None,
        None => rust_is_crate_root(relative).then(|| rust_target(relative)),
    }
}

fn append_root_relative(base: &str, relative: &str) -> Option<String> {
    if relative.starts_with('/') || relative.contains('\0') {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn rust_module_overrides(
    files: &[FileRecord],
    map: &ModuleMap,
    base_infos: &[ModuleInfo],
    references: &[RustModulePathRef<'_>],
    path_targets: &HashMap<(u32, u32), u32>,
) -> (Vec<ModuleInfo>, Vec<bool>) {
    let mut incoming = vec![Vec::<(usize, String)>::new(); files.len()];
    for reference in references {
        let (Ok(source), Ok(reference_index)) = (
            u32::try_from(reference.source),
            u32::try_from(reference.reference),
        ) else {
            continue;
        };
        let Some(target) = path_targets.get(&(source, reference_index)) else {
            continue;
        };
        let target = *target as usize;
        if base_infos
            .get(target)
            .is_some_and(|info| info.file_type == Some(FileType::Rust))
            && let Some(candidates) = incoming.get_mut(target)
        {
            candidates.push((reference.source, reference.module_suffix.to_owned()));
        }
    }
    for candidates in &mut incoming {
        candidates.sort_unstable();
        candidates.dedup();
    }

    let mut parents = vec![None; files.len()];
    let mut suffixes = vec![None; files.len()];
    let mut ambiguous = vec![false; files.len()];
    for (target, candidates) in incoming.iter().enumerate() {
        match candidates.as_slice() {
            [(parent, suffix)] => {
                parents[target] = Some(*parent);
                suffixes[target] = Some(suffix.clone());
            }
            [] => {}
            _ => ambiguous[target] = true,
        }
    }
    for (file, record) in files.iter().enumerate() {
        if matches!(
            map.rust_target_roots.get(&record.path),
            Some(CargoRootKind::Ambiguous)
        ) {
            ambiguous[file] = true;
        }
    }

    let base_logical: Vec<String> = files
        .iter()
        .map(|record| {
            let (crate_dir, _) = rust_crate(map, &record.path);
            let relative = record
                .path
                .strip_prefix(crate_dir.as_str())
                .unwrap_or(&record.path)
                .trim_start_matches('/');
            if rust_is_root(map, &record.path, relative) {
                String::new()
            } else {
                rust_module_path_in(map, &record.path, relative)
            }
        })
        .collect();
    let (mut infos, logical_paths, mut valid) = {
        let mut resolver = RustModuleInfoResolver {
            base_infos,
            base_logical: &base_logical,
            parents: &parents,
            suffixes: &suffixes,
            ambiguous: &ambiguous,
            state: vec![0; files.len()],
            effective: vec![None; files.len()],
            valid: vec![false; files.len()],
        };
        let mut infos = Vec::with_capacity(files.len());
        let mut logical_paths = Vec::with_capacity(files.len());
        for file in 0..files.len() {
            let (info, logical, _) = resolver.resolve(file);
            infos.push(info);
            logical_paths.push(logical);
        }
        (infos, logical_paths, resolver.valid)
    };

    let mut path_target_files = HashSet::new();
    let mut path_claims = HashMap::<String, HashSet<usize>>::new();
    for reference in references {
        let (Ok(source), Ok(reference_index)) = (
            u32::try_from(reference.source),
            u32::try_from(reference.reference),
        ) else {
            continue;
        };
        let Some(target) = path_targets.get(&(source, reference_index)) else {
            continue;
        };
        let target = *target as usize;
        if !base_infos
            .get(target)
            .is_some_and(|info| info.file_type == Some(FileType::Rust))
        {
            continue;
        }
        path_target_files.insert(target);
        if valid.get(reference.source).copied().unwrap_or(false)
            && let (Some(parent), Some(logical)) = (
                infos.get(reference.source),
                logical_paths.get(reference.source),
            )
        {
            let logical = append_rust_module_path(logical, reference.module_suffix);
            let key = format!("{}::{logical}", parent.method_scope);
            path_claims.entry(key).or_default().insert(target);
        }
    }
    for targets in path_claims.values().filter(|targets| targets.len() > 1) {
        for &target in targets {
            valid[target] = false;
        }
    }
    for (file, info) in infos.iter_mut().enumerate() {
        let owns_unique_path = path_claims
            .get(&info.key)
            .is_some_and(|targets| targets.len() == 1 && targets.contains(&file));
        if owns_unique_path
            || (!path_target_files.contains(&file) && !path_claims.contains_key(&info.key))
        {
            continue;
        }
        let physical_module = format!("file:{}", files[file].path);
        info.key = physical_module.clone();
        info.name = physical_module.clone();
        info.method_scope = physical_module;
    }
    (infos, valid)
}

struct RustModuleInfoResolver<'a> {
    base_infos: &'a [ModuleInfo],
    base_logical: &'a [String],
    parents: &'a [Option<usize>],
    suffixes: &'a [Option<String>],
    ambiguous: &'a [bool],
    state: Vec<u8>,
    effective: Vec<Option<(ModuleInfo, String)>>,
    valid: Vec<bool>,
}

impl RustModuleInfoResolver<'_> {
    fn resolve(&mut self, file: usize) -> (ModuleInfo, String, bool) {
        if self.state[file] == 2 {
            if let Some((info, logical)) = &self.effective[file] {
                return (info.clone(), logical.clone(), self.valid[file]);
            }
            return (
                self.base_infos[file].clone(),
                self.base_logical[file].clone(),
                false,
            );
        }
        if self.state[file] == 1 {
            return (
                self.base_infos[file].clone(),
                self.base_logical[file].clone(),
                false,
            );
        }
        self.state[file] = 1;

        let (info, logical, is_valid) = if self.ambiguous[file] {
            (
                self.base_infos[file].clone(),
                self.base_logical[file].clone(),
                false,
            )
        } else if let (Some(parent), Some(suffix)) =
            (self.parents[file], self.suffixes[file].clone())
        {
            let (parent_info, parent_logical, parent_valid) = self.resolve(parent);
            if parent_valid {
                let logical = append_rust_module_path(&parent_logical, &suffix);
                let key = format!("{}::{logical}", parent_info.method_scope);
                (
                    ModuleInfo {
                        key,
                        name: logical.clone(),
                        separator: "::",
                        method_scope: parent_info.method_scope,
                        file_type: Some(FileType::Rust),
                    },
                    logical,
                    true,
                )
            } else {
                (
                    self.base_infos[file].clone(),
                    self.base_logical[file].clone(),
                    false,
                )
            }
        } else {
            (
                self.base_infos[file].clone(),
                self.base_logical[file].clone(),
                true,
            )
        };

        self.state[file] = 2;
        self.valid[file] = is_valid;
        self.effective[file] = Some((info.clone(), logical.clone()));
        (info, logical, is_valid)
    }
}

fn append_rust_module_path(parent: &str, suffix: &str) -> String {
    if parent.is_empty() {
        suffix.to_owned()
    } else {
        format!("{parent}::{suffix}")
    }
}

fn module_of(map: &ModuleMap, record: &FileRecord, facts: Option<&Facts>) -> ModuleInfo {
    let dir = parent_dir(&record.path);
    let extracted_code = record.role.is_extracted() && record.role != Role::Manifest;
    match record.file_type {
        Some(
            FileType::TypeScript
            | FileType::JavaScript
            | FileType::Vue
            | FileType::Svelte
            | FileType::Astro,
        ) if extracted_code => {
            let key = super::js::module_key(&record.path);
            ModuleInfo {
                name: super::js::strip_extension(&record.path).to_owned(),
                method_scope: key.clone(),
                key,
                separator: ".",
                file_type: record.file_type,
            }
        }
        Some(FileType::Bash) if extracted_code => ModuleInfo {
            key: format!("file:{}", record.path),
            name: record.path.clone(),
            separator: ".",
            method_scope: format!("file:{}", record.path),
            file_type: Some(FileType::Bash),
        },
        Some(FileType::Python) if extracted_code => {
            let dotted = python_module(&record.path);
            ModuleInfo {
                key: format!("py:{dotted}"),
                method_scope: format!("py:{dotted}"),
                name: dotted,
                separator: ".",
                file_type: Some(FileType::Python),
            }
        }
        Some(FileType::Java | FileType::Kotlin | FileType::Scala | FileType::Groovy)
            if extracted_code =>
        {
            // The package declaration, shared across files, directories and JVM languages.
            let package = facts
                .and_then(|facts| facts.package.clone())
                .unwrap_or_default();
            let key = super::jvm::package_key(&package);
            ModuleInfo {
                name: if package.is_empty() {
                    ".".to_owned()
                } else {
                    package
                },
                method_scope: key.clone(),
                key,
                separator: ".",
                file_type: record.file_type,
            }
        }
        Some(FileType::CSharp | FileType::VbNet) if extracted_code => {
            // The namespace declaration, shared across files, directories and both languages.
            let namespace = facts
                .and_then(|facts| facts.package.clone())
                .unwrap_or_default();
            let key = super::dotnet::namespace_key(&namespace);
            ModuleInfo {
                name: if namespace.is_empty() {
                    ".".to_owned()
                } else {
                    namespace
                },
                method_scope: key.clone(),
                key,
                separator: ".",
                file_type: record.file_type,
            }
        }
        Some(FileType::C | FileType::Cpp | FileType::ObjC) if extracted_code => {
            // Every file is a module; methods of a class may be defined in another file (`.h`
            // and `.cpp`), so method lookup spans the root.
            let key = super::cfamily::file_key(&record.path);
            ModuleInfo {
                name: record
                    .path
                    .rsplit_once('.')
                    .map_or(record.path.as_str(), |(stem, _)| stem)
                    .to_owned(),
                method_scope: "cfamily".to_owned(),
                key,
                separator: if record.file_type == Some(FileType::Cpp) {
                    "::"
                } else {
                    "."
                },
                file_type: record.file_type,
            }
        }
        Some(FileType::Swift) if extracted_code => {
            // SwiftPM target (`Sources/<T>/`, `Tests/<T>/`), else the top-level directory.
            let segments: Vec<&str> = record.path.split('/').collect();
            let target = segments
                .windows(2)
                .find(|pair| matches!(pair[0], "Sources" | "Tests") && segments.len() > 2)
                .map(|pair| pair[1])
                .or_else(|| (segments.len() > 1).then(|| segments[0]))
                .unwrap_or(".");
            let key = format!("swift:{target}");
            ModuleInfo {
                name: target.to_owned(),
                method_scope: key.clone(),
                key,
                separator: ".",
                file_type: Some(FileType::Swift),
            }
        }
        Some(FileType::Php) if extracted_code => {
            // The namespace declaration, independent of directories (PSR-4 maps them anyway).
            let namespace = facts
                .and_then(|facts| facts.package.clone())
                .unwrap_or_default();
            let key = format!("php:{namespace}");
            ModuleInfo {
                name: if namespace.is_empty() {
                    ".".to_owned()
                } else {
                    namespace.replace('\\', ".")
                },
                method_scope: key.clone(),
                key,
                separator: ".",
                file_type: Some(FileType::Php),
            }
        }
        Some(
            FileType::Elixir
            | FileType::Erlang
            | FileType::Lua
            | FileType::Zig
            | FileType::Julia
            | FileType::OCaml
            | FileType::Fortran
            | FileType::Pascal
            | FileType::Lisp
            | FileType::Solidity
            | FileType::PowerShell
            | FileType::Robot,
        ) if extracted_code => {
            // One module per file: Erlang modules are named by the file stem (`m:f()`), OCaml
            // by the capitalized stem (`store.ml` and `store.mli` are `Store`), Elixir and
            // Julia modules are resolved by their `module` names, Lua and Zig files are tables
            // or structs bound by `require` and `@import`.
            let stem = record
                .path
                .rsplit_once('.')
                .map_or(record.path.as_str(), |(stem, _)| stem);
            let file_name = stem.rsplit('/').next().unwrap_or(stem);
            let key = match record.file_type {
                Some(FileType::Erlang) => format!("erl:{file_name}"),
                Some(FileType::OCaml) => format!("ocaml:{}", capitalize(file_name)),
                // Units are named by their file; names are case-insensitive.
                Some(FileType::Pascal) if is_pascal_form(&record.path) => {
                    format!("file:{}", record.path)
                }
                Some(FileType::Pascal) => format!("pas:{}", file_name.to_ascii_lowercase()),
                // Files of one `in-package` form one module.
                Some(FileType::Lisp) => match facts.and_then(|facts| facts.package.as_deref()) {
                    Some(package) => format!("lisp:{}", lisp_package(package)),
                    None => format!("file:{}", record.path),
                },
                _ => format!("file:{}", record.path),
            };
            ModuleInfo {
                name: stem.to_owned(),
                method_scope: key.clone(),
                key,
                separator: ".",
                file_type: record.file_type,
            }
        }
        Some(FileType::Dart) if extracted_code => {
            let key = format!("dart:{}", record.path);
            ModuleInfo {
                name: record.path.trim_end_matches(".dart").to_owned(),
                method_scope: "dart".to_owned(),
                key,
                separator: ".",
                file_type: Some(FileType::Dart),
            }
        }
        Some(FileType::Go) if extracted_code => {
            let key = go_import_path(map, dir);
            ModuleInfo {
                name: if dir.is_empty() {
                    ".".to_owned()
                } else {
                    dir.to_owned()
                },
                method_scope: key.clone(),
                key,
                separator: ".",
                file_type: Some(FileType::Go),
            }
        }
        Some(FileType::Rust) if extracted_code => {
            let (crate_dir, crate_name) = rust_crate(map, &record.path);
            let relative = record
                .path
                .strip_prefix(crate_dir.as_str())
                .unwrap_or(&record.path)
                .trim_start_matches('/');
            let is_root = rust_is_root(map, &record.path, relative);
            let path = if is_root {
                String::new()
            } else {
                rust_module_path_in(map, &record.path, relative)
            };
            let target = rust_root_target(map, &record.path, relative);
            if is_root && target.is_none() {
                let key = format!("file:{}", record.path);
                return ModuleInfo {
                    name: record.path.clone(),
                    method_scope: key.clone(),
                    key,
                    separator: "::",
                    file_type: Some(FileType::Rust),
                };
            }
            let key = if path.is_empty() {
                format!(
                    "{crate_name}::{}",
                    target.unwrap_or_else(|| rust_target(relative))
                )
            } else {
                format!("{crate_name}::{path}")
            };
            ModuleInfo {
                name: if path.is_empty() {
                    crate_name.clone()
                } else {
                    path
                },
                method_scope: crate_name,
                key,
                separator: "::",
                file_type: Some(FileType::Rust),
            }
        }
        file_type => ModuleInfo {
            key: format!("dir:{dir}"),
            name: if dir.is_empty() {
                ".".to_owned()
            } else {
                dir.to_owned()
            },
            method_scope: format!("dir:{dir}"),
            separator: ".",
            file_type: file_type.filter(|_| extracted_code),
        },
    }
}

/// Go import path of a package directory, from the nearest `go.mod`.
pub fn go_import_path(map: &ModuleMap, dir: &str) -> String {
    for (module, module_dir) in &map.go_modules {
        if module_dir.is_empty() {
            return join_path(module, dir);
        }
        if dir == module_dir {
            return module.clone();
        }
        if let Some(rest) = dir
            .strip_prefix(module_dir.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return join_path(module, rest);
        }
    }
    format!("dir:{dir}")
}

fn join_path(base: &str, rest: &str) -> String {
    if rest.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{rest}")
    }
}

/// Crate directory and name of a Rust file; without a `Cargo.toml` the crate is `crate`.
pub fn rust_crate(map: &ModuleMap, path: &str) -> (String, String) {
    for (dir, name) in &map.crates {
        if dir.is_empty() || path.starts_with(&format!("{dir}/")) {
            return (dir.clone(), name.clone());
        }
    }
    (String::new(), "crate".to_owned())
}

/// Module path of a Rust file: below an explicit Cargo target root outside `src/`
/// (`[[bin]] path = "crates/core/main.rs"`) the root's directory is the module root.
fn rust_module_path_in(map: &ModuleMap, path: &str, relative: &str) -> String {
    let root_directory = map
        .rust_target_roots
        .keys()
        .map(|root| parent_dir(root))
        .filter(|directory| {
            !directory.is_empty()
                && !rust_conventional_source_dir(map, directory)
                && path.starts_with(&format!("{directory}/"))
        })
        .max_by_key(|directory| directory.len());
    match root_directory {
        Some(directory) => {
            let inner = path[directory.len() + 1..].trim_end_matches(".rs");
            let inner = inner.strip_suffix("/mod").unwrap_or(inner);
            inner.replace('/', "::")
        }
        None => rust_module_path(relative),
    }
}

fn rust_conventional_source_dir(map: &ModuleMap, directory: &str) -> bool {
    let crate_dir = map
        .crates
        .iter()
        .map(|(dir, _)| dir.as_str())
        .find(|dir| {
            dir.is_empty()
                || directory == *dir
                || directory
                    .strip_prefix(*dir)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .unwrap_or("");
    directory
        .strip_prefix(crate_dir)
        .unwrap_or(directory)
        .trim_start_matches('/')
        == "src"
}

/// Module path from conventional Rust file layout; explicit Cargo target roots are handled by the
/// module map.
pub fn rust_module_path(relative: &str) -> String {
    let Some(inner) = relative.strip_prefix("src/") else {
        return String::new();
    };
    if inner.starts_with("bin/") {
        return String::new();
    }
    let inner = inner.trim_end_matches(".rs");
    let inner = inner.strip_suffix("/mod").unwrap_or(inner);
    if matches!(inner, "lib" | "main") {
        return String::new();
    }
    inner.replace('/', "::")
}

/// Distinguishes crate roots that share a crate name (library, binaries, integration tests).
fn rust_target(relative: &str) -> String {
    match relative {
        "src/lib.rs" | "src/main.rs" => String::new(),
        other => other.trim_end_matches(".rs").replace('/', "::"),
    }
}

/// Dotted Python module of a file: source root `src/` stripped, `__init__` names the package.
pub fn python_module(path: &str) -> String {
    let path = path.strip_prefix("src/").unwrap_or(path);
    let path = path.trim_end_matches(".pyi").trim_end_matches(".py");
    let path = path.strip_suffix("/__init__").unwrap_or(path);
    if path == "__init__" {
        return String::new();
    }
    path.replace('/', ".")
}

pub fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `store_db` -> `Store_db` (OCaml file module names).
fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// `:store`, `#:store`, `"STORE"` -> `store`.
pub(super) fn lisp_package(name: &str) -> String {
    name.trim_start_matches('#')
        .trim_start_matches(':')
        .trim_matches('"')
        .to_ascii_lowercase()
}

/// Delphi and Lazarus form files (`.dfm`, `.lfm`), which are not units.
fn is_pascal_form(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".dfm") || lower.ends_with(".lfm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_modules_follow_packages_and_source_roots() {
        assert_eq!(python_module("pkg/store/db.py"), "pkg.store.db");
        assert_eq!(python_module("src/pkg/__init__.py"), "pkg");
        assert_eq!(python_module("tools/run.pyi"), "tools.run");
    }

    #[test]
    fn rust_module_paths_follow_file_layout() {
        let cases = [
            ("src/lib.rs", ""),
            ("src/main.rs", ""),
            ("src/store/mod.rs", "store"),
            ("src/infra/store/codec.rs", "infra::store::codec"),
            ("src/bin/tool.rs", ""),
            ("tests/cli.rs", ""),
            ("build.rs", ""),
        ];
        for (path, want) in cases {
            assert_eq!(rust_module_path(path), want, "{path}");
        }
    }

    #[test]
    fn rust_alias_claims_do_not_resolve_to_layout_decoys() -> std::io::Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/resolve/rust-path-alias-decoys");
        let graph = super::super::tests::graph_of(&root)?;
        for alias in ["first", "second"] {
            let decoy = format!("file:src/{alias}.rs::Thing");
            assert!(
                graph.symbols.iter().any(|symbol| symbol.qualified == decoy),
                "missing quarantined decoy {decoy}"
            );
            let wrong = format!("{alias}::Thing");
            assert!(
                graph.symbols.iter().all(|symbol| symbol.qualified != wrong),
                "a layout decoy acquired claimed module {wrong}"
            );
        }
        Ok(())
    }
}
