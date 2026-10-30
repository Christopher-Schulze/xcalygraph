//! Python module rules (`docs/spec.md` section 6): absolute and relative imports, package
//! `__init__` modules, dotted qualifiers through the package tree.

use super::{Imports, Index, Target};
use crate::domain::facts::{Facts, SymKind};

impl Index<'_> {
    pub(super) fn python_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
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
                    Some(alias) => (alias.clone(), module_path.clone()),
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
                let local = import.alias.clone().unwrap_or_else(|| name.clone());
                let child = join(&module_path, name);
                let target = match (self.python_module(&child), base) {
                    (Some(module), _) => Some(Target::Module(module)),
                    (None, Some(module)) => {
                        let found: Vec<u32> = self
                            .module_symbol(module, name, None)
                            .into_iter()
                            .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Mod)
                            .collect();
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
                    imports.names.insert(local, target);
                }
            }
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
    ) -> Option<Vec<u32>> {
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
        let found: Vec<u32> = self
            .module_symbol(module, name, None)
            .into_iter()
            .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Mod)
            .collect();
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

#[cfg(test)]
mod tests {
    use super::*;

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
