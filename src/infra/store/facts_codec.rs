//! Encoding of per-file facts inside fact shards.

use super::codec::{DecodeError, DecodeResult, Decoder, Encoder};
use crate::domain::facts::{
    AssetMeta, Binding, ConfigKey, Coverage, EnvDef, EnvRead, EnvSource, Facts, FieldName, Fields,
    GoEmbed, Import, JuliaCallArgument, Manifest, Ref, RefKind, RustTargetRoot, SourceIdentity,
    SourceIdentityKind, SymKind, Symbol, TextSegment, ValueKind,
};

/// Search terms as a token list, so each token is interned once in the shard's string table.
fn terms(encoder: &mut Encoder, value: &str) {
    let tokens: Vec<&str> = value.split(' ').filter(|token| !token.is_empty()).collect();
    encoder.var_usize(tokens.len());
    for token in tokens {
        encoder.str(token);
    }
}

fn read_terms(decoder: &mut Decoder) -> DecodeResult<String> {
    let mut value = String::new();
    for index in 0..decoder.len()? {
        if index > 0 {
            value.push(' ');
        }
        value.push_str(decoder.str()?);
    }
    Ok(value)
}

fn typescript_namespace_path(symbols: &[Symbol], symbol: &Symbol) -> Option<Vec<String>> {
    let mut scopes = Vec::new();
    let mut parent = symbol.parent;
    let mut remaining = symbols.len();
    while let Some(index) = parent {
        if remaining == 0 {
            return None;
        }
        remaining -= 1;
        let module = symbols.get(index as usize)?;
        if module.kind() != SymKind::Mod
            || *module.name == *"global"
            || module.name.starts_with('"')
            || module.name.starts_with('\'')
        {
            return None;
        }
        let parts: Vec<_> = module
            .name
            .split('.')
            .map(|part| part.trim().to_owned())
            .collect();
        if parts.iter().any(String::is_empty) {
            return None;
        }
        scopes.push(parts);
        parent = module.parent;
    }
    scopes.reverse();
    Some(scopes.into_iter().flatten().collect())
}

fn same_typescript_namespace(symbols: &[Symbol], left: &Symbol, right: &Symbol) -> bool {
    typescript_namespace_path(symbols, left)
        .zip(typescript_namespace_path(symbols, right))
        .is_some_and(|(left, right)| left == right)
}

fn opt_index(encoder: &mut Encoder, value: Option<u32>) {
    encoder.var(value.map_or(0, |index| u64::from(index) + 1));
}

fn read_opt_index(decoder: &mut Decoder) -> DecodeResult<Option<u32>> {
    let raw = decoder.var_u32()?;
    Ok(raw.checked_sub(1))
}

fn opt_u32(encoder: &mut Encoder, value: Option<u32>) {
    encoder.var(value.map_or(0, |line| u64::from(line) + 1));
}

fn read_opt_u32(decoder: &mut Decoder) -> DecodeResult<Option<u32>> {
    match decoder.var()? {
        0 => Ok(None),
        value => u32::try_from(value - 1)
            .map(Some)
            .map_err(|_| DecodeError("u32 overflow")),
    }
}

pub fn encode(encoder: &mut Encoder, facts: &Facts) {
    encoder.opt_str(facts.package.as_deref());
    encoder.var_u32(facts.lines);
    encoder.str(&facts.doc);
    terms(encoder, &facts.terms);
    encoder.u8(u8::from(facts.entry));
    encoder.u8(facts.coverage as u8);
    encoder.var_usize(facts.symbols.len());
    for symbol in &facts.symbols {
        encoder.u8(symbol.kind() as u8);
        encoder.str(&symbol.name);
        opt_index(encoder, symbol.parent);
        encoder.str(&symbol.owner);
        encoder.var_u32(symbol.line);
        encoder.var_u32(symbol.end_line.saturating_sub(symbol.line));
        encoder.str(&symbol.signature);
        encoder.str(&symbol.doc);
        terms(encoder, &symbol.terms);
        encoder.var_u32(u32::from(u8::from(symbol.exported)) | (u32::from(symbol.flags) << 1));
    }
    encoder.var_usize(facts.imports.len());
    for import in &facts.imports {
        encoder.str(&import.spec);
        encoder.opt_str(import.alias.as_deref());
        encoder.strs(&import.names);
        encoder.var_u32(import.line);
        encoder.u8(u8::from(import.reexport) | (u8::from(import.type_only) << 1));
    }
    encoder.var_usize(facts.refs.len());
    for reference in &facts.refs {
        encoder.u8(reference.kind as u8);
        encoder.str(&reference.name);
        encoder.opt_str(reference.qualifier.as_deref());
        opt_index(encoder, reference.scope);
        encoder.var_u32(reference.line);
        opt_u32(
            encoder,
            (reference.byte != crate::domain::facts::NO_BYTE).then_some(reference.byte),
        );
        encoder.u8(reference.arity);
    }
    encoder.var_usize(facts.go_embeds.len());
    for embed in &facts.go_embeds {
        encoder.var_u32(embed.symbol);
        encoder.var_u32(embed.line);
        encoder.var_usize(embed.pattern.len());
        for &byte in &embed.pattern {
            encoder.u8(byte);
        }
    }
    encoder.var_usize(facts.julia_builtin_calls.len());
    for &reference in &facts.julia_builtin_calls {
        encoder.var_u32(reference);
    }
    encoder.var_usize(facts.julia_call_arguments.len());
    for argument in &facts.julia_call_arguments {
        encoder.var_u32(argument.reference);
        encoder.u8(argument.position);
        encoder.str(&argument.binding);
    }
    encoder.var_usize(facts.bindings.len());
    for binding in &facts.bindings {
        opt_index(encoder, binding.scope);
        encoder.str(&binding.name);
        encoder.str(&binding.type_name);
        encoder.var_u32(binding.line);
        opt_u32(
            encoder,
            binding.visibility.as_ref().map(|range| range.start),
        );
        if let Some(range) = &binding.visibility {
            encoder.var_u32(range.end);
        }
        opt_u32(
            encoder,
            (binding.value_byte != crate::domain::facts::NO_BYTE).then_some(binding.value_byte),
        );
        encoder.u8(u8::from(binding.declared));
    }
    encoder.var_usize(facts.envs.len());
    for env in &facts.envs {
        encoder.str(&env.name);
        opt_index(encoder, env.scope);
        encoder.var_u32(env.line);
    }
    encoder.var_usize(facts.fields.len());
    for fields in &facts.fields {
        encoder.var_u32(fields.owner);
        encoder.var_usize(fields.names.len());
        for name in &fields.names {
            match name {
                FieldName::Text(value) => {
                    encoder.u8(0);
                    encoder.str(value);
                }
                FieldName::Utf16(units) => {
                    encoder.u8(1);
                    encoder.var_usize(units.len());
                    for &unit in units {
                        encoder.var_u32(u32::from(unit));
                    }
                }
            }
        }
        encoder.strs(&fields.types);
    }
    encoder.var_usize(facts.env_defs.len());
    for def in &facts.env_defs {
        encoder.str(&def.name);
        encoder.u8(def.source as u8);
        encoder.var_u32(def.line);
    }
    encoder.var_usize(facts.config_keys.len());
    for key in &facts.config_keys {
        encoder.str(&key.path);
        encoder.u8(key.kind as u8);
        encoder.var_u32(key.line);
    }
    encoder.var_usize(facts.texts.len());
    for text in &facts.texts {
        encoder.str(&text.heading);
        encoder.var_u32(text.locator);
        encoder.str(&text.text);
    }
    match &facts.asset {
        Some(asset) => {
            encoder.u8(1);
            encoder.str(&asset.format);
            encoder.var_u32(asset.width);
            encoder.var_u32(asset.height);
            encoder.var_u32(asset.duration_ms);
        }
        None => encoder.u8(0),
    }
    match &facts.manifest {
        Some(manifest) => {
            encoder.u8(1);
            encoder.opt_str(manifest.module.as_deref());
            encoder.var_usize(manifest.deps.len());
            for (name, requirement) in &manifest.deps {
                encoder.str(name);
                encoder.str(requirement);
            }
            encoder.strs(&manifest.members);
            encoder.strs(&manifest.source_roots);
            encoder.strs(&manifest.include_dirs);
            encoder.strs(&manifest.entrypoints);
            encoder.var_usize(manifest.rust_target_roots.len());
            for root in &manifest.rust_target_roots {
                encoder.str(&root.path);
                encoder.u8(u8::from(root.is_library));
            }
            encoder.strs(&manifest.project_links);
        }
        None => encoder.u8(0),
    }
    let identities = facts.source_identities.as_deref().unwrap_or_default();
    encoder.var_usize(identities.len());
    for identity in identities {
        encoder.u8(identity.kind as u8);
        encoder.var_u32(identity.index);
        encoder.str(&identity.identity);
    }
}

pub fn decode(decoder: &mut Decoder) -> DecodeResult<Facts> {
    let mut facts = Facts {
        package: decoder.opt_string()?,
        lines: decoder.var_u32()?,
        doc: decoder.string()?,
        terms: read_terms(decoder)?,
        entry: decoder.u8()? != 0,
        coverage: Coverage::from_u8(decoder.u8()?).ok_or(DecodeError("coverage"))?,
        ..Facts::default()
    };
    let count = decoder.len()?;
    facts.symbols.reserve_exact(count);
    for _ in 0..count {
        let kind = SymKind::from_u8(decoder.u8()?).ok_or(DecodeError("symbol kind"))?;
        let name = decoder.string()?;
        let parent = read_opt_index(decoder)?;
        let owner = decoder.string()?;
        let line = decoder.var_u32()?;
        let end_line = line.saturating_add(decoder.var_u32()?);
        let signature = decoder.string()?;
        let doc = decoder.string()?;
        let terms = read_terms(decoder)?;
        let bits = decoder.var_u32()?;
        let flags = u8::try_from(bits >> 1).map_err(|_| DecodeError("symbol flags overflow"))?;
        facts.symbols.push(Symbol {
            kind: Some(kind),
            name: name.into(),
            parent,
            owner: owner.into(),
            line,
            end_line,
            signature: signature.into(),
            doc: doc.into(),
            terms: terms.into(),
            exported: bits & 1 != 0,
            flags,
        });
    }
    let count = decoder.len()?;
    facts.imports.reserve_exact(count);
    for _ in 0..count {
        let spec = decoder.name()?;
        let alias = decoder.opt_name()?;
        let names = decoder.names()?;
        let line = decoder.var_u32()?;
        let flags = decoder.u8()?;
        if flags & !3 != 0 {
            return Err(DecodeError("import flags"));
        }
        facts.imports.push(Import {
            spec,
            alias,
            names,
            line,
            reexport: flags & 1 != 0,
            type_only: flags & 2 != 0,
        });
    }
    let count = decoder.len()?;
    facts.refs.reserve_exact(count);
    for _ in 0..count {
        facts.refs.push(Ref {
            kind: RefKind::from_u8(decoder.u8()?).ok_or(DecodeError("ref kind"))?,
            name: decoder.name()?,
            qualifier: decoder.opt_name()?,
            scope: read_opt_index(decoder)?,
            line: decoder.var_u32()?,
            byte: read_opt_u32(decoder)?.unwrap_or(crate::domain::facts::NO_BYTE),
            arity: decoder.u8()?,
        });
    }
    let count = decoder.len()?;
    facts.go_embeds.reserve_exact(count);
    for _ in 0..count {
        let symbol = decoder.var_u32()?;
        let line = decoder.var_u32()?;
        let length = decoder.len()?;
        if !facts
            .symbols
            .get(symbol as usize)
            .is_some_and(|item| item.kind() == SymKind::Var)
            || line == 0
        {
            return Err(DecodeError("Go embed owner"));
        }
        let pattern = (0..length)
            .map(|_| decoder.u8())
            .collect::<DecodeResult<Vec<_>>>()?;
        facts.go_embeds.push(GoEmbed {
            symbol,
            line,
            pattern,
        });
    }
    let mut previous_builtin = None;
    let count = decoder.len()?;
    facts.julia_builtin_calls.reserve_exact(count);
    for _ in 0..count {
        let reference = decoder.var_u32()?;
        let Some(fact) = facts.refs.get(reference as usize) else {
            return Err(DecodeError("Julia builtin reference"));
        };
        if fact.kind != RefKind::Call
            || fact.qualifier.is_some()
            || previous_builtin.is_some_and(|previous| previous >= reference)
        {
            return Err(DecodeError("Julia builtin reference"));
        }
        facts.julia_builtin_calls.push(reference);
        previous_builtin = Some(reference);
    }
    let mut previous_argument = None;
    let count = decoder.len()?;
    facts.julia_call_arguments.reserve_exact(count);
    for _ in 0..count {
        let reference = decoder.var_u32()?;
        let position = decoder.u8()?;
        let binding = decoder.string()?;
        let Some(fact) = facts.refs.get(reference as usize) else {
            return Err(DecodeError("Julia call argument reference"));
        };
        let key = (reference, position);
        if fact.kind != RefKind::Call
            || (fact.arity != crate::domain::facts::NO_ARITY && position >= fact.arity)
            || binding.is_empty()
            || previous_argument.is_some_and(|previous| previous >= key)
        {
            return Err(DecodeError("Julia call argument"));
        }
        facts.julia_call_arguments.push(JuliaCallArgument {
            reference,
            position,
            binding,
        });
        previous_argument = Some(key);
    }
    let count = decoder.len()?;
    facts.bindings.reserve_exact(count);
    for _ in 0..count {
        facts.bindings.push(Binding {
            scope: read_opt_index(decoder)?,
            name: decoder.name()?,
            type_name: decoder.name()?,
            line: decoder.var_u32()?,
            visibility: match read_opt_u32(decoder)? {
                Some(start) => {
                    let end = decoder.var_u32()?;
                    if start >= end {
                        return Err(DecodeError("binding visibility"));
                    }
                    Some(start..end)
                }
                None => None,
            },
            value_byte: read_opt_u32(decoder)?.unwrap_or(crate::domain::facts::NO_BYTE),
            declared: decoder.u8()? != 0,
        });
    }
    let count = decoder.len()?;
    facts.envs.reserve_exact(count);
    for _ in 0..count {
        facts.envs.push(EnvRead {
            name: decoder.string()?,
            scope: read_opt_index(decoder)?,
            line: decoder.var_u32()?,
        });
    }
    let count = decoder.len()?;
    facts.fields.reserve_exact(count);
    for _ in 0..count {
        let owner = decoder.var_u32()?;
        let mut names = Vec::new();
        for _ in 0..decoder.len()? {
            let name = match decoder.u8()? {
                0 => FieldName::Text(decoder.string()?),
                1 => {
                    let mut units = Vec::new();
                    for _ in 0..decoder.len()? {
                        units.push(
                            u16::try_from(decoder.var_u32()?)
                                .map_err(|_| DecodeError("field UTF-16 unit"))?,
                        );
                    }
                    if String::from_utf16(&units).is_ok() {
                        return Err(DecodeError("noncanonical field UTF-16"));
                    }
                    FieldName::Utf16(units)
                }
                _ => return Err(DecodeError("field name tag")),
            };
            names.push(name);
        }
        facts.fields.push(Fields {
            owner,
            names,
            types: decoder.strings()?,
        });
    }
    let count = decoder.len()?;
    facts.env_defs.reserve_exact(count);
    for _ in 0..count {
        facts.env_defs.push(EnvDef {
            name: decoder.string()?,
            source: EnvSource::from_u8(decoder.u8()?).ok_or(DecodeError("env source"))?,
            line: decoder.var_u32()?,
        });
    }
    let count = decoder.len()?;
    facts.config_keys.reserve_exact(count);
    for _ in 0..count {
        facts.config_keys.push(ConfigKey {
            path: decoder.string()?,
            kind: ValueKind::from_u8(decoder.u8()?).ok_or(DecodeError("value kind"))?,
            line: decoder.var_u32()?,
        });
    }
    let count = decoder.len()?;
    facts.texts.reserve_exact(count);
    for _ in 0..count {
        facts.texts.push(TextSegment {
            heading: decoder.string()?,
            locator: decoder.var_u32()?,
            text: decoder.string()?,
        });
    }
    if decoder.u8()? == 1 {
        facts.asset = Some(AssetMeta {
            format: decoder.string()?,
            width: decoder.var_u32()?,
            height: decoder.var_u32()?,
            duration_ms: decoder.var_u32()?,
        });
    }
    if decoder.u8()? == 1 {
        let module = decoder.opt_string()?;
        let mut deps = Vec::new();
        for _ in 0..decoder.len()? {
            deps.push((decoder.string()?, decoder.string()?));
        }
        facts.manifest = Some(Manifest {
            module,
            deps,
            members: decoder.strings()?,
            source_roots: decoder.strings()?,
            include_dirs: decoder.strings()?,
            entrypoints: decoder.strings()?,
            rust_target_roots: {
                let mut roots = Vec::new();
                for _ in 0..decoder.len()? {
                    roots.push(RustTargetRoot {
                        path: decoder.string()?,
                        is_library: match decoder.u8()? {
                            0 => false,
                            1 => true,
                            _ => return Err(DecodeError("Rust target root kind")),
                        },
                    });
                }
                roots
            },
            project_links: decoder.strings()?,
        });
    }
    let mut identities: Vec<SourceIdentity> = Vec::new();
    let mut previous = None;
    for _ in 0..decoder.len()? {
        let kind = SourceIdentityKind::from_u8(decoder.u8()?)
            .ok_or(DecodeError("source identity kind"))?;
        let index = decoder.var_u32()?;
        let key = (kind, index);
        let limit = match kind {
            SourceIdentityKind::Symbol
            | SourceIdentityKind::InterfaceHeader
            | SourceIdentityKind::InterfaceMembers
            | SourceIdentityKind::InterfaceGroup
            | SourceIdentityKind::InterfaceGenericFields
            | SourceIdentityKind::TypeScriptAugmentation
            | SourceIdentityKind::PythonDirectDefinition => facts.symbols.len(),
            SourceIdentityKind::Reference
            | SourceIdentityKind::PythonShadowedImport
            | SourceIdentityKind::PythonCallableImportFallback => facts.refs.len(),
            SourceIdentityKind::TypeScriptScope
            | SourceIdentityKind::TypeScriptComplexExports
            | SourceIdentityKind::TypeScriptModuleSyntax
            | SourceIdentityKind::TypeScriptProject
            | SourceIdentityKind::TypeScriptGlobalAugmentationScope
            | SourceIdentityKind::TypeScriptPackageType
            | SourceIdentityKind::CommonJsExport => 1,
            SourceIdentityKind::CommonJsRequire
            | SourceIdentityKind::PythonDirectImport
            | SourceIdentityKind::PythonImportScope
            | SourceIdentityKind::PythonImportAfterTry
            | SourceIdentityKind::PythonImportOptionalTry
            | SourceIdentityKind::PythonImportNoModuleWrite => facts.imports.len(),
            SourceIdentityKind::TypeScriptGlobalAugmentation => facts.symbols.len(),
        };
        if usize::try_from(index).map_or(true, |index| index >= limit)
            || previous.is_some_and(|previous| previous >= key)
            || (matches!(
                kind,
                SourceIdentityKind::InterfaceHeader
                    | SourceIdentityKind::InterfaceMembers
                    | SourceIdentityKind::InterfaceGroup
                    | SourceIdentityKind::InterfaceGenericFields
                    | SourceIdentityKind::TypeScriptAugmentation
                    | SourceIdentityKind::TypeScriptGlobalAugmentation
            ) && facts
                .symbols
                .get(index as usize)
                .is_none_or(|symbol| symbol.kind != Some(SymKind::Iface)))
        {
            return Err(DecodeError("source identity index"));
        }
        let identity = decoder.string()?;
        if identity.is_empty() {
            return Err(DecodeError("empty source identity"));
        }
        if kind == SourceIdentityKind::PythonDirectImport && identity != "direct" {
            return Err(DecodeError("Python direct import"));
        }
        if kind == SourceIdentityKind::PythonImportNoModuleWrite && identity != "unwritten" {
            return Err(DecodeError("Python module import write"));
        }
        if kind == SourceIdentityKind::PythonDirectDefinition
            && (identity != "direct"
                || facts
                    .symbols
                    .get(index as usize)
                    .is_none_or(|symbol| symbol.kind() != SymKind::Fn || symbol.parent.is_some()))
        {
            return Err(DecodeError("Python direct definition"));
        }
        if kind == SourceIdentityKind::PythonCallableImportFallback
            && (facts.refs.get(index as usize).is_none_or(|reference| {
                reference.kind != RefKind::Call || reference.qualifier.is_some()
            }) || identity
                .split_once(':')
                .and_then(|(first, second)| {
                    Some((first.parse::<usize>().ok()?, second.parse::<usize>().ok()?))
                })
                .is_none_or(|(first, second)| {
                    first == second || first >= facts.imports.len() || second >= facts.imports.len()
                }))
        {
            return Err(DecodeError("Python callable import fallback"));
        }
        if kind == SourceIdentityKind::PythonShadowedImport && identity != "shadowed" {
            return Err(DecodeError("Python shadowed import"));
        }
        if matches!(
            kind,
            SourceIdentityKind::PythonImportScope
                | SourceIdentityKind::PythonImportAfterTry
                | SourceIdentityKind::PythonImportOptionalTry
        ) && crate::domain::facts::python_import_scope(&identity).is_none_or(|(owner, _)| {
            owner.is_some_and(|index| index as usize >= facts.symbols.len())
        }) {
            return Err(DecodeError("Python import scope"));
        }
        if kind == SourceIdentityKind::TypeScriptScope
            && !matches!(
                identity.as_str(),
                "clear" | "augmentation" | "augmentations" | "incomplete"
            )
        {
            return Err(DecodeError("TypeScript scope"));
        }
        if kind == SourceIdentityKind::TypeScriptAugmentation
            && (!(identity.starts_with("./") || identity.starts_with("../"))
                || identity.contains('\\')
                || identity.chars().any(char::is_control)
                || facts.symbols[index as usize].parent.is_some())
        {
            return Err(DecodeError("TypeScript augmentation"));
        }
        if kind == SourceIdentityKind::TypeScriptComplexExports && identity != "complex" {
            return Err(DecodeError("TypeScript exports"));
        }
        if kind == SourceIdentityKind::TypeScriptModuleSyntax
            && !matches!(
                identity.as_str(),
                "none" | "indicator" | "jsx" | "incomplete" | "declaration"
            )
        {
            return Err(DecodeError("TypeScript module syntax"));
        }
        if kind == SourceIdentityKind::TypeScriptProject
            && identity != "unproven"
            && crate::domain::facts::typescript_project_config(&identity).is_none()
        {
            return Err(DecodeError("TypeScript project"));
        }
        if kind == SourceIdentityKind::TypeScriptPackageType
            && identity != "unproven"
            && crate::domain::facts::typescript_package_type(&identity).is_none()
        {
            return Err(DecodeError("TypeScript package type"));
        }
        if kind == SourceIdentityKind::TypeScriptGlobalAugmentationScope
            && !matches!(identity.as_str(), "complete" | "unproven")
        {
            return Err(DecodeError("TypeScript global augmentation scope"));
        }
        if kind == SourceIdentityKind::TypeScriptGlobalAugmentation && identity != "global" {
            return Err(DecodeError("TypeScript global augmentation"));
        }
        if kind == SourceIdentityKind::InterfaceGroup {
            let canonical = identity
                .parse::<u32>()
                .map_err(|_| DecodeError("interface group"))?;
            let owner = &facts.symbols[index as usize];
            let target = facts
                .symbols
                .get(canonical as usize)
                .ok_or(DecodeError("interface group"))?;
            if canonical > index
                || target.kind != Some(SymKind::Iface)
                || owner.name != target.name
                || owner.exported != target.exported
                || !same_typescript_namespace(&facts.symbols, owner, target)
            {
                return Err(DecodeError("interface group"));
            }
        }
        if kind == SourceIdentityKind::InterfaceGenericFields {
            let fields = crate::domain::facts::interface_generic_fields(&identity)
                .ok_or(DecodeError("interface generic fields"))?;
            if !identities.iter().any(|entry| {
                entry.kind == SourceIdentityKind::InterfaceGroup
                    && entry.index == index
                    && entry.identity == index.to_string()
            }) || fields.iter().any(|&(base, name, _)| {
                base == index
                    || facts.symbols.get(base as usize).is_none_or(|symbol| {
                        symbol.kind != Some(SymKind::Iface)
                            || symbol.parent.is_some()
                            || symbol.exported
                    })
                    || !facts.fields.iter().any(|fields| {
                        fields.owner == base
                            && fields.names.iter().any(|field| field.text() == Some(name))
                    })
            }) {
                return Err(DecodeError("interface generic fields"));
            }
        }
        identities.push(SourceIdentity {
            kind,
            index,
            identity,
        });
        previous = Some(key);
    }
    let complete_augmentations = identities.iter().any(|entry| {
        entry.kind == SourceIdentityKind::TypeScriptScope && entry.identity == "augmentations"
    });
    let augmentations: Vec<_> = identities
        .iter()
        .filter(|entry| entry.kind == SourceIdentityKind::TypeScriptAugmentation)
        .collect();
    if complete_augmentations == augmentations.is_empty()
        || augmentations.iter().any(|augmentation| {
            facts.symbols[augmentation.index as usize].flags
                & crate::domain::facts::FLAG_TYPESCRIPT_AUGMENTATION
                == 0
                || ![
                    SourceIdentityKind::InterfaceHeader,
                    SourceIdentityKind::InterfaceMembers,
                ]
                .iter()
                .all(|kind| {
                    identities
                        .iter()
                        .any(|entry| entry.kind == *kind && entry.index == augmentation.index)
                })
        })
        || facts.symbols.iter().enumerate().any(|(index, symbol)| {
            symbol.flags & crate::domain::facts::FLAG_TYPESCRIPT_AUGMENTATION != 0
                && !augmentations
                    .iter()
                    .any(|entry| entry.index as usize == index)
        })
    {
        return Err(DecodeError("TypeScript augmentation evidence"));
    }
    let global_scope = identities
        .iter()
        .find(|entry| entry.kind == SourceIdentityKind::TypeScriptGlobalAugmentationScope);
    let global_parts: Vec<_> = identities
        .iter()
        .filter(|entry| entry.kind == SourceIdentityKind::TypeScriptGlobalAugmentation)
        .collect();
    let module_syntax = identities
        .iter()
        .find(|entry| entry.kind == SourceIdentityKind::TypeScriptModuleSyntax);
    let invalid_global_scope = match global_scope.map(|scope| scope.identity.as_str()) {
        Some("complete") => module_syntax
            .is_none_or(|syntax| matches!(syntax.identity.as_str(), "incomplete" | "declaration")),
        Some("unproven") => !global_parts.is_empty(),
        None => !global_parts.is_empty(),
        Some(_) => true,
    };
    if invalid_global_scope
        || global_parts.iter().any(|part| {
            ![
                SourceIdentityKind::InterfaceHeader,
                SourceIdentityKind::InterfaceMembers,
            ]
            .iter()
            .all(|kind| {
                identities
                    .iter()
                    .any(|entry| entry.kind == *kind && entry.index == part.index)
            })
        })
    {
        return Err(DecodeError("TypeScript global augmentation evidence"));
    }
    if !identities.is_empty() {
        facts.source_identities = Some(identities.into_boxed_slice());
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::facts::{FLAG_RUST_CFG_NOT_UNIX, FLAG_RUST_CFG_UNIX, FLAG_TEST};

    pub fn sample() -> Facts {
        Facts {
            package: Some("store".to_owned()),
            lines: 40,
            doc: "Package store keeps rows.".to_owned(),
            terms: "main serve".to_owned(),
            entry: true,
            coverage: Coverage::ParseErrors,
            symbols: vec![
                Symbol {
                    kind: Some(SymKind::Struct),
                    name: "Store".into(),
                    parent: None,
                    owner: String::new().into(),
                    line: 3,
                    end_line: 9,
                    signature: "type Store struct".into(),
                    doc: "Store keeps rows.".into(),
                    terms: "rows mutex insert".into(),
                    exported: true,
                    flags: 0,
                },
                Symbol {
                    kind: Some(SymKind::Method),
                    name: "Get".into(),
                    parent: Some(0),
                    owner: "Store".into(),
                    line: 11,
                    end_line: 15,
                    signature: "func (s *Store) Get(id int) Row".into(),
                    doc: String::new().into(),
                    terms: String::new().into(),
                    exported: true,
                    flags: FLAG_TEST,
                },
            ],
            imports: vec![Import {
                spec: "example.com/db".into(),
                alias: Some("database".into()),
                names: vec!["Open".into()],
                line: 2,
                reexport: true,
                type_only: true,
            }],
            refs: vec![
                Ref {
                    kind: RefKind::Call,
                    name: "Query".into(),
                    qualifier: Some("s.db".into()),
                    scope: Some(1),
                    line: 12,
                    byte: 140,
                    arity: 2,
                },
                Ref {
                    kind: RefKind::Call,
                    name: "length".into(),
                    qualifier: None,
                    scope: Some(1),
                    line: 13,
                    byte: crate::domain::facts::NO_BYTE,
                    arity: 1,
                },
            ],
            go_embeds: Vec::new(),
            julia_builtin_calls: vec![1],
            julia_call_arguments: vec![JuliaCallArgument {
                reference: 1,
                position: 0,
                binding: "rows".to_owned(),
            }],
            source_identities: Some(
                vec![
                    SourceIdentity {
                        kind: SourceIdentityKind::Symbol,
                        index: 1,
                        identity: "Store.Get(id:)".to_owned(),
                    },
                    SourceIdentity {
                        kind: SourceIdentityKind::Reference,
                        index: 0,
                        identity: "Query(sql:)".to_owned(),
                    },
                ]
                .into_boxed_slice(),
            ),
            bindings: vec![Binding {
                scope: Some(1),
                name: "s".into(),
                type_name: "Store".into(),
                line: 11,
                visibility: None,
                value_byte: crate::domain::facts::NO_BYTE,
                declared: true,
            }],
            envs: vec![EnvRead {
                name: "DB_URL".to_owned(),
                scope: None,
                line: 20,
            }],
            fields: vec![Fields {
                owner: 0,
                names: vec![FieldName::Text("db".to_owned())],
                types: vec!["DB".to_owned()],
            }],
            env_defs: vec![EnvDef {
                name: "PORT".to_owned(),
                source: EnvSource::Compose,
                line: 4,
            }],
            config_keys: vec![ConfigKey {
                path: "services.api.image".to_owned(),
                kind: ValueKind::String,
                line: 3,
            }],
            texts: vec![TextSegment {
                heading: "Intro".to_owned(),
                locator: 1,
                text: "hello".to_owned(),
            }],
            asset: Some(AssetMeta {
                format: "png".to_owned(),
                width: 10,
                height: 20,
                duration_ms: 0,
            }),
            manifest: Some(Manifest {
                module: Some("example.com/app".to_owned()),
                deps: vec![("serde".to_owned(), "1".to_owned())],
                members: vec!["crates/*".to_owned()],
                source_roots: vec!["src".to_owned()],
                include_dirs: vec!["include".to_owned()],
                entrypoints: vec!["src/main.rs".to_owned()],
                rust_target_roots: vec![RustTargetRoot {
                    path: "src/lib.rs".to_owned(),
                    is_library: true,
                }],
                project_links: vec!["extends=../base.json".to_owned()],
            }),
        }
    }

    #[test]
    fn facts_round_trip() -> DecodeResult<()> {
        let facts = sample();
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        encode(&mut encoder, &Facts::default());
        let mut decoder = Decoder::new(&encoder.bytes);
        assert_eq!(decode(&mut decoder)?, facts);
        assert_eq!(decode(&mut decoder)?, Facts::default());
        assert!(decoder.is_done());
        Ok(())
    }

    #[test]
    fn python_direct_import_identity_round_trips_and_is_validated() -> DecodeResult<()> {
        let mut facts = Facts {
            imports: vec![Import {
                spec: "pkg".into(),
                ..Import::default()
            }],
            source_identities: Some(
                vec![SourceIdentity {
                    kind: SourceIdentityKind::PythonDirectImport,
                    index: 0,
                    identity: "direct".to_owned(),
                }]
                .into_boxed_slice(),
            ),
            ..Facts::default()
        };
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identity"))?[0]
            .identity = "unknown".to_owned();
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        facts.source_identities = Some(
            vec![SourceIdentity {
                kind: SourceIdentityKind::PythonImportNoModuleWrite,
                index: 0,
                identity: "unwritten".to_owned(),
            }]
            .into_boxed_slice(),
        );
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identity"))?[0]
            .identity = "unknown".to_owned();
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn python_direct_definition_round_trips_and_rejects_nested_symbols() -> DecodeResult<()> {
        let mut facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Fn),
                ..Symbol::default()
            }],
            source_identities: Some(
                vec![SourceIdentity {
                    kind: SourceIdentityKind::PythonDirectDefinition,
                    index: 0,
                    identity: "direct".to_owned(),
                }]
                .into_boxed_slice(),
            ),
            ..Facts::default()
        };
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        facts.symbols[0].parent = Some(0);
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn python_callable_import_fallback_round_trips_and_rejects_invalid_indices() -> DecodeResult<()>
    {
        let mut facts = sample();
        facts.imports.push(Import::default());
        facts.refs[0].qualifier = None;
        facts.source_identities = Some(
            vec![SourceIdentity {
                kind: SourceIdentityKind::PythonCallableImportFallback,
                index: 0,
                identity: "0:1".to_owned(),
            }]
            .into_boxed_slice(),
        );
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        for invalid in ["0:0", "0:2", "2:0", "0:1:2", "bad"] {
            facts
                .source_identities
                .as_mut()
                .ok_or(DecodeError("identity"))?[0]
                .identity = invalid.to_owned();
            let mut encoder = Encoder::default();
            encode(&mut encoder, &facts);
            assert!(
                decode(&mut Decoder::new(&encoder.bytes)).is_err(),
                "{invalid}"
            );
        }
        Ok(())
    }

    #[test]
    fn python_import_scope_identity_round_trips_and_rejects_invalid_bounds() -> DecodeResult<()> {
        let mut facts = Facts {
            imports: vec![Import {
                spec: "pkg".into(),
                ..Import::default()
            }],
            symbols: vec![Symbol {
                kind: Some(SymKind::Fn),
                ..Symbol::default()
            }],
            source_identities: Some(
                vec![SourceIdentity {
                    kind: SourceIdentityKind::PythonImportScope,
                    index: 0,
                    identity: "1:10:20".to_owned(),
                }]
                .into_boxed_slice(),
            ),
            ..Facts::default()
        };
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        for invalid in ["2:10:20", "1:20:10", "1:10:10", "1:10:20:0"] {
            facts
                .source_identities
                .as_mut()
                .ok_or(DecodeError("identity"))?[0]
                .identity = invalid.to_owned();
            let mut encoder = Encoder::default();
            encode(&mut encoder, &facts);
            assert!(
                decode(&mut Decoder::new(&encoder.bytes)).is_err(),
                "{invalid}"
            );
        }
        Ok(())
    }

    #[test]
    fn coverage_round_trips_and_unknown_values_are_rejected() -> DecodeResult<()> {
        for coverage in [
            Coverage::Complete,
            Coverage::ParseErrors,
            Coverage::MatchLimit,
            Coverage::NoGrammar,
            Coverage::Failed,
            Coverage::Unreadable,
        ] {
            let facts = Facts {
                coverage,
                ..Facts::default()
            };
            let mut encoder = Encoder::default();
            encode(&mut encoder, &facts);
            assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        }
        let facts = Facts::default();
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        let complete = encoder.bytes.clone();
        let unknown = Facts {
            coverage: Coverage::Unreadable,
            ..Facts::default()
        };
        let mut encoder = Encoder::default();
        encode(&mut encoder, &unknown);
        let offset = encoder
            .bytes
            .iter()
            .zip(&complete)
            .position(|(a, b)| a != b)
            .ok_or(DecodeError("coverage byte"))?;
        let mut corrupt = encoder.bytes.clone();
        corrupt[offset] = 200;
        assert!(decode(&mut Decoder::new(&corrupt)).is_err());
        Ok(())
    }

    #[test]
    fn import_roles_round_trip_and_reserved_flags_are_rejected() -> DecodeResult<()> {
        let mut facts = Facts {
            imports: vec![Import {
                spec: "./api.js".into(),
                names: vec!["Api".into()],
                ..Import::default()
            }],
            ..Facts::default()
        };
        for (reexport, type_only) in [(false, false), (true, false), (false, true), (true, true)] {
            facts.imports[0].reexport = reexport;
            facts.imports[0].type_only = type_only;
            let mut encoder = Encoder::default();
            encode(&mut encoder, &facts);
            assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        }
        facts.imports[0].type_only = false;
        let mut value = Encoder::default();
        encode(&mut value, &facts);
        facts.imports[0].type_only = true;
        let mut typed = Encoder::default();
        encode(&mut typed, &facts);
        assert_eq!(value.bytes.len(), typed.bytes.len());
        let mut differences = value
            .bytes
            .iter()
            .zip(&typed.bytes)
            .enumerate()
            .filter(|(_, (left, right))| left != right);
        let offset = differences.next().ok_or(DecodeError("import role byte"))?.0;
        assert!(differences.next().is_none());
        for reserved in [4, 128, 255] {
            let mut corrupt = typed.bytes.clone();
            corrupt[offset] = reserved;
            assert!(decode(&mut Decoder::new(&corrupt)).is_err());
        }
        Ok(())
    }

    #[test]
    fn rust_platform_cfg_flags_round_trip() -> DecodeResult<()> {
        let facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Fn),
                name: "platform_call".into(),
                exported: true,
                flags: FLAG_TEST | FLAG_RUST_CFG_UNIX | FLAG_RUST_CFG_NOT_UNIX,
                ..Symbol::default()
            }],
            ..Facts::default()
        };
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        Ok(())
    }

    #[test]
    fn untyped_local_binding_round_trips_for_shadowing() -> DecodeResult<()> {
        let mut facts = sample();
        facts.bindings.push(Binding {
            scope: Some(1),
            name: "parse".into(),
            type_name: String::new().into(),
            line: 14,
            visibility: Some(150..220),
            value_byte: 140,
            declared: false,
        });
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        Ok(())
    }

    #[test]
    fn interface_header_identity_round_trips_only_for_interface_symbols() -> DecodeResult<()> {
        let mut facts = sample();
        facts.symbols[0].kind = Some(SymKind::Iface);
        let mut identities = facts
            .source_identities
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        identities.push(SourceIdentity {
            kind: SourceIdentityKind::InterfaceHeader,
            index: 0,
            identity: "interface_declaration:6:Config".to_owned(),
        });
        identities.push(SourceIdentity {
            kind: SourceIdentityKind::InterfaceMembers,
            index: 0,
            identity: "interface_members:p1:x1:y".to_owned(),
        });
        identities.push(SourceIdentity {
            kind: SourceIdentityKind::InterfaceGroup,
            index: 0,
            identity: "0".to_owned(),
        });
        facts.source_identities = Some(identities.into_boxed_slice());
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        facts.symbols[0].kind = Some(SymKind::Struct);
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn typescript_scope_evidence_validates_file_level_identity() -> DecodeResult<()> {
        let mut facts = Facts {
            source_identities: Some(
                vec![SourceIdentity {
                    kind: SourceIdentityKind::TypeScriptScope,
                    index: 0,
                    identity: "augmentation".to_owned(),
                }]
                .into_boxed_slice(),
            ),
            ..Facts::default()
        };
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        let marker = facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identities"))?
            .first_mut()
            .ok_or(DecodeError("identity"))?;
        marker.identity = "unknown".to_owned();
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        let marker = facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identities"))?
            .first_mut()
            .ok_or(DecodeError("identity"))?;
        marker.kind = SourceIdentityKind::TypeScriptComplexExports;
        marker.identity = "complex".to_owned();
        let mut valid = Encoder::default();
        encode(&mut valid, &facts);
        assert_eq!(decode(&mut Decoder::new(&valid.bytes))?, facts);
        let marker = facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identities"))?
            .first_mut()
            .ok_or(DecodeError("identity"))?;
        marker.identity = "unknown".to_owned();
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn typescript_project_and_global_scope_evidence_round_trip_and_reject_corruption()
    -> DecodeResult<()> {
        let project = crate::domain::facts::TypeScriptProjectConfig {
            files: Some(vec!["a.ts".to_owned()]),
            include: Some(vec!["src/**/*.ts".to_owned()]),
            exclude: Some(vec!["build".to_owned()]),
            out_dir: Some("build".to_owned()),
            module_detection: Some("legacy".to_owned()),
            module: Some("commonjs".to_owned()),
            jsx: None,
        };
        let mut facts = Facts {
            symbols: vec![Symbol {
                kind: Some(SymKind::Iface),
                name: "Window".into(),
                ..Symbol::default()
            }],
            source_identities: Some(
                vec![
                    SourceIdentity {
                        kind: SourceIdentityKind::InterfaceHeader,
                        index: 0,
                        identity: "header".to_owned(),
                    },
                    SourceIdentity {
                        kind: SourceIdentityKind::InterfaceMembers,
                        index: 0,
                        identity: "members".to_owned(),
                    },
                    SourceIdentity {
                        kind: SourceIdentityKind::TypeScriptModuleSyntax,
                        index: 0,
                        identity: "indicator".to_owned(),
                    },
                    SourceIdentity {
                        kind: SourceIdentityKind::TypeScriptProject,
                        index: 0,
                        identity: crate::domain::facts::typescript_project_identity(&project),
                    },
                    SourceIdentity {
                        kind: SourceIdentityKind::TypeScriptGlobalAugmentationScope,
                        index: 0,
                        identity: "complete".to_owned(),
                    },
                    SourceIdentity {
                        kind: SourceIdentityKind::TypeScriptGlobalAugmentation,
                        index: 0,
                        identity: "global".to_owned(),
                    },
                    SourceIdentity {
                        kind: SourceIdentityKind::TypeScriptPackageType,
                        index: 0,
                        identity: "typescript_package:v1:module".to_owned(),
                    },
                ]
                .into_boxed_slice(),
            ),
            ..Facts::default()
        };
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);

        {
            let identities = facts
                .source_identities
                .as_mut()
                .ok_or(DecodeError("identities"))?;
            identities
                .get_mut(5)
                .ok_or(DecodeError("global identity"))?
                .identity = "external".to_owned();
        }
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());

        {
            let identities = facts
                .source_identities
                .as_mut()
                .ok_or(DecodeError("identities"))?;
            identities
                .get_mut(5)
                .ok_or(DecodeError("global identity"))?
                .identity = "global".to_owned();
            identities
                .get_mut(3)
                .ok_or(DecodeError("project identity"))?
                .identity = "typescript_project:v1:invalid".to_owned();
        }
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        {
            let identities = facts
                .source_identities
                .as_mut()
                .ok_or(DecodeError("identities"))?;
            identities
                .get_mut(3)
                .ok_or(DecodeError("project identity"))?
                .identity = crate::domain::facts::typescript_project_identity(&project);
            identities
                .get_mut(6)
                .ok_or(DecodeError("package identity"))?
                .identity = "typescript_package:v1:invalid".to_owned();
        }
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn typescript_augmentation_requires_complete_owned_evidence() -> DecodeResult<()> {
        let mut facts = sample();
        facts.symbols[0].kind = Some(SymKind::Iface);
        facts.symbols[0].flags |= crate::domain::facts::FLAG_TYPESCRIPT_AUGMENTATION;
        facts.source_identities = Some(
            vec![
                SourceIdentity {
                    kind: SourceIdentityKind::InterfaceHeader,
                    index: 0,
                    identity: "header".to_owned(),
                },
                SourceIdentity {
                    kind: SourceIdentityKind::InterfaceMembers,
                    index: 0,
                    identity: "interface_members:".to_owned(),
                },
                SourceIdentity {
                    kind: SourceIdentityKind::TypeScriptScope,
                    index: 0,
                    identity: "augmentations".to_owned(),
                },
                SourceIdentity {
                    kind: SourceIdentityKind::TypeScriptAugmentation,
                    index: 0,
                    identity: "./base".to_owned(),
                },
            ]
            .into_boxed_slice(),
        );
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identities"))?[3]
            .identity = "external".to_owned();
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        let identities = facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identities"))?;
        identities[3].identity = "./base".to_owned();
        identities[2].identity = "clear".to_owned();
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn generic_field_evidence_requires_a_group_and_real_base_fields() -> DecodeResult<()> {
        let mut facts = sample();
        facts.symbols[0].kind = Some(SymKind::Iface);
        let mut base = facts.symbols[0].clone();
        base.name = "Box".into();
        base.exported = false;
        facts.symbols.push(base);
        facts.fields.push(Fields {
            owner: 2,
            names: vec![FieldName::Text("item".to_owned())],
            types: vec!["T".to_owned()],
        });
        let mut identities = facts
            .source_identities
            .as_deref()
            .unwrap_or_default()
            .to_vec();
        identities.push(SourceIdentity {
            kind: SourceIdentityKind::InterfaceGroup,
            index: 0,
            identity: "0".to_owned(),
        });
        identities.push(SourceIdentity {
            kind: SourceIdentityKind::InterfaceGenericFields,
            index: 0,
            identity: crate::domain::facts::interface_generic_fields_identity(&[(
                2, "item", "Service",
            )]),
        });
        facts.source_identities = Some(identities.into_boxed_slice());
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        let identities = facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identities"))?;
        identities
            .last_mut()
            .ok_or(DecodeError("identity"))?
            .identity =
            crate::domain::facts::interface_generic_fields_identity(&[(2, "missing", "Service")]);
        let mut invalid = Encoder::default();
        encode(&mut invalid, &facts);
        assert!(decode(&mut Decoder::new(&invalid.bytes)).is_err());
        let identities = facts
            .source_identities
            .as_mut()
            .ok_or(DecodeError("identities"))?;
        identities
            .last_mut()
            .ok_or(DecodeError("identity"))?
            .identity = crate::domain::facts::interface_generic_fields_identity(&[
            (2, "item", "Service"),
            (2, "item", "T"),
        ]);
        let mut duplicate = Encoder::default();
        encode(&mut duplicate, &facts);
        assert!(decode(&mut Decoder::new(&duplicate.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn field_keys_round_trip_without_surrogate_replacement() -> DecodeResult<()> {
        let mut facts = sample();
        facts.fields[0].names.extend([
            FieldName::Text("a,b".to_owned()),
            FieldName::Text("\\ud800".to_owned()),
            FieldName::Utf16(vec![0xd800]),
        ]);
        facts.fields[0]
            .types
            .extend([String::new(), String::new(), String::new()]);
        for table in [false, true] {
            let mut encoder = if table {
                Encoder::with_table()
            } else {
                Encoder::default()
            };
            encode(&mut encoder, &facts);
            let table_bytes = encoder.table_bytes();
            let mut decoder = if table {
                Decoder::with_table(&encoder.bytes, &table_bytes)?
            } else {
                Decoder::new(&encoder.bytes)
            };
            assert_eq!(decode(&mut decoder)?, facts);
            assert!(decoder.is_done());
        }
        Ok(())
    }

    #[test]
    fn go_embed_bytes_round_trip_and_invalid_owners_are_rejected() -> DecodeResult<()> {
        let mut facts = sample();
        let mut variable = facts.symbols[0].clone();
        variable.kind = Some(SymKind::Var);
        facts.symbols.push(variable);
        facts.go_embeds.push(GoEmbed {
            symbol: 2,
            line: 5,
            pattern: vec![b'a', 0xff],
        });
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert_eq!(decode(&mut Decoder::new(&encoder.bytes))?, facts);
        facts.go_embeds[0].symbol = 0;
        let mut encoder = Encoder::default();
        encode(&mut encoder, &facts);
        assert!(decode(&mut Decoder::new(&encoder.bytes)).is_err());
        Ok(())
    }

    #[test]
    fn truncated_facts_fail_to_decode() {
        let mut encoder = Encoder::default();
        encode(&mut encoder, &sample());
        for cut in [1, encoder.bytes.len() / 2, encoder.bytes.len() - 1] {
            assert!(
                decode(&mut Decoder::new(&encoder.bytes[..cut])).is_err(),
                "cut {cut}"
            );
        }
    }
}
