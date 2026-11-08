//! Python module rules (`docs/spec.md` section 6): absolute and relative imports, package
//! `__init__` modules, dotted qualifiers through the package tree.

use super::Ids;
use super::{Imports, Index, MAX_DISPATCH, Outcome, Pair, Target};
use crate::domain::facts;
use crate::domain::facts::{Facts, SourceIdentityKind, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::Confidence;
use crate::domain::hash::FastSet as HashSet;

pub(super) struct PythonImport {
    index: u32,
    owner: Option<u32>,
    range: Option<std::ops::Range<u32>>,
    after_try: Option<std::ops::Range<u32>>,
    optional_try: Option<std::ops::Range<u32>>,
    line: u32,
    direct: bool,
    type_only: bool,
    redefined: bool,
    target: Option<Target>,
}

/// Modules a re-exported name is followed through (`pkg/__init__` -> `pkg/api` -> `pkg/impl`).
const MAX_REEXPORT_DEPTH: usize = 3;

impl Index<'_> {
    pub(super) fn python_context_manager_result(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        binding: &facts::Binding,
    ) -> Option<Ids> {
        if self.python_module("contextlib").is_some() || self.python_module("typing").is_some() {
            return None;
        }
        let facts = self.facts(file)?;
        let start = facts
            .bindings
            .partition_point(|item| item.line < binding.line);
        let index = facts.bindings[start..]
            .iter()
            .take_while(|item| item.line == binding.line)
            .position(|item| std::ptr::eq(item, binding))?
            + start;
        let identity = facts.source_identity(
            SourceIdentityKind::PythonContextBindingCall,
            u32::try_from(index).ok()?,
        )?;
        let (_, location) = identity.split_once(':')?;
        let (byte, line) = location.split_once(':')?;
        let byte = byte.parse::<u32>().ok()?;
        let line = line.parse::<u32>().ok()?;
        let (callee, prefix) = if let Some(callee) = binding
            .type_name
            .strip_prefix("await:")
            .and_then(|value| value.strip_suffix("().__aenter__()"))
        {
            (callee, "async:")
        } else {
            (binding.type_name.strip_suffix("().__enter__()")?, "sync:")
        };
        let reference = super::receiver_call(callee, scope, line, byte);
        if let Some(qualifier) = reference.qualifier.as_deref() {
            let head = qualifier.split('.').next()?;
            if head != "self"
                && self.binding(file, scope, head, line, byte).is_none()
                && !matches!(imports.names.get(head), Some(Target::Module(_)))
            {
                return None;
            }
        }
        let outcome = self
            .python_import_reference(file, facts, imports, &reference)
            .unwrap_or_else(|| self.resolve_ref(file, imports, &reference));
        let Outcome::Targets(targets, _) = outcome else {
            return None;
        };
        let [(callee, _)] = targets.as_slice() else {
            return None;
        };
        let symbol = self.symbols.get(*callee as usize)?;
        if self.file_type(symbol.file) != Some(FileType::Python) {
            return None;
        }
        let local = callee.checked_sub(self.base[symbol.file as usize])?;
        let result = self
            .facts(symbol.file)?
            .source_identity(SourceIdentityKind::PythonContextManager, local)?
            .strip_prefix(prefix)?;
        self.resolve_result_type(symbol.file, Some(local), result)
            .map(|(types, _)| types)
    }

    pub(super) fn python_structural_implementations(&self) -> Vec<Pair> {
        let mut protocols = Vec::new();
        let mut concrete = Vec::new();
        for (file, facts) in self.input.facts.iter().enumerate() {
            if self.file_type(file as u32) != Some(FileType::Python) {
                continue;
            }
            let Some(facts) = facts else { continue };
            for (local, class) in facts.symbols.iter().enumerate() {
                let Some(class_evidence) = facts
                    .source_identity(SourceIdentityKind::Symbol, local as u32)
                    .and_then(|identity| identity.strip_prefix("python:plain-class:"))
                else {
                    continue;
                };
                if class.kind() != SymKind::Class {
                    continue;
                }
                let methods: Vec<_> = facts
                    .symbols
                    .iter()
                    .enumerate()
                    .skip(local + 1)
                    .take_while(|(_, method)| method.line <= class.end_line)
                    .filter(|(_, method)| {
                        method.kind() == SymKind::Method && method.parent == Some(local as u32)
                    })
                    .map(|(index, method)| {
                        Some((
                            method.name.as_ref(),
                            facts.source_identity(SourceIdentityKind::Symbol, index as u32)?,
                        ))
                    })
                    .collect::<Option<_>>()
                    .unwrap_or_default();
                if methods.is_empty()
                    || methods.iter().enumerate().any(|(index, (name, _))| {
                        methods[..index].iter().any(|(other, _)| name == other)
                    })
                {
                    continue;
                }
                let id = self.base[file] + local as u32;
                if self.python_is_direct_protocol(facts, class, class_evidence) {
                    protocols.push((id, methods));
                } else if *class.signature == *format!("class {}:", class.name) {
                    concrete.push((id, methods));
                }
            }
        }
        let mut pairs = Vec::new();
        for (candidate, methods) in &concrete {
            for (protocol, required) in &protocols {
                if self.is_test(*candidate) && !self.is_test(*protocol) {
                    continue;
                }
                if required.iter().all(|required| methods.contains(required)) {
                    pairs.push((
                        *candidate,
                        *protocol,
                        Confidence::Inferred,
                        self.symbols[*candidate as usize].line,
                    ));
                }
            }
        }
        pairs
    }

    fn python_is_direct_protocol(
        &self,
        facts: &Facts,
        class: &crate::domain::facts::Symbol,
        direct_import_lines: &str,
    ) -> bool {
        facts.imports.iter().any(|import| {
            if !matches!(import.spec.as_str(), "typing" | "typing_extensions")
                || self.python_module(&import.spec).is_some()
                || import.names.as_slice() != ["Protocol"]
            {
                return false;
            }
            let local = import.alias.as_deref().unwrap_or("Protocol");
            direct_import_lines
                .split(',')
                .any(|line| line.parse::<u32>() == Ok(import.line))
                && import.line < class.line
                && *class.signature == *format!("class {}({local}):", class.name)
                && !facts.imports.iter().any(|other| {
                    if std::ptr::eq(other, import) || other.line >= class.line {
                        return false;
                    }
                    let bound = other.alias.as_deref().or_else(|| {
                        other
                            .names
                            .first()
                            .map(|name| name.as_str())
                            .or_else(|| other.spec.split('.').next())
                    });
                    bound == Some(local) || other.names.iter().any(|name| name == "*")
                })
                && !facts.symbols.iter().any(|symbol| {
                    symbol.parent.is_none() && *symbol.name == *local && symbol.line < class.line
                })
        })
    }

    pub(super) fn python_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        self.python_module_imports(file, facts, imports);
        // File edges to every imported module's files (module import graph at file level).
        let mut targets: Ids = imports.modules.clone().into();
        targets.sort_unstable();
        targets.dedup();
        for (other, &module) in self.modules.file_module.iter().enumerate() {
            if targets.binary_search(&module).is_ok() && other as u32 != file {
                imports.files.push((file, other as u32));
            }
        }
    }

    fn python_module_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        let current = self.modules.modules[self.module_of(file) as usize]
            .name
            .clone();
        let is_package = self.input.files[file as usize]
            .path
            .ends_with("__init__.py");
        for (index, import) in facts.imports.iter().enumerate() {
            let Some(module_path) = absolute(&import.spec, &current, is_package) else {
                let local = import
                    .alias
                    .as_deref()
                    .or_else(|| import.names.first().map(|name| name.as_str()))
                    .or_else(|| import.spec.split('.').next())
                    .filter(|name| !name.is_empty() && *name != "*");
                if let Some(local) = local {
                    self.record_python_import(facts, imports, index, local.to_owned(), None);
                }
                continue;
            };
            if import.names.is_empty() {
                // `import a.b` binds `a`; `import a.b as c` binds `c` to `a.b`.
                let (local, bound) = match &import.alias {
                    Some(alias) => (alias.to_string(), module_path.clone()),
                    None => {
                        let first = module_path
                            .split('.')
                            .next()
                            .unwrap_or(&module_path)
                            .to_owned();
                        (first.clone(), first)
                    }
                };
                match self.python_module(&bound) {
                    Some(module) => {
                        self.record_python_import(
                            facts,
                            imports,
                            index,
                            local,
                            Some(Target::Module(module)),
                        );
                        if let Some(full) = self.python_module(&module_path) {
                            imports.modules.push(full);
                        }
                    }
                    None => {
                        let top = module_path
                            .split('.')
                            .next()
                            .unwrap_or(&module_path)
                            .to_owned();
                        self.record_python_import(
                            facts,
                            imports,
                            index,
                            local,
                            Some(Target::External(top.clone())),
                        );
                        imports.externals.push(top);
                    }
                }
                continue;
            }
            let base = self.python_module(&module_path);
            for name in &import.names {
                if name == "*" {
                    if let Some(module) = base {
                        imports.globs.push(module);
                        imports.modules.push(module);
                    }
                    continue;
                }
                let local = import.alias.unwrap_or(*name);
                let child = join(&module_path, name);
                let target = match (self.python_module(&child), base) {
                    (Some(module), _) => Some(Target::Module(module)),
                    (None, Some(module)) => {
                        let found = self.python_exported(module, name, 0);
                        imports.modules.push(module);
                        (!found.is_empty()).then_some(Target::Symbols(found))
                    }
                    (None, None) => {
                        let top = module_path
                            .split('.')
                            .next()
                            .unwrap_or(&module_path)
                            .to_owned();
                        if !top.is_empty() {
                            imports.externals.push(top.clone());
                        }
                        Some(Target::External(top))
                    }
                };
                if let Some(Target::Module(module)) = &target {
                    imports.modules.push(*module);
                }
                self.record_python_import(facts, imports, index, local.to_string(), target);
            }
        }
    }

    fn record_python_import(
        &self,
        facts: &Facts,
        imports: &mut Imports,
        index: usize,
        local: String,
        target: Option<Target>,
    ) {
        let import = &facts.imports[index];
        let Ok(index) = u32::try_from(index) else {
            return;
        };
        let direct =
            facts.source_identity(SourceIdentityKind::PythonDirectImport, index) == Some("direct");
        let scope = facts
            .source_identity(SourceIdentityKind::PythonImportScope, index)
            .and_then(facts::python_import_scope);
        let after_try = facts
            .source_identity(SourceIdentityKind::PythonImportAfterTry, index)
            .and_then(facts::python_import_scope)
            .filter(|(owner, _)| scope.as_ref().is_none_or(|(other, _)| owner == other));
        let optional_try = facts
            .source_identity(SourceIdentityKind::PythonImportOptionalTry, index)
            .and_then(facts::python_import_scope)
            .filter(|(owner, _)| {
                scope
                    .as_ref()
                    .or(after_try.as_ref())
                    .is_none_or(|(other, _)| owner == other)
            });
        let owner = scope
            .as_ref()
            .or(after_try.as_ref())
            .or(optional_try.as_ref())
            .map(|(owner, _)| *owner)
            .unwrap_or_else(|| {
                if direct {
                    return None;
                }
                facts
                    .symbols
                    .iter()
                    .enumerate()
                    .filter(|(_, symbol)| {
                        matches!(
                            symbol.kind(),
                            SymKind::Fn | SymKind::Method | SymKind::Class
                        ) && symbol.line <= import.line
                            && import.line <= symbol.end_line
                    })
                    .min_by_key(|(_, symbol)| symbol.end_line.saturating_sub(symbol.line))
                    .and_then(|(index, _)| u32::try_from(index).ok())
            });
        let type_only = import.type_only && self.python_module("typing").is_none();
        let redefined = target.as_ref().is_some_and(|target| {
            let Target::Symbols(symbols) = target else {
                return false;
            };
            let [id] = symbols.as_slice() else {
                return false;
            };
            let symbol = &self.symbols[*id as usize];
            self.file_type(symbol.file) == Some(FileType::Python)
                && self
                    .by_module
                    .get(&(self.module_of(symbol.file), symbol.name.as_str()))
                    .is_some_and(|symbols| symbols.len() > 1)
        });
        if (direct || type_only)
            && let Some(target) = &target
        {
            imports.names.insert(local.clone(), target.clone());
        }
        imports
            .python_names
            .get_or_insert_with(Default::default)
            .entry(local)
            .or_default()
            .push(PythonImport {
                index,
                owner,
                range: scope.map(|(_, range)| range),
                after_try: after_try.map(|(_, range)| range),
                optional_try: optional_try.map(|(_, range)| range),
                line: import.line,
                direct,
                type_only,
                redefined,
                target,
            });
    }

    pub(super) fn python_callable_import_fallback(
        &self,
        facts: &Facts,
        imports: &Imports,
        identity: &str,
    ) -> Outcome {
        let Some((selected, fallback)) =
            identity.split_once(':').and_then(|(selected, fallback)| {
                Some((selected.parse::<u32>().ok()?, fallback.parse::<u32>().ok()?))
            })
        else {
            return Outcome::Ambiguous;
        };
        let Some(names) = imports.python_names.as_ref() else {
            return Outcome::Ambiguous;
        };
        let mut targets = super::Targets::new();
        for index in [selected, fallback] {
            let Some(import) = facts.imports.get(index as usize) else {
                return Outcome::Ambiguous;
            };
            let Some(name) = import
                .alias
                .as_deref()
                .or_else(|| import.names.first().map(|name| name.as_str()))
                .or_else(|| import.spec.split('.').next())
            else {
                return Outcome::Ambiguous;
            };
            let Some(Target::Symbols(symbols)) = names
                .get(name)
                .and_then(|items| items.iter().find(|item| item.index == index))
                .and_then(|item| item.target.as_ref())
            else {
                return Outcome::Ambiguous;
            };
            let [symbol] = symbols.as_slice() else {
                return Outcome::Ambiguous;
            };
            let kind = self.symbols[*symbol as usize].kind;
            if !kind.is_callable() && !kind.is_type() {
                return Outcome::Ambiguous;
            }
            if !targets.iter().any(|(target, _)| target == symbol) {
                targets.push((*symbol, Confidence::Inferred));
            }
        }
        Outcome::Targets(targets, Confidence::Inferred)
    }

    pub(super) fn python_import_reference(
        &self,
        file: u32,
        facts: &Facts,
        imports: &Imports,
        reference: &facts::Ref,
    ) -> Option<Outcome> {
        if !matches!(
            reference.kind,
            facts::RefKind::Call
                | facts::RefKind::Value
                | facts::RefKind::Type
                | facts::RefKind::Inherit
        ) {
            return None;
        }
        let head = reference
            .qualifier
            .as_deref()
            .map_or(reference.name.as_str(), |qualifier| {
                qualifier.split('.').next().unwrap_or_default()
            });
        let candidates = imports.python_names.as_ref()?.get(head)?;
        let callable = reference
            .scope
            .and_then(|scope| facts.symbols.get(scope as usize))
            .is_some_and(|symbol| symbol.kind().is_callable());
        let owners = super::scope_chain(facts, reference.scope)
            .map(Some)
            .chain(std::iter::once(None));
        for owner in owners {
            if callable
                && owner != reference.scope
                && owner
                    .and_then(|scope| facts.symbols.get(scope as usize))
                    .is_some_and(|symbol| symbol.kind() == SymKind::Class)
            {
                continue;
            }
            if !candidates.iter().any(|candidate| candidate.owner == owner) {
                continue;
            }
            let chosen = candidates
                .iter()
                .filter(|candidate| candidate.owner == owner)
                .filter(|candidate| {
                    if candidate.type_only {
                        reference.kind == facts::RefKind::Type
                    } else {
                        candidate
                            .range
                            .as_ref()
                            .is_some_and(|range| range.contains(&reference.byte))
                            || candidate
                                .after_try
                                .as_ref()
                                .is_some_and(|range| range.contains(&reference.byte))
                            || (reference.kind == facts::RefKind::Call
                                && candidate
                                    .optional_try
                                    .as_ref()
                                    .is_some_and(|range| range.contains(&reference.byte)))
                            || (candidate.direct
                                && owner.is_none()
                                && (callable || candidate.line <= reference.line))
                    }
                })
                .max_by_key(|candidate| candidate.line);
            let Some(chosen) = chosen else {
                return Some(Outcome::Ambiguous);
            };
            if candidates
                .iter()
                .filter(|candidate| candidate.owner == owner && candidate.line == chosen.line)
                .count()
                != 1
                || candidates.iter().any(|candidate| {
                    candidate.owner == owner
                        && candidate.line > chosen.line
                        && (callable && owner.is_none() || candidate.line <= reference.line)
                })
            {
                return Some(Outcome::Ambiguous);
            }
            let Some(target) = chosen.target.as_ref() else {
                return Some(Outcome::Ambiguous);
            };
            let optional = chosen
                .optional_try
                .as_ref()
                .is_some_and(|range| range.contains(&reference.byte))
                && !chosen
                    .range
                    .as_ref()
                    .is_some_and(|range| range.contains(&reference.byte))
                && !chosen
                    .after_try
                    .as_ref()
                    .is_some_and(|range| range.contains(&reference.byte));
            let inferred = optional || chosen.redefined;
            if reference.qualifier.is_some() {
                if chosen.direct && imports.names.get(head) == Some(target) {
                    return None;
                }
                let mut local = Imports::default();
                local.names.clone_from(&imports.names);
                local.globs.clone_from(&imports.globs);
                local.modules.clone_from(&imports.modules);
                local.names.insert(head.to_owned(), target.clone());
                let outcome = match reference.kind {
                    facts::RefKind::Value => self.resolve_value(file, &local, reference),
                    facts::RefKind::Inherit => self.resolve_supertype(file, &local, reference),
                    _ => self.resolve_ref(file, &local, reference),
                };
                return Some(if inferred {
                    match outcome {
                        Outcome::Targets(targets, _) => Outcome::Targets(
                            targets
                                .into_iter()
                                .map(|(target, _)| (target, Confidence::Inferred))
                                .collect(),
                            Confidence::Inferred,
                        ),
                        other => other,
                    }
                } else {
                    outcome
                });
            }
            return Some(match target {
                Target::Symbols(symbols) => {
                    let selected: Ids = symbols
                        .iter()
                        .copied()
                        .filter(|&index| {
                            let kind = self.symbols[index as usize].kind;
                            match reference.kind {
                                facts::RefKind::Type | facts::RefKind::Inherit => kind.is_type(),
                                facts::RefKind::Value => {
                                    kind.is_callable()
                                        || kind.is_type()
                                        || matches!(kind, SymKind::Const | SymKind::Var)
                                }
                                _ => kind.is_callable() || kind.is_type(),
                            }
                        })
                        .collect();
                    super::pick(
                        &selected,
                        if inferred {
                            Confidence::Inferred
                        } else {
                            Confidence::Exact
                        },
                    )
                }
                Target::Module(_) | Target::External(_) => Outcome::External,
            });
        }
        Some(Outcome::Ambiguous)
    }

    /// Symbols a module offers as `name`: its own definitions, else names it imports itself
    /// (`__init__.py` and facade re-exports such as `from .install import claude_install`),
    /// followed through at most a few modules.
    fn python_exported(&self, module: u32, name: &str, depth: usize) -> Ids {
        let mut own: Ids = self
            .module_symbol(module, name, None)
            .into_iter()
            .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Mod)
            .collect();
        if own.len() > 1 {
            let file = self.symbols[own[0] as usize].file;
            let direct = self.input.facts[file as usize]
                .as_ref()
                .is_some_and(|facts| {
                    own.iter().all(|symbol| {
                        let item = &self.symbols[*symbol as usize];
                        item.file == file
                            && item.kind == SymKind::Fn
                            && symbol
                                .checked_sub(self.base[file as usize])
                                .and_then(|local| {
                                    facts.source_identity(
                                        SourceIdentityKind::PythonDirectDefinition,
                                        local,
                                    )
                                })
                                == Some("direct")
                    })
                });
            if direct
                && let Some(&last) = own.iter().max_by_key(|id| self.symbols[**id as usize].line)
            {
                let line = self.symbols[last as usize].line;
                if own
                    .iter()
                    .filter(|id| self.symbols[**id as usize].line == line)
                    .count()
                    == 1
                {
                    own.clear();
                    own.push(last);
                }
            }
        }
        if depth >= MAX_REEXPORT_DEPTH {
            return own;
        }
        let current = self.modules.modules[module as usize].name.clone();
        let mut exported = own.clone();
        let mut uncertain = false;
        for (file, &file_module) in self.modules.file_module.iter().enumerate() {
            if file_module != module {
                continue;
            }
            let Some(facts) = self.input.facts[file].as_ref() else {
                continue;
            };
            uncertain |= facts.coverage != facts::Coverage::Complete;
            let is_package = self.input.files[file].path.ends_with("__init__.py");
            for (index, import) in facts.imports.iter().enumerate() {
                if import.type_only {
                    continue;
                }
                // `from .models import *` offers the public names of that module.
                let star = import.names.iter().any(|imported| imported == "*");
                let original =
                    if star && !name.starts_with('_') {
                        name
                    } else if let Some(original) = import.names.iter().find(|imported| {
                        import.alias.as_deref().unwrap_or(imported.as_str()) == name
                    }) {
                        original.as_str()
                    } else {
                        continue;
                    };
                // A function or class body binds its imports locally; its name is not a
                // module export merely because the import appears in this file.
                if facts.symbols.iter().any(|symbol| {
                    matches!(
                        symbol.kind(),
                        SymKind::Fn | SymKind::Method | SymKind::Class
                    ) && symbol.line <= import.line
                        && import.line <= symbol.end_line
                }) {
                    continue;
                }
                if let [symbol] = own.as_slice() {
                    let definition = &self.symbols[*symbol as usize];
                    if definition.file as usize == file
                        && definition.line > import.line
                        && symbol.checked_sub(self.base[file]).and_then(|local| {
                            facts.source_identity(SourceIdentityKind::PythonDirectDefinition, local)
                        }) == Some("direct")
                    {
                        continue;
                    }
                }
                let overwrites = u32::try_from(index).ok().and_then(|index| {
                    facts.source_identity(SourceIdentityKind::PythonDirectImport, index)
                }) == Some("direct")
                    && facts
                        .imports
                        .iter()
                        .filter(|other| {
                            other.line == import.line
                                && other.names.iter().any(|original| {
                                    (original == "*" && !name.starts_with('_'))
                                        || other.alias.as_deref().unwrap_or(original.as_str())
                                            == name
                                })
                        })
                        .count()
                        == 1;
                let replaces_own = own.is_empty()
                    || (u32::try_from(index).ok().and_then(|index| {
                        facts.source_identity(SourceIdentityKind::PythonImportNoModuleWrite, index)
                    }) == Some("unwritten")
                        && own.iter().all(|symbol| {
                            let symbol = &self.symbols[*symbol as usize];
                            symbol.file as usize == file && symbol.line < import.line
                        }));
                if overwrites && !star && replaces_own {
                    exported.clear();
                    uncertain = false;
                }
                let Some(source) = absolute(&import.spec, &current, is_package)
                    .and_then(|path| self.python_module(&path))
                else {
                    if star {
                        continue;
                    }
                    uncertain = true;
                    continue;
                };
                if source == module {
                    if star {
                        continue;
                    }
                    uncertain = true;
                    continue;
                }
                let found = self.python_exported(source, original, depth + 1);
                if found.is_empty() {
                    if star {
                        continue;
                    }
                    uncertain = true;
                    continue;
                }
                if overwrites && star && replaces_own {
                    exported.clear();
                    uncertain = false;
                }
                exported.extend(found);
            }
        }
        exported.sort_unstable();
        exported.dedup();
        if !uncertain && exported.len() == 1 {
            exported
        } else {
            Ids::new()
        }
    }

    fn python_module(&self, dotted: &str) -> Option<u32> {
        self.modules.get(&format!("py:{dotted}"))
    }

    /// `pkg.mod.name` where `pkg` is an imported module: walk the package tree, then the name.
    pub(super) fn python_qualified(
        &self,
        imports: &Imports,
        qualifier: &str,
        name: &str,
    ) -> Option<Ids> {
        let mut segments = qualifier.split('.');
        let head = segments.next()?;
        let Target::Module(start) = imports.names.get(head)? else {
            return None;
        };
        let mut dotted = self.modules.modules[*start as usize].name.clone();
        let mut module = *start;
        for segment in segments {
            dotted = join(&dotted, segment);
            module = self.python_module(&dotted)?;
        }
        let found = self.python_exported(module, name, 0);
        (!found.is_empty()).then_some(found)
    }
}

/// Absolute dotted module of an import spec; leading dots climb from the current package.
fn absolute(spec: &str, current: &str, is_package: bool) -> Option<String> {
    let dots = spec.chars().take_while(|&ch| ch == '.').count();
    if dots == 0 {
        return Some(spec.to_owned());
    }
    let mut package: Vec<&str> = current.split('.').filter(|part| !part.is_empty()).collect();
    if !is_package {
        package.pop();
    }
    for _ in 1..dots {
        package.pop()?;
    }
    let rest = &spec[dots..];
    let base = package.join(".");
    Some(if rest.is_empty() {
        base
    } else {
        join(&base, rest)
    })
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_owned()
    } else {
        format!("{base}.{name}")
    }
}

impl<'a> Index<'a> {
    pub(super) fn python_stub_implementations(
        &self,
        base: u32,
        name: &str,
        arity: u8,
        caller_is_test: bool,
    ) -> Option<Ids> {
        let mut pending = vec![base];
        let mut visited = HashSet::default();
        visited.insert(base);
        let mut targets = Vec::new();
        let mut target_set = HashSet::default();
        let mut cursor = 0;
        while cursor < pending.len() {
            let current = pending[cursor];
            cursor += 1;
            for &implementation in self.implementations.get(&current).into_iter().flatten() {
                if !visited.insert(implementation)
                    || self.file_type(self.symbols[implementation as usize].file)
                        != Some(FileType::Python)
                    || self.symbols[implementation as usize].kind != SymKind::Class
                    || (!caller_is_test && self.is_test(implementation))
                {
                    continue;
                }
                pending.push(implementation);
                let concrete: Ids = self
                    .members_direct(implementation, name)
                    .into_iter()
                    .filter(|&method| self.symbols[method as usize].parent == implementation)
                    .filter(|&method| {
                        self.symbols[method as usize].flags & facts::FLAG_PYTHON_ABSTRACT_STUB == 0
                    })
                    .collect();
                let (selected, _) = self.overloads(&concrete, arity);
                for method in selected {
                    if target_set.insert(method) {
                        targets.push(method);
                        if targets.len() > MAX_DISPATCH {
                            return None;
                        }
                    }
                }
            }
        }
        targets.sort_unstable();
        Some(targets.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_branch_imports_resolve_at_their_own_call_sites() {
        use crate::domain::corpus::FileRecord;
        use crate::domain::graph::{Confidence, EdgeKind};
        use crate::domain::role::Role;

        let sources = [
            (
                "app.py",
                "def branches(flag):\n    if flag:\n        from left import left as call\n        call()\n    else:\n        from right import right as call\n        call()\n\ndef after_branch(flag):\n    if flag:\n        from left import left as call\n    else:\n        from right import right as call\n    call()\n\ndef repeated_import():\n    from api import repeated\n    repeated()\n\ndef conditional_import():\n    from conditional import disputed\n    disputed()\n",
            ),
            ("left.py", "def left(): pass\n"),
            ("right.py", "def right(): pass\n"),
            ("api.py", "def repeated(): pass\n\ndef repeated(): pass\n"),
            (
                "conditional.py",
                "def disputed(): pass\nif flag:\n    def disputed(): pass\n",
            ),
        ];
        let mut files = Vec::new();
        let mut facts = Vec::new();
        for (path, source) in sources {
            let record = FileRecord {
                path: path.to_owned(),
                size: source.len() as u64,
                mtime_ns: 0,
                identity: None,
                hash: 0,
                role: Role::Src,
                file_type: Some(FileType::Python),
            };
            facts.push(Some(Box::new(crate::extract::extract(
                &record,
                source.as_bytes(),
            ))));
            files.push(record);
        }
        let graph = super::super::build(&files, facts);
        let mut calls: Vec<_> = graph
            .edges
            .iter()
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                (source.qualified == "app.branches" && edge.kind == EdgeKind::Calls)
                    .then(|| (edge.line, target.qualified.to_string(), edge.confidence))
            })
            .collect();
        calls.sort_unstable();
        assert_eq!(
            calls,
            [
                (4, "left.left".to_owned(), Confidence::Exact),
                (7, "right.right".to_owned(), Confidence::Exact)
            ]
        );
        assert!(!graph.edges.iter().any(|edge| {
            graph
                .symbols
                .get(edge.source as usize)
                .is_some_and(|symbol| {
                    symbol.qualified == "app.after_branch"
                        && edge.kind == EdgeKind::Calls
                        && edge.confidence == Confidence::Exact
                })
        }));
        let repeated: Vec<_> = graph
            .edges
            .iter()
            .filter_map(|edge| {
                let source = graph.symbols.get(edge.source as usize)?;
                let target = graph.symbols.get(edge.target as usize)?;
                (source.qualified == "app.repeated_import" && edge.kind == EdgeKind::Calls)
                    .then_some((target.line, edge.confidence))
            })
            .collect();
        assert_eq!(repeated, [(3, Confidence::Inferred)]);
        assert!(!graph.edges.iter().any(|edge| {
            graph
                .symbols
                .get(edge.source as usize)
                .is_some_and(|symbol| symbol.qualified == "app.conditional_import")
                && edge.kind == EdgeKind::Calls
                && edge.confidence == Confidence::Exact
        }));
    }

    #[test]
    fn structural_protocol_requires_all_methods_with_matching_shapes() -> std::io::Result<()> {
        use crate::domain::graph::{Confidence, EdgeKind};

        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/python-protocol");
        let graph = super::super::tests::graph_of(&root)?;
        let symbol = |name: &str| {
            graph
                .symbols
                .iter()
                .position(|symbol| symbol.qualified == name)
                .map(|index| index as u32)
                .ok_or_else(|| std::io::Error::other(format!("missing symbol {name}")))
        };
        let protocol = symbol("app.protocols.Reader")?;
        let implements = |source| {
            graph.edges.iter().any(|edge| {
                edge.source == source
                    && edge.target == protocol
                    && edge.kind == EdgeKind::Implements
                    && edge.confidence == Confidence::Inferred
            })
        };
        assert!(implements(symbol("app.implementations.FileReader")?));
        assert!(!implements(symbol("app.implementations.MissingClose")?));
        assert!(!implements(symbol("app.implementations.WrongShape")?));
        assert!(!implements(symbol("app.implementations.DecoratedReader")?));
        let file_reader = symbol("app.implementations.FileReader")?;
        for name in [
            "app.protocols.NeedsFlag",
            "app.protocols.DefaultValue",
            "app.shadowed.NotAProtocol",
        ] {
            let protocol = symbol(name)?;
            assert!(!graph.edges.iter().any(|edge| {
                edge.source == file_reader
                    && edge.target == protocol
                    && edge.kind == EdgeKind::Implements
            }));
        }
        let long_wrong = symbol("app.implementations.LongWrong")?;
        let long_protocol = symbol("app.protocols.LongReader")?;
        assert!(!graph.edges.iter().any(|edge| {
            edge.source == long_wrong
                && edge.target == long_protocol
                && edge.kind == EdgeKind::Implements
        }));
        let literal_wrong = symbol("app.implementations.LiteralWrong")?;
        let literal_protocol = symbol("app.protocols.LiteralReader")?;
        assert!(!graph.edges.iter().any(|edge| {
            edge.source == literal_wrong
                && edge.target == literal_protocol
                && edge.kind == EdgeKind::Implements
        }));
        Ok(())
    }

    #[test]
    fn relative_imports_climb_packages() {
        assert_eq!(
            absolute(".", "pkg.store.db", false).as_deref(),
            Some("pkg.store")
        );
        assert_eq!(
            absolute(".util", "pkg.store.db", false).as_deref(),
            Some("pkg.store.util")
        );
        assert_eq!(
            absolute("..core", "pkg.store.db", false).as_deref(),
            Some("pkg.core")
        );
        assert_eq!(
            absolute(".db", "pkg.store", true).as_deref(),
            Some("pkg.store.db")
        );
        assert_eq!(
            absolute("os.path", "pkg", false).as_deref(),
            Some("os.path")
        );
        assert_eq!(absolute("...x", "a.b", false), None);
    }
}
