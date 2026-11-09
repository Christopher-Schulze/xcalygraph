//! Rust module paths, imports, inline modules, generic bounds and member lookup.

use super::Ids;
use super::*;
use crate::domain::name::Name;

/// Parsed tuple-struct pattern identity: `PATH#INDEX` plus the match scrutinee.
struct VariantPattern<'a> {
    enum_path: Option<&'a str>,
    variant: &'a str,
    index: i64,
    scrutinee: Option<&'a str>,
}

impl<'a> Index<'a> {
    /// Rust impls can live outside the declaring module. Resolve the declared owner there
    /// before associating it with a same-name type from the crate-wide method table.
    pub(super) fn rust_impl_owner_matches(&self, owner: u32, method: u32) -> bool {
        let candidate = &self.symbols[method as usize];
        let target = &self.symbols[owner as usize];
        if candidate.parent != NONE && self.symbols[candidate.parent as usize].kind.is_type() {
            return candidate.parent == owner;
        }
        let scope = method - self.base[candidate.file as usize];
        let Some(facts) = self.facts(candidate.file) else {
            return false;
        };
        let name = facts.symbols[scope as usize].owner.as_str();
        if candidate.file == target.file
            && candidate.parent == target.parent
            && name == target.name.as_str()
        {
            return true;
        }
        let Some(_guard) = DepthGuard::enter() else {
            return false;
        };
        self.resolve_type(candidate.file, None, Some(scope), name)
            .is_some_and(|(types, _)| types.as_slice() == [owner])
    }

    pub(super) fn rust_imports(&self, file: u32, facts: &Facts, imports: &mut Imports) {
        for import in &facts.imports {
            let segments: Vec<&str> = import.spec.split("::").filter(|s| !s.is_empty()).collect();
            let Some(name) = import.names.first() else {
                // `extern crate name [as alias];` names a crate root.
                if let Some(module) = self.rust_crate_root(&import.spec) {
                    let local = import.alias.as_ref().unwrap_or(&import.spec);
                    imports.modules.push(module);
                    imports
                        .names
                        .insert(local.to_string(), Target::Module(module));
                }
                continue;
            };
            let base = if segments.is_empty() {
                self.rust_root_segment(file, name)
                    .map(|target| (target, true))
            } else {
                self.rust_path(file, &segments)
                    .map(|target| (target, false))
            };
            let Some((base, is_leaf)) = base else {
                continue;
            };
            let target = if is_leaf {
                Some(base)
            } else if name == "*" {
                if let Target::Module(module) = base {
                    imports.globs.push(module);
                    imports.modules.push(module);
                }
                None
            } else {
                self.rust_member(file, &base, name)
            };
            let Some(target) = target else { continue };
            let local = import.alias.unwrap_or(*name);
            match &target {
                Target::Module(module) => imports.modules.push(*module),
                Target::External(external) => imports.externals.push(external.clone()),
                Target::Symbols(symbols) => {
                    if let Some(&symbol) = symbols.first() {
                        imports
                            .modules
                            .push(self.module_of(self.symbols[symbol as usize].file));
                    }
                }
            }
            imports.names.insert(local.to_string(), target);
        }
        // `mod child;` declarations make child modules visible by name.
        let module = self.module_of(file);
        let key = self.modules.modules[module as usize].key.clone();
        for (symbol_index, symbol) in facts.symbols.iter().enumerate() {
            if symbol.kind() == SymKind::Mod && symbol.end_line == symbol.line {
                if let Some(reference) = self
                    .rust_path_refs
                    .get(&(file, symbol_index as u32))
                    .copied()
                {
                    if let Some(target) = self.modules.rust_path_target(file, reference)
                        && target != file
                        && !imports.files.contains(&(file, target))
                    {
                        imports.files.push((file, target));
                    }
                    if symbol.parent.is_none()
                        && let Some(child) = self.modules.rust_path_module(file, reference)
                    {
                        imports
                            .names
                            .entry(symbol.name.to_string())
                            .or_insert(Target::Module(child));
                    }
                    continue;
                }
                if symbol.parent.is_some() {
                    continue;
                }
                let Some(child) = self.rust_child(&key, &symbol.name) else {
                    continue;
                };
                imports
                    .names
                    .entry(symbol.name.to_string())
                    .or_insert(Target::Module(child));
                for (other, &other_module) in self.modules.file_module.iter().enumerate() {
                    if other_module == child {
                        imports.files.push((file, other as u32));
                    }
                }
            }
        }
    }

    /// Resolves a Rust path prefix (`crate::a::b`, `super::x`, `other_crate::y`).
    pub(super) fn rust_path(&self, file: u32, segments: &[&str]) -> Option<Target> {
        let module = self.module_of(file);
        let key = self.modules.modules[module as usize].key.as_str();
        let (crate_name, current) = key.split_once("::").unwrap_or((key, ""));
        let current_path: Vec<&str> = if current.contains('/') || current.starts_with("tests::") {
            Vec::new()
        } else {
            current.split("::").filter(|s| !s.is_empty()).collect()
        };
        let mut path: Vec<&str>;
        let mut rest = segments;
        let mut crate_name = crate_name.to_owned();
        match segments.first().copied()? {
            "crate" => {
                path = Vec::new();
                rest = &segments[1..];
            }
            "self" => {
                path = current_path.clone();
                rest = &segments[1..];
            }
            "super" => {
                path = current_path.clone();
                while rest.first() == Some(&"super") {
                    path.pop();
                    rest = &rest[1..];
                }
            }
            first => {
                // `extern crate name as alias;` renames the crate within this file.
                let first = self
                    .facts(file)
                    .and_then(|facts| {
                        facts.imports.iter().find(|import| {
                            import.names.is_empty() && import.alias.as_deref() == Some(first)
                        })
                    })
                    .map_or(first, |import| import.spec.as_str());
                if self.rust_crate_root(first).is_some() && first != crate_name {
                    crate_name = first.to_owned();
                    path = Vec::new();
                    rest = &segments[1..];
                } else {
                    let mut child = current_path.clone();
                    child.push(first);
                    if self
                        .modules
                        .get(&format!("{crate_name}::{}", child.join("::")))
                        .is_some()
                    {
                        path = current_path.clone();
                    } else if self
                        .modules
                        .get(&format!("{crate_name}::{first}"))
                        .is_some()
                    {
                        path = Vec::new();
                    } else if let Some((types, _)) = self.resolve_type(file, None, None, first) {
                        let mut target = Target::Symbols(types);
                        for segment in &segments[1..] {
                            target = self.rust_member(file, &target, segment)?;
                        }
                        return Some(target);
                    } else {
                        return Some(Target::External(first.to_owned()));
                    }
                }
            }
        }
        let base = self
            .modules
            .get(&format!("{crate_name}::{}", path.join("::")))
            .or_else(|| {
                // The crate of an explicit `[[bin]] path` has its root module under the target.
                let root = self
                    .modules
                    .rust_target_root_of(&self.input.files[file as usize].path)
                    .filter(|_| path.is_empty())?;
                self.path_index.get(root).map(|&root| self.module_of(root))
            })?;
        let mut target = Target::Module(base);
        // A re-export leaves the module tree `path` describes; members follow the target from there.
        let mut left_tree = false;
        for segment in rest {
            let mut next_path = path.clone();
            next_path.push(segment);
            if !left_tree
                && let Some(module) = self
                    .modules
                    .get(&format!("{crate_name}::{}", next_path.join("::")))
            {
                path = next_path;
                target = Target::Module(module);
            } else {
                target = self.rust_member(file, &target, segment)?;
                left_tree = true;
            }
        }
        Some(target)
    }

    /// Member `name` of a target: a symbol of a module, or an associated item of a type.
    pub(super) fn rust_member(&self, file: u32, base: &Target, name: &str) -> Option<Target> {
        match base {
            Target::Module(module) => {
                // A child module wins over its `mod name;` declaration symbol.
                let key = &self.modules.modules[*module as usize].key;
                if let Some(child) = self.rust_child(key, name) {
                    return Some(Target::Module(child));
                }
                let found: Ids = self
                    .module_symbol(*module, name, None)
                    .into_iter()
                    .filter(|&symbol| self.symbols[symbol as usize].kind != SymKind::Mod)
                    .collect();
                if found.is_empty() {
                    return self.rust_reexport(*module, name);
                }
                Some(Target::Symbols(found))
            }
            Target::Symbols(symbols) => {
                let found: Ids = symbols
                    .iter()
                    .flat_map(|&symbol| self.members(symbol, name))
                    .collect();
                (!found.is_empty()).then_some(Target::Symbols(found))
            }
            Target::External(external) => {
                let _ = file;
                Some(Target::External(external.clone()))
            }
        }
    }

    /// What a module re-exports as `name`: `pub use` items and globs, `pub extern crate`.
    fn rust_reexport(&self, module: u32, name: &str) -> Option<Target> {
        let _guard = DepthGuard::enter()?;
        let files = self.rust_reexporting.get(&module)?;
        // An explicit item shadows every glob, whatever the order of the `use` lines.
        for globs in [false, true] {
            for &file in files {
                let Some(facts) = self.facts(file) else {
                    continue;
                };
                for import in facts.imports.iter().filter(|import| import.reexport) {
                    let Some(imported) = import.names.first() else {
                        if !globs && import.alias.as_ref().unwrap_or(&import.spec) == name {
                            return self.rust_root_segment(file, &import.spec);
                        }
                        continue;
                    };
                    let glob = imported == "*";
                    if glob != globs || (!glob && import.alias.as_ref().unwrap_or(imported) != name)
                    {
                        continue;
                    }
                    let segments: Vec<&str> =
                        import.spec.split("::").filter(|s| !s.is_empty()).collect();
                    let target = if segments.is_empty() {
                        self.rust_root_segment(file, imported)
                    } else {
                        // A glob of an external crate proves nothing about this name.
                        self.rust_path(file, &segments)
                            .filter(|base| !glob || !matches!(base, Target::External(_)))
                            .and_then(|base| {
                                self.rust_member(file, &base, if glob { name } else { imported })
                            })
                    };
                    if target.is_some() {
                        return target;
                    }
                }
            }
        }
        None
    }

    /// Element type of a `const` or `static` named `name`, from its declaration `: &[&dyn T] =`.
    pub(super) fn rust_collection_element(
        &self,
        file: u32,
        imports: &Imports,
        name: &str,
    ) -> Option<(Ids, Confidence)> {
        let symbol = match imports.names.get(name) {
            Some(Target::Symbols(symbols)) => symbols.first().copied(),
            _ => self
                .module_symbol(self.module_of(file), name, None)
                .first()
                .copied(),
        }?;
        let record = &self.symbols[symbol as usize];
        let local = (symbol - self.base[record.file as usize]) as usize;
        let signature = &self.facts(record.file)?.symbols.get(local)?.signature;
        let (_, declared) = signature.split_once(':')?;
        let declared = declared.split('=').next()?.trim();
        let declared = declared.trim_start_matches('&');
        let declared = declared.strip_prefix("'static ").unwrap_or(declared).trim();
        let element = declared.strip_prefix('[')?.split([';', ']']).next()?;
        let element = crate::extract::engine::type_name(element);
        self.resolve_type(record.file, None, None, &element)
            .map(|(types, _)| (types, Confidence::Inferred))
    }

    /// `self::name` (qualifier `self::`, unlike the receiver of `self.name()`), `super::name` and
    /// `crate::name` from `scope`: inline modules around the scope count as path levels before the
    /// module of the file.
    pub(super) fn rust_relative_member(
        &self,
        file: u32,
        scope: Option<u32>,
        qualifier: &str,
        name: &str,
    ) -> Option<Ids> {
        let facts = self.facts(file)?;
        let mut inline = Vec::new();
        let mut current = scope;
        while let Some(index) = current {
            let symbol = facts.symbols.get(index as usize)?;
            if symbol.kind() == SymKind::Mod {
                inline.push(index);
            }
            current = symbol.parent;
        }
        let (enclosing, path) = match qualifier {
            "self::" => (inline.first(), "self"),
            "super" => (
                inline.get(1),
                if inline.is_empty() { "super" } else { "self" },
            ),
            _ => (None, "crate"),
        };
        if let Some(&module) = enclosing {
            let base = self.base[file as usize];
            let found: Ids = facts
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, symbol)| {
                    symbol.parent == Some(module)
                        && *symbol.name == *name
                        && symbol.kind() != SymKind::Mod
                })
                .map(|(index, _)| base + index as u32)
                .collect();
            return (!found.is_empty()).then_some(found);
        }
        let module = self.rust_path(file, &[path])?;
        match self.rust_member(file, &module, name)? {
            Target::Symbols(symbols) => Some(symbols),
            _ => None,
        }
    }

    // ---------------------------------------------------------------- lookups

    /// Members `name` of the inline module path `qualifier` declared in `file`.
    pub(super) fn rust_inline_module(&self, file: u32, qualifier: &str, name: &str) -> Option<Ids> {
        let facts = self.facts(file)?;
        let base = self.base[file as usize];
        let mut parent: Option<u32> = None;
        let mut external_target = None;
        for segment in qualifier.split("::") {
            if let Some(target) = external_target.as_ref() {
                external_target = self.rust_member(file, target, segment);
                if external_target.is_none() {
                    return Some(Ids::new());
                }
                continue;
            }
            let found = facts.symbols.iter().position(|symbol| {
                symbol.kind() == SymKind::Mod
                    && *symbol.name == *segment
                    && (parent.is_none() || symbol.parent == parent)
            })?;
            parent = Some(found as u32);
            if let Some(reference) = self.rust_path_refs.get(&(file, found as u32)).copied() {
                let Some(module) = self.modules.rust_path_module(file, reference) else {
                    return Some(Ids::new());
                };
                external_target = Some(Target::Module(module));
            }
        }
        if let Some(target) = external_target {
            return match self.rust_member(file, &target, name) {
                Some(Target::Symbols(symbols)) => Some(symbols),
                Some(Target::Module(_) | Target::External(_)) => Some(Ids::new()),
                None => Some(Ids::new()),
            };
        }
        let module = parent?;
        let members: Ids = facts
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| symbol.parent == Some(module) && *symbol.name == *name)
            .map(|(local, _)| base + local as u32)
            .collect();
        (!members.is_empty()).then_some(members)
    }

    /// Resolves a Rust function type parameter only through its source-backed trait bounds.
    /// `Some(empty)` means the binding is generic but no local trait target is provable, which
    /// prevents a same-named concrete type elsewhere in the crate from becoming a false target.
    pub(super) fn rust_generic_bound_types(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        parameter: &str,
    ) -> Option<Ids> {
        if self.file_type(file) != Some(FileType::Rust) {
            return None;
        }
        let scope = scope?;
        let key = (file, scope);
        if !self
            .rust_generic_parameters
            .get(&key)
            .is_some_and(|parameters| parameters.iter().any(|known| **known == *parameter))
        {
            return None;
        }
        let mut traits = Vec::new();
        for (_, bound) in self
            .rust_generic_bounds
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|(known, _)| **known == *parameter)
        {
            let Some((types, Confidence::Exact)) =
                self.resolve_type(file, Some(imports), Some(scope), bound)
            else {
                continue;
            };
            for trait_symbol in types {
                if self.symbols[trait_symbol as usize].kind == SymKind::Trait
                    && !traits.contains(&trait_symbol)
                {
                    traits.push(trait_symbol);
                }
            }
        }
        Some(traits.into())
    }

    /// Whether a qualified path leaves the root, respecting locally imported module aliases.
    pub(super) fn rust_external_path(&self, file: u32, imports: &Imports, path: &str) -> bool {
        if !path.contains("::") {
            return false;
        }
        let segments: Vec<&str> = path.split("::").collect();
        let target = if let Some(root) = imports.names.get(segments[0]) {
            segments[1..]
                .iter()
                .try_fold(root.clone(), |target, segment| {
                    self.rust_member(file, &target, segment)
                })
        } else {
            self.rust_path(file, &segments)
        };
        matches!(target, Some(Target::External(_)))
    }

    /// Pending Rust result layers of a source-resolved call. Ambiguous returns stay unknown.
    pub(super) fn call_result_layers(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        callee: &str,
    ) -> Option<usize> {
        let (qualifier, name) = split_qualified(callee)
            .map_or((None, callee), |(qualifier, name)| (Some(qualifier), name));
        let call = Ref {
            kind: RefKind::Call,
            qualifier: qualifier.map(Name::from),
            name: name.into(),
            scope: reference.scope,
            line: reference.line,
            arity: facts::NO_ARITY,
            byte: reference.byte,
        };
        let _guard = DepthGuard::enter()?;
        let Outcome::Targets(targets, _) = self.resolve_ref(file, imports, &call) else {
            // Existing constructor inference may still establish a plain type without a
            // local method declaration; binding_type independently checks that evidence.
            return Some(0);
        };
        let mut layers = None;
        for (symbol, _) in targets {
            let owner = self.symbols[symbol as usize].file;
            let scope = Some(symbol - self.base[owner as usize]);
            let next = self
                .returns
                .get(&symbol)
                .map_or(0, |result| self.rust_result(owner, scope, result).1);
            if layers.is_some_and(|previous| previous != next) {
                return None;
            }
            layers = Some(next);
        }
        layers
    }

    /// Keep standard result layers apart from methods declared on a local type of the same name.
    pub(super) fn rust_result<'r>(
        &self,
        file: u32,
        scope: Option<u32>,
        mut result: &'r str,
    ) -> (&'r str, usize) {
        let mut layers = 0;
        loop {
            let Some((outer, inner)) = result.trim_start().split_once('<') else {
                return (result, layers);
            };
            if !matches!(
                outer.trim(),
                "Result"
                    | "Option"
                    | "io::Result"
                    | "anyhow::Result"
                    | "std::result::Result"
                    | "core::result::Result"
                    | "std::option::Option"
                    | "core::option::Option"
            ) || matches!(
                self.resolve_type(file, None, scope, outer.trim()),
                Some((_, Confidence::Exact))
            ) {
                return (result, layers);
            }
            layers += 1;
            result = inner;
        }
    }

    /// Field type of a tuple-struct pattern binding: `pattern:PATH#INDEX` plus the match
    /// scrutinee (`~EXPR`) for unqualified variants, proven through the variant table.
    /// `Ok`/`Err`/`Some` stay unresolved: generic arguments live in no fact.
    pub(super) fn rust_variant_field_type(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        binding: &Binding,
        coded: &str,
    ) -> Option<(Ids, Confidence)> {
        let (head, scrutinee) = coded
            .split_once('~')
            .map_or((coded, None), |(head, tail)| (head, Some(tail)));
        let (path, index) = head.rsplit_once('#')?;
        let index = index.parse().ok()?;
        let (enum_path, variant) = match path.rsplit_once("::") {
            Some((head, tail)) if !head.is_empty() && !tail.is_empty() => (Some(head), tail),
            None if !path.is_empty() => (None, path),
            _ => return None,
        };
        let scrutinee = scrutinee.filter(|text| !text.is_empty());
        let pattern = VariantPattern {
            enum_path,
            variant,
            index,
            scrutinee,
        };
        self.variant_table_field(file, imports, scope, binding, &pattern)
    }

    /// Declared variant-field type through the enum of a qualified path or scrutinee.
    fn variant_table_field(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        binding: &Binding,
        pattern: &VariantPattern,
    ) -> Option<(Ids, Confidence)> {
        let (mut enums, confidence) = match pattern.enum_path {
            Some(path) => self.resolve_type(file, Some(imports), scope, path)?,
            None => self.scrutinee_types(file, imports, scope, binding, pattern.scrutinee?)?,
        };
        enums.retain(|symbol| self.symbols[*symbol as usize].kind == SymKind::Enum);
        let &[owner] = enums.as_slice() else {
            return None;
        };
        let owner_file = self.symbols[owner as usize].file;
        let local = owner - self.base[owner_file as usize];
        let fields = &self.facts(owner_file)?.variant_fields;
        let group = (local, pattern.variant);
        let start = fields.partition_point(|field| (field.owner, field.variant.as_str()) < group);
        let rest = &fields[start..];
        let end = rest.partition_point(|field| (field.owner, field.variant.as_str()) == group);
        let fields = &rest[..end];
        let index = if pattern.index < 0 {
            i64::from(fields.last()?.index) + 1 + pattern.index
        } else {
            pattern.index
        };
        let index = u32::try_from(index).ok()?;
        let field = &fields[fields
            .binary_search_by_key(&index, |field| field.index)
            .ok()?];
        self.resolve_type(owner_file, None, Some(local), field.type_name.as_str())
            .map(|(types, field_confidence)| (types, confidence.max(field_confidence)))
    }

    /// Types of a match scrutinee: a call result like a call binding, else a receiver chain.
    fn scrutinee_types(
        &self,
        file: u32,
        imports: &Imports,
        scope: Option<u32>,
        binding: &Binding,
        scrutinee: &str,
    ) -> Option<(Ids, Confidence)> {
        if scrutinee.contains('(') {
            let receiver = Binding {
                scope,
                name: String::new().into(),
                type_name: scrutinee.into(),
                line: binding.line,
                visibility: None,
                value_byte: binding.value_byte,
                declared: false,
            };
            return self.binding_type(file, imports, scope, &receiver);
        }
        let chain: Vec<&str> = scrutinee.trim_start_matches('*').split('.').collect();
        if chain.is_empty() || !chain.iter().all(|segment| is_identifier(segment)) {
            return None;
        }
        let reference = Ref {
            kind: RefKind::Type,
            name: String::new().into(),
            qualifier: None,
            scope,
            line: binding.line,
            byte: binding.value_byte,
            arity: facts::NO_ARITY,
        };
        match self.receiver_type(file, imports, &reference, &chain) {
            Receiver::Known(types, confidence) => Some((types, confidence)),
            _ => None,
        }
    }
}
