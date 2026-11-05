//! TypeScript declaration pairing, augmentation and interface-group linking over the resolver's
//! symbol tables.

use super::*;

impl<'a> Index<'a> {
    #[cfg(feature = "lang-typescript")]
    pub(super) fn pair_typescript_declarations(&self) -> HashMap<u32, u32> {
        let mut pairs = HashMap::default();
        for (file, record) in self.input.files.iter().enumerate() {
            if record.file_type != Some(FileType::TypeScript) || !record.path.ends_with(".d.ts") {
                continue;
            }
            let Some(facts) = self.facts(file as u32) else {
                continue;
            };
            let module = self.module_of(file as u32);
            for (local, symbol) in facts.symbols.iter().enumerate() {
                if symbol.kind() != SymKind::Fn
                    || symbol.parent.is_some()
                    || !symbol.owner.is_empty()
                    || !symbol.exported
                    || symbol.flags & FLAG_DECLARATION == 0
                {
                    continue;
                }
                let Ok(local) = u32::try_from(local) else {
                    continue;
                };
                let Some(id) = self.base[file].checked_add(local) else {
                    continue;
                };
                let Some(identity) = self.source_identity_of_symbol(id) else {
                    continue;
                };
                if !identity.starts_with(crate::domain::facts::TYPESCRIPT_CALLABLE_IDENTITY_PREFIX)
                {
                    continue;
                }
                let candidates = self.module_symbol(module, &symbol.name, None);
                let declarations: Vec<u32> = candidates
                    .iter()
                    .copied()
                    .filter(|&candidate| {
                        let other = &self.symbols[candidate as usize];
                        other.kind == SymKind::Fn
                            && other.parent == NONE
                            && other.exported
                            && other.flags & FLAG_DECLARATION != 0
                            && self.is_typescript_declaration_file(other.file)
                            && self.source_identity_of_symbol(candidate) == Some(identity)
                    })
                    .collect();
                if !matches!(declarations.as_slice(), [only] if *only == id) {
                    continue;
                }
                let definitions: Vec<u32> = candidates
                    .into_iter()
                    .filter(|&candidate| {
                        let other = &self.symbols[candidate as usize];
                        let path = &self.input.files[other.file as usize].path;
                        other.kind == SymKind::Fn
                            && other.parent == NONE
                            && other.exported
                            && other.flags & FLAG_DECLARATION == 0
                            && (path.ends_with(".ts") || path.ends_with(".tsx"))
                            && self.source_identity_of_symbol(candidate) == Some(identity)
                    })
                    .collect();
                if let [definition] = definitions.as_slice() {
                    pairs.insert(id, *definition);
                }
            }
        }
        pairs
    }

    /// Link only complete, uniquely targeted relative module augmentations.
    #[cfg(feature = "lang-typescript")]
    pub(super) fn link_typescript_augmentations(&mut self) -> Option<bool> {
        let mut groups: HashMap<u32, Vec<u32>> = HashMap::default();
        for (file, record) in self.input.files.iter().enumerate() {
            match record.file_type {
                Some(FileType::Vue | FileType::Svelte | FileType::Astro) => return None,
                Some(FileType::TypeScript) => {}
                _ => continue,
            }
            let facts = self.facts(file as u32)?;
            match facts.source_identity(SourceIdentityKind::TypeScriptScope, 0)? {
                "clear" => continue,
                "augmentations" => {}
                _ => return None,
            }
            for identity in facts
                .source_identities
                .as_deref()?
                .iter()
                .filter(|identity| identity.kind == SourceIdentityKind::TypeScriptAugmentation)
            {
                let augmentation = facts.symbols.get(identity.index as usize)?;
                let directory = modules::parent_dir(&record.path);
                let mut depth = directory.split('/').filter(|part| !part.is_empty()).count();
                for part in identity.identity.split('/') {
                    match part {
                        "" | "." => {}
                        ".." => depth = depth.checked_sub(1)?,
                        _ => depth += 1,
                    }
                }
                let module = self.js_module(file as u32, &identity.identity)?;
                let mut candidates = Vec::new();
                for (other, &other_module) in self.modules.file_module.iter().enumerate() {
                    if other_module != module {
                        continue;
                    }
                    let other_facts = self.facts(other as u32)?;
                    for (local, symbol) in other_facts.symbols.iter().enumerate() {
                        if symbol.parent.is_none()
                            && crate::extract::lang::typescript::declaration_name_may_match(
                                &symbol.name,
                                &augmentation.name,
                            )
                        {
                            candidates.push(self.base[other] + u32::try_from(local).ok()?);
                        }
                    }
                }
                let [target] = candidates.as_slice() else {
                    return None;
                };
                let original = self.symbols.get(*target as usize)?;
                if original.file == file as u32
                    || original.kind != SymKind::Iface
                    || original.parent != NONE
                    || !original.exported
                    || self.file_type(original.file) != Some(FileType::TypeScript)
                    || original.flags & facts::FLAG_TYPESCRIPT_AUGMENTATION != 0
                    || self.interface_canonical.contains_key(target)
                {
                    return None;
                }
                let expected = format!(
                    "interface_declaration:9:interface:{}:{}",
                    augmentation.name.len(),
                    augmentation.name
                );
                if facts.source_identity(SourceIdentityKind::InterfaceHeader, identity.index)
                    != Some(expected.as_str())
                {
                    return None;
                }
                groups
                    .entry(*target)
                    .or_default()
                    .push(self.base[file] + identity.index);
            }
        }
        for (&target, parts) in &groups {
            let original = &self.symbols[target as usize];
            let local = target.checked_sub(self.base[original.file as usize])?;
            let original_facts = self.facts(original.file)?;
            let expected = format!(
                "interface_declaration:9:interface:{}:{}",
                original.name.len(),
                original.name
            );
            if original_facts.source_identity(SourceIdentityKind::InterfaceHeader, local)
                != Some(expected.as_str())
            {
                return None;
            }
            let original_members =
                original_facts.source_identity(SourceIdentityKind::InterfaceMembers, local)?;
            let mut members = Vec::new();
            for &part in parts {
                let symbol = &self.symbols[part as usize];
                let local = part.checked_sub(self.base[symbol.file as usize])?;
                members.push(
                    self.facts(symbol.file)?
                        .source_identity(SourceIdentityKind::InterfaceMembers, local)?,
                );
            }
            if !crate::extract::lang::typescript::imported_base_members_are_disjoint(
                original_members,
                &members,
            ) {
                return None;
            }
            for (index, member) in members.iter().enumerate() {
                if !crate::extract::lang::typescript::imported_base_members_are_disjoint(
                    member,
                    &members[index + 1..],
                ) {
                    return None;
                }
            }
        }
        for (&target, parts) in &groups {
            self.interface_canonical.insert(target, target);
            for &part in parts {
                self.interface_canonical.insert(part, target);
            }
        }
        Some(!groups.is_empty())
    }

    /// Add same-file groups only after all TypeScript scope and imported base evidence is known.
    #[cfg(feature = "lang-typescript")]
    pub(super) fn link_typescript_imported_groups(&mut self) -> bool {
        if self
            .input
            .files
            .iter()
            .enumerate()
            .any(|(file, record)| match record.file_type {
                Some(FileType::Vue | FileType::Svelte | FileType::Astro) => true,
                Some(FileType::TypeScript) => self
                    .facts(file as u32)
                    .and_then(|facts| facts.source_identity(SourceIdentityKind::TypeScriptScope, 0))
                    .is_none_or(|scope| !matches!(scope, "clear" | "augmentations")),
                _ => false,
            })
        {
            return false;
        }
        let mut module_files: HashMap<u32, Vec<u32>> = HashMap::default();
        for (file, record) in self.input.files.iter().enumerate() {
            if record.file_type == Some(FileType::TypeScript) {
                let file = file as u32;
                module_files
                    .entry(self.module_of(file))
                    .or_default()
                    .push(file);
            }
        }
        let mut groups = Vec::new();
        for (file, facts) in self.input.facts.iter().enumerate() {
            if self.file_type(file as u32) != Some(FileType::TypeScript) {
                continue;
            }
            let Some(facts) = facts else { continue };
            if facts
                .symbols
                .iter()
                .any(|symbol| symbol.parent.is_none() && symbol.name.contains('\\'))
            {
                continue;
            }
            let mut names: HashMap<&str, Vec<u32>> = HashMap::default();
            for (local, symbol) in facts
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, symbol)| symbol.parent.is_none())
            {
                if let Ok(local) = u32::try_from(local) {
                    names.entry(&symbol.name).or_default().push(local);
                }
            }
            for parts in names.values().filter(|parts| parts.len() > 1) {
                if let Some(group) =
                    self.prove_typescript_imported_group(file as u32, facts, parts, &module_files)
                {
                    groups.push(group);
                }
            }
        }
        for group in &groups {
            let canonical = group[0];
            for &part in group {
                self.interface_canonical.insert(part, canonical);
            }
        }
        !groups.is_empty()
    }

    #[cfg(feature = "lang-typescript")]
    pub(super) fn prove_typescript_imported_group(
        &self,
        file: u32,
        facts: &Facts,
        parts: &[u32],
        module_files: &HashMap<u32, Vec<u32>>,
    ) -> Option<Vec<u32>> {
        let first = facts.symbols.get(parts[0] as usize)?;
        if first.kind != Some(SymKind::Iface)
            || self
                .interface_canonical
                .contains_key(&(self.base[file as usize] + parts[0]))
        {
            return None;
        }
        if self.typescript_module_has_other_declaration(file, &first.name, module_files) {
            return None;
        }
        let mut child_members = Vec::new();
        let mut base_name = None;
        for &part in parts {
            let symbol = facts.symbols.get(part as usize)?;
            if symbol.kind != Some(SymKind::Iface) || symbol.exported != first.exported {
                return None;
            }
            let mut inherits = facts.refs.iter().filter(|reference| {
                reference.scope == Some(part) && reference.kind == RefKind::Inherit
            });
            let reference = inherits.next()?;
            if inherits.next().is_some() || reference.qualifier.is_some() || reference.arity != 0 {
                return None;
            }
            if base_name.is_some_and(|name| name != reference.name) {
                return None;
            }
            base_name = Some(reference.name.as_str());
            let expected = format!(
                "interface_declaration:9:interface:{}:{}:7:extends:{}:{}",
                first.name.len(),
                first.name,
                reference.name.len(),
                reference.name
            );
            if facts.source_identity(SourceIdentityKind::InterfaceHeader, part)
                != Some(expected.as_str())
            {
                return None;
            }
            child_members.push(facts.source_identity(SourceIdentityKind::InterfaceMembers, part)?);
        }
        let base_name = base_name?;
        if facts
            .symbols
            .iter()
            .any(|symbol| symbol.parent.is_none() && symbol.name == base_name)
        {
            return None;
        }
        let Target::Symbols(targets) =
            self.file_imports.get(file as usize)?.names.get(base_name)?
        else {
            return None;
        };
        let [target] = targets.as_slice() else {
            return None;
        };
        let symbol = self.symbols.get(*target as usize)?;
        if symbol.file == file
            || symbol.kind != SymKind::Iface
            || symbol.parent != NONE
            || !symbol.exported
            || self.file_type(symbol.file) != Some(FileType::TypeScript)
        {
            return None;
        }
        if self.typescript_module_has_other_declaration(symbol.file, &symbol.name, module_files) {
            return None;
        }
        let base_facts = self.facts(symbol.file)?;
        if base_facts
            .symbols
            .iter()
            .any(|symbol| symbol.parent.is_none() && symbol.name.contains('\\'))
            || base_facts
                .symbols
                .iter()
                .filter(|candidate| candidate.parent.is_none() && candidate.name == symbol.name)
                .count()
                != 1
        {
            return None;
        }
        let local = target.checked_sub(self.base[symbol.file as usize])?;
        let expected = format!(
            "interface_declaration:9:interface:{}:{}",
            symbol.name.len(),
            symbol.name
        );
        if base_facts.source_identity(SourceIdentityKind::InterfaceHeader, local)
            != Some(expected.as_str())
        {
            return None;
        }
        let base_members =
            base_facts.source_identity(SourceIdentityKind::InterfaceMembers, local)?;
        if !crate::extract::lang::typescript::imported_base_members_are_disjoint(
            base_members,
            &child_members,
        ) {
            return None;
        }
        for (&part, &canonical) in &self.interface_canonical {
            if canonical != *target || part == *target {
                continue;
            }
            let symbol = self.symbols.get(part as usize)?;
            let local = part.checked_sub(self.base[symbol.file as usize])?;
            let members = self
                .facts(symbol.file)?
                .source_identity(SourceIdentityKind::InterfaceMembers, local)?;
            if !crate::extract::lang::typescript::imported_base_members_are_disjoint(
                members,
                &child_members,
            ) {
                return None;
            }
        }
        Some(
            parts
                .iter()
                .map(|&part| self.base[file as usize] + part)
                .collect(),
        )
    }

    #[cfg(feature = "lang-typescript")]
    pub(super) fn typescript_module_has_other_declaration(
        &self,
        file: u32,
        name: &str,
        module_files: &HashMap<u32, Vec<u32>>,
    ) -> bool {
        let module = self.module_of(file);
        module_files
            .get(&module)
            .into_iter()
            .flatten()
            .any(|&other| {
                other != file
                    && self.facts(other).is_some_and(|facts| {
                        facts.symbols.iter().any(|symbol| {
                            symbol.parent.is_none()
                                && crate::extract::lang::typescript::declaration_name_may_match(
                                    &symbol.name,
                                    name,
                                )
                        })
                    })
            })
    }

    #[cfg(feature = "lang-typescript")]
    pub(super) fn typescript_composite_field_type(
        &self,
        file: u32,
        scope: Option<u32>,
        type_name: &str,
    ) -> Option<(Vec<Vec<u32>>, Confidence)> {
        use facts::{
            TYPESCRIPT_INTERSECTION_FIELD_TYPE_PREFIX, TYPESCRIPT_UNION_FIELD_TYPE_PREFIX,
        };

        let (prefix, separator, union) =
            if type_name.starts_with(TYPESCRIPT_UNION_FIELD_TYPE_PREFIX) {
                (TYPESCRIPT_UNION_FIELD_TYPE_PREFIX, '|', true)
            } else if type_name.starts_with(TYPESCRIPT_INTERSECTION_FIELD_TYPE_PREFIX) {
                (TYPESCRIPT_INTERSECTION_FIELD_TYPE_PREFIX, '&', false)
            } else {
                return None;
            };
        let names: Vec<&str> = type_name.strip_prefix(prefix)?.split(separator).collect();
        if names.len() < 2 || names.iter().any(|name| name.is_empty()) {
            return None;
        }
        let mut types = Vec::with_capacity(names.len());
        for name in names {
            let (mut resolved, confidence) = self.resolve_type(file, None, scope, name)?;
            if confidence != Confidence::Exact || resolved.len() != 1 {
                return None;
            }
            types.push(resolved.pop()?);
        }
        types.sort_unstable();
        types.dedup();
        let branches = if union {
            types
                .into_iter()
                .map(|type_symbol| vec![type_symbol])
                .collect()
        } else {
            vec![types]
        };
        Some((branches, Confidence::Exact))
    }
}
