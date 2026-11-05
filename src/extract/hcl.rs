//! Terraform and HCL facts (`docs/spec.md` sections 5 and 6): blocks as symbols named by their
//! Terraform address (`var.region`, `local.tags`, `module.network`, `output.id`,
//! `aws_s3_bucket.logs`, `data.aws_ami.base`), references to such addresses in expressions as
//! value references, and local `module` sources as imports. A line scanner: HCL blocks are
//! regular, and no grammar compilation is needed.

use crate::domain::facts::{self, Facts, Import, NO_ARITY, Ref, RefKind, SymKind, Symbol};

/// Address prefixes that name a block in the same module.
const ADDRESS_PREFIXES: [&str; 4] = ["var.", "local.", "module.", "data."];

pub fn extract(text: &str, facts: &mut Facts) {
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = index as u32 + 1;
        let trimmed = lines[index].trim();
        index += 1;
        // One-line blocks (`variable "name" {}`) have no body.
        let one_line = trimmed.ends_with("{}");
        let header = trimmed.trim_end_matches("{}").trim_end_matches('{');
        let words: Vec<&str> = header.split_whitespace().collect();
        let Some(&block) = words.first() else {
            continue;
        };
        if !(trimmed.ends_with('{') || one_line) || lines[index - 1].starts_with([' ', '\t']) {
            continue;
        }
        let labels: Vec<&str> = words[1..]
            .iter()
            .map(|word| word.trim_matches('"'))
            .collect();
        let end = if one_line {
            index - 1
        } else {
            block_end(&lines, index - 1)
        };
        let body = &lines[index.min(end)..end.min(lines.len())];
        match (block, labels.as_slice()) {
            ("resource", [kind, name]) => push_block(
                facts,
                SymKind::Struct,
                &format!("{kind}.{name}"),
                trimmed,
                line,
                end,
            ),
            ("data", [kind, name]) => push_block(
                facts,
                SymKind::Struct,
                &format!("data.{kind}.{name}"),
                trimmed,
                line,
                end,
            ),
            ("variable", [name]) => push_block(
                facts,
                SymKind::Var,
                &format!("var.{name}"),
                trimmed,
                line,
                end,
            ),
            ("output", [name]) => push_block(
                facts,
                SymKind::Const,
                &format!("output.{name}"),
                trimmed,
                line,
                end,
            ),
            ("module", [name]) => {
                push_block(
                    facts,
                    SymKind::Mod,
                    &format!("module.{name}"),
                    trimmed,
                    line,
                    end,
                );
                // `source = "./modules/network"`: a local module directory.
                if let Some(source) = body.iter().find_map(|body_line| {
                    let (key, value) = body_line.split_once('=')?;
                    (key.trim() == "source").then(|| value.trim().trim_matches('"').to_owned())
                }) && (source.starts_with("./") || source.starts_with("../"))
                {
                    facts.imports.push(Import {
                        spec: source,
                        line,
                        ..Import::default()
                    });
                }
            }
            ("locals", []) => {
                for (offset, body_line) in body.iter().enumerate() {
                    let Some((key, _)) = body_line.split_once('=') else {
                        continue;
                    };
                    let key = key.trim();
                    let top_level = body_line.len() - body_line.trim_start().len() <= 4;
                    if top_level
                        && !key.is_empty()
                        && key
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                    {
                        let at = line + 1 + offset as u32;
                        push_block(
                            facts,
                            SymKind::Var,
                            &format!("local.{key}"),
                            body_line.trim(),
                            at,
                            at as usize - 1,
                        );
                    }
                }
            }
            _ => {}
        }
        let scope = facts
            .symbols
            .iter()
            .rposition(|symbol| symbol.line == line)
            .map(|position| position as u32);
        for (offset, body_line) in body.iter().enumerate() {
            let at = line + 1 + offset as u32;
            let scope = if block == "locals" {
                facts
                    .symbols
                    .iter()
                    .rposition(|symbol| symbol.line == at)
                    .map(|position| position as u32)
                    .or(scope)
            } else {
                scope
            };
            for address in addresses(body_line) {
                push_ref(facts, scope, &address, at);
            }
        }
        index = end.max(index);
    }
}

fn push_block(facts: &mut Facts, kind: SymKind, name: &str, header: &str, line: u32, end: usize) {
    facts.symbols.push(Symbol {
        kind: Some(kind),
        name: name.to_owned(),
        line,
        end_line: (end as u32 + 1).max(line),
        signature: facts::clip_space(header.trim_end_matches('{'), 160),
        exported: true,
        ..Symbol::default()
    });
}

fn push_ref(facts: &mut Facts, scope: Option<u32>, address: &str, line: u32) {
    let own = scope.is_some_and(|scope| facts.symbols[scope as usize].name == address);
    let duplicate = facts
        .refs
        .iter()
        .any(|reference| reference.scope == scope && reference.name == address);
    if !own && !duplicate {
        facts.refs.push(Ref {
            kind: RefKind::Value,
            name: address.to_owned(),
            qualifier: None,
            scope,
            line,
            arity: NO_ARITY,
            byte: crate::domain::facts::NO_BYTE,
        });
    }
}

/// Line index of the closing brace of the block opened on line `start` (braces counted outside
/// strings and heredocs are rare enough to ignore).
fn block_end(lines: &[&str], start: usize) -> usize {
    let mut depth = 0i32;
    for (index, line) in lines.iter().enumerate().skip(start) {
        let code = line.split('#').next().unwrap_or_default();
        depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
        if depth <= 0 && index > start {
            return index;
        }
    }
    lines.len().saturating_sub(1)
}

/// Terraform addresses in an expression: `var.x`, `local.x`, `module.x`, `data.t.n` and resource
/// addresses `type.name` (type with an underscore, as provider resource types have).
fn addresses(line: &str) -> Vec<String> {
    let code = line.split(['#']).next().unwrap_or_default();
    let mut found = Vec::new();
    let tokens =
        code.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-'));
    for token in tokens {
        let parts: Vec<&str> = token.split('.').filter(|part| !part.is_empty()).collect();
        let address = if ADDRESS_PREFIXES
            .iter()
            .any(|prefix| token.starts_with(prefix))
        {
            let length = if token.starts_with("data.") { 3 } else { 2 };
            (parts.len() >= length).then(|| parts[..length].join("."))
        } else if parts.len() >= 2
            && parts[0].contains('_')
            && parts[0]
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
            && parts[1]
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        {
            Some(format!("{}.{}", parts[0], parts[1]))
        } else {
            None
        };
        if let Some(address) = address
            && !found.contains(&address)
        {
            found.push(address);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_addresses_and_local_module_sources() {
        let mut facts = Facts::default();
        extract(
            "variable \"region\" {\n  default = \"eu-west-1\"\n}\n\nlocals {\n  name = \"app-${var.region}\"\n  tags = { Name = local.name }\n}\n\nmodule \"network\" {\n  source = \"./modules/network\"\n  cidr   = var.cidr\n}\n\nresource \"aws_s3_bucket\" \"logs\" {\n  bucket = local.name\n  tags   = local.tags\n  vpc    = module.network.vpc_id\n}\n\noutput \"bucket\" {\n  value = aws_s3_bucket.logs.arn\n}\n",
            &mut facts,
        );
        let symbols: Vec<String> = facts
            .symbols
            .iter()
            .map(|symbol| format!("{}-{} {}", symbol.line, symbol.end_line, symbol.name))
            .collect();
        assert_eq!(
            symbols,
            [
                "1-3 var.region",
                "6-6 local.name",
                "7-7 local.tags",
                "10-13 module.network",
                "15-19 aws_s3_bucket.logs",
                "21-23 output.bucket"
            ]
        );
        let refs: Vec<String> = facts
            .refs
            .iter()
            .map(|reference| {
                let scope = reference.scope.map_or("-".to_owned(), |scope| {
                    facts.symbols[scope as usize].name.clone()
                });
                format!("{} {scope} -> {}", reference.line, reference.name)
            })
            .collect();
        assert_eq!(
            refs,
            [
                "6 local.name -> var.region",
                "7 local.tags -> local.name",
                "12 module.network -> var.cidr",
                "16 aws_s3_bucket.logs -> local.name",
                "17 aws_s3_bucket.logs -> local.tags",
                "18 aws_s3_bucket.logs -> module.network",
                "22 output.bucket -> aws_s3_bucket.logs",
            ]
        );
        assert_eq!(
            facts
                .imports
                .iter()
                .map(|import| import.spec.as_str())
                .collect::<Vec<_>>(),
            ["./modules/network"]
        );
    }
}
