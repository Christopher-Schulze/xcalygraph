//! Module units per language (`docs/spec.md` section 6, module unit column).

use std::collections::HashMap;

use crate::domain::corpus::FileRecord;
use crate::domain::facts::Facts;
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
    /// Scope of method lookup by owner type: the crate for Rust, else the module itself.
    pub method_scope: String,
    pub file_type: Option<FileType>,
}

pub struct ModuleMap {
    pub modules: Vec<ModuleInfo>,
    pub file_module: Vec<u32>,
    pub by_key: HashMap<String, u32>,
    /// Go module paths (from `go.mod`) with their directory, longest first.
    pub go_modules: Vec<(String, String)>,
    /// Rust crates: directory and crate name.
    pub crates: Vec<(String, String)>,
}

impl ModuleMap {
    pub fn get(&self, key: &str) -> Option<u32> {
        self.by_key.get(key).copied()
    }
}

pub fn assign(files: &[FileRecord], facts: &[Option<Facts>]) -> ModuleMap {
    let mut go_modules: Vec<(String, String)> = Vec::new();
    let mut crates: Vec<(String, String)> = Vec::new();
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
            }
            _ => {}
        }
    }
    go_modules.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.cmp(b)));
    crates.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.cmp(b)));
    let mut map = ModuleMap {
        modules: Vec::new(),
        file_module: Vec::with_capacity(files.len()),
        by_key: HashMap::new(),
        go_modules,
        crates,
    };
    for (record, facts) in files.iter().zip(facts) {
        let info = module_of(&map, record, facts.as_ref());
        let id = match map.by_key.get(&info.key) {
            Some(&id) => id,
            None => {
                let id = map.modules.len() as u32;
                map.by_key.insert(info.key.clone(), id);
                map.modules.push(info);
                id
            }
        };
        map.file_module.push(id);
    }
    map
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
            let path = rust_module_path(relative);
            let key = if path.is_empty() {
                format!("{crate_name}::{}", rust_target(relative))
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

/// Module path of a crate-relative Rust file: `src/a/b.rs` and `src/a/b/mod.rs` -> `a::b`;
/// crate roots (`src/lib.rs`, `src/main.rs`, `src/bin/x.rs`, `tests/x.rs`) -> empty.
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
}
