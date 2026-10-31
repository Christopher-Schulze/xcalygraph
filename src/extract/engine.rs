//! Generic query-driven extraction: one tree-sitter query per language maps captures to facts.
//!
//! Capture convention (the standard tags names plus extensions):
//! - `@definition.<kind>` with `@name`, optional `@owner` (receiver type of a method);
//!   kinds: function, method, class, struct, interface, trait, enum, type, constant, variable,
//!   macro, module.
//! - `@reference.<kind>` with `@name`, optional `@qualifier`; kinds: call, type, inherit,
//!   implementation.
//! - `@impl` with `@impl.type` and optional `@impl.trait`: methods inside belong to the type.
//! - `@import` with `@import.path` or `@import.tree`, optional `@import.alias`, `@import.name`,
//!   `@import.reexport`.
//! - `@binding` with `@binding.name` and one of `@binding.type` (declared), `@binding.ctor`
//!   (constructed type), `@binding.call` (result of a call), `@binding.range` (element of the
//!   named collection).
//! - `@return` with `@return.type` inside a definition: its declared result type.
//! - `@field` with `@field.name` and `@field.type` inside a type definition.
//! - `@env` with `@env.name`; `@package` for the declared package or namespace.
//! - Captures starting with `_` serve predicates only.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

use crate::domain::facts::{
    self, Binding, EnvRead, Facts, Fields, Import, Ref, RefKind, SymKind, Symbol,
};

/// Language-specific behavior the query cannot express.
pub struct Hooks {
    /// Whether a definition is visible outside its module.
    pub exported: fn(&Definition) -> bool,
    /// Keyword prepended to signatures of definitions whose node omits it (Go `type`, `const`).
    pub signature_prefix: fn(SymKind, &str) -> &'static str,
    /// Test and entry flags of a definition.
    pub flags: fn(&Definition, &FileContext) -> u8,
    /// Expands an `@import.tree` capture (Rust use trees) into imports.
    pub expand_import: fn(&str, u32, bool) -> Vec<Import>,
    /// Unqualified names of builtins and prelude items; references to them are dropped.
    pub builtin: fn(&str) -> bool,
}

/// What hooks may inspect about a definition.
pub struct Definition<'a> {
    pub kind: SymKind,
    pub name: &'a str,
    /// Source text of the definition node.
    pub text: &'a str,
    /// Text of attribute, decorator and annotation nodes directly before the definition.
    pub decorations: &'a str,
    /// Flags inherited from the enclosing definition.
    pub inherited: u8,
    /// Kind and visibility of the enclosing definition.
    pub parent: Option<(SymKind, bool)>,
    /// Inside an implementation of a trait or interface (`impl Trait for T`).
    pub in_trait_impl: bool,
    /// Wrapped by an `export` statement (TypeScript, JavaScript).
    pub export_wrapper: bool,
    /// Grammar node kind of the definition (`test_case_definition`).
    pub node_kind: &'a str,
}

pub struct FileContext<'a> {
    pub path: &'a str,
    pub package: Option<&'a str>,
}

/// Default import expansion: the tree text is the module specifier.
pub fn single_import(text: &str, line: u32, reexport: bool) -> Vec<Import> {
    vec![Import {
        spec: unquote(text).to_owned(),
        line,
        reexport,
        ..Import::default()
    }]
}

thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

#[derive(Default)]
struct RawDef<'tree> {
    node: Option<Node<'tree>>,
    kind: Option<SymKind>,
    name: String,
    name_node: usize,
    owner: Option<String>,
    /// Last clause of a multi-clause definition (Elixir `def f(nil)` then `def f(x)`).
    last_clause: Option<Node<'tree>>,
}

#[derive(Default)]
struct RawRef {
    kind: Option<RefKind>,
    name: String,
    qualifier: Option<String>,
    position: usize,
    line: u32,
    arity: u8,
}

#[derive(Default)]
struct RawImport {
    path: Option<String>,
    tree: Option<String>,
    alias: Option<String>,
    names: Vec<String>,
    reexport: bool,
    line: u32,
}

#[derive(Default)]
struct RawBinding {
    name: Option<String>,
    type_name: Option<String>,
    declared: bool,
    call: bool,
    position: usize,
    line: u32,
}

#[derive(Default)]
struct RawField {
    name: Option<String>,
    type_name: String,
    position: usize,
}

/// Extracts facts from `source` with the given grammar, compiled query and hooks.
pub fn extract(
    language: &Language,
    query: &Query,
    hooks: &Hooks,
    path: &str,
    source: &[u8],
) -> Facts {
    PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        if parser.set_language(language).is_err() {
            return Facts::default();
        }
        match parser.parse(source, None) {
            Some(tree) => collect(query, hooks, path, source, tree.root_node()),
            None => Facts::default(),
        }
    })
}

fn collect(query: &Query, hooks: &Hooks, path: &str, source: &[u8], root: Node) -> Facts {
    let elixir = path.ends_with(".ex") || path.ends_with(".exs");
    let lisp = [".lisp", ".lsp", ".cl", ".asd"]
        .iter()
        .any(|suffix| path.ends_with(suffix));
    let names = query.capture_names();
    let text = |node: Node| node.utf8_text(source).unwrap_or_default();
    let mut defs: HashMap<usize, RawDef> = HashMap::new();
    let mut impls: Vec<(Node, String, Option<String>)> = Vec::new();
    let mut refs: HashMap<usize, RawRef> = HashMap::new();
    let mut imports: HashMap<usize, RawImport> = HashMap::new();
    let mut bindings: HashMap<usize, RawBinding> = HashMap::new();
    let mut returns: Vec<(usize, String)> = Vec::new();
    let mut fields: HashMap<usize, RawField> = HashMap::new();
    let mut envs: HashMap<usize, (String, usize, u32)> = HashMap::new();
    let mut package = None;
    let mut entry = false;
    // Variables assigned in the file (Bash): their reads are no environment reads.
    let mut assigned: Vec<String> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_match_limit(4096);
    let mut matches = cursor.matches(query, root, source);
    while let Some(found) = matches.next() {
        let captures = found.captures();
        for capture in captures
            .iter()
            .filter(|capture| names[capture.index as usize] == "assigned")
        {
            assigned.push(text(capture.node).to_owned());
        }
        let Some((primary, primary_node)) = captures.iter().find_map(|capture| {
            let name = names[capture.index as usize];
            is_primary(name).then_some((name, capture.node))
        }) else {
            continue;
        };
        let get = |wanted: &str| {
            captures
                .iter()
                .find(|capture| names[capture.index as usize] == wanted)
                .map(|capture| capture.node)
        };
        let key = primary_node.id();
        let line = row(primary_node);
        if let Some(kind) = primary.strip_prefix("definition.") {
            let Some(name) = get("name") else { continue };
            let entry = defs.entry(key).or_default();
            entry.node = Some(primary_node);
            entry.kind = def_kind(kind);
            entry.name = text(name).to_owned();
            entry.name_node = name.id();
            if let Some(owner) = get("owner") {
                entry.owner = Some(type_name(text(owner)));
            }
            // A result type captured with the definition (one pattern instead of two).
            if let Some(node) = get("return.type") {
                returns.push((primary_node.start_byte(), result_type(text(node))));
            }
        } else if let Some(kind) = primary.strip_prefix("reference.") {
            let Some(name) = get("name") else { continue };
            if elixir && kind == "call" && in_type_specification(name, source) {
                continue;
            }
            if lisp && kind == "call" && is_slot_specification(name, source) {
                continue;
            }
            let reference = refs.entry(name.id()).or_default();
            let kind = ref_kind(kind);
            if priority(kind) < priority(reference.kind) {
                continue;
            }
            reference.kind = kind;
            reference.arity = match kind {
                Some(RefKind::Call) => arity(primary_node),
                _ => facts::NO_ARITY,
            };
            reference.name = text(name).to_owned();
            reference.position = name.start_byte();
            reference.line = row(name);
            if let Some(qualifier) = get("qualifier") {
                reference.qualifier =
                    Some(facts::clip(&facts::normalize_space(text(qualifier)), 80));
            }
        } else {
            match primary {
                "impl" => {
                    if let Some(target) = get("impl.type") {
                        let tr = get("impl.trait").map(|node| type_name(text(node)));
                        if !impls.iter().any(|(node, _, _)| node.id() == key) {
                            impls.push((primary_node, type_name(text(target)), tr));
                        } else if let Some(entry) =
                            impls.iter_mut().find(|(node, _, _)| node.id() == key)
                            && entry.2.is_none()
                        {
                            entry.2 = tr;
                        }
                    }
                }
                "import" => {
                    let entry = imports.entry(key).or_default();
                    entry.line = line;
                    if let Some(node) = get("import.path") {
                        entry.path = Some(unquote(text(node)).to_owned());
                    }
                    if let Some(node) = get("import.tree") {
                        entry.tree = Some(text(node).to_owned());
                    }
                    if let Some(node) = get("import.alias") {
                        entry.alias = Some(text(node).to_owned());
                    }
                    for capture in captures
                        .iter()
                        .filter(|c| names[c.index as usize] == "import.name")
                    {
                        let name = text(capture.node).to_owned();
                        if !entry.names.contains(&name) {
                            entry.names.push(name);
                        }
                    }
                    if get("import.reexport").is_some() {
                        entry.reexport = true;
                    }
                }
                "binding" => {
                    let entry = bindings.entry(key).or_default();
                    entry.position = primary_node.start_byte();
                    entry.line = line;
                    if let Some(node) = get("binding.name") {
                        entry.name = Some(text(node).to_owned());
                    }
                    if let Some(node) = get("binding.type") {
                        entry.type_name = Some(type_name(text(node)));
                        entry.declared = true;
                    } else if let Some(node) = get("binding.ctor") {
                        // A constructor call fixes the type as firmly as a declaration.
                        entry.type_name = Some(type_name(text(node)));
                        entry.declared = true;
                    } else if let Some(node) = get("binding.range") {
                        entry.type_name = Some(format!("range:{}", text(node)));
                    } else if let Some(node) = get("binding.call") {
                        entry.type_name = Some(facts::normalize_space(text(node)));
                        entry.call = true;
                        entry.declared = false;
                    }
                }
                "return" => {
                    if let Some(node) = get("return.type") {
                        returns.push((primary_node.start_byte(), result_type(text(node))));
                    }
                }
                "field" => {
                    let entry = fields.entry(key).or_default();
                    entry.position = primary_node.start_byte();
                    if let Some(node) = get("field.name") {
                        entry.name = Some(text(node).to_owned());
                    }
                    if let Some(node) = get("field.type") {
                        entry.type_name = type_name(text(node));
                    }
                }
                "env" => {
                    if let Some(node) = get("env.name") {
                        let name = unquote(text(node)).to_owned();
                        envs.insert(key, (name, primary_node.start_byte(), line));
                    }
                }
                "entry" => entry = true,
                "package" if package.is_none() => {
                    package = Some(unquote(text(primary_node)).to_owned());
                }
                _ => {}
            }
        }
    }
    envs.retain(|_, (name, _, _)| !assigned.contains(name));
    let doc = file_doc(root, source);
    let mut facts = assemble(Assembly {
        hooks,
        path,
        source,
        package,
        defs,
        impls,
        refs,
        imports,
        bindings,
        returns,
        fields,
        envs,
    });
    facts.doc = doc;
    facts.entry |= entry;
    facts
}

struct Assembly<'a, 'tree> {
    hooks: &'a Hooks,
    path: &'a str,
    source: &'a [u8],
    package: Option<String>,
    defs: HashMap<usize, RawDef<'tree>>,
    impls: Vec<(Node<'tree>, String, Option<String>)>,
    refs: HashMap<usize, RawRef>,
    imports: HashMap<usize, RawImport>,
    bindings: HashMap<usize, RawBinding>,
    returns: Vec<(usize, String)>,
    fields: HashMap<usize, RawField>,
    envs: HashMap<usize, (String, usize, u32)>,
}

fn assemble(assembly: Assembly) -> Facts {
    let Assembly {
        hooks,
        path,
        source,
        package,
        defs,
        impls,
        refs,
        imports,
        bindings,
        returns,
        fields,
        envs,
    } = assembly;
    let text = |node: Node| node.utf8_text(source).unwrap_or_default();
    let mut defs: Vec<RawDef> = defs
        .into_values()
        .filter(|def| def.node.is_some())
        .collect();
    defs.sort_by_key(|def| {
        def.node.map_or((0, 0), |node| {
            (node.start_byte(), usize::MAX - node.end_byte())
        })
    });
    let def_name_nodes: Vec<usize> = defs.iter().map(|def| def.name_node).collect();
    let defs = merge_clauses(defs);
    let ranges: Vec<(usize, usize)> = defs
        .iter()
        .filter_map(|def| {
            def.node.map(|node| {
                (
                    node.start_byte(),
                    def.last_clause.unwrap_or(node).end_byte(),
                )
            })
        })
        .collect();
    let context = FileContext {
        path,
        package: package.as_deref(),
    };
    let mut symbols: Vec<Symbol> = Vec::with_capacity(defs.len());
    let mut stack: Vec<usize> = Vec::new();
    for (index, def) in defs.iter().enumerate() {
        let Some(node) = def.node else { continue };
        let (start, _) = ranges[index];
        while let Some(&top) = stack.last() {
            if ranges[top].1 <= start {
                stack.pop();
            } else {
                break;
            }
        }
        let parent = stack.last().copied();
        stack.push(index);
        let mut kind = def.kind.unwrap_or(SymKind::Fn);
        let enclosing_impl = impls
            .iter()
            .filter(|(impl_node, _, _)| {
                impl_node.start_byte() <= start && start < impl_node.end_byte()
            })
            .min_by_key(|(impl_node, _, _)| impl_node.end_byte() - impl_node.start_byte());
        let parent_is_type = parent.is_some_and(|p| symbols[p].kind().is_type());
        if kind == SymKind::Fn && (enclosing_impl.is_some() || parent_is_type) {
            kind = SymKind::Method;
        }
        let owner = def
            .owner
            .clone()
            .or_else(|| enclosing_impl.map(|(_, target, _)| target.clone()))
            .or_else(|| {
                parent
                    .filter(|&p| kind == SymKind::Method && symbols[p].kind().is_type())
                    .map(|p| symbols[p].name.clone())
            })
            .unwrap_or_default();
        let decorations = decorations(node, source);
        let node_text = text(node);
        let inherited = parent.map_or(0, |p| symbols[p].flags & facts::FLAG_TEST);
        let definition = Definition {
            node_kind: node.kind(),
            kind,
            name: &def.name,
            text: node_text,
            decorations: &decorations,
            inherited,
            parent: parent.map(|p| (symbols[p].kind(), symbols[p].exported)),
            in_trait_impl: enclosing_impl.is_some_and(|(_, _, trait_name)| trait_name.is_some()),
            export_wrapper: node
                .parent()
                .into_iter()
                .chain(node.parent().and_then(|parent| parent.parent()))
                .any(|ancestor| ancestor.kind() == "export_statement"),
        };
        let exported = (hooks.exported)(&definition);
        let flags = (hooks.flags)(&definition, &context) | inherited;
        let prefix = (hooks.signature_prefix)(kind, node_text);
        let signature = facts::clip(&format!("{prefix}{}", signature(node, source)), 160);
        symbols.push(Symbol {
            kind: Some(kind),
            name: def.name.clone(),
            parent: parent.and_then(|p| u32::try_from(p).ok()),
            owner,
            line: row(node),
            end_line: def.last_clause.unwrap_or(node).end_position().row as u32 + 1,
            signature,
            doc: doc_comment(node, source),
            exported,
            flags,
        });
    }
    let scope_of = |position: usize| -> Option<u32> {
        let after = ranges.partition_point(|&(start, _)| start <= position);
        let mut candidate = after.checked_sub(1)?;
        loop {
            if ranges[candidate].0 <= position && position < ranges[candidate].1 {
                return u32::try_from(candidate).ok();
            }
            candidate = symbols[candidate].parent? as usize;
        }
    };
    // A builtin name the file defines as a function names that definition: a Python module
    // shadowing `open`, Julia `Base.length(q::Queue)` extending `length` for its type.
    let local_functions: std::collections::HashSet<&str> = symbols
        .iter()
        .filter(|symbol| symbol.kind == Some(SymKind::Fn))
        .map(|symbol| symbol.name.as_str())
        .collect();
    let mut facts_refs: Vec<Ref> = refs
        .into_iter()
        .filter(|(name_node, raw)| {
            !def_name_nodes.contains(name_node)
                && !(raw.qualifier.is_none()
                    && (hooks.builtin)(&raw.name)
                    && !local_functions.contains(raw.name.as_str()))
        })
        .filter_map(|(_, raw)| {
            Some(Ref {
                kind: raw.kind?,
                name: raw.name,
                qualifier: raw.qualifier,
                scope: scope_of(raw.position),
                line: raw.line,
                arity: raw.arity,
            })
        })
        .collect();
    for (impl_node, target, trait_name) in &impls {
        if let Some(trait_name) = trait_name {
            facts_refs.push(Ref {
                kind: RefKind::Implement,
                name: trait_name.clone(),
                qualifier: Some(target.clone()),
                scope: None,
                line: row(*impl_node),
                arity: facts::NO_ARITY,
            });
        }
    }
    facts_refs.sort_by(|a, b| {
        (a.line, &a.name, a.kind, &a.qualifier, a.scope).cmp(&(
            b.line,
            &b.name,
            b.kind,
            &b.qualifier,
            b.scope,
        ))
    });
    // A value reference repeated in its scope adds no edge: binding shadowing only grows with
    // the line and edges keep their first line, so the first occurrence decides.
    let mut seen_values = HashSet::new();
    facts_refs.retain(|reference| {
        reference.kind != RefKind::Value
            || seen_values.insert((
                reference.name.clone(),
                reference.qualifier.clone(),
                reference.scope,
            ))
    });
    let mut facts_imports: Vec<Import> = imports
        .into_values()
        .flat_map(|raw| match (raw.tree, raw.path) {
            (Some(tree), _) => (hooks.expand_import)(&tree, raw.line, raw.reexport),
            (None, Some(path)) => vec![Import {
                spec: path,
                alias: raw.alias,
                names: raw.names,
                line: raw.line,
                reexport: raw.reexport,
            }],
            (None, None) => Vec::new(),
        })
        .collect();
    facts_imports.sort_by(|a, b| {
        (a.line, &a.spec, &a.names, &a.alias).cmp(&(b.line, &b.spec, &b.names, &b.alias))
    });
    let mut facts_bindings: Vec<Binding> = bindings
        .into_values()
        .filter_map(|raw| {
            let type_name = raw.type_name.filter(|name| !name.is_empty())?;
            Some(Binding {
                scope: scope_of(raw.position),
                name: raw.name?,
                type_name: if raw.call {
                    format!("{type_name}()")
                } else {
                    type_name
                },
                line: raw.line,
                declared: raw.declared,
            })
        })
        .collect();
    for (position, type_name) in returns {
        if let Some(scope) = scope_of(position) {
            facts_bindings.push(Binding {
                scope: Some(scope),
                name: String::new(),
                type_name,
                line: symbols[scope as usize].line,
                declared: true,
            });
        }
    }
    facts_bindings.sort_by(|a, b| {
        (a.line, &a.name, &a.type_name, a.scope, a.declared).cmp(&(
            b.line,
            &b.name,
            &b.type_name,
            b.scope,
            b.declared,
        ))
    });
    facts_bindings.dedup();
    let mut grouped: Vec<Fields> = Vec::new();
    let mut raw_fields: Vec<RawField> = fields.into_values().collect();
    raw_fields.sort_by_key(|field| field.position);
    for field in raw_fields {
        let (Some(name), Some(mut owner)) = (field.name, scope_of(field.position)) else {
            continue;
        };
        // `self.x = ...` inside a method belongs to the enclosing type.
        while !symbols[owner as usize].kind().is_type() {
            match symbols[owner as usize].parent {
                Some(parent) => owner = parent,
                None => break,
            }
        }
        if !symbols[owner as usize].kind().is_type() {
            continue;
        }
        match grouped.iter_mut().find(|group| group.owner == owner) {
            Some(group) => {
                group.names.push(name);
                group.types.push(field.type_name);
            }
            None => grouped.push(Fields {
                owner,
                names: vec![name],
                types: vec![field.type_name],
            }),
        }
    }
    let mut facts_envs: Vec<EnvRead> = envs
        .into_values()
        .filter(|(name, _, _)| !name.is_empty())
        .map(|(name, position, line)| EnvRead {
            name,
            scope: scope_of(position),
            line,
        })
        .collect();
    facts_envs.sort_by(|a, b| (a.line, &a.name, a.scope).cmp(&(b.line, &b.name, b.scope)));
    Facts {
        package,
        symbols,
        imports: facts_imports,
        refs: facts_refs,
        bindings: facts_bindings,
        envs: facts_envs,
        fields: grouped,
        ..Facts::default()
    }
}

fn is_primary(name: &str) -> bool {
    name.starts_with("definition.")
        || name.starts_with("reference.")
        || matches!(
            name,
            "impl" | "import" | "binding" | "return" | "field" | "env" | "package"
        )
}

fn def_kind(kind: &str) -> Option<SymKind> {
    Some(match kind {
        "function" => SymKind::Fn,
        "method" => SymKind::Method,
        "class" => SymKind::Class,
        "struct" => SymKind::Struct,
        "interface" => SymKind::Iface,
        "trait" => SymKind::Trait,
        "enum" => SymKind::Enum,
        "type" => SymKind::Alias,
        "constant" => SymKind::Const,
        "variable" => SymKind::Var,
        "macro" => SymKind::Macro,
        "module" => SymKind::Mod,
        _ => return None,
    })
}

fn ref_kind(kind: &str) -> Option<RefKind> {
    Some(match kind {
        "call" => RefKind::Call,
        "type" => RefKind::Type,
        "inherit" | "class" => RefKind::Inherit,
        "implementation" | "implement" => RefKind::Implement,
        "value" => RefKind::Value,
        _ => return None,
    })
}

/// Stronger reference kinds win when several patterns capture the same name.
fn priority(kind: Option<RefKind>) -> u8 {
    match kind {
        Some(RefKind::Inherit | RefKind::Implement) => 3,
        Some(RefKind::Call) => 2,
        Some(RefKind::Value) => 0,
        Some(_) => 1,
        None => 0,
    }
}

/// Argument count of a call node: its `arguments` field, else an argument list child (Kotlin
/// `value_arguments`); a trailing lambda counts as an argument.
/// Elixir `@spec`, `@type`, `@callback` attributes: their call syntax names types and the
/// specified function, not calls.
fn in_type_specification(name: Node, source: &[u8]) -> bool {
    let mut current = name;
    for _ in 0..8 {
        let Some(parent) = current.parent() else {
            return false;
        };
        if parent.kind() == "unary_operator" {
            return parent
                .child_by_field_name("operator")
                .is_some_and(|op| op.kind() == "@")
                && parent
                    .child_by_field_name("operand")
                    .and_then(|operand| operand.child_by_field_name("target"))
                    .and_then(|target| target.utf8_text(source).ok())
                    .is_some_and(|attribute| {
                        matches!(
                            attribute,
                            "spec" | "type" | "typep" | "opaque" | "callback" | "macrocallback"
                        )
                    });
        }
        current = parent;
    }
    false
}

/// Common Lisp slot specification `(rows :initarg :rows :accessor rows)`: the list head names a
/// slot, not a called function.
fn is_slot_specification(name: Node, source: &[u8]) -> bool {
    name.next_named_sibling()
        .filter(|next| next.kind() == "kwd_lit")
        .and_then(|next| next.utf8_text(source).ok())
        .is_some_and(|keyword| {
            matches!(
                keyword.to_ascii_lowercase().as_str(),
                ":initarg"
                    | ":initform"
                    | ":accessor"
                    | ":reader"
                    | ":writer"
                    | ":allocation"
                    | ":documentation"
            )
        })
}

/// Consecutive sibling clauses of one function with the same name and parameter count (Elixir
/// `def`/`defp` calls, Erlang `fun_decl`s) become one definition spanning all clauses.
fn merge_clauses(defs: Vec<RawDef<'_>>) -> Vec<RawDef<'_>> {
    let mut merged: Vec<RawDef> = Vec::with_capacity(defs.len());
    for def in defs {
        if let (Some(previous), Some(node)) = (merged.last_mut(), def.node)
            && matches!(node.kind(), "call" | "fun_decl")
            && previous.name == def.name
            && previous.kind == def.kind
            && previous
                .last_clause
                .or(previous.node)
                .and_then(|last| last.next_named_sibling())
                == Some(node)
            && previous.node.and_then(clause_parameters) == clause_parameters(node)
        {
            previous.last_clause = Some(node);
            continue;
        }
        merged.push(def);
    }
    merged
}

/// Parameter count of an Elixir definition call (`def name(a, b) when ...`) or an Erlang
/// function clause.
fn clause_parameters(node: Node) -> Option<usize> {
    if node.kind() == "fun_decl" {
        let clause = node.child_by_field_name("clause")?;
        return Some(clause.child_by_field_name("args")?.named_child_count());
    }
    let mut walker = node.walk();
    let arguments = node
        .named_children(&mut walker)
        .find(|child| child.kind() == "arguments")?;
    let mut head = arguments.named_child(0)?;
    if head.kind() == "binary_operator" {
        head = head.child_by_field_name("left")?;
    }
    if head.kind() != "call" {
        return Some(0);
    }
    let mut inner = head.walk();
    let count = head
        .named_children(&mut inner)
        .find(|child| child.kind() == "arguments")
        .map_or(0, |list| list.named_child_count());
    Some(count)
}

fn arity(call: Node) -> u8 {
    // Elixir `x |> f(y)`: the pipe passes the first argument.
    if call.parent().is_some_and(|parent| {
        parent.kind() == "binary_operator"
            && parent
                .child_by_field_name("operator")
                .is_some_and(|op| op.kind() == "|>")
    }) {
        return facts::NO_ARITY;
    }
    // Objective-C `[receiver part:a other:b]`: the arguments are direct children.
    if call.kind() == "message_expression" {
        let mut cursor = call.walk();
        let mut count = 0usize;
        if cursor.goto_first_child() {
            loop {
                let node = cursor.node();
                if node.is_named()
                    && cursor.field_name().is_none()
                    && !node.kind().contains("comment")
                {
                    count += 1;
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
        return u8::try_from(count).unwrap_or(facts::NO_ARITY - 1);
    }
    let mut walker = call.walk();
    let lists: Vec<Node> = match call.child_by_field_name("arguments") {
        Some(arguments) => vec![arguments],
        None => call
            .named_children(&mut walker)
            .flat_map(|child| {
                // Swift wraps arguments and trailing closures in a `call_suffix`.
                if child.kind() == "call_suffix" {
                    let mut inner = child.walk();
                    child.named_children(&mut inner).collect::<Vec<_>>()
                } else {
                    vec![child]
                }
            })
            .filter(|child| {
                matches!(
                    child.kind(),
                    "value_arguments"
                        | "argument_list"
                        | "arguments"
                        | "expr_args"
                        | "exprArgs"
                        | "annotated_lambda"
                        | "lambda_literal"
                )
            })
            .collect(),
    };
    if lists.is_empty() {
        return facts::NO_ARITY;
    }
    let count: usize = lists
        .iter()
        .map(|list| {
            if matches!(list.kind(), "annotated_lambda" | "lambda_literal") {
                return 1;
            }
            let mut inner = list.walk();
            list.named_children(&mut inner)
                .filter(|argument| !argument.kind().contains("comment"))
                .count()
        })
        .sum();
    u8::try_from(count).unwrap_or(facts::NO_ARITY - 1)
}

fn row(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Strips string quotes and backticks.
pub fn unquote(text: &str) -> &str {
    text.trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
}

/// Bare type name of a type expression: pointers, references, slices, generics and
/// lifetimes removed; qualified names keep their qualifier (`pkg.T`, `a::T`).
pub fn type_name(text: &str) -> String {
    let mut text = facts::normalize_space(text);
    if text.starts_with('(') {
        text = text
            .trim_start_matches('(')
            .split(',')
            .next()
            .unwrap_or_default()
            .trim_end_matches(')')
            .to_owned();
    }
    let mut rest = text.as_str().trim();
    // `map[K]V` and `chan T` hold values of their element type.
    if let Some(map) = rest.strip_prefix("map[")
        && let Some(close) = map.find(']')
    {
        return type_name(&map[close + 1..]);
    }
    for prefix in ["chan ", "<-chan ", "chan<- "] {
        if let Some(element) = rest.strip_prefix(prefix) {
            return type_name(element);
        }
    }
    loop {
        let before = rest;
        for prefix in [
            "*",
            "&",
            "[]",
            "mut ",
            "dyn ",
            "impl ",
            "const ",
            "...",
            "?",
            "'static ",
            // C and C++ elaborated and qualified types.
            "struct ",
            "union ",
            "enum ",
            "class ",
            "volatile ",
            "unsigned ",
            "static ",
        ] {
            rest = rest.trim_start_matches(prefix).trim_start();
        }
        if rest.starts_with('\'') {
            rest = rest.split_once(' ').map_or("", |(_, tail)| tail);
        }
        if rest.starts_with('[')
            && let Some(close) = rest.find(']')
        {
            rest = &rest[close + 1..];
        }
        if rest == before {
            break;
        }
    }
    for wrapper in [
        "Box<", "Arc<", "Rc<", "RefCell<", "Mutex<", "RwLock<", "Pin<", "Cow<",
    ] {
        if let Some(inner) = rest.strip_prefix(wrapper) {
            let inner = inner.trim_start_matches("'_, ").trim_start_matches("'a, ");
            return type_name(inner);
        }
    }
    let end = rest
        .find(|ch: char| {
            matches!(
                ch,
                '<' | '[' | '(' | '{' | ' ' | ';' | '+' | '|' | ',' | '>'
            )
        })
        .unwrap_or(rest.len());
    rest[..end].trim_end_matches(['?', '!']).to_owned()
}

/// Type a call result binds: like [`type_name`], with `Result` and `Option` unwrapped.
pub fn result_type(text: &str) -> String {
    let mut text = facts::normalize_space(text);
    loop {
        let unwrapped = ["Result<", "Option<", "io::Result<", "anyhow::Result<"]
            .iter()
            .find_map(|wrapper| text.strip_prefix(wrapper).map(str::to_owned));
        match unwrapped {
            Some(inner) => text = inner,
            None => break,
        }
    }
    type_name(&text)
}

/// Definition text up to its body, on one line.
fn signature(node: Node, source: &[u8]) -> String {
    let start = node.start_byte();
    let mut walker = node.walk();
    // Julia `function f(x::Union{A, B}) where T`: braces are type parameters, the `signature`
    // child ends the header.
    if matches!(node.kind(), "function_definition" | "macro_definition")
        && let Some(header) = node
            .named_children(&mut walker)
            .find(|child| child.kind() == "signature")
    {
        let raw = String::from_utf8_lossy(&source[start..header.end_byte().min(source.len())]);
        return tidy_brackets(&facts::normalize_space(&raw));
    }
    let mut walker = node.walk();
    let body = node
        .child_by_field_name("body")
        .or_else(|| {
            // Grammars without a `body` field (Kotlin `class_body`, `function_body`).
            node.named_children(&mut walker)
                .find(|child| child.kind().ends_with("_body") && child.kind() != "type_body")
        })
        .map(|body| body.start_byte())
        .or_else(|| {
            node.child_by_field_name("type")
                .filter(|child| matches!(child.kind(), "struct_type" | "interface_type"))
                .and_then(|child| first_brace(child, source))
        });
    let end = body.unwrap_or_else(|| node.end_byte()).max(start);
    let raw = String::from_utf8_lossy(&source[start..end.min(source.len())]);
    let raw = match body {
        Some(_) => raw.as_ref(),
        None => raw.split(['{', '\n']).next().unwrap_or_default(),
    };
    let text = tidy_brackets(&facts::normalize_space(raw));
    text.trim_end_matches(|ch: char| ch == '{' || ch == ';' || ch.is_whitespace())
        .trim_end_matches(" =")
        .to_owned()
}

/// Removes the spaces and trailing commas that line breaks leave inside brackets.
fn tidy_brackets(text: &str) -> String {
    let mut out = text.to_owned();
    for (from, to) in [
        ("( ", "("),
        (" )", ")"),
        (",)", ")"),
        ("[ ", "["),
        (" ]", "]"),
        (",]", "]"),
        (" ,", ","),
    ] {
        while out.contains(from) {
            out = out.replace(from, to);
        }
    }
    out
}

fn first_brace(node: Node, source: &[u8]) -> Option<usize> {
    source[node.start_byte()..node.end_byte()]
        .iter()
        .position(|&byte| byte == b'{')
        .map(|offset| node.start_byte() + offset)
}

/// The node whose preceding siblings carry doc comments and decorations: a wrapper that starts
/// on the same line (Go `type_declaration` around `type_spec`) is used instead.
fn anchor(node: Node) -> Node {
    let mut anchor = node;
    while let Some(parent) = anchor.parent() {
        let wrapper = parent.start_position().row == node.start_position().row
            && parent.named_child_count() == 1
            && parent.parent().is_some();
        if !wrapper {
            break;
        }
        anchor = parent;
    }
    anchor
}

fn decorations(node: Node, source: &[u8]) -> String {
    let mut parts = Vec::new();
    let mut current = anchor(node).prev_named_sibling();
    while let Some(sibling) = current {
        let kind = sibling.kind();
        if kind.contains("attribute") || kind.contains("decorator") || kind.contains("annotation") {
            parts.push(sibling.utf8_text(source).unwrap_or_default());
        } else if !kind.contains("comment") {
            break;
        }
        current = sibling.prev_named_sibling();
    }
    parts.reverse();
    parts.join(" ")
}

/// Leading comments of the file before its first other node: module or package documentation.
fn file_doc(root: Node, source: &[u8]) -> String {
    if let Some(docstring) = docstring(root, source) {
        return facts::clip(&facts::normalize_space(&docstring), 400);
    }
    let mut lines: Vec<String> = Vec::new();
    for index in 0..root.named_child_count() {
        let Some(child) = root.named_child(index as u32) else {
            break;
        };
        if !child.kind().contains("comment") {
            break;
        }
        let text = child.utf8_text(source).unwrap_or_default();
        if text.starts_with("#!") {
            continue;
        }
        lines.push(strip_comment(
            text.trim_start_matches("//!").trim_start_matches("/*!"),
        ));
    }
    facts::clip(&facts::normalize_space(&lines.join(" ")), 400)
}

/// First sentence of the comment block directly above the definition.
fn doc_comment(node: Node, source: &[u8]) -> String {
    let anchor = anchor(node);
    let mut expected_row = anchor.start_position().row;
    let mut lines: Vec<String> = Vec::new();
    let mut current = anchor.prev_named_sibling();
    while let Some(sibling) = current {
        let kind = sibling.kind();
        let end_row = sibling.end_position().row;
        let decoration = kind.contains("attribute") || kind.contains("decorator");
        if !(kind.contains("comment") || decoration) || end_row + 1 < expected_row {
            break;
        }
        if kind.contains("comment") {
            let text = sibling.utf8_text(source).unwrap_or_default();
            if text.starts_with("//!") || text.starts_with("/*!") {
                break;
            }
            lines.push(strip_comment(text));
        }
        expected_row = sibling.start_position().row;
        current = sibling.prev_named_sibling();
    }
    lines.reverse();
    if lines.is_empty()
        && let Some(docstring) = node
            .child_by_field_name("body")
            .and_then(|body| docstring(body, source))
    {
        return facts::first_sentence(&docstring);
    }
    facts::first_sentence(&lines.join(" "))
}

/// A string literal as the first statement of a body or module (Python docstrings).
fn docstring(body: Node, source: &[u8]) -> Option<String> {
    let first = body.named_child(0)?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let string = first.named_child(0)?;
    if string.kind() != "string" {
        return None;
    }
    let text = string.utf8_text(source).ok()?;
    Some(text.trim_matches(|ch| ch == '"' || ch == '\'').to_owned())
}

fn strip_comment(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.trim()
                .trim_start_matches("///")
                .trim_start_matches("//")
                .trim_start_matches("/**")
                .trim_start_matches("/*")
                .trim_end_matches("*/")
                .trim_start_matches('*')
                .trim_start_matches('#')
                .trim_start_matches("--")
                .trim()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_drop_decoration() {
        let cases = [
            ("*Store", "Store"),
            ("&mut Vec<T>", "Vec"),
            ("[]byte", "byte"),
            ("pkg.Client", "pkg.Client"),
            ("&'a dyn Reader + Send", "Reader"),
            ("(*Row, error)", "Row"),
            ("Option<String>", "Option"),
            ("crate::store::Index", "crate::store::Index"),
            ("[4]int", "int"),
            ("impl Iterator<Item = u8>", "Iterator"),
            ("Arc<Mutex<Store>>", "Store"),
            ("Box<dyn Reader + Send>", "Reader"),
            ("map[string]*atsCorpus", "atsCorpus"),
            ("chan Event", "Event"),
        ];
        for (input, want) in cases {
            assert_eq!(type_name(input), want, "{input}");
        }
    }

    #[test]
    fn brackets_lose_line_break_spacing() {
        assert_eq!(
            tidy_brackets("fn f( a: u8, b: u8, ) -> u8"),
            "fn f(a: u8, b: u8) -> u8"
        );
    }

    #[test]
    fn result_types_unwrap_result_and_option() {
        assert_eq!(result_type("Result<Self, Error>"), "Self");
        assert_eq!(result_type("io::Result<Option<Box<Store>>>"), "Store");
        assert_eq!(result_type("(*Store, error)"), "Store");
    }

    #[test]
    fn comments_lose_their_markers() {
        assert_eq!(strip_comment("/// Opens the store."), "Opens the store.");
        assert_eq!(strip_comment("/**\n * Multi\n * line\n */"), " Multi line ");
        assert_eq!(unquote("\"os\""), "os");
        assert_eq!(unquote("`raw`"), "raw");
    }
}
