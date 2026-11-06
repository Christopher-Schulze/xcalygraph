//! .NET module rules (`docs/spec.md` section 6): namespaces shared by C# and VB.NET; `using`
//! and `Imports` make whole namespaces visible, aliases name one type or namespace, `global
//! using` applies to every file; enclosing namespaces are visible; extension methods
//! (`static T M(this R r)`) serve member calls on their receiver.

use super::Ids;
use super::{Imports, Index, Outcome, Target};
use crate::domain::facts::{Facts, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::Confidence;

/// Namespace prefixes of the platform: never external dependencies.
const PLATFORM: [&str; 3] = ["System", "Microsoft", "Windows"];

/// Module key of a .NET namespace.
pub(super) fn namespace_key(namespace: &str) -> String {
    format!("net:{namespace}")
}

impl Index<'_> {
    pub(super) fn dotnet_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        // Enclosing namespaces: `A.B.C` sees `A.B` and `A`.
        let own = facts.package.clone().unwrap_or_default();
        let mut prefix = own.as_str();
        while let Some((parent, _)) = prefix.rsplit_once('.') {
            if let Some(module) = self.modules.get(&namespace_key(parent)) {
                imports.globs.push(module);
            }
            prefix = parent;
        }
        let file_type = self.file_type(file);
        let global = self
            .global_usings
            .iter()
            .filter(|(source, _)| self.file_type(*source) == file_type);
        let usings = facts
            .imports
            .iter()
            .chain(global.map(|(_, import)| *import));
        for import in usings {
            for name in &import.names {
                self.dotnet_import(file, &import.spec, name, import.alias.as_deref(), imports);
            }
        }
    }

    fn dotnet_import(
        &self,
        file: u32,
        spec: &str,
        name: &str,
        alias: Option<&str>,
        imports: &mut Imports,
    ) {
        if let Some(class) = spec.strip_prefix("static:") {
            // `using static A.B.C`: the class's members through the enclosing-class rule only.
            if let Some((namespace, _)) = class.rsplit_once('.')
                && let Some(module) = self.modules.get(&namespace_key(namespace))
            {
                imports.modules.push(module);
            }
            return;
        }
        if name == "*" {
            match self.modules.get(&namespace_key(spec)) {
                Some(module) => {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None => self.dotnet_external(spec, imports),
            }
            return;
        }
        let local = alias.unwrap_or(name).to_owned();
        let full = if spec.is_empty() {
            name.to_owned()
        } else {
            format!("{spec}.{name}")
        };
        if let Some(module) = self.modules.get(&namespace_key(&full)) {
            imports.modules.push(module);
            imports.names.insert(local, Target::Module(module));
            return;
        }
        match self.modules.get(&namespace_key(spec)) {
            Some(module) => {
                imports.modules.push(module);
                let found = self.module_symbol(module, name, None);
                for &symbol in &found {
                    let target_file = self.symbols[symbol as usize].file;
                    if target_file != file {
                        imports.files.push((file, target_file));
                    }
                }
                if !found.is_empty() {
                    imports.names.insert(local, Target::Symbols(found));
                }
            }
            None => {
                imports.names.insert(local, Target::External(String::new()));
                self.dotnet_external(spec, imports);
            }
        }
    }

    /// A namespace outside the root: platform namespaces are dropped, others are external
    /// dependencies labeled by their first two segments.
    fn dotnet_external(&self, spec: &str, imports: &mut Imports) {
        let first = spec.split('.').next().unwrap_or_default();
        if PLATFORM.contains(&first) || spec.is_empty() {
            return;
        }
        imports
            .externals
            .push(spec.split('.').take(2).collect::<Vec<_>>().join("."));
    }

    /// C# extension methods `name` with a `this` parameter in a namespace visible from the file,
    /// narrowed by the receiver's declared type when known.
    pub(super) fn csharp_extension(
        &self,
        file: u32,
        imports: &Imports,
        name: &str,
        receiver: Option<&str>,
    ) -> Option<Outcome> {
        if self.file_type(file) != Some(FileType::CSharp) {
            return None;
        }
        let own = self.module_of(file);
        let mut candidates: Ids = self
            .by_name
            .get(name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&candidate| {
                let symbol = &self.symbols[candidate as usize];
                let module = self.module_of(symbol.file);
                symbol.kind == SymKind::Method
                    && symbol.signature.contains("(this ")
                    && (module == own
                        || imports.globs.contains(&module)
                        || imports.modules.contains(&module))
            })
            .collect();
        if let Some(receiver) = receiver {
            let receiver = receiver
                .split('<')
                .next()
                .unwrap_or(receiver)
                .trim_end_matches('?');
            let matching: Ids = candidates
                .iter()
                .copied()
                .filter(|&candidate| {
                    let signature = &self.symbols[candidate as usize].signature;
                    signature
                        .split_once("(this ")
                        .map(|(_, rest)| rest.split([' ', '<', '?']).next().unwrap_or_default())
                        .is_some_and(|declared| declared.rsplit('.').next() == Some(receiver))
                })
                .collect();
            if !matching.is_empty() {
                candidates = matching;
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        let confidence = if candidates.len() == 1 {
            Confidence::Exact
        } else {
            Confidence::Inferred
        };
        (!candidates.is_empty()).then(|| {
            Outcome::Targets(
                candidates.iter().map(|&c| (c, confidence)).collect(),
                confidence,
            )
        })
    }
}
