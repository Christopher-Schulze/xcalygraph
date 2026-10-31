//! DM (BYOND): type paths nest as classes, procs and verbs belong to their type; `src.proc()`
//! calls the own type.

use super::Pack;
use crate::domain::facts::{Facts, NO_ARITY, Ref, RefKind, SymKind, Symbol};
use crate::domain::filetype::FileType;
use crate::extract::engine::{self, Hooks};

pub static PACK: Pack = Pack {
    suffixes: &[],
    file_type: FileType::Dm,
    language: |_| tree_sitter_dm::LANGUAGE.into(),
    query: include_str!("../queries/dm.scm"),
    hooks: Hooks {
        exported: |_| true,
        signature_prefix: |_, _| "",
        flags: |_, _| 0,
        expand_import: engine::single_import,
        builtin: |name| {
            matches!(
                name,
                "new"
                    | "del"
                    | "istype"
                    | "locate"
                    | "spawn"
                    | "sleep"
                    | "world"
                    | "text"
                    | "list"
                    | "length"
                    | "min"
                    | "max"
                    | "round"
                    | "rand"
                    | "prob"
                    | "pick"
                    | "num2text"
                    | "text2num"
                    | "isnull"
                    | "isnum"
                    | "istext"
                    | "islist"
                    | "copytext"
                    | "findtext"
                    | "replacetext"
                    | "range"
                    | "view"
                    | "orange"
                    | "get_dist"
                    | "get_step"
                    | "abs"
                    | "CRASH"
                    | "html_encode"
                    | "jointext"
                    | "splittext"
            )
        },
    },
};

/// Collapses nested path segments into types named by their full path: `/obj/item/store`
/// with its procs owned by that path; pure path prefixes are dropped; every type inherits from
/// its parent path (`/obj/item/store/big` from `/obj/item/store`).
pub fn type_paths(facts: &mut Facts) {
    let symbols = std::mem::take(&mut facts.symbols);
    let is_type = |symbol: &Symbol| symbol.kind == Some(SymKind::Class);
    let paths: Vec<String> = (0..symbols.len())
        .map(|index| {
            let mut segments = Vec::new();
            let mut current = Some(index as u32);
            while let Some(position) = current {
                let symbol = &symbols[position as usize];
                if is_type(symbol) {
                    segments.push(symbol.name.as_str());
                }
                current = symbol.parent;
            }
            segments.reverse();
            format!("/{}", segments.join("/"))
        })
        .collect();
    let owns_members = |index: usize| {
        symbols
            .iter()
            .any(|other| other.parent == Some(index as u32) && !is_type(other))
    };
    let has_children = |index: usize| {
        symbols
            .iter()
            .any(|other| other.parent == Some(index as u32))
    };
    let keep: Vec<bool> = symbols
        .iter()
        .enumerate()
        .map(|(index, symbol)| !is_type(symbol) || owns_members(index) || !has_children(index))
        .collect();
    let mut remap: Vec<Option<u32>> = vec![None; symbols.len()];
    let mut kept: Vec<Symbol> = Vec::new();
    for (index, mut symbol) in symbols.iter().cloned().enumerate() {
        if !keep[index] {
            continue;
        }
        let mut parent = symbol.parent;
        while let Some(position) = parent.filter(|&position| !keep[position as usize]) {
            parent = symbols[position as usize].parent;
        }
        symbol.parent = parent.and_then(|position| remap[position as usize]);
        if is_type(&symbol) {
            symbol.name = paths[index].clone();
            symbol.parent = None;
        } else if let Some(owner) = parent.filter(|&position| is_type(&symbols[position as usize]))
        {
            symbol.owner = paths[owner as usize].clone();
        }
        remap[index] = Some(kept.len() as u32);
        kept.push(symbol);
    }
    for reference in &mut facts.refs {
        let mut scope = reference.scope;
        while let Some(position) = scope.filter(|&position| !keep[position as usize]) {
            scope = symbols[position as usize].parent;
        }
        reference.scope = scope.and_then(|position| remap[position as usize]);
    }
    for (index, symbol) in kept.iter().enumerate() {
        if symbol.kind != Some(SymKind::Class) {
            continue;
        }
        if let Some((parent_path, _)) = symbol
            .name
            .rsplit_once('/')
            .filter(|(parent, _)| !parent.is_empty())
        {
            facts.refs.push(Ref {
                kind: RefKind::Inherit,
                name: parent_path.to_owned(),
                qualifier: None,
                scope: Some(index as u32),
                line: symbol.line,
                arity: NO_ARITY,
            });
        }
    }
    facts.symbols = kept;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_paths_collapse_to_full_paths() {
        let class = |name: &str, parent: Option<u32>, line: u32| Symbol {
            kind: Some(SymKind::Class),
            name: name.to_owned(),
            parent,
            line,
            end_line: line + 1,
            ..Symbol::default()
        };
        let mut facts = Facts {
            symbols: vec![
                class("obj", None, 1),
                class("item", Some(0), 1),
                class("store", Some(1), 1),
                Symbol {
                    kind: Some(SymKind::Method),
                    name: "open".to_owned(),
                    parent: Some(2),
                    owner: "store".to_owned(),
                    line: 2,
                    ..Symbol::default()
                },
                class("obj", None, 5),
                class("item", Some(4), 5),
                class("store", Some(5), 5),
                class("big", Some(6), 5),
            ],
            refs: vec![Ref {
                kind: RefKind::Call,
                name: "helper".to_owned(),
                qualifier: None,
                scope: Some(3),
                line: 3,
                arity: NO_ARITY,
            }],
            ..Facts::default()
        };
        type_paths(&mut facts);
        let symbols: Vec<String> = facts
            .symbols
            .iter()
            .map(|symbol| format!("{} {:?} {}", symbol.name, symbol.parent, symbol.owner))
            .collect();
        assert_eq!(
            symbols,
            [
                "/obj/item/store None ",
                "open Some(0) /obj/item/store",
                "/obj/item/store/big None "
            ]
        );
        assert_eq!(facts.refs[0].scope, Some(1));
        let inherits: Vec<(&str, Option<u32>)> = facts
            .refs
            .iter()
            .filter(|reference| reference.kind == RefKind::Inherit)
            .map(|reference| (reference.name.as_str(), reference.scope))
            .collect();
        assert_eq!(
            inherits,
            [("/obj/item", Some(0)), ("/obj/item/store", Some(2))]
        );
    }
}
