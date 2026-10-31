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

pub(super) fn is_dotnet(file_type: Option<FileType>) -> bool {
    matches!(file_type, Some(FileType::CSharp | FileType::VbNet))
}

/// JVM and .NET: names come from the class body, the package or namespace and imports only;
/// members of the enclosing class need no `this`.
pub(super) fn is_class_based(file_type: Option<FileType>) -> bool {
    is_jvm(file_type) || is_dotnet(file_type) || file_type == Some(FileType::Php)
}

/// Class bodies where members need no `this`: JVM, .NET and C++.
pub(super) fn has_implicit_this(file_type: Option<FileType>) -> bool {
    // PHP calls methods only through `$this->`; a bare call is a function.
    (is_class_based(file_type) && file_type != Some(FileType::Php))
        || matches!(
            file_type,
            Some(
                FileType::Cpp
                    | FileType::Swift
                    | FileType::Ruby
                    | FileType::Dart
                    | FileType::Elixir
                    | FileType::Pascal
                    | FileType::Solidity
                    | FileType::Dm
                    | FileType::Verilog
                    | FileType::Apex
            )
        )
}

/// Module key of a JVM package or .NET namespace.
pub(super) fn namespace_key(file_type: Option<FileType>, name: &str) -> String {
    if is_dotnet(file_type) {
        super::dotnet::namespace_key(name)
    } else if file_type == Some(FileType::Php) {
        format!("php:{name}")
    } else {
        package_key(name)
    }
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
                let Some(module) = self
                    .modules
                    .get(&namespace_key(self.file_type(file), &import.spec))
                else {
                    // `import a.b.Outer.Inner`: the spec names a class of a package.
                    let nested = import.spec.rsplit_once('.').and_then(|(package, outer)| {
                        let module = self
                            .modules
                            .get(&namespace_key(self.file_type(file), package))?;
                        let found =
                            self.module_symbol(module, outer, Some(|kind: SymKind| kind.is_type()));
                        (!found.is_empty()).then_some((module, found))
                    });
                    match nested {
                        Some((module, outers)) if name != "*" => {
                            imports.modules.push(module);
                            if let Some(types) = self.types.get(&(module, name.as_str())) {
                                self.push_symbols(file, types.clone(), local, imports);
                            } else {
                                // Scala `import a.b.Util.{f, g}`: members of the object.
                                let members: Vec<u32> = outers
                                    .iter()
                                    .flat_map(|&outer| self.members_direct(outer, name))
                                    .collect();
                                if !members.is_empty() {
                                    self.push_symbols(file, members, local, imports);
                                }
                            }
                        }
                        Some((module, outers)) => {
                            // Scala `import a.b.Util._`: every member of the object or class.
                            imports.modules.push(module);
                            for outer in outers {
                                self.import_members(file, outer, imports);
                            }
                        }
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
                    if let Some(child) = self.modules.get(&namespace_key(
                        self.file_type(file),
                        &format!("{}.{name}", import.spec),
                    )) {
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
        let Some(module) = self
            .modules
            .get(&namespace_key(self.file_type(file), package))
        else {
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

    /// Binds the direct members of `outer` by their names (a wildcard import of a type, Elixir
    /// `import Mod`).
    pub(super) fn import_members(&self, file: u32, outer: u32, imports: &mut Imports) {
        let owner_file = self.symbols[outer as usize].file as usize;
        let end = self
            .base
            .get(owner_file + 1)
            .copied()
            .unwrap_or(self.symbols.len() as u32);
        let mut members: std::collections::BTreeMap<&str, Vec<u32>> = Default::default();
        for member in self.base[owner_file]..end {
            let symbol = &self.symbols[member as usize];
            // Private members (`defp`, Scala `private def`) are not imported.
            if symbol.parent == outer && symbol.exported {
                members
                    .entry(symbol.name.as_str())
                    .or_default()
                    .push(member);
            }
        }
        if !members.is_empty() && owner_file as u32 != file {
            imports.files.push((file, owner_file as u32));
        }
        for (name, symbols) in members {
            imports
                .names
                .entry(name.to_owned())
                .or_insert(Target::Symbols(symbols));
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

    /// Kotlin extension functions `fun Receiver.name(...)` visible in the file (own package,
    /// imports, wildcard imports, members of the enclosing class): the target of `x.name()` when the receiver type is outside the
    /// root or has no such member. Several candidates are inferred.
    pub(super) fn extension(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        name: &str,
        receiver: Option<&str>,
    ) -> Option<super::Outcome> {
        if self.file_type(file) == Some(FileType::CSharp) {
            return self.csharp_extension(file, imports, name, receiver);
        }
        if self.file_type(file) != Some(FileType::Kotlin) {
            return None;
        }
        // Member extensions of the enclosing class or object come first.
        let mut candidates = self.enclosing_members(file, scope, name);
        candidates.extend(self.module_symbol(self.module_of(file), name, None));
        if let Some(Target::Symbols(symbols)) = imports.names.get(name) {
            candidates.extend(symbols);
        }
        for &glob in &imports.globs {
            candidates.extend(self.module_symbol(glob, name, None));
        }
        candidates.retain(|&candidate| {
            let symbol = &self.symbols[candidate as usize];
            matches!(symbol.kind, SymKind::Fn | SymKind::Method)
                && (symbol.signature.contains(&format!(".{name}("))
                    || symbol.signature.contains(&format!(".{name}<")))
        });
        candidates.sort_unstable();
        candidates.dedup();
        if let Some(receiver) = receiver.map(simple_type) {
            let matching: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|&candidate| {
                    let symbol = &self.symbols[candidate as usize];
                    extension_receiver(&symbol.signature, name)
                        .is_some_and(|declared| declared == receiver)
                })
                .collect();
            if !matching.is_empty() {
                candidates = matching;
            }
        }
        let confidence = if candidates.len() == 1 {
            crate::domain::graph::Confidence::Exact
        } else {
            crate::domain::graph::Confidence::Inferred
        };
        (!candidates.is_empty()).then(|| {
            super::Outcome::Targets(
                candidates.iter().map(|&c| (c, confidence)).collect(),
                confidence,
            )
        })
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

/// Simple name of a type expression: `kotlin.collections.List<T>?` -> `List`.
fn simple_type(text: &str) -> &str {
    let base = text
        .split('<')
        .next()
        .unwrap_or(text)
        .trim()
        .trim_end_matches('?');
    base.rsplit('.').next().unwrap_or(base)
}

/// Receiver type of an extension declaration `fun <T> Receiver<T>.name(...)` in a signature.
fn extension_receiver<'a>(signature: &'a str, name: &str) -> Option<&'a str> {
    let end = signature
        .find(&format!(".{name}("))
        .or_else(|| signature.find(&format!(".{name}<")))?;
    let head = &signature[..end];
    let mut receiver = head
        .rsplit_once("fun ")
        .map_or(head, |(_, rest)| rest)
        .trim();
    if receiver.starts_with('<') {
        let mut depth = 0;
        let close = receiver.char_indices().find_map(|(index, ch)| {
            match ch {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
            (depth == 0).then_some(index)
        })?;
        receiver = receiver[close + 1..].trim();
    }
    Some(simple_type(receiver))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_receivers_come_from_the_declaration() {
        assert_eq!(
            extension_receiver("public fun TypeMirror.asTypeName(): TypeName", "asTypeName"),
            Some("TypeMirror")
        );
        assert_eq!(
            extension_receiver(
                "fun <T : Any> KClass<T>.asTypeName(): ClassName",
                "asTypeName"
            ),
            Some("KClass")
        );
        assert_eq!(
            extension_receiver("internal fun kotlin.String?.shout(n: Int)", "shout"),
            Some("String")
        );
        assert_eq!(extension_receiver("fun plain(): Int", "plain"), None);
    }
}
