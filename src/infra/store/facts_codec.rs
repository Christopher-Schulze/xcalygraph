//! Encoding of per-file facts inside fact shards.

use super::codec::{DecodeError, DecodeResult, Decoder, Encoder};
use crate::domain::facts::{
    AssetMeta, Binding, ConfigKey, EnvDef, EnvRead, EnvSource, Facts, Fields, Import, Manifest,
    Ref, RefKind, SymKind, Symbol, TextSegment,
};

fn opt_index(encoder: &mut Encoder, value: Option<u32>) {
    encoder.var(value.map_or(0, |index| u64::from(index) + 1));
}

fn read_opt_index(decoder: &mut Decoder) -> DecodeResult<Option<u32>> {
    let raw = decoder.var_u32()?;
    Ok(raw.checked_sub(1))
}

pub fn encode(encoder: &mut Encoder, facts: &Facts) {
    encoder.opt_str(facts.package.as_deref());
    encoder.var_u32(facts.lines);
    encoder.str(&facts.doc);
    encoder.u8(u8::from(facts.entry));
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
        encoder.u8(u8::from(symbol.exported) | symbol.flags << 1);
    }
    encoder.var_usize(facts.imports.len());
    for import in &facts.imports {
        encoder.str(&import.spec);
        encoder.opt_str(import.alias.as_deref());
        encoder.strs(&import.names);
        encoder.var_u32(import.line);
        encoder.u8(u8::from(import.reexport));
    }
    encoder.var_usize(facts.refs.len());
    for reference in &facts.refs {
        encoder.u8(reference.kind as u8);
        encoder.str(&reference.name);
        encoder.opt_str(reference.qualifier.as_deref());
        opt_index(encoder, reference.scope);
        encoder.var_u32(reference.line);
    }
    encoder.var_usize(facts.bindings.len());
    for binding in &facts.bindings {
        opt_index(encoder, binding.scope);
        encoder.str(&binding.name);
        encoder.str(&binding.type_name);
        encoder.var_u32(binding.line);
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
        encoder.strs(&fields.names);
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
        }
        None => encoder.u8(0),
    }
}

pub fn decode(decoder: &mut Decoder) -> DecodeResult<Facts> {
    let mut facts = Facts {
        package: decoder.opt_string()?,
        lines: decoder.var_u32()?,
        doc: decoder.string()?,
        entry: decoder.u8()? != 0,
        ..Facts::default()
    };
    for _ in 0..decoder.len()? {
        let kind = SymKind::from_u8(decoder.u8()?).ok_or(DecodeError("symbol kind"))?;
        let name = decoder.string()?;
        let parent = read_opt_index(decoder)?;
        let owner = decoder.string()?;
        let line = decoder.var_u32()?;
        let end_line = line.saturating_add(decoder.var_u32()?);
        let signature = decoder.string()?;
        let doc = decoder.string()?;
        let bits = decoder.u8()?;
        facts.symbols.push(Symbol {
            kind: Some(kind),
            name,
            parent,
            owner,
            line,
            end_line,
            signature,
            doc,
            exported: bits & 1 != 0,
            flags: bits >> 1,
        });
    }
    for _ in 0..decoder.len()? {
        facts.imports.push(Import {
            spec: decoder.string()?,
            alias: decoder.opt_string()?,
            names: decoder.strings()?,
            line: decoder.var_u32()?,
            reexport: decoder.u8()? != 0,
        });
    }
    for _ in 0..decoder.len()? {
        facts.refs.push(Ref {
            kind: RefKind::from_u8(decoder.u8()?).ok_or(DecodeError("ref kind"))?,
            name: decoder.string()?,
            qualifier: decoder.opt_string()?,
            scope: read_opt_index(decoder)?,
            line: decoder.var_u32()?,
        });
    }
    for _ in 0..decoder.len()? {
        facts.bindings.push(Binding {
            scope: read_opt_index(decoder)?,
            name: decoder.string()?,
            type_name: decoder.string()?,
            line: decoder.var_u32()?,
            declared: decoder.u8()? != 0,
        });
    }
    for _ in 0..decoder.len()? {
        facts.envs.push(EnvRead {
            name: decoder.string()?,
            scope: read_opt_index(decoder)?,
            line: decoder.var_u32()?,
        });
    }
    for _ in 0..decoder.len()? {
        facts.fields.push(Fields {
            owner: decoder.var_u32()?,
            names: decoder.strings()?,
            types: decoder.strings()?,
        });
    }
    for _ in 0..decoder.len()? {
        facts.env_defs.push(EnvDef {
            name: decoder.string()?,
            source: EnvSource::from_u8(decoder.u8()?).ok_or(DecodeError("env source"))?,
            line: decoder.var_u32()?,
        });
    }
    for _ in 0..decoder.len()? {
        facts.config_keys.push(ConfigKey {
            path: decoder.string()?,
            line: decoder.var_u32()?,
        });
    }
    for _ in 0..decoder.len()? {
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
        });
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::facts::FLAG_TEST;

    pub fn sample() -> Facts {
        Facts {
            package: Some("store".to_owned()),
            lines: 40,
            doc: "Package store keeps rows.".to_owned(),
            entry: true,
            symbols: vec![
                Symbol {
                    kind: Some(SymKind::Struct),
                    name: "Store".to_owned(),
                    parent: None,
                    owner: String::new(),
                    line: 3,
                    end_line: 9,
                    signature: "type Store struct".to_owned(),
                    doc: "Store keeps rows.".to_owned(),
                    exported: true,
                    flags: 0,
                },
                Symbol {
                    kind: Some(SymKind::Method),
                    name: "Get".to_owned(),
                    parent: Some(0),
                    owner: "Store".to_owned(),
                    line: 11,
                    end_line: 15,
                    signature: "func (s *Store) Get(id int) Row".to_owned(),
                    doc: String::new(),
                    exported: true,
                    flags: FLAG_TEST,
                },
            ],
            imports: vec![Import {
                spec: "example.com/db".to_owned(),
                alias: Some("database".to_owned()),
                names: vec!["Open".to_owned()],
                line: 2,
                reexport: true,
            }],
            refs: vec![Ref {
                kind: RefKind::Call,
                name: "Query".to_owned(),
                qualifier: Some("s.db".to_owned()),
                scope: Some(1),
                line: 12,
            }],
            bindings: vec![Binding {
                scope: Some(1),
                name: "s".to_owned(),
                type_name: "Store".to_owned(),
                line: 11,
                declared: true,
            }],
            envs: vec![EnvRead {
                name: "DB_URL".to_owned(),
                scope: None,
                line: 20,
            }],
            fields: vec![Fields {
                owner: 0,
                names: vec!["db".to_owned()],
                types: vec!["DB".to_owned()],
            }],
            env_defs: vec![EnvDef {
                name: "PORT".to_owned(),
                source: EnvSource::Compose,
                line: 4,
            }],
            config_keys: vec![ConfigKey {
                path: "services.api.image".to_owned(),
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
