//! JVM module rules (`docs/spec.md` section 6): packages from the package declaration, shared by
//! Java, Kotlin, Scala and Groovy; single-type, wildcard and static imports; members of the
//! enclosing class visible without `this`.

use super::{Imports, Index, Target};
use crate::domain::facts::{Facts, SymKind};
use crate::domain::filetype::FileType;

/// Package prefixes of the platform and language runtimes: never external dependencies.
const PLATFORM: [&str; 9] = [
    "java.", "javax.", "jdk.", "sun.", "kotlin.", "kotlinx.", "scala.", "groovy.", "android.",
];

pub(super) fn is_jvm(file_type: Option<FileType>) -> bool {
    matches!(
        file_type,
        Some(FileType::Java | FileType::Kotlin | FileType::Scala | FileType::Groovy)
    )
}

/// Module key of a JVM package.
pub(super) fn package_key(package: &str) -> String {
    format!("jvm:{package}")
}

impl Index<'_> {
    pub(super) fn jvm_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            for name in &import.names {
                let local = import.alias.clone().unwrap_or_else(|| name.clone());
                if let Some(class_path) = import.spec.strip_prefix("static:") {
                    self.jvm_static_import(file, class_path, name, local, imports);
                    continue;
                }
                let Some(module) = self.modules.get(&package_key(&import.spec)) else {
                    // `import a.b.Outer.Inner`: the spec names a class of a package.
                    let nested = import.spec.rsplit_once('.').and_then(|(package, outer)| {
                        let module = self.modules.get(&package_key(package))?;
                        let found =
                            self.module_symbol(module, outer, Some(|kind: SymKind| kind.is_type()));
                        (!found.is_empty()).then_some(module)
                    });
                    match nested {
                        Some(module) if name != "*" => {
                            imports.modules.push(module);
                            if let Some(types) = self.types.get(&(module, name.as_str())) {
                                self.push_symbols(file, types.clone(), local, imports);
                            }
                        }
                        Some(module) => imports.modules.push(module),
                        None => self.jvm_external(&import.spec, name, local, imports),
                    }
                    continue;
                };
                imports.modules.push(module);
                if name == "*" {
                    imports.globs.push(module);
                    continue;
                }
                let found = self.module_symbol(module, name, None);
                if found.is_empty() {
                    // Kotlin top-level functions and Scala objects may live in sub-packages.
                    if let Some(child) = self
                        .modules
                        .get(&package_key(&format!("{}.{name}", import.spec)))
                    {
                        imports.names.insert(local, Target::Module(child));
                    }
                    continue;
                }
                self.push_symbols(file, found, local, imports);
            }
        }
    }

    /// `import static a.b.C.m`: members `m` of class `C`; `*` makes no names visible here
    /// (class members are found through the enclosing-class rule when inherited).
    fn jvm_static_import(
        &self,
        file: u32,
        class_path: &str,
        name: &str,
        local: String,
        imports: &mut Imports,
    ) {
        let Some((package, class)) = class_path.rsplit_once('.') else {
            return;
        };
        let Some(module) = self.modules.get(&package_key(package)) else {
            self.jvm_external(class_path, name, local, imports);
            return;
        };
        imports.modules.push(module);
        if name == "*" {
            return;
        }
        let members: Vec<u32> = self
            .module_symbol(module, class, Some(|kind: SymKind| kind.is_type()))
            .into_iter()
            .flat_map(|class| self.members(class, name))
            .collect();
        if !members.is_empty() {
            self.push_symbols(file, members, local, imports);
        }
    }

    fn push_symbols(&self, file: u32, symbols: Vec<u32>, local: String, imports: &mut Imports) {
        for &symbol in &symbols {
            let target_file = self.symbols[symbol as usize].file;
            if target_file != file {
                imports.files.push((file, target_file));
            }
        }
        imports.names.insert(local, Target::Symbols(symbols));
    }

    /// An import outside the root: platform packages are dropped, others are external
    /// dependencies labeled by their first two package segments (`org.junit`).
    fn jvm_external(&self, spec: &str, name: &str, local: String, imports: &mut Imports) {
        let dotted = format!("{spec}.");
        if PLATFORM.iter().any(|prefix| dotted.starts_with(prefix)) {
            if name != "*" {
                imports.names.insert(local, Target::External(String::new()));
            }
            return;
        }
        let label: String = spec.split('.').take(2).collect::<Vec<_>>().join(".");
        if name != "*" {
            imports.names.insert(local, Target::External(label.clone()));
        }
        if !label.is_empty() {
            imports.externals.push(label);
        }
    }

    /// Members `name` of the class enclosing `scope`, its supertypes included: unqualified calls
    /// and constants inside a class body.
    pub(super) fn enclosing_members(&self, file: u32, scope: Option<u32>, name: &str) -> Vec<u32> {
        let Some(types) = self.owner_type(file, scope) else {
            return Vec::new();
        };
        let mut found: Vec<u32> = types
            .iter()
            .flat_map(|&owner| self.members(owner, name))
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// Type of a field of the enclosing class used without `this`: `None` when the class has no
    /// such field, `Some(None)` when the field's type is outside the root.
    pub(super) fn enclosing_field_type(
        &self,
        file: u32,
        scope: Option<u32>,
        name: &str,
    ) -> Option<Option<(Vec<u32>, crate::domain::graph::Confidence)>> {
        let types = self.owner_type(file, scope)?;
        types.iter().find_map(|&owner| {
            let field_type = self.field_types.get(&(owner, name))?;
            let owner_file = self.symbols[owner as usize].file;
            Some(self.resolve_type(owner_file, None, None, field_type))
        })
    }
}
