//! Receivers that are expressions: `Builder::new().depth(1).build()`, `c.req.header()`. The type
//! flows through declared return and field types; every such edge is inferred.

use super::*;

/// One step of a receiver expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Step<'a> {
    pub name: &'a str,
    pub call: bool,
    pub tries: usize,
}

pub(super) enum ChainFailure {
    Unknown,
    External,
}

/// Splits a receiver expression at its top-level member accesses; `None` for anything else, such
/// as an index or an operator. Rust try operators are retained; TypeScript's non-null assertion
/// and optional access do not change the recorded type.
pub(super) fn receiver_steps(qualifier: &str) -> Option<Vec<Step<'_>>> {
    let text = qualifier.trim();
    let text = text.strip_prefix("new ").map_or(text, str::trim_start);
    let bytes = text.as_bytes();
    let mut steps = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let start = position;
        while position < bytes.len() {
            match bytes[position] {
                byte if byte.is_ascii_alphanumeric()
                    || matches!(byte, b'_' | b'#' | b'$')
                    || byte >= 0x80 =>
                {
                    position += 1;
                }
                b':' if bytes.get(position + 1) == Some(&b':')
                    && bytes.get(position + 2) != Some(&b'<') =>
                {
                    position += 2;
                }
                _ => break,
            }
        }
        if position == start {
            return None;
        }
        let name = &text[start..position];
        if bytes[position..].starts_with(b"::<") {
            position = closing(bytes, position + 2, b'<', b'>')? + 1;
        }
        // `new Hono<Env>()`: type arguments of a call.
        if bytes.get(position) == Some(&b'<')
            && let Some(close) = closing(bytes, position, b'<', b'>')
            && bytes.get(close + 1) == Some(&b'(')
        {
            position = close + 1;
        }
        let call = bytes.get(position) == Some(&b'(');
        if call {
            position = closing(bytes, position, b'(', b')')? + 1;
        }
        let mut tries = 0;
        while matches!(bytes.get(position), Some(b'?' | b'!')) {
            if bytes.get(position + 1) == Some(&b'.') {
                tries += usize::from(bytes[position] == b'?');
                position += 1;
                break;
            }
            tries += usize::from(bytes[position] == b'?');
            position += 1;
        }
        if name != "await" {
            steps.push(Step { name, call, tries });
        } else if let Some(previous) = steps.last_mut() {
            previous.tries += tries;
        }
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
        if position == bytes.len() {
            break;
        }
        if matches!(bytes[position], b'?' | b'!') {
            position += 1;
        }
        if bytes.get(position) != Some(&b'.') {
            return None;
        }
        position += 1;
        while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            position += 1;
        }
    }
    (!steps.is_empty()).then_some(steps)
}

/// Position of the bracket closing the one at `open`; brackets inside string literals do not count.
pub(super) fn closing(bytes: &[u8], open: usize, left: u8, right: u8) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut position = open;
    while let Some(&byte) = bytes.get(position) {
        if let Some(mark) = quote {
            if byte == b'\\' {
                position += 1;
            } else if byte == mark {
                quote = None;
            }
        } else if matches!(byte, b'"' | b'\'' | b'`') {
            // A Rust lifetime is no string; an unclosed quote fails the whole expression.
            if byte != b'\'' || left == b'(' {
                quote = Some(byte);
            }
        } else if byte == left {
            depth += 1;
        } else if byte == right {
            depth -= 1;
            if depth == 0 {
                return Some(position);
            }
        }
        position += 1;
    }
    None
}

impl Index<'_> {
    /// Resolve the normalized result while keeping full Rust return syntax in cached facts.
    pub(super) fn resolve_result_type(
        &self,
        file: u32,
        scope: Option<u32>,
        result: &str,
    ) -> Option<(Vec<u32>, Confidence)> {
        if self.file_type(file) == Some(FileType::Rust) {
            let (result, _) = self.rust_result(file, scope, result);
            let result = crate::extract::engine::type_name(result);
            self.resolve_type(file, None, scope, &result)
        } else {
            self.resolve_type(file, None, scope, result)
        }
    }

    fn member_result_symbol(&self, types: &[u32], name: &str) -> Option<u32> {
        types.iter().find_map(|&symbol| {
            let member = *self.members(symbol, name).first()?;
            self.returns.contains_key(&member).then_some(member)
        })
    }

    /// The declared result type of a member named `name` of one of `types`, in its own file.
    pub(super) fn member_result(&self, types: &[u32], name: &str) -> Option<Vec<u32>> {
        types.iter().find_map(|&type_symbol| {
            let member = *self.members(type_symbol, name).first()?;
            let result = *self.returns.get(&member)?;
            let file = self.symbols[member as usize].file;
            let scope = Some(member - self.base[file as usize]);
            self.resolve_result_type(file, scope, result)
                .map(|(types, _)| types)
        })
    }

    /// Type of a receiver expression containing a call, step by step. A Rust unwrap/expect
    /// consumes a pending Option/Result layer; on a plain type it is an ordinary declared method.
    pub(super) fn chain_type(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        steps: &[Step],
    ) -> Result<Vec<u32>, ChainFailure> {
        let (first, rest) = steps.split_first().ok_or(ChainFailure::Unknown)?;
        let rust = self.file_type(file) == Some(FileType::Rust);
        let mut layers = if rust && first.call {
            self.call_result_layers(file, imports, reference, first.name)
                .ok_or(ChainFailure::Unknown)?
        } else if rust {
            self.binding(file, reference.scope, first.name, reference.line)
                .and_then(|binding| call_binding(&binding.type_name))
                .map(|(callee, tries)| {
                    self.call_result_layers(file, imports, reference, callee)?
                        .checked_sub(tries)
                })
                .unwrap_or(Some(0))
                .ok_or(ChainFailure::Unknown)?
        } else {
            0
        };
        let mut types = if first.call {
            let binding = Binding {
                scope: reference.scope,
                name: String::new(),
                type_name: format!("{}()", first.name),
                line: reference.line,
                scope_end_line: None,
                declared: false,
            };
            match self.binding_type(file, imports, reference.scope, &binding) {
                Some((types, _)) => types,
                None if self.call_result_is_external(
                    file,
                    imports,
                    reference.scope,
                    reference.line,
                    first.name,
                ) =>
                {
                    return Err(ChainFailure::External);
                }
                None => return Err(ChainFailure::Unknown),
            }
        } else {
            let chain = [first.name];
            match self.receiver_type(file, imports, reference.scope, &chain, reference.line) {
                Receiver::Known(types, _) => types,
                Receiver::External => return Err(ChainFailure::External),
                _ => return Err(ChainFailure::Unknown),
            }
        };
        if rust {
            layers = layers
                .checked_sub(first.tries)
                .ok_or(ChainFailure::Unknown)?;
        }
        for step in rest {
            if rust && layers > 0 && step.call && matches!(step.name, "unwrap" | "expect") {
                layers = (layers - 1)
                    .checked_sub(step.tries)
                    .ok_or(ChainFailure::Unknown)?;
                continue;
            }
            if rust && layers > 0 {
                return Err(ChainFailure::External);
            }
            let field = (!step.call)
                .then(|| {
                    types.iter().find_map(|&part| {
                        let field_type = *self.field_types.get(&(part, step.name))?;
                        let owner = self.symbols[part as usize].file;
                        self.resolve_type(owner, None, None, field_type)
                            .map(|(types, _)| types)
                    })
                })
                .flatten();
            types = match field {
                Some(found) => found,
                None => {
                    if rust {
                        let member = self
                            .member_result_symbol(&types, step.name)
                            .ok_or(ChainFailure::Unknown)?;
                        let owner = self.symbols[member as usize].file;
                        let scope = Some(member - self.base[owner as usize]);
                        let result = self.returns.get(&member).ok_or(ChainFailure::Unknown)?;
                        layers = self.rust_result(owner, scope, result).1;
                    }
                    self.member_result(&types, step.name).ok_or(if rust {
                        ChainFailure::External
                    } else {
                        ChainFailure::Unknown
                    })?
                }
            };
            if rust {
                layers = layers
                    .checked_sub(step.tries)
                    .ok_or(ChainFailure::Unknown)?;
            }
        }
        if layers == 0 {
            Ok(types)
        } else {
            Err(ChainFailure::External)
        }
    }
}

/// Callee and try operations retained in a call-result binding, without argument contents.
pub(super) fn call_binding(type_name: &str) -> Option<(&str, usize)> {
    let call = type_name.trim_end_matches('?');
    let tries = type_name.len() - call.len();
    Some((call.strip_suffix("()")?, tries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(qualifier: &str) -> Option<Vec<(&str, bool)>> {
        Some(
            receiver_steps(qualifier)?
                .iter()
                .map(|step| (step.name, step.call))
                .collect(),
        )
    }

    #[test]
    fn receiver_expressions_split_at_member_accesses() {
        let cases = [
            ("c", Some(vec![("c", false)])),
            ("c.req", Some(vec![("c", false), ("req", false)])),
            (
                "Builder::new().depth(1)",
                Some(vec![("Builder::new", true), ("depth", true)]),
            ),
            (
                "a.b(f(x), \")\").c",
                Some(vec![("a", false), ("b", true), ("c", false)]),
            ),
            (
                "open()?.read().await",
                Some(vec![("open", true), ("read", true)]),
            ),
            (
                "a!.b?.c()",
                Some(vec![("a", false), ("b", false), ("c", true)]),
            ),
            ("new Store()", Some(vec![("Store", true)])),
            (
                "x\n    .y::<T>()\n    .z",
                Some(vec![("x", false), ("y", true), ("z", false)]),
            ),
            ("a[0].b", None),
        ];
        for (qualifier, expected) in cases {
            assert_eq!(names(qualifier), expected, "{qualifier}");
        }
        assert_eq!(
            names("new Hono<Env>().route"),
            Some(vec![("Hono", true), ("route", false)])
        );
        for rejected in ["a + b", "(a).b", "f(", ""] {
            assert_eq!(names(rejected), None, "{rejected}");
        }
    }
}
