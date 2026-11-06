//! C, C++ and Objective-C module rules (`docs/spec.md` section 6): every file is a module;
//! `#include` and `#import` resolve relative to the file, the root, manifest include directories
//! (`CMakeLists.txt`, `compile_commands.json`) and `include/`, `src/`; included headers are
//! visible transitively; references to declarations prefer their definition, the header's paired
//! implementation file first; C++ `ns::name` and `Type::member` resolve by qualified suffix.

use super::Ids;
use super::{Imports, Index, modules};
use crate::domain::facts::{FLAG_DECLARATION, Facts, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::has_qualified_suffix;

/// Transitive include depth and the most headers made visible to one file.
const MAX_INCLUDE_DEPTH: usize = 6;
const MAX_VISIBLE_FILES: usize = 256;

pub(super) fn is_cfamily(file_type: Option<FileType>) -> bool {
    matches!(
        file_type,
        Some(FileType::C | FileType::Cpp | FileType::ObjC)
    )
}

/// Module key of a C-family file.
pub(super) fn file_key(path: &str) -> String {
    format!("file:{path}")
}

/// Root-relative include directories from every manifest: CMake directories relative to their
/// `CMakeLists.txt`, absolute `compile_commands.json` directories mapped onto the longest root
/// directory they end with.
pub(super) fn include_dirs(
    files: &[crate::domain::corpus::FileRecord],
    facts: &[Option<Box<Facts>>],
) -> Vec<String> {
    let mut root_dirs: Vec<&str> = files
        .iter()
        .map(|file| modules::parent_dir(&file.path))
        .collect();
    root_dirs.sort_unstable();
    root_dirs.dedup();
    let mut dirs: Vec<String> = Vec::new();
    for (file, facts) in files.iter().zip(facts) {
        let Some(manifest) = facts.as_ref().and_then(|facts| facts.manifest.as_ref()) else {
            continue;
        };
        let base = modules::parent_dir(&file.path);
        for dir in &manifest.include_dirs {
            let relative = if dir.starts_with('/') {
                root_dirs
                    .iter()
                    .filter(|candidate| {
                        !candidate.is_empty()
                            && dir
                                .trim_end_matches('/')
                                .ends_with(&format!("/{candidate}"))
                    })
                    .max_by_key(|candidate| candidate.len())
                    .map(|candidate| (*candidate).to_owned())
            } else {
                Some(normalize(&join(base, dir)))
            };
            if let Some(relative) = relative
                && !dirs.contains(&relative)
            {
                dirs.push(relative);
            }
        }
    }
    dirs
}

fn join(base: &str, path: &str) -> String {
    match (base.is_empty(), path) {
        (_, ".") => base.to_owned(),
        (true, _) => path.to_owned(),
        _ => format!("{base}/{path}"),
    }
}

/// Resolves `.` and `..` segments.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

impl Index<'_> {
    /// Files an include of `file` names, if they are in the root.
    pub(super) fn include_target(&self, file: u32, spec: &str) -> Option<u32> {
        let (system, path) = match spec.strip_prefix("sys:") {
            Some(path) => (true, path),
            None => (false, spec),
        };
        let dir = modules::parent_dir(&self.input.files[file as usize].path);
        let mut candidates: Vec<String> = Vec::new();
        if !system {
            candidates.push(normalize(&join(dir, path)));
            candidates.push(normalize(path));
        }
        candidates.extend(
            self.include_dirs
                .iter()
                .map(|include| normalize(&join(include, path))),
        );
        if !system {
            candidates.push(normalize(&format!("include/{path}")));
            candidates.push(normalize(&format!("src/{path}")));
        }
        candidates
            .iter()
            .find_map(|candidate| self.path_index.get(candidate.as_str()).copied())
            .or_else(|| self.include_by_suffix(system, path))
            .filter(|&target| target != file)
    }

    /// Header search paths the tree does not declare (Xcode targets, framework-style
    /// `<Kit/Header.h>`): the unique file whose path ends with the include path. System includes
    /// qualify only with a directory part, so `<string.h>` never binds to a local `string.h`.
    fn include_by_suffix(&self, system: bool, path: &str) -> Option<u32> {
        let path = normalize(path);
        if system && !path.contains('/') {
            return None;
        }
        let name = path.rsplit('/').next()?;
        let suffix = format!("/{path}");
        let mut found = self
            .name_index
            .get(name)?
            .iter()
            .copied()
            .filter(|&candidate| {
                let candidate_path = self.input.files[candidate as usize].path.as_str();
                candidate_path == path || candidate_path.ends_with(&suffix)
            });
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Direct includes as files, external system headers as dependencies, the transitive closure
    /// of included files as visible modules.
    pub(super) fn cfamily_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            match self.include_target(file, &import.spec) {
                Some(target) => imports.files.push((file, target)),
                None => {
                    // `<boost/x.hpp>` names a dependency; `<stdio.h>` is the platform.
                    if let Some(path) = import.spec.strip_prefix("sys:")
                        && let Some((library, _)) = path.split_once('/')
                        && !matches!(
                            library,
                            "sys" | "bits" | "linux" | "arpa" | "netinet" | "net" | "mach"
                        )
                    {
                        imports.externals.push(library.to_owned());
                    }
                }
            }
        }
        let mut visible: Ids = Ids::new();
        let mut frontier: Ids = smallvec::smallvec![file];
        for _ in 0..MAX_INCLUDE_DEPTH {
            let mut next = Vec::new();
            for &current in &frontier {
                for &target in &self.includes[current as usize] {
                    if target != file
                        && !visible.contains(&target)
                        && visible.len() < MAX_VISIBLE_FILES
                    {
                        visible.push(target);
                        next.push(target);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next.into();
        }
        for target in visible {
            let module = self.module_of(target);
            imports.globs.push(module);
            imports.modules.push(module);
        }
    }

    /// The definition a reference to a declaration means: the single definition of the same name,
    /// kind and owner, preferring the declaring header's paired implementation file.
    pub(super) fn definition_of(&self, declaration: u32) -> Option<u32> {
        let symbol = self.symbols.get(declaration as usize)?;
        if symbol.flags & FLAG_DECLARATION == 0 {
            return None;
        }
        let owner = self.owner_name(declaration);
        let definitions: Ids = self
            .by_name
            .get(symbol.name.as_str())
            .into_iter()
            .flatten()
            .copied()
            .filter(|&candidate| {
                let other = &self.symbols[candidate as usize];
                other.flags & FLAG_DECLARATION == 0
                    && matches!(other.kind, SymKind::Fn | SymKind::Method)
                    && is_cfamily(self.file_type(other.file))
                    && self.owner_name(candidate) == owner
            })
            .collect();
        match definitions.as_slice() {
            [] => None,
            [only] => Some(*only),
            several => {
                // Overloads: the definition with the declaration's parameter count.
                let count = |candidate: u32| {
                    let other = &self.symbols[candidate as usize];
                    super::parameter_count(&other.signature, &other.name).map(|(_, total, _)| total)
                };
                let declared = count(declaration);
                let several: Ids = several
                    .iter()
                    .copied()
                    .filter(|&candidate| count(candidate) == declared)
                    .collect();
                if let [only] = several.as_slice() {
                    return Some(*only);
                }
                let stem = stem(&self.input.files[symbol.file as usize].path);
                let paired: Ids = several
                    .iter()
                    .copied()
                    .filter(|&candidate| {
                        stem == self::stem(
                            &self.input.files[self.symbols[candidate as usize].file as usize].path,
                        )
                    })
                    .collect();
                (paired.len() == 1).then(|| paired[0])
            }
        }
    }

    /// Owning type name of a member: its parent type or its `Type::` qualifier.
    pub(super) fn owner_name(&self, symbol: u32) -> String {
        let symbol_record = &self.symbols[symbol as usize];
        if symbol_record.parent != super::NONE {
            let parent = &self.symbols[symbol_record.parent as usize];
            if parent.kind.is_type() {
                return parent.name.to_string();
            }
        }
        self.facts(symbol_record.file)
            .and_then(|facts| {
                facts
                    .symbols
                    .get((symbol - self.base[symbol_record.file as usize]) as usize)
            })
            .map(|local| local.owner)
            .unwrap_or_default()
            .to_string()
    }

    /// C++ `ns::name`, `Type::member`, `ns::Type::member`: symbols whose qualified name ends with
    /// the written one.
    pub(super) fn cpp_qualified(&self, qualifier: &str, name: &str) -> Option<Ids> {
        let written = format!("{}.{name}", qualifier.replace("::", "."));
        let found: Ids = self
            .by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&candidate| {
                let symbol = &self.symbols[candidate as usize];
                matches!(
                    self.file_type(symbol.file),
                    Some(FileType::Cpp | FileType::Ruby)
                ) && has_qualified_suffix(&symbol.qualified.replace("::", "."), &written)
            })
            .collect();
        (!found.is_empty()).then_some(found)
    }
}

/// File name without directory and extension: `src/db.c` and `include/db.h` pair as `db`.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}
