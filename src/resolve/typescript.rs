//! Source-proven TypeScript global interface groups.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use crate::domain::facts::{self, SourceIdentityKind, SymKind, TypeScriptProjectConfig};
use crate::domain::filetype::FileType;
use crate::domain::graph::FLAG_TYPESCRIPT_GLOBAL_INTERFACE;

#[allow(unused_imports)]
use super::{Index, modules};

struct Project {
    root: String,
    config: TypeScriptProjectConfig,
    direct_files: Vec<u32>,
    files: Vec<u32>,
    complete: bool,
}

type GlobalInterfaceKey = (Vec<String>, String);

struct GlobalInterfaceCandidates {
    groups: BTreeMap<GlobalInterfaceKey, Vec<u32>>,
    blocked: BTreeSet<GlobalInterfaceKey>,
}

impl Index<'_> {
    pub(super) fn link_typescript_global_interfaces(&mut self) -> bool {
        let mut projects = Vec::new();
        let mut unproven_roots = Vec::new();
        for (file, record) in self.input.files.iter().enumerate() {
            let name = record.path.rsplit('/').next().unwrap_or(&record.path);
            if name == "tsconfig.json" {
                let root = modules::parent_dir(&record.path).to_owned();
                let config = self
                    .facts(file as u32)
                    .and_then(|facts| {
                        facts.source_identity(SourceIdentityKind::TypeScriptProject, 0)
                    })
                    .and_then(facts::typescript_project_config);
                match config.filter(project_config_supported) {
                    Some(config) => projects.push(Project {
                        root,
                        config,
                        direct_files: Vec::new(),
                        files: Vec::new(),
                        complete: true,
                    }),
                    None => unproven_roots.push(root),
                }
            } else if name.starts_with("tsconfig.") && name.ends_with(".json") {
                unproven_roots.push(modules::parent_dir(&record.path).to_owned());
            }
        }
        if projects.is_empty() {
            return false;
        }

        let mut direct_owners = vec![Vec::new(); self.input.files.len()];
        for (project_index, project) in projects.iter_mut().enumerate() {
            for (file, record) in self.input.files.iter().enumerate() {
                if record.file_type != Some(FileType::TypeScript) {
                    continue;
                }
                match project_includes_file(&project.root, &project.config, &record.path) {
                    Some(true) => {
                        let Ok(file) = u32::try_from(file) else {
                            project.complete = false;
                            continue;
                        };
                        project.direct_files.push(file);
                        direct_owners[file as usize].push(project_index);
                    }
                    Some(false) => {}
                    None => project.complete = false,
                }
            }
            if project.direct_files.is_empty()
                || project.direct_files.iter().any(|&file| {
                    unproven_roots
                        .iter()
                        .any(|root| path_is_under(&self.input.files[file as usize].path, root))
                })
            {
                project.complete = false;
            }
        }
        for owners in &direct_owners {
            if owners.len() > 1 {
                for &owner in owners {
                    projects[owner].complete = false;
                }
            }
        }

        let mut files_by_module: HashMap<u32, Vec<u32>> = HashMap::default();
        for (file, record) in self.input.files.iter().enumerate() {
            if matches!(
                record.file_type,
                Some(
                    FileType::TypeScript
                        | FileType::JavaScript
                        | FileType::Vue
                        | FileType::Svelte
                        | FileType::Astro
                )
            ) && let Ok(file) = u32::try_from(file)
            {
                files_by_module
                    .entry(self.module_of(file))
                    .or_default()
                    .push(file);
            }
        }
        for (project_index, project) in projects.iter_mut().enumerate() {
            if !project.complete {
                continue;
            }
            let mut visited: HashSet<u32> = HashSet::default();
            let mut pending: VecDeque<u32> = project.direct_files.iter().copied().collect();
            while let Some(file) = pending.pop_front() {
                if !visited.insert(file) {
                    continue;
                }
                let record = &self.input.files[file as usize];
                if unproven_roots
                    .iter()
                    .any(|root| path_is_under(&record.path, root))
                {
                    project.complete = false;
                    break;
                }
                let Some(imports) = self.file_imports.get(file as usize) else {
                    project.complete = false;
                    break;
                };
                let Some(facts) = self.facts(file) else {
                    project.complete = false;
                    break;
                };
                if facts.imports.iter().any(|import| {
                    import.spec.starts_with('.') && self.js_module(file, &import.spec).is_none()
                }) {
                    project.complete = false;
                    break;
                }
                for &(source, target) in &imports.files {
                    if source != file || self.file_type(target) != Some(FileType::TypeScript) {
                        continue;
                    }
                    let target_module = self.module_of(target);
                    if files_by_module
                        .get(&target_module)
                        .is_none_or(|siblings| siblings.len() != 1)
                        || direct_owners[target as usize]
                            .iter()
                            .any(|&owner| owner != project_index)
                    {
                        project.complete = false;
                        break;
                    }
                    pending.push_back(target);
                }
                if !project.complete {
                    break;
                }
            }
            project.files = visited.into_iter().collect();
            project.files.sort_unstable();
        }

        let mut closure_owners: HashMap<u32, Vec<usize>> = HashMap::default();
        for (project_index, project) in projects.iter().enumerate() {
            if project.complete {
                for &file in &project.files {
                    closure_owners.entry(file).or_default().push(project_index);
                }
            }
        }
        for owners in closure_owners.values().filter(|owners| owners.len() > 1) {
            for &owner in owners {
                projects[owner].complete = false;
            }
        }
        let mut grouped = false;
        for project in projects.iter().filter(|project| project.complete) {
            let Some(GlobalInterfaceCandidates { groups, blocked }) =
                self.global_interface_candidates(project)
            else {
                continue;
            };
            for (key, mut parts) in groups {
                parts.sort_unstable();
                parts.dedup();
                if parts.len() < 2 || blocked.contains(&key) {
                    continue;
                }
                let files: HashSet<_> = parts
                    .iter()
                    .filter_map(|&part| self.symbols.get(part as usize).map(|symbol| symbol.file))
                    .collect();
                let has_global_augmentation = parts.iter().any(|&part| {
                    let Some(symbol) = self.symbols.get(part as usize) else {
                        return false;
                    };
                    let Some(base) = self.base.get(symbol.file as usize) else {
                        return false;
                    };
                    part.checked_sub(*base)
                        .and_then(|local| {
                            self.facts(symbol.file).and_then(|facts| {
                                facts.source_identity(
                                    SourceIdentityKind::TypeScriptGlobalAugmentation,
                                    local,
                                )
                            })
                        })
                        .is_some()
                });
                if files.len() < 2 && !has_global_augmentation {
                    continue;
                }
                let mut headers = Vec::with_capacity(parts.len());
                let mut members = Vec::with_capacity(parts.len());
                let mut complete = true;
                for &part in &parts {
                    let Some(symbol) = self.symbols.get(part as usize) else {
                        complete = false;
                        break;
                    };
                    let file = symbol.file;
                    let Some(local) = self
                        .base
                        .get(file as usize)
                        .and_then(|base| part.checked_sub(*base))
                    else {
                        complete = false;
                        break;
                    };
                    let Some(facts) = self.facts(file) else {
                        complete = false;
                        break;
                    };
                    let (Some(header), Some(member)) = (
                        facts.source_identity(SourceIdentityKind::InterfaceHeader, local),
                        facts.source_identity(SourceIdentityKind::InterfaceMembers, local),
                    ) else {
                        complete = false;
                        break;
                    };
                    headers.push(header);
                    members.push(member);
                }
                if !complete {
                    continue;
                }
                if !crate::extract::lang::typescript::compatible_global_interface_declarations(
                    &headers, &members,
                ) {
                    continue;
                }
                let canonical = parts[0];
                for part in parts {
                    self.interface_canonical.insert(part, canonical);
                    if let Some(symbol) = self.symbols.get_mut(part as usize) {
                        symbol.flags |= FLAG_TYPESCRIPT_GLOBAL_INTERFACE;
                    }
                }
                grouped = true;
            }
        }
        grouped
    }

    fn global_interface_candidates(&self, project: &Project) -> Option<GlobalInterfaceCandidates> {
        let mut candidates = BTreeMap::new();
        let mut blocked = BTreeSet::new();
        for &file in &project.files {
            let facts = self.facts(file)?;
            let scope = facts.source_identity(SourceIdentityKind::TypeScriptScope, 0)?;
            if scope == "incomplete" {
                return None;
            }
            if facts
                .symbols
                .iter()
                .any(|symbol| symbol.kind == Some(SymKind::Mod) && symbol.name.contains('\\'))
            {
                return None;
            }
            let syntax = facts.source_identity(SourceIdentityKind::TypeScriptModuleSyntax, 0)?;
            if syntax == "incomplete" {
                return None;
            }
            let global_scope =
                facts.source_identity(SourceIdentityKind::TypeScriptGlobalAugmentationScope, 0);
            if global_scope == Some("unproven") {
                return None;
            }
            if syntax == "declaration" {
                for (local, symbol) in facts.symbols.iter().enumerate() {
                    let Some(namespace) = namespace_path(facts, local) else {
                        continue;
                    };
                    blocked.insert((namespace, symbol.name.clone()));
                }
                continue;
            }
            let is_module = self.typescript_module_kind(project, file, syntax)?;
            let global_parts: HashSet<u32> = facts
                .source_identities
                .as_deref()?
                .iter()
                .filter(|identity| {
                    identity.kind == SourceIdentityKind::TypeScriptGlobalAugmentation
                })
                .map(|identity| identity.index)
                .collect();
            if !global_parts.is_empty() && !is_module {
                return None;
            }
            if global_scope == Some("complete") && !is_module {
                return None;
            }
            for (local, symbol) in facts.symbols.iter().enumerate() {
                let is_global_augmentation = global_parts.contains(&u32::try_from(local).ok()?);
                if is_module && !is_global_augmentation || !is_module && is_global_augmentation {
                    continue;
                }
                let Some(namespace) = namespace_path(facts, local) else {
                    continue;
                };
                if !is_global_augmentation && !namespace.is_empty() && !symbol.exported {
                    continue;
                }
                let key = (namespace, symbol.name.clone());
                if symbol.name.contains('\\') {
                    return None;
                }
                if symbol.kind != Some(SymKind::Iface) {
                    if symbol.kind().is_type() {
                        blocked.insert(key);
                    }
                    continue;
                }
                let Ok(local) = u32::try_from(local) else {
                    return None;
                };
                if facts
                    .source_identity(SourceIdentityKind::InterfaceHeader, local)
                    .is_none()
                    || facts
                        .source_identity(SourceIdentityKind::InterfaceMembers, local)
                        .is_none()
                {
                    blocked.insert(key);
                    continue;
                }
                let part = self.base.get(file as usize)?.checked_add(local)?;
                candidates.entry(key).or_insert_with(Vec::new).push(part);
            }
        }
        Some(GlobalInterfaceCandidates {
            groups: candidates,
            blocked,
        })
    }

    fn typescript_module_kind(&self, project: &Project, file: u32, syntax: &str) -> Option<bool> {
        if !compiler_source_path(&self.input.files.get(file as usize)?.path) {
            return None;
        }
        if syntax == "indicator" {
            return Some(true);
        }
        let mode = project.config.module_detection.as_deref().unwrap_or("auto");
        match mode {
            "force" => Some(true),
            "legacy" => Some(false),
            "auto" => {
                let path = &self.input.files.get(file as usize)?.path;
                if path.ends_with(".mts") || path.ends_with(".cts") {
                    return Some(true);
                }
                if syntax == "jsx"
                    && matches!(
                        project.config.jsx.as_deref(),
                        Some("react-jsx" | "react-jsxdev")
                    )
                {
                    return Some(true);
                }
                match project.config.module.as_deref() {
                    Some("node16" | "node18" | "node20" | "nodenext") => {
                        Some(self.typescript_package_type(file)? == "module")
                    }
                    _ => Some(false),
                }
            }
            _ => None,
        }
    }

    fn typescript_package_type(&self, file: u32) -> Option<&str> {
        let path = &self.input.files.get(file as usize)?.path;
        let mut directory = modules::parent_dir(path).to_owned();
        loop {
            let package = if directory.is_empty() {
                "package.json".to_owned()
            } else {
                format!("{directory}/package.json")
            };
            if let Some(&package_file) = self.path_index.get(package.as_str()) {
                return self
                    .facts(package_file)?
                    .source_identity(SourceIdentityKind::TypeScriptPackageType, 0)
                    .and_then(facts::typescript_package_type);
            }
            if directory.is_empty() {
                return None;
            }
            directory = modules::parent_dir(&directory).to_owned();
        }
    }
}

fn project_config_supported(config: &TypeScriptProjectConfig) -> bool {
    let paths_supported = config
        .files
        .iter()
        .chain(config.include.iter())
        .chain(config.exclude.iter())
        .flatten()
        .all(|path| normalize_project_path(path).is_some());
    paths_supported
        && config
            .include
            .iter()
            .chain(config.exclude.iter())
            .flatten()
            .all(|pattern| pattern_supported(pattern))
        && config
            .out_dir
            .as_deref()
            .is_none_or(|path| normalize_project_path(path).is_some())
}

fn project_includes_file(root: &str, config: &TypeScriptProjectConfig, path: &str) -> Option<bool> {
    let Some(relative) = path_relative_to_root(path, root) else {
        return Some(false);
    };
    if relative.is_empty() {
        return Some(false);
    }
    if config.files.as_ref().is_some_and(|files| {
        files.iter().any(|file| {
            normalize_project_path(file).is_some_and(|file| join_project_path(root, &file) == path)
        })
    }) {
        return Some(true);
    }
    let included = match config.include.as_deref() {
        Some(patterns) => {
            let mut included = false;
            for pattern in patterns {
                included |= project_pattern_matches(pattern, relative)?;
            }
            included
        }
        None if config.files.is_none() => {
            default_typescript_extension(path) && !default_excluded(relative, config)?
        }
        None => false,
    };
    if !included || default_excluded(relative, config)? {
        return Some(false);
    }
    for pattern in config.exclude.as_deref().unwrap_or_default() {
        if project_pattern_matches(pattern, relative)? {
            return Some(false);
        }
    }
    Some(true)
}

fn default_excluded(path: &str, config: &TypeScriptProjectConfig) -> Option<bool> {
    if path
        .split('/')
        .any(|part| matches!(part, "node_modules" | "bower_components" | "jspm_packages"))
    {
        return Some(true);
    }
    if config.exclude.is_some() {
        return Some(false);
    }
    let Some(out_dir) = config.out_dir.as_deref() else {
        return Some(false);
    };
    let out_dir = normalize_project_path(out_dir)?;
    Some(path_is_under(path, &out_dir))
}

fn project_pattern_matches(pattern: &str, path: &str) -> Option<bool> {
    let pattern = normalize_project_path(pattern)?;
    if pattern.is_empty() {
        return Some(true);
    }
    let has_glob = pattern
        .chars()
        .any(|character| matches!(character, '*' | '?'));
    if !has_glob {
        if path == pattern {
            return Some(true);
        }
        return Some(!compiler_source_path(&pattern) && path_is_under(path, &pattern));
    }
    let pattern_parts: Vec<_> = pattern.split('/').collect();
    let path_parts: Vec<_> = path.split('/').collect();
    glob_path_matches(&pattern_parts, &path_parts)
}

fn pattern_supported(pattern: &str) -> bool {
    normalize_project_path(pattern).is_some()
        && !pattern
            .chars()
            .any(|character| matches!(character, '[' | ']' | '{' | '}' | '!'))
}

fn normalize_project_path(path: &str) -> Option<String> {
    if path.starts_with('/')
        || path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn path_relative_to_root<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if root.is_empty() {
        return Some(path);
    }
    path.strip_prefix(root)?.strip_prefix('/')
}

fn join_project_path(root: &str, relative: &str) -> String {
    if root.is_empty() {
        relative.to_owned()
    } else if relative.is_empty() {
        root.to_owned()
    } else {
        format!("{root}/{relative}")
    }
}

fn path_is_under(path: &str, root: &str) -> bool {
    root.is_empty()
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn default_typescript_extension(path: &str) -> bool {
    [".ts", ".tsx", ".mts", ".cts"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

fn compiler_source_path(path: &str) -> bool {
    default_typescript_extension(path) || declaration_path(path)
}

fn declaration_path(path: &str) -> bool {
    [".d.ts", ".d.mts", ".d.cts"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

fn glob_path_matches(pattern: &[&str], path: &[&str]) -> Option<bool> {
    let mut matched = vec![false; path.len() + 1];
    matched[0] = true;
    for part in pattern {
        let mut next = vec![false; path.len() + 1];
        if *part == "**" {
            let mut prefix = false;
            for index in 0..=path.len() {
                prefix |= matched[index];
                next[index] = prefix;
            }
        } else {
            for index in 1..=path.len() {
                if matched[index - 1] && glob_segment_matches(part, path[index - 1]) {
                    next[index] = true;
                }
            }
        }
        matched = next;
    }
    Some(matched[path.len()])
}

fn glob_segment_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<_> = pattern.chars().collect();
    let value: Vec<_> = value.chars().collect();
    let mut matched = vec![vec![false; value.len() + 1]; pattern.len() + 1];
    matched[0][0] = true;
    for (pattern_index, character) in pattern.iter().enumerate() {
        for value_index in 0..=value.len() {
            if !matched[pattern_index][value_index] {
                continue;
            }
            match character {
                '*' => {
                    matched[pattern_index + 1][value_index] = true;
                    if value_index < value.len() {
                        matched[pattern_index][value_index + 1] = true;
                    }
                }
                '?' if value_index < value.len() => {
                    matched[pattern_index + 1][value_index + 1] = true;
                }
                literal if value.get(value_index) == Some(literal) => {
                    matched[pattern_index + 1][value_index + 1] = true;
                }
                _ => {}
            }
        }
    }
    matched[pattern.len()][value.len()]
}

fn namespace_path(facts: &facts::Facts, symbol: usize) -> Option<Vec<String>> {
    let symbol = facts.symbols.get(symbol)?;
    let mut parent = symbol.parent;
    let mut scopes = Vec::new();
    let mut remaining = facts.symbols.len();
    while let Some(index) = parent {
        if remaining == 0 {
            return None;
        }
        remaining -= 1;
        let scope = facts.symbols.get(index as usize)?;
        if scope.kind != Some(SymKind::Mod)
            || scope.name.starts_with('"')
            || scope.name.starts_with('\'')
        {
            return None;
        }
        let parts: Vec<_> = scope.name.split('.').map(str::trim).collect();
        if parts.iter().any(|part| part.is_empty()) {
            return None;
        }
        scopes.push(parts.into_iter().map(str::to_owned).collect::<Vec<_>>());
        parent = scope.parent;
    }
    scopes.reverse();
    Some(scopes.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::{project_includes_file, project_pattern_matches};
    use crate::domain::facts::TypeScriptProjectConfig;

    #[test]
    fn explicit_tsconfig_excludes_replace_implicit_outdir_exclusion() {
        let mut config = TypeScriptProjectConfig {
            files: None,
            include: Some(vec!["**/*.ts".to_owned()]),
            exclude: None,
            out_dir: Some("out".to_owned()),
            module_detection: None,
            module: None,
            jsx: None,
        };
        assert_eq!(
            project_includes_file("", &config, "out/generated.ts"),
            Some(false)
        );

        config.exclude = Some(Vec::new());
        assert_eq!(
            project_includes_file("", &config, "out/generated.ts"),
            Some(true)
        );
        assert_eq!(
            project_includes_file("", &config, "node_modules/package/index.ts"),
            Some(false)
        );

        config.exclude = Some(vec!["src/ignored".to_owned()]);
        assert_eq!(
            project_includes_file("", &config, "out/generated.ts"),
            Some(true)
        );

        config.out_dir = Some(".".to_owned());
        config.exclude = Some(Vec::new());
        assert_eq!(project_includes_file("", &config, "index.ts"), Some(true));
    }

    #[test]
    fn typescript_project_membership_respects_patterns_exclusions_and_files() {
        let config = TypeScriptProjectConfig {
            files: None,
            include: Some(vec!["src/**/*.ts".to_owned()]),
            exclude: Some(vec!["src/generated".to_owned()]),
            out_dir: Some("dist".to_owned()),
            module_detection: None,
            module: None,
            jsx: None,
        };
        assert_eq!(
            project_includes_file("packages/app", &config, "packages/app/src/ui/page.ts"),
            Some(true)
        );
        assert_eq!(
            project_includes_file(
                "packages/app",
                &config,
                "packages/app/src/generated/types.ts"
            ),
            Some(false)
        );
        assert_eq!(
            project_includes_file("packages/app", &config, "packages/app/dist/types.ts"),
            Some(false)
        );
        assert_eq!(
            project_includes_file("packages/app", &config, "packages/apps/src/page.ts"),
            Some(false)
        );

        let explicit = TypeScriptProjectConfig {
            files: Some(vec!["src/generated/types.ts".to_owned()]),
            ..config
        };
        assert_eq!(
            project_includes_file(
                "packages/app",
                &explicit,
                "packages/app/src/generated/types.ts"
            ),
            Some(true)
        );
    }

    #[test]
    fn typescript_include_globs_match_recursive_and_single_path_segments() {
        assert_eq!(project_pattern_matches("**/*.ts", "index.ts"), Some(true));
        assert_eq!(
            project_pattern_matches("src/**/*.t?", "src/components/card.ts"),
            Some(true)
        );
        assert_eq!(
            project_pattern_matches("src/*.ts", "src/nested/card.ts"),
            Some(false)
        );
    }
}
