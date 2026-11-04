//! Receivers that are expressions: `Builder::new().depth(1).build()`, `c.req.header()`. The type
//! flows through declared return and field types; every such edge is inferred.

use super::*;

/// One step of a receiver expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Step<'a> {
    pub name: &'a str,
    pub call: bool,
}

/// Splits a receiver expression at its top-level member accesses; `None` for anything else, such
/// as an index or an operator. `?`, `!` and `.await` keep the type: return types are unwrapped
/// when they are recorded.
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
        if name != "await" {
            steps.push(Step { name, call });
        }
        while matches!(bytes.get(position), Some(b'?' | b'!')) {
            if bytes.get(position + 1) == Some(&b'.') {
                break;
            }
            position += 1;
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
fn closing(bytes: &[u8], open: usize, left: u8, right: u8) -> Option<usize> {
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
    /// The declared result type of a member named `name` of one of `types`, in its own file.
    pub(super) fn member_result(&self, types: &[u32], name: &str) -> Option<Vec<u32>> {
        types.iter().find_map(|&type_symbol| {
            let member = *self.members(type_symbol, name).first()?;
            let result = *self.returns.get(&member)?;
            let file = self.symbols[member as usize].file;
            let scope = Some(member - self.base[file as usize]);
            self.resolve_type(file, None, scope, result)
                .map(|(types, _)| types)
        })
    }

    /// Type of a receiver expression containing a call, step by step. Rust's `unwrap` and `expect`
    /// keep the type like `?`, because recorded return types are unwrapped.
    pub(super) fn chain_type(
        &self,
        file: u32,
        imports: &Imports,
        reference: &Ref,
        steps: &[Step],
    ) -> Option<Vec<u32>> {
        let (first, rest) = steps.split_first()?;
        let mut types = if first.call {
            let binding = Binding {
                scope: reference.scope,
                name: String::new(),
                type_name: format!("{}()", first.name),
                line: reference.line,
                scope_end_line: None,
                declared: false,
            };
            self.binding_type(file, imports, reference.scope, &binding)?
                .0
        } else {
            let chain = [first.name];
            match self.receiver_type(file, imports, reference.scope, &chain, reference.line) {
                Receiver::Known(types, _) => types,
                _ => return None,
            }
        };
        let rust = self.file_type(file) == Some(FileType::Rust);
        for step in rest {
            if rust && step.call && matches!(step.name, "unwrap" | "expect") {
                continue;
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
                None => self.member_result(&types, step.name)?,
            };
        }
        Some(types)
    }
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
