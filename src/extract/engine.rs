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
    self, Binding, EnvRead, Facts, FieldName, Fields, Import, JuliaCallArgument, Ref, RefKind,
    SourceIdentity, SourceIdentityKind, SymKind, Symbol,
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
    source_identity: Option<String>,
    name_node: usize,
    owner: Option<String>,
    /// Last clause of a multi-clause definition (Elixir `def f(nil)` then `def f(x)`).
    last_clause: Option<Node<'tree>>,
}

#[derive(Default)]
struct RawRef {
    kind: Option<RefKind>,
    name: String,
    source_identity: Option<String>,
    qualifier: Option<String>,
    position: usize,
    line: u32,
    arity: u8,
    julia_arguments: Vec<(u8, String)>,
}

struct AssembledRef {
    reference: Ref,
    identity: Option<String>,
    julia_builtin: bool,
    arguments: Vec<(u8, String)>,
    position: usize,
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
    name: Option<FieldName>,
    type_name: String,
    position: usize,
}

/// Byte range of each extracted symbol in the source, aligned with `Facts::symbols`.
pub type Spans = Vec<(usize, usize)>;

/// Transient extraction details; complete signature identities are only captured for diff queries.
#[derive(Default)]
pub struct Parsed {
    pub facts: Facts,
    pub spans: Spans,
    pub identities: Vec<Option<String>>,
    pub incomplete: bool,
}

/// Extracts facts from `source` with the given grammar, compiled query and hooks, plus the
/// syntax-tree byte range of every symbol (for body-word ownership; not stored).
pub fn extract(
    language: &Language,
    query: &Query,
    hooks: &Hooks,
    path: &str,
    source: &[u8],
    capture_identities: bool,
) -> Parsed {
    PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        if parser.set_language(language).is_err() {
            return Parsed {
                incomplete: capture_identities,
                ..Parsed::default()
            };
        }
        // A NUL-only suffix cannot contain a declaration. Ignore it for strict diff parsing so
        // an otherwise complete source prefix remains usable, while other parse errors fail.
        let parse_source = if capture_identities && source.last() == Some(&0) {
            let end = source
                .iter()
                .rposition(|&byte| byte != 0)
                .map_or(0, |end| end + 1);
            &source[..end]
        } else {
            source
        };
        match parser.parse(parse_source, None) {
            Some(tree) => collect(
                query,
                hooks,
                path,
                source,
                tree.root_node(),
                capture_identities,
            ),
            None => Parsed {
                incomplete: capture_identities,
                ..Parsed::default()
            },
        }
    })
}

fn collect(
    query: &Query,
    hooks: &Hooks,
    path: &str,
    source: &[u8],
    root: Node,
    capture_identities: bool,
) -> Parsed {
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
    // Local variables and parameters (`@local`): a bare value reference of that name in the same
    // scope reads the local, not a method (Ruby `x = 1; x`).
    let mut locals: Vec<(usize, String)> = Vec::new();
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
        for capture in captures
            .iter()
            .filter(|capture| names[capture.index as usize] == "local")
        {
            locals.push((capture.node.start_byte(), text(capture.node).to_owned()));
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
            entry.source_identity =
                source_definition_identity(path, primary_node, &entry.name, source);
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
            if path.ends_with(".rs") && kind == "path" {
                let Some(module) = rust_path_module(primary_node, source) else {
                    continue;
                };
                let Some(path_value) = rust_string_literal(text(name)) else {
                    continue;
                };
                let Some((module_path, inline_path)) = rust_module_path_context(module, source)
                else {
                    continue;
                };
                refs.insert(
                    name.id(),
                    RawRef {
                        kind: Some(RefKind::Path),
                        name: path_value,
                        qualifier: Some(facts::rust_module_path_ref_qualifier(
                            &module_path,
                            &inline_path,
                        )),
                        position: module.start_byte(),
                        line,
                        arity: facts::NO_ARITY,
                        ..RawRef::default()
                    },
                );
                continue;
            }
            // Rust macro token trees: `.method(...)` after anything but a plain receiver name
            // (`a.b().len()`) has an unknown receiver; query anchors skip the anonymous `.`.
            if primary_node.kind() == "token_tree"
                && get("qualifier").is_none()
                && name
                    .prev_sibling()
                    .is_some_and(|previous| previous.kind() == ".")
            {
                continue;
            }
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
                Some(RefKind::Call) if path.ends_with(".jl") => julia_arity(primary_node),
                Some(RefKind::Call) => arity(primary_node),
                // Base lists: the type argument count tells `IRequest` from `IRequest<T>`.
                Some(RefKind::Inherit | RefKind::Implement) => type_argument_count(name),
                _ => facts::NO_ARITY,
            };
            let (reference_name, source_identity) = source_reference_identity(
                path,
                primary_node,
                kind,
                text(name),
                source,
                reference.arity,
            );
            reference.name = if path.ends_with(".rs")
                && kind == Some(RefKind::Implement)
                && primary_node.kind() == "trait_bounds"
            {
                type_name(&reference_name)
            } else {
                reference_name
            };
            reference.source_identity = source_identity;
            reference.position = if path.ends_with(".rs") && kind == Some(RefKind::Implement) {
                get("generic.scope").map_or(name.start_byte(), |scope| scope.start_byte())
            } else {
                name.start_byte()
            };
            reference.line = row(name);
            if let Some(qualifier) = get("qualifier") {
                reference.qualifier =
                    Some(facts::clip(&facts::normalize_space(text(qualifier)), 80));
            }
            if path.ends_with(".jl")
                && kind == Some(RefKind::Call)
                && reference.qualifier.is_none()
                && let Some(arguments) = get("call.arguments")
            {
                reference.julia_arguments = julia_arguments(arguments, source);
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
                        let type_text = text(node);
                        entry.type_name = Some(if path.ends_with(".jl") {
                            facts::normalize_space(type_text)
                        } else {
                            type_name(type_text)
                        });
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
                    if primary_node.kind() == "property_signature"
                        && primary_node
                            .parent()
                            .and_then(|body| body.parent())
                            .is_some_and(|interface| {
                                interface.kind() == "interface_declaration" && interface.has_error()
                            })
                    {
                        continue;
                    }
                    let entry = fields.entry(key).or_default();
                    entry.position = primary_node.start_byte();
                    if let Some(node) = get("field.name") {
                        if let Some(literal) = get("field.literal") {
                            #[cfg(feature = "lang-typescript")]
                            {
                                entry.name =
                                    super::lang::typescript::simple_property_key(literal, source);
                            }
                            #[cfg(not(feature = "lang-typescript"))]
                            let _ = literal;
                        } else {
                            entry.name = Some(FieldName::Text(text(node).to_owned()));
                        }
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
    drop(matches);
    let incomplete = capture_identities && (root.has_error() || cursor.did_exceed_match_limit());
    envs.retain(|_, (name, _, _)| !assigned.contains(name));
    let doc = file_doc(root, source);
    let mut parsed = assemble(Assembly {
        hooks,
        path,
        source,
        capture_identities,
        package,
        defs,
        impls,
        refs,
        imports,
        bindings,
        returns,
        fields,
        envs,
        locals,
    });
    #[cfg(feature = "lang-go")]
    if path.ends_with(".go") {
        super::go_embed::collect(root, source, &mut parsed);
    }
    parsed.facts.doc = doc;
    parsed.facts.entry |= entry;
    parsed.incomplete = incomplete;
    if incomplete {
        parsed.identities.fill(None);
    }
    parsed
}

struct Assembly<'a, 'tree> {
    hooks: &'a Hooks,
    path: &'a str,
    source: &'a [u8],
    capture_identities: bool,
    package: Option<String>,
    defs: HashMap<usize, RawDef<'tree>>,
    impls: Vec<(Node<'tree>, String, Option<String>)>,
    refs: HashMap<usize, RawRef>,
    imports: HashMap<usize, RawImport>,
    bindings: HashMap<usize, RawBinding>,
    returns: Vec<(usize, String)>,
    fields: HashMap<usize, RawField>,
    envs: HashMap<usize, (String, usize, u32)>,
    locals: Vec<(usize, String)>,
}

fn assemble(assembly: Assembly) -> Parsed {
    let Assembly {
        hooks,
        path,
        source,
        capture_identities,
        package,
        defs,
        impls,
        refs,
        imports,
        bindings,
        returns,
        fields,
        envs,
        locals,
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
    #[cfg(feature = "lang-typescript")]
    let repeated_interfaces = {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        if path
            .rsplit_once('.')
            .is_some_and(|(_, suffix)| matches!(suffix, "ts" | "tsx" | "mts" | "cts" | "ets"))
        {
            for def in &defs {
                if def.kind == Some(SymKind::Iface)
                    && def
                        .node
                        .is_some_and(|node| node.kind() == "interface_declaration")
                {
                    *counts.entry(&def.name).or_default() += 1;
                }
            }
        }
        counts
    };
    let ranges: Vec<(usize, usize)> = defs
        .iter()
        .filter_map(|def| {
            def.node.map(|node| {
                // Python decorators belong to the decorated definition's scope.
                let start = node
                    .parent()
                    .filter(|parent| parent.kind() == "decorated_definition")
                    .map_or(node.start_byte(), |parent| parent.start_byte());
                (start, def.last_clause.unwrap_or(node).end_byte())
            })
        })
        .collect();
    let context = FileContext {
        path,
        package: package.as_deref(),
    };
    let mut symbols: Vec<Symbol> = Vec::with_capacity(defs.len());
    let mut source_identities = Vec::new();
    let mut identities = Vec::new();
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
        let signature = facts::clip(
            &compact_parameters(
                &format!("{prefix}{}", signature(node, source)),
                &def.name,
                160,
            ),
            160,
        );
        if capture_identities {
            identities.push(
                def.last_clause
                    .is_none()
                    .then(|| signature_identity(node, source))
                    .flatten(),
            );
        }
        symbols.push(Symbol {
            kind: Some(kind),
            name: def.name.clone(),
            parent: parent.and_then(|p| u32::try_from(p).ok()),
            owner,
            line: row(node),
            end_line: def.last_clause.unwrap_or(node).end_position().row as u32 + 1,
            signature,
            doc: doc_comment(node, source),
            terms: String::new(),
            exported,
            flags,
        });
        if let (Some(identity), Ok(index)) = (def.source_identity.as_ref(), u32::try_from(index)) {
            source_identities.push(SourceIdentity {
                kind: SourceIdentityKind::Symbol,
                index,
                identity: identity.clone(),
            });
        }
        #[cfg(feature = "lang-typescript")]
        if kind == SymKind::Iface
            && parent.is_none()
            && repeated_interfaces
                .get(def.name.as_str())
                .is_some_and(|&count| count > 1)
            && let Some(identity) = signature_identity(node, source)
            && let Ok(index) = u32::try_from(index)
        {
            source_identities.push(SourceIdentity {
                kind: SourceIdentityKind::InterfaceHeader,
                index,
                identity,
            });
            if let Some(identity) = super::lang::typescript::interface_member_identity(node, source)
            {
                source_identities.push(SourceIdentity {
                    kind: SourceIdentityKind::InterfaceMembers,
                    index,
                    identity,
                });
            }
        }
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
    let elixir = path.ends_with(".ex") || path.ends_with(".exs");
    let local_functions: std::collections::HashSet<&str> = symbols
        .iter()
        .filter(|symbol| {
            // Functions, and functions of a module (Julia and OCaml module blocks, Elixir
            // `defmodule` with `defp derive`); methods of a class never shadow a builtin call
            // (`len(x)` next to `def len(self)`).
            symbol.kind == Some(SymKind::Fn)
                || (symbol.kind == Some(SymKind::Method)
                    && symbol.parent.is_some_and(|parent| {
                        let parent_kind = symbols[parent as usize].kind;
                        parent_kind == Some(SymKind::Mod)
                            || (elixir && parent_kind == Some(SymKind::Class))
                    }))
        })
        .map(|symbol| symbol.name.as_str())
        .collect();
    let mut facts_refs: Vec<AssembledRef> = refs
        .into_iter()
        .filter(|(name_node, _)| !def_name_nodes.contains(name_node))
        .filter_map(|(_, mut raw)| {
            let kind = raw.kind?;
            let builtin = raw.qualifier.is_none() && (hooks.builtin)(&raw.name);
            let local_function = local_functions.contains(raw.name.as_str());
            let julia_builtin =
                path.ends_with(".jl") && builtin && !local_function && kind == RefKind::Call;
            if builtin && !local_function && !julia_builtin {
                return None;
            }
            let arguments = if julia_builtin {
                std::mem::take(&mut raw.julia_arguments)
            } else {
                Vec::new()
            };
            Some(AssembledRef {
                reference: Ref {
                    kind,
                    name: raw.name,
                    qualifier: raw.qualifier,
                    scope: scope_of(raw.position),
                    line: raw.line,
                    arity: raw.arity,
                },
                identity: raw.source_identity,
                julia_builtin,
                arguments,
                position: raw.position,
            })
        })
        .collect();
    if !locals.is_empty() {
        let local_names: std::collections::HashSet<(Option<u32>, &str)> = locals
            .iter()
            .map(|(position, name)| (scope_of(*position), name.as_str()))
            .collect();
        // A local shadows every unqualified use of its name (`count := rc.Len; count()`); in
        // Ruby only bare names, since `foo(1)` calls the method even when a local `foo` exists.
        let ruby = path.ends_with(".rb");
        facts_refs.retain(|entry| {
            let reference = &entry.reference;
            let shadowable = match reference.kind {
                RefKind::Value => true,
                RefKind::Call => !ruby || reference.arity == facts::NO_ARITY,
                _ => false,
            };
            !shadowable
                || reference.qualifier.is_some()
                || !local_names.contains(&(reference.scope, reference.name.as_str()))
        });
    }
    for (impl_node, target, trait_name) in &impls {
        if let Some(trait_name) = trait_name {
            facts_refs.push(AssembledRef {
                reference: Ref {
                    kind: RefKind::Implement,
                    name: trait_name.clone(),
                    qualifier: Some(target.clone()),
                    scope: None,
                    line: row(*impl_node),
                    arity: facts::NO_ARITY,
                },
                identity: None,
                julia_builtin: false,
                arguments: Vec::new(),
                position: impl_node.start_byte(),
            });
        }
    }
    facts_refs.sort_by(|a, b| {
        (
            a.reference.line,
            &a.reference.name,
            a.reference.kind,
            &a.reference.qualifier,
            a.reference.scope,
            a.position,
        )
            .cmp(&(
                b.reference.line,
                &b.reference.name,
                b.reference.kind,
                &b.reference.qualifier,
                b.reference.scope,
                b.position,
            ))
    });
    // A value reference repeated in its scope adds no edge: binding shadowing only grows with
    // the line and edges keep their first line, so the first occurrence decides.
    let mut seen_values = HashSet::new();
    facts_refs.retain(|entry| {
        let reference = &entry.reference;
        reference.kind != RefKind::Value
            || seen_values.insert((
                reference.name.clone(),
                reference.qualifier.clone(),
                reference.scope,
                entry.identity.clone(),
            ))
    });
    let mut refs = Vec::with_capacity(facts_refs.len());
    let mut julia_builtin_calls = Vec::new();
    let mut julia_call_arguments = Vec::new();
    for (index, entry) in facts_refs.into_iter().enumerate() {
        if let Ok(reference_index) = u32::try_from(index) {
            if entry.julia_builtin {
                julia_builtin_calls.push(reference_index);
            }
            for (position, binding) in entry.arguments {
                julia_call_arguments.push(JuliaCallArgument {
                    reference: reference_index,
                    position,
                    binding,
                });
            }
            if let Some(identity) = entry.identity {
                source_identities.push(SourceIdentity {
                    kind: SourceIdentityKind::Reference,
                    index: reference_index,
                    identity,
                });
            }
        }
        refs.push(entry.reference);
    }
    #[cfg(feature = "lang-typescript")]
    if repeated_interfaces.values().any(|&count| count > 1) {
        let nodes: Vec<_> = defs.iter().map(|def| def.node).collect();
        let groups =
            super::lang::typescript::interface_groups(&symbols, &source_identities, &nodes, source);
        source_identities.extend(groups);
    }
    source_identities.sort_unstable_by_key(|identity| (identity.kind, identity.index));
    let source_identities =
        (!source_identities.is_empty()).then(|| source_identities.into_boxed_slice());
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
    let spans = if ranges.len() == symbols.len() {
        ranges
    } else {
        Spans::new()
    };
    Parsed {
        facts: Facts {
            package,
            symbols,
            imports: facts_imports,
            refs,
            julia_builtin_calls,
            julia_call_arguments,
            source_identities,
            bindings: facts_bindings,
            envs: facts_envs,
            fields: grouped,
            ..Facts::default()
        },
        spans,
        identities,
        incomplete: false,
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
        "path" => RefKind::Path,
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

/// Type arguments written after a type name (`Base<A, B>` has two, `Base` none): the type
/// argument list is a sibling of the name in a generic type node (C# `generic_name`, Java
/// `generic_type`, Kotlin `user_type`).
fn type_argument_count(name: Node) -> u8 {
    let Some(parent) = name.parent() else {
        return 0;
    };
    let mut walker = parent.walk();
    let count = parent
        .named_children(&mut walker)
        .find(|child| child.kind().starts_with("type_argument"))
        .map_or(0, |list| {
            let mut inner = list.walk();
            list.named_children(&mut inner)
                .filter(|argument| !argument.kind().contains("comment"))
                .count()
        });
    u8::try_from(count).unwrap_or(facts::NO_ARITY - 1)
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

fn julia_arity(call: Node) -> u8 {
    let mut walker = call.walk();
    let Some(arguments) = call
        .named_children(&mut walker)
        .find(|child| child.kind() == "argument_list")
    else {
        return facts::NO_ARITY;
    };
    let mut inner = arguments.walk();
    let count = arguments
        .named_children(&mut inner)
        .filter(|argument| {
            argument.kind() != "named_argument" && !argument.kind().contains("comment")
        })
        .count();
    u8::try_from(count).unwrap_or(facts::NO_ARITY - 1)
}

fn julia_arguments(arguments: Node, source: &[u8]) -> Vec<(u8, String)> {
    let mut result = Vec::new();
    let mut position = 0usize;
    let mut walker = arguments.walk();
    for argument in arguments.named_children(&mut walker) {
        if argument.kind() == "named_argument" || argument.kind().contains("comment") {
            continue;
        }
        if position < usize::from(facts::NO_ARITY)
            && argument.kind() == "identifier"
            && let Ok(position) = u8::try_from(position)
            && let Ok(binding) = argument.utf8_text(source)
        {
            result.push((position, binding.to_owned()));
        }
        position += 1;
    }
    result
}

fn source_definition_identity(path: &str, node: Node, name: &str, source: &[u8]) -> Option<String> {
    #[cfg(feature = "lang-python")]
    if path.ends_with(".py") || path.ends_with(".pyi") {
        return super::lang::python::structural_identity(node, source);
    }
    if path.ends_with(".swift")
        && matches!(
            node.kind(),
            "function_declaration" | "protocol_function_declaration" | "init_declaration"
        )
    {
        return swift_definition_identity(node, name, source);
    }
    if matches!(node.kind(), "method_declaration" | "method_definition") {
        return objective_c_definition_identity(node, source);
    }
    None
}

fn swift_definition_identity(node: Node, name: &str, source: &[u8]) -> Option<String> {
    let mut labels = Vec::new();
    let mut defaulted = Vec::new();
    for index in 0..node.named_child_count() {
        let child_index = u32::try_from(index).ok()?;
        let child = node.named_child(child_index)?;
        if child.kind() == "parameter" {
            let label = child
                .child_by_field_name("external_name")
                .or_else(|| child.child_by_field_name("name"))?
                .utf8_text(source)
                .ok()?
                .to_owned();
            labels.push(label);
            defaulted.push(false);
        } else if node.field_name_for_named_child(child_index) == Some("default_value") {
            *defaulted.last_mut()? = true;
        }
    }
    (!labels.is_empty())
        .then(|| swift_labeled_identity(name, &labels, Some(&defaulted)))
        .flatten()
}

fn swift_call_identity(call: Node, name: &str, source: &[u8], arity: u8) -> Option<String> {
    let mut cursor = call.walk();
    let suffix = call
        .named_children(&mut cursor)
        .find(|child| child.kind() == "call_suffix")?;
    let mut suffix_cursor = suffix.walk();
    let arguments = suffix
        .named_children(&mut suffix_cursor)
        .find(|child| child.kind() == "value_arguments")?;
    let mut arguments_cursor = arguments.walk();
    let labels: Option<Vec<String>> = arguments
        .named_children(&mut arguments_cursor)
        .filter(|argument| argument.kind() == "value_argument")
        .map(|argument| {
            argument
                .child_by_field_name("name")
                .map_or(Some("_".to_owned()), |label| {
                    label.utf8_text(source).ok().map(str::to_owned)
                })
        })
        .collect();
    let labels = labels?;
    if labels.is_empty() || u8::try_from(labels.len()).ok() != Some(arity) {
        return None;
    }
    swift_labeled_identity(name, &labels, None)
}

fn swift_labeled_identity(
    name: &str,
    labels: &[String],
    defaulted: Option<&[bool]>,
) -> Option<String> {
    if labels.is_empty()
        || labels.iter().any(String::is_empty)
        || defaulted.is_some_and(|parameters| parameters.len() != labels.len())
    {
        return None;
    }
    let optional_markers = defaulted.map_or(0, |parameters| {
        parameters
            .iter()
            .filter(|&&is_defaulted| is_defaulted)
            .count()
    });
    let capacity = name.len()
        + labels.iter().map(|label| label.len() + 1).sum::<usize>()
        + optional_markers
        + 2;
    let mut identity = String::with_capacity(capacity);
    identity.push_str(name);
    identity.push('(');
    for (index, label) in labels.iter().enumerate() {
        identity.push_str(label);
        if defaulted.is_some_and(|parameters| parameters[index]) {
            identity.push('?');
        }
        identity.push(':');
    }
    identity.push(')');
    Some(identity)
}

fn objective_c_definition_identity(node: Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    if !children.iter().any(|child| child.kind() == "method_type") {
        return None;
    }
    let mut identity = String::new();
    for (index, child) in children.iter().enumerate() {
        if child.kind() != "identifier" {
            continue;
        }
        identity.push_str(child.utf8_text(source).ok()?);
        if children
            .get(index + 1)
            .is_some_and(|next| next.kind() == "method_parameter")
        {
            identity.push(':');
        }
    }
    (!identity.is_empty()).then_some(identity)
}

fn objective_c_message_identity(node: Node, source: &[u8], arity: u8) -> Option<String> {
    if arity == facts::NO_ARITY {
        return None;
    }
    let mut cursor = node.walk();
    let mut parts = Vec::new();
    if cursor.goto_first_child() {
        loop {
            if cursor.field_name() == Some("method") && cursor.node().kind() == "identifier" {
                parts.push(cursor.node().utf8_text(source).ok()?);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    let expected_parts = if arity == 0 { 1 } else { usize::from(arity) };
    if parts.len() != expected_parts {
        return None;
    }
    if arity == 0 {
        return Some(parts[0].to_owned());
    }
    let mut identity = String::new();
    for part in parts {
        identity.push_str(part);
        identity.push(':');
    }
    Some(identity)
}

fn source_reference_identity(
    path: &str,
    node: Node,
    kind: Option<RefKind>,
    name: &str,
    source: &[u8],
    arity: u8,
) -> (String, Option<String>) {
    if path.ends_with(".swift")
        && kind == Some(RefKind::Call)
        && let Some(identity) = swift_call_identity(node, name, source, arity)
    {
        return (name.to_owned(), Some(identity));
    }
    if node.kind() == "message_expression"
        && kind == Some(RefKind::Call)
        && let Some(identity) = objective_c_message_identity(node, source, arity)
    {
        return (name.to_owned(), Some(identity));
    }
    if node.kind() == "selector_expression"
        && kind == Some(RefKind::Value)
        && let Ok(expression) = node.utf8_text(source)
        && let Some(selector) = expression
            .trim()
            .strip_prefix("@selector(")
            .and_then(|selector| selector.strip_suffix(')'))
            .map(str::trim)
    {
        let base = selector.split(':').next().unwrap_or(selector).trim();
        if !base.is_empty() {
            return (base.to_owned(), Some(selector.to_owned()));
        }
    }
    (name.to_owned(), None)
}

fn row(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

fn rust_path_module<'tree>(attribute_item: Node<'tree>, source: &[u8]) -> Option<Node<'tree>> {
    let mut sibling = attribute_item.next_named_sibling()?;
    while sibling.kind() == "attribute_item" || sibling.kind().contains("comment") {
        sibling = sibling.next_named_sibling()?;
    }
    if sibling.kind() != "mod_item" || sibling.child_by_field_name("body").is_some() {
        return None;
    }
    let (path_attribute, _) = rust_module_path_attribute(sibling, source).ok()??;
    (path_attribute.id() == attribute_item.id()).then_some(sibling)
}

fn rust_module_path_attribute<'tree>(
    module: Node<'tree>,
    source: &[u8],
) -> Result<Option<(Node<'tree>, String)>, ()> {
    let mut current = module.prev_named_sibling();
    let mut found = None;
    while let Some(sibling) = current {
        if sibling.kind().contains("comment") {
            current = sibling.prev_named_sibling();
            continue;
        }
        if sibling.kind() != "attribute_item" {
            break;
        }
        if let Some(value) = rust_path_attribute_value(sibling, source)? {
            if found.is_some() {
                return Err(());
            }
            found = Some(value);
        }
        current = sibling.prev_named_sibling();
    }
    Ok(found)
}

fn rust_path_attribute_value<'tree>(
    attribute_item: Node<'tree>,
    source: &[u8],
) -> Result<Option<(Node<'tree>, String)>, ()> {
    let Some(attribute) = attribute_item.named_child(0) else {
        return Err(());
    };
    if attribute.kind() != "attribute" {
        return Err(());
    }
    let Some(name) = attribute.named_child(0) else {
        return Err(());
    };
    if name.kind() != "identifier" || name.utf8_text(source).map_err(|_| ())? != "path" {
        return Ok(None);
    }
    let Some(value) = attribute.child_by_field_name("value") else {
        return Err(());
    };
    let decoded = rust_string_literal(value.utf8_text(source).map_err(|_| ())?).ok_or(())?;
    Ok(Some((attribute_item, decoded)))
}

fn rust_module_path_context(module: Node, source: &[u8]) -> Option<(String, String)> {
    let mut inline_modules = Vec::new();
    let mut ancestor = module.parent();
    while let Some(node) = ancestor {
        if node.kind() == "mod_item" && node.child_by_field_name("body").is_some() {
            inline_modules.push(node);
        }
        ancestor = node.parent();
    }
    inline_modules.reverse();

    let mut module_path = String::new();
    let mut inline_path = String::new();
    for inline in inline_modules {
        let name = inline.child_by_field_name("name")?.utf8_text(source).ok()?;
        append_module_path(&mut module_path, name);
        let component = match rust_module_path_attribute(inline, source).ok()? {
            Some((_, path)) => path,
            None => name.strip_prefix("r#").unwrap_or(name).to_owned(),
        };
        append_path_expression(&mut inline_path, &component);
    }
    let name = module.child_by_field_name("name")?.utf8_text(source).ok()?;
    append_module_path(&mut module_path, name);
    Some((module_path, inline_path))
}

fn append_module_path(path: &mut String, name: &str) {
    if !path.is_empty() {
        path.push_str("::");
    }
    path.push_str(name);
}

fn append_path_expression(path: &mut String, component: &str) {
    if component.starts_with('/') {
        *path = component.to_owned();
        return;
    }
    if component.is_empty() {
        return;
    }
    if !path.is_empty() && !path.ends_with('/') {
        path.push('/');
    }
    path.push_str(component);
}

fn rust_string_literal(text: &str) -> Option<String> {
    if let Some(raw) = text.strip_prefix('r') {
        let hashes = raw.bytes().take_while(|byte| *byte == b'#').count();
        if raw.as_bytes().get(hashes) != Some(&b'"') {
            return None;
        }
        let prefix_len = 1usize.checked_add(hashes)?.checked_add(1)?;
        let suffix_len = hashes.checked_add(1)?;
        let content_end = text.len().checked_sub(suffix_len)?;
        let closing = text.get(content_end..)?;
        if closing != format!("\"{}", "#".repeat(hashes)) {
            return None;
        }
        return Some(text.get(prefix_len..content_end)?.to_owned());
    }

    let content = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut chars = content.chars().peekable();
    let mut decoded = String::with_capacity(content.len());
    while let Some(character) = chars.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match chars.next()? {
            '\'' => decoded.push('\''),
            '"' => decoded.push('"'),
            '\\' => decoded.push('\\'),
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '0' => decoded.push('\0'),
            'x' => {
                let high = chars.next()?.to_digit(16)?;
                let low = chars.next()?.to_digit(16)?;
                let value = high.checked_mul(16)?.checked_add(low)?;
                if value > 0x7f {
                    return None;
                }
                decoded.push(char::from_u32(value)?);
            }
            'u' => {
                if chars.next()? != '{' {
                    return None;
                }
                let mut digits = String::new();
                loop {
                    match chars.next()? {
                        '}' if !digits.is_empty() => break,
                        '_' => {}
                        digit if digit.is_ascii_hexdigit() => digits.push(digit),
                        _ => return None,
                    }
                }
                let value = u32::from_str_radix(&digits, 16).ok()?;
                decoded.push(char::from_u32(value)?);
            }
            '\n' => skip_rust_string_continuation(&mut chars),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                skip_rust_string_continuation(&mut chars);
            }
            _ => return None,
        }
    }
    Some(decoded)
}

fn skip_rust_string_continuation(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars
        .peek()
        .is_some_and(|character| character.is_whitespace())
    {
        chars.next();
    }
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
    let body = body_start(node, source);
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

/// The parser's body boundary, or the opening brace of a Go structural type.
fn body_start(node: Node, source: &[u8]) -> Option<usize> {
    let mut walker = node.walk();
    node.child_by_field_name("body")
        .or_else(|| {
            node.named_children(&mut walker)
                .find(|child| child.kind().ends_with("_body") && child.kind() != "type_body")
        })
        .map(|body| body.start_byte())
        .or_else(|| {
            node.child_by_field_name("type")
                .filter(|child| matches!(child.kind(), "struct_type" | "interface_type"))
                .and_then(|child| first_brace(child, source))
        })
}

/// Complete header tokens, ignoring formatting and comments but preserving literal contents.
/// Unknown boundaries and malformed declarations stay unknown rather than compare as equal.
pub(in crate::extract) fn signature_identity(node: Node, source: &[u8]) -> Option<String> {
    signature_identity_with_exclusion(node, source, None)
}

/// A separately encoded property key is excluded without altering the remaining source tokens.
#[cfg(feature = "lang-typescript")]
pub(in crate::extract) fn signature_identity_without_node(
    node: Node,
    source: &[u8],
    excluded: Node,
) -> Option<String> {
    signature_identity_with_exclusion(node, source, Some(excluded.id()))
}

fn signature_identity_with_exclusion(
    node: Node,
    source: &[u8],
    excluded: Option<usize>,
) -> Option<String> {
    if node.has_error() || node.is_missing() {
        return None;
    }
    let mut walker = node.walk();
    let header = if matches!(node.kind(), "function_definition" | "macro_definition") {
        node.named_children(&mut walker)
            .find(|child| child.kind() == "signature")
            .map(|child| child.end_byte())
    } else {
        None
    };
    let end = header.or_else(|| body_start(node, source)).or_else(|| {
        (node.kind().ends_with("signature") || node.kind().ends_with("signature_item"))
            .then_some(node.end_byte())
    })?;
    let mut identity = node.kind().to_owned();
    let mut cursor = node.walk();
    loop {
        let current = cursor.node();
        let kind = current.kind();
        let literal = kind.contains("string")
            || kind.contains("char_literal")
            || kind.contains("rune_literal")
            || kind.contains("regex");
        let interpolated = if literal {
            let mut children = current.walk();
            current.named_children(&mut children).any(|child| {
                child.kind().contains("interpolation") || child.kind().contains("substitution")
            })
        } else {
            false
        };
        let atomic = current.child_count() == 0 || (literal && !interpolated);
        let within = current.start_byte() < end;
        let comment = kind.contains("comment");
        let skip = excluded.is_some_and(|id| id == current.id());
        if within && !comment && !skip && atomic && current.end_byte() <= end {
            let fragment =
                std::str::from_utf8(source.get(current.start_byte()..current.end_byte())?).ok()?;
            identity.push(':');
            identity.push_str(&fragment.len().to_string());
            identity.push(':');
            identity.push_str(fragment);
        }
        if within && !comment && !skip && !atomic && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return Some(identity);
            }
        }
    }
}

/// A signature over `max` characters keeps its parameter list whole: each parameter shrinks to
/// its label and name (`for _`, `columns = …`), so the parameter count and the defaults survive
/// clipping. Shorter signatures stay as written.
fn compact_parameters(signature: &str, name: &str, max: usize) -> String {
    if signature.chars().count() <= max {
        return signature.to_owned();
    }
    // The declared name, not a type that contains it: a whole word followed by `(` or `<`.
    let bytes = signature.as_bytes();
    let Some(name_end) = signature
        .match_indices(name)
        .map(|(start, _)| (start, start + name.len()))
        .find(|&(start, end)| {
            (start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_'))
                && matches!(bytes.get(end), Some(b'(' | b'<'))
        })
        .map(|(_, end)| end)
    else {
        return signature.to_owned();
    };
    let rest = &signature[name_end..];
    // Generic parameters (`<T: P>`) stay; the list starts at the first `(` after them.
    let Some(open) = rest.find('(').map(|offset| name_end + offset) else {
        return signature.to_owned();
    };
    let mut depth = 0i32;
    let mut close = None;
    let mut cuts = vec![open + 1];
    let mut quote: Option<char> = None;
    for (offset, ch) in signature[open + 1..].char_indices() {
        let index = open + 1 + offset;
        // Default values may be strings with brackets or commas (`separator: String = ", "`).
        if let Some(open_quote) = quote {
            if ch == open_quote && !signature[..index].ends_with('\\') {
                quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' if depth == 0 => {
                close = Some(index);
                break;
            }
            ')' | ']' | '}' => depth -= 1,
            '>' if !signature[..index].ends_with('-') && !signature[..index].ends_with('=') => {
                depth -= 1
            }
            ',' if depth == 0 => cuts.push(index + 1),
            _ => {}
        }
    }
    let Some(close) = close else {
        return signature.to_owned();
    };
    cuts.push(close + 1);
    let parameters: Vec<String> = cuts
        .windows(2)
        .map(|pair| {
            let parameter = signature[pair[0]..pair[1] - 1].trim();
            let has_default = top_level_byte(parameter, b'=').is_some();
            let head = match top_level_byte(parameter, b':') {
                // `label name: Type` (Swift, Kotlin, TypeScript, Python, Rust).
                Some(colon) => parameter[..colon].trim(),
                // `Type name` (Java, C#, C, C++): the last word.
                None => {
                    let before_default = top_level_byte(parameter, b'=')
                        .map_or(parameter, |equals| parameter[..equals].trim());
                    before_default
                        .rsplit(|ch: char| ch.is_whitespace() || ch == '*' || ch == '&')
                        .next()
                        .unwrap_or(before_default)
                }
            };
            if has_default {
                format!("{head} = …")
            } else {
                head.to_owned()
            }
        })
        .filter(|parameter| !parameter.is_empty())
        .collect();
    format!(
        "{}{}{}",
        &signature[..=open],
        parameters.join(", "),
        &signature[close..]
    )
}

/// Byte offset of `wanted` outside brackets and string quotes.
fn top_level_byte(text: &str, wanted: u8) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    for (index, byte) in text.bytes().enumerate() {
        if let Some(open_quote) = quote {
            if byte == open_quote {
                quote = None;
            }
            continue;
        }
        if byte == b'"' || byte == b'\'' {
            quote = Some(byte);
            continue;
        }
        match byte {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
            _ if byte == wanted && depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
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
    fn long_signatures_keep_their_parameter_list() {
        let swift = "public func requireHelp<T: ParsableArguments>(_ visibility: ArgumentVisibility, for _: T.Type, columns: Int? = 80, equals expected: String, sourceLocation: SourceLocation = #_sourceLocation) throws";
        assert_eq!(
            compact_parameters(swift, "requireHelp", 160),
            "public func requireHelp<T: ParsableArguments>(_ visibility, for _, columns = …, equals expected, sourceLocation = …) throws"
        );
        let java = "public static <T> List<T> merge(final Map<String, List<T>> left, final Map<String, List<T>> right, Comparator<? super T> comparator, int limit = 10)";
        assert_eq!(
            compact_parameters(java, "merge", 80),
            "public static <T> List<T> merge(left, right, comparator, limit = …)"
        );
        // String defaults with commas or brackets stay one parameter.
        let defaults = "public func render<T>(_ value: T, separator: String = \", \", open: String = \"(\", terminator: String = \"\\n\") -> String where T: CustomStringConvertible, T: Sendable";
        assert_eq!(
            compact_parameters(defaults, "render", 100),
            "public func render<T>(_ value, separator = …, open = …, terminator = …) -> String where T: CustomStringConvertible, T: Sendable"
        );
        // Short signatures stay as written.
        assert_eq!(compact_parameters("fn f(a: u8)", "f", 160), "fn f(a: u8)");
    }

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

    #[test]
    fn rust_path_strings_decode_supported_literals() {
        assert_eq!(
            rust_string_literal("\"custom\\u{5f}widget.rs\""),
            Some("custom_widget.rs".to_owned())
        );
        assert_eq!(
            rust_string_literal("\"quoted\\'widget.rs\""),
            Some("quoted'widget.rs".to_owned())
        );
        assert_eq!(
            rust_string_literal("r##\"custom/widget.rs\"##"),
            Some("custom/widget.rs".to_owned())
        );
        assert_eq!(
            rust_string_literal("\"line\\\n    /file.rs\""),
            Some("line/file.rs".to_owned())
        );
        assert_eq!(rust_string_literal("r\"unterminated\"#"), None);
        assert_eq!(rust_string_literal("\"\\xFF.rs\""), None);
    }
}
