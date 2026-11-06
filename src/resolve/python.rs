//! Python module rules (`docs/spec.md` section 6): absolute and relative imports, package
//! `__init__` modules, dotted qualifiers through the package tree.

use super::Ids;
use super::{Imports, Index, MAX_DISPATCH, Pair, Target};
use crate::domain::facts;
use crate::domain::facts::{Facts, SourceIdentityKind, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::Confidence;
use crate::domain::hash::FastSet as HashSet;

/// Modules a re-exported name is followed through (`pkg/__init__` -> `pkg/api` -> `pkg/impl`).
const MAX_REEXPORT_DEPTH: usize = 3;

impl Index<'_> {
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
        for import in &facts.imports {
            let Some(module_path) = absolute(&import.spec, &current, is_package) else {
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
                        imports.names.insert(local, Target::Module(module));
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
                        imports.names.insert(local, Target::External(top.clone()));
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
                if let Some(target) = target {
                    if let Target::Module(module) = &target {
                        imports.modules.push(*module);
                    }
                    imports.names.insert(local.to_string(), target);
                }
            }
        }
    }

    /// Symbols a module offers as `name`: its own definitions, else names it imports itself
    /// (`__init__.py` and facade re-exports such as `from .install import claude_install`),
    /// followed through at most a few modules.
    fn python_exported(&self, module: u32, name: &str, depth: usize) -> Ids {
        let own: Ids = self
            .module_symbol(module, name, None)
            .into_iter()
            .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Mod)
            .collect();
        if !own.is_empty() || depth >= MAX_REEXPORT_DEPTH {
            return own;
        }
        let current = self.modules.modules[module as usize].name.clone();
        for (file, &file_module) in self.modules.file_module.iter().enumerate() {
            if file_module != module {
                continue;
            }
            let Some(facts) = self.input.facts[file].as_ref() else {
                continue;
            };
            let is_package = self.input.files[file].path.ends_with("__init__.py");
            for import in &facts.imports {
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
                let Some(source) = absolute(&import.spec, &current, is_package)
                    .and_then(|path| self.python_module(&path))
                else {
                    continue;
                };
                if source != module {
                    let found = self.python_exported(source, original, depth + 1);
                    if !found.is_empty() {
                        return found;
                    }
                }
            }
        }
        Ids::new()
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
