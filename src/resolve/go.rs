//! Go imports and structural interface implementations.

use super::Ids;
use super::*;

impl<'a> Index<'a> {
    pub(super) fn go_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let target = self.modules.get(&import.spec);
            let local = import.alias.unwrap_or_else(|| match target {
                Some(module) => self.go_package_name(module).into(),
                None => import
                    .spec
                    .rsplit('/')
                    .next()
                    .unwrap_or(&import.spec)
                    .into(),
            });
            if local == "_" {
                continue;
            }
            match target {
                Some(module) => {
                    if local == "." {
                        imports.globs.push(module);
                    } else {
                        imports
                            .names
                            .insert(local.to_string(), Target::Module(module));
                    }
                    imports.modules.push(module);
                }
                None => {
                    let external = self.go_external(&import.spec);
                    imports
                        .names
                        .insert(local.to_string(), Target::External(external.clone()));
                    if let Some(external) = external.strip_prefix("ext:") {
                        imports.externals.push(external.to_owned());
                    }
                }
            }
        }
        let _ = file;
    }

    pub(super) fn go_structural_implementations(&self) -> Vec<(u32, u32, Confidence, u32)> {
        let mut interfaces: Vec<(u32, Vec<(&str, usize)>)> = Vec::new();
        let mut concrete: Ids = Ids::new();
        for (id, symbol) in self.symbols.iter().enumerate() {
            if self.file_type(symbol.file) != Some(FileType::Go) {
                continue;
            }
            match symbol.kind {
                SymKind::Iface => {
                    let methods: Vec<(&str, usize)> = self
                        .symbols
                        .iter()
                        .filter(|child| child.parent == id as u32 && child.kind == SymKind::Method)
                        .map(|child| (child.name.as_str(), param_count(&child.signature)))
                        .collect();
                    if !methods.is_empty() {
                        interfaces.push((id as u32, methods));
                    }
                }
                SymKind::Struct | SymKind::Alias => concrete.push(id as u32),
                _ => {}
            }
        }
        // Method names with parameter counts per (scope, owner), computed once.
        let mut by_owner: HashMap<(u32, &str), HashMap<&str, usize>> = HashMap::default();
        for (&(scope, owner, name), methods) in &self.methods {
            for &method in methods {
                let params = param_count_method(&self.symbols[method as usize].signature);
                by_owner
                    .entry((scope, owner))
                    .or_default()
                    .insert(name, params);
            }
        }
        let empty = HashMap::default();
        let method_sets: Vec<&HashMap<&str, usize>> = concrete
            .iter()
            .map(|&type_symbol| {
                let symbol = &self.symbols[type_symbol as usize];
                let scope = self.scope_ids[self.module_of(symbol.file) as usize];
                by_owner
                    .get(&(scope, symbol.name.as_str()))
                    .unwrap_or(&empty)
            })
            .collect();
        let mut module_imports: HashSet<(u32, u32)> = HashSet::default();
        for (file, imports) in self.file_imports.iter().enumerate() {
            let source = self.modules.file_module[file];
            for &target in &imports.modules {
                module_imports.insert((source, target));
            }
        }
        let mut pairs = Vec::new();
        for (position, &type_symbol) in concrete.iter().enumerate() {
            let set = method_sets[position];
            if set.is_empty() {
                continue;
            }
            let type_module = self.module_of(self.symbols[type_symbol as usize].file);
            for (interface, methods) in &interfaces {
                let interface_symbol = &self.symbols[*interface as usize];
                let interface_module = self.module_of(interface_symbol.file);
                // A test type implements only test interfaces; an unexported interface of another
                // package can only receive types its package imports.
                if self.is_test(type_symbol) && !self.is_test(*interface) {
                    continue;
                }
                if !interface_symbol.exported
                    && interface_module != type_module
                    && !module_imports.contains(&(interface_module, type_module))
                {
                    continue;
                }
                if methods
                    .iter()
                    .all(|(name, params)| set.get(name) == Some(params))
                {
                    let line = self.symbols[type_symbol as usize].line;
                    pairs.push((type_symbol, *interface, Confidence::Inferred, line));
                }
            }
        }
        pairs
    }

    // ---------------------------------------------------------------- graph
}
