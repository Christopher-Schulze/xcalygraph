//! `impact`: changed and affected symbols, tests and run commands (`docs/spec.md` section 7.5).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use super::output::{Value, Writer};
use super::{Filter, Index, Node};
use crate::app::error::AppError;
use crate::domain::corpus::FileRecord as CorpusRecord;
use crate::domain::facts::{FLAG_TEST, Facts, RefKind, SymKind};
use crate::domain::filetype::FileType;
use crate::domain::graph::EdgeKind;
use crate::extract;
use crate::infra::git;

const AFFECTING: [EdgeKind; 6] = [
    EdgeKind::Calls,
    EdgeKind::Uses,
    EdgeKind::Inherits,
    EdgeKind::Implements,
    EdgeKind::References,
    EdgeKind::Queries,
];

/// Lines per section; tests and run commands are never displaced by long change lists.
/// Run commands get their budget reserved first (half the budget at most).
const CHANGED_LINES: usize = 15;
const AFFECTED_LINES: usize = 25;
const TEST_LINES: usize = 15;
/// Test names per run command before it runs the whole package or crate instead.
const RUN_NAMES: usize = 20;
/// Run commands shown; a change reaching more packages needs a full test run anyway.
const RUN_LINES: usize = 8;

/// A changed symbol or file and what changed about it.
struct Target {
    node: u32,
    /// `sig` for an exported signature change, `new` for a symbol absent at the revision,
    /// `new N` for a new file with N symbols.
    note: String,
    /// Further BFS seeds summarized by this line (the symbols of a new file).
    seeds: Vec<u32>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DefinitionKey {
    kind: SymKind,
    parts: Vec<String>,
}

struct Definition {
    key: DefinitionKey,
    flags: u8,
    signature: Option<String>,
    line: u32,
    matched: bool,
    ambiguous: bool,
    local: bool,
    signature_changed: bool,
}

struct RevisionFile {
    path: String,
    file_type: Option<FileType>,
    role: crate::domain::role::Role,
    package: Option<String>,
    content: Vec<u8>,
    symbols: Vec<Definition>,
    incomplete: bool,
}

struct CurrentFile {
    file: u32,
    path: String,
    file_type: Option<FileType>,
    package: Option<String>,
    hash: Option<u128>,
    symbols: Vec<Definition>,
    incomplete: bool,
}

struct Change {
    diff: git::FileDiff,
    previous: Option<RevisionFile>,
    current: Option<CurrentFile>,
}

struct RevisionLocation<'a> {
    name: String,
    path: &'a str,
    line: u32,
    ambiguous: bool,
}

struct Move<'a> {
    previous: RevisionLocation<'a>,
    path: &'a str,
    line: u32,
    signature_changed: bool,
}

#[derive(Default)]
struct DiffOutput<'a> {
    removed: &'a [RevisionLocation<'a>],
    moved: &'a [Move<'a>],
    survivors: &'a [u32],
    incomplete: &'a [(&'a str, &'a str)],
}

pub struct ImpactOptions<'a> {
    pub root: &'a Path,
    /// Fact shards for current definitions and surviving references.
    pub store: &'a crate::infra::store::Store,
    pub depth: usize,
    pub json: bool,
    pub budget: usize,
}

/// Targets from NAME arguments.
pub fn named(
    index: &Index,
    filter: &Filter,
    names: &[String],
    json: bool,
    budget: usize,
) -> Result<Vec<u32>, AppError> {
    names
        .iter()
        .map(|name| super::names::resolve_one(index, filter, name, json, budget))
        .collect()
}

pub fn render_names(
    index: &Index,
    filter: &Filter,
    targets: Vec<u32>,
    options: &ImpactOptions,
) -> String {
    let targets: Vec<Target> = targets
        .into_iter()
        .map(|node| Target {
            node,
            note: String::new(),
            seeds: Vec::new(),
        })
        .collect();
    render(index, filter, &targets, DiffOutput::default(), options)
}

/// Targets from `git diff REV` (work tree included) with exported signature changes.
pub fn render_diff(
    index: &Index,
    filter: &Filter,
    rev: &str,
    options: &ImpactOptions,
) -> Result<String, AppError> {
    let diffs = git::diff(options.root, rev)
        .ok_or_else(|| AppError::Invalid(format!("git diff against '{rev}' unavailable")))?;
    let mut changes = collect_changes(index, options.store, options.root, rev, diffs)?;
    for change in &mut changes {
        if let (Some(previous), Some(current)) = (&mut change.previous, &mut change.current)
            && previous.package == current.package
        {
            match_local(&mut previous.symbols, &mut current.symbols);
        }
    }
    // An incomplete same-language file may hide a move, so unmatched identities stay uncertain.
    let incomplete_current: BTreeSet<_> = changes
        .iter()
        .filter_map(|change| {
            change
                .current
                .as_ref()
                .filter(|file| file.incomplete)
                .map(|file| file.file_type)
        })
        .collect();
    let incomplete_revision: BTreeSet<_> = changes
        .iter()
        .filter_map(|change| {
            change
                .previous
                .as_ref()
                .filter(|file| file.incomplete)
                .map(|file| file.file_type)
        })
        .collect();
    for change in &mut changes {
        if let Some(previous) = &mut change.previous
            && (previous.incomplete || incomplete_current.contains(&previous.file_type))
        {
            for symbol in previous.symbols.iter_mut().filter(|symbol| !symbol.matched) {
                symbol.ambiguous = true;
            }
        }
        if let Some(current) = &mut change.current
            && (current.incomplete || incomplete_revision.contains(&current.file_type))
        {
            for symbol in current.symbols.iter_mut().filter(|symbol| !symbol.matched) {
                symbol.ambiguous = true;
            }
        }
    }
    let moves = match_moves(&mut changes, options.root, rev)?;
    let targets = changed_targets(index, filter, &changes);
    let mut removed = Vec::new();
    let mut removed_names: BTreeSet<String> = BTreeSet::new();
    for change in &changes {
        let Some(previous) = &change.previous else {
            continue;
        };
        if !filter.removed_file(&previous.path, previous.file_type, previous.role) {
            continue;
        }
        for symbol in previous.symbols.iter().filter(|symbol| !symbol.matched) {
            if !filter.removed_symbol(symbol.key.kind, symbol.flags)
                || (symbol.ambiguous && filter.exact)
            {
                continue;
            }
            removed.push(RevisionLocation {
                name: symbol.key.parts.join("."),
                path: &previous.path,
                line: symbol.line,
                ambiguous: symbol.ambiguous,
            });
            if !symbol.ambiguous
                && let Some(name) = symbol.key.parts.last()
            {
                removed_names.insert(name.to_owned());
            }
        }
    }
    removed.sort_by(|a, b| {
        a.path
            .cmp(b.path)
            .then(a.name.cmp(&b.name))
            .then(a.line.cmp(&b.line))
    });
    let mut moved = Vec::new();
    for &(old_file, old_symbol, new_file, new_symbol) in &moves {
        let (Some(previous), Some(current)) =
            (&changes[old_file].previous, &changes[new_file].current)
        else {
            continue;
        };
        if !filter.removed_file(&previous.path, previous.file_type, previous.role) {
            continue;
        }
        let old = &previous.symbols[old_symbol];
        let new = &current.symbols[new_symbol];
        if !filter.removed_symbol(old.key.kind, old.flags) {
            continue;
        }
        let current_node =
            index.derived.files()[current.file as usize].first_symbol + new_symbol as u32;
        if !filter.symbol(index, &index.derived.symbols()[current_node as usize]) {
            if !filter.exact {
                removed.push(RevisionLocation {
                    name: old.key.parts.join("."),
                    path: &previous.path,
                    line: old.line,
                    ambiguous: false,
                });
                if let Some(name) = old.key.parts.last() {
                    removed_names.insert(name.to_owned());
                }
            }
            continue;
        }
        if !filter.exact
            && !same_scope(previous, current)
            && let Some(name) = old.key.parts.last()
        {
            removed_names.insert(name.to_owned());
        }
        if filter.exact {
            continue;
        }
        moved.push(Move {
            previous: RevisionLocation {
                name: old.key.parts.join("."),
                path: &previous.path,
                line: old.line,
                ambiguous: false,
            },
            path: &current.path,
            line: new.line,
            signature_changed: matches!(
                index.node(index.derived.files()[current.file as usize].first_symbol + new_symbol as u32),
                Node::Symbol(symbol) if symbol.exported != 0
            ) && old.signature.as_ref().zip(new.signature.as_ref())
                .is_some_and(|(before, after)| before != after),
        });
    }
    let survivors = if filter.exact {
        Vec::new()
    } else {
        surviving_references(index, filter, options.store, &removed_names)?
    };
    let mut incomplete = Vec::new();
    for change in &changes {
        if let Some(previous) = &change.previous
            && previous.incomplete
        {
            incomplete.push((previous.path.as_str(), "revision"));
        }
        if let Some(current) = &change.current
            && current.incomplete
        {
            incomplete.push((current.path.as_str(), "current"));
        }
    }
    Ok(render(
        index,
        filter,
        &targets,
        DiffOutput {
            removed: &removed,
            moved: &moved,
            survivors: &survivors,
            incomplete: &incomplete,
        },
        options,
    ))
}

/// Reference kinds through which code depends on a definition.
const DEPENDING_REFS: [RefKind; 5] = [
    RefKind::Call,
    RefKind::Type,
    RefKind::Inherit,
    RefKind::Implement,
    RefKind::Value,
];

/// Current symbols (or files) that still name a removed definition and whose reference resolves
/// to no current symbol of that name: the dependents the refreshed graph can no longer show.
/// Name-based, hence inferred. Reads fact shards only when something was removed.
fn surviving_references(
    index: &Index,
    filter: &Filter,
    store: &crate::infra::store::Store,
    names: &BTreeSet<String>,
) -> Result<Vec<u32>, AppError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let records = store
        .read_files()
        .map_err(|error| AppError::Invalid(format!("cannot read index file records: {error}")))?;
    let mut expected: Vec<Vec<&CorpusRecord>> = (0..crate::infra::store::SHARDS)
        .map(|_| Vec::new())
        .collect();
    for record in records.iter().filter(|record| record.role.is_extracted()) {
        expected[crate::infra::store::shard_of(&record.path)].push(record);
    }
    let positions: HashMap<&str, usize> = index
        .derived
        .files()
        .iter()
        .enumerate()
        .map(|(position, file)| (index.string(file.path), position))
        .collect();
    let mut found: BTreeSet<u32> = BTreeSet::new();
    for (shard, expected_files) in expected.iter().enumerate() {
        if expected_files.is_empty() {
            continue;
        }
        let files = store.read_shard(shard).map_err(|error| {
            AppError::RecoverFacts(format!(
                "fact shard {shard:02} cannot be read ({error}); run 'xcalygraph index'"
            ))
        })?;
        if files.len() != expected_files.len()
            || files
                .iter()
                .zip(expected_files)
                .any(|((path, content_hash, _), record)| {
                    path != &record.path || *content_hash != record.hash
                })
        {
            return Err(AppError::RecoverFacts(format!(
                "fact shard {shard:02} does not match indexed sources; run 'xcalygraph index'"
            )));
        }
        for (path, _, facts) in &files {
            let Some(&position) = positions.get(path.as_str()) else {
                return Err(AppError::RecoverFacts(format!(
                    "fact shard {shard:02} references a file missing from derived data; run 'xcalygraph index'"
                )));
            };
            let record = &index.derived.files()[position];
            for reference in &facts.refs {
                if !DEPENDING_REFS.contains(&reference.kind) || !names.contains(&reference.name) {
                    continue;
                }
                let node = reference
                    .scope
                    .map_or(index.symbol_count() + position as u32, |scope| {
                        record.first_symbol + scope
                    });
                let resolved = index.derived.outgoing(node).iter().any(|edge| {
                    edge.line == reference.line
                        && index.label(edge.node).rsplit(['.', ':']).next()
                            == Some(reference.name.as_str())
                });
                if !resolved && filter.includes(index, node) {
                    found.insert(node);
                }
            }
        }
    }
    Ok(found.into_iter().collect())
}

/// Symbols whose range intersects a changed range; an enclosing symbol is dropped when one of
/// its children is hit.
fn innermost_hits(
    index: &Index,
    filter: &Filter,
    symbols: &[u32],
    ranges: &[(u32, u32)],
) -> Vec<u32> {
    let all = index.derived.symbols();
    let hit: Vec<u32> = symbols
        .iter()
        .copied()
        .filter(|&s| {
            let symbol = &all[s as usize];
            filter.symbol(index, symbol)
                && symbol.kind() != SymKind::Mod
                && ranges
                    .iter()
                    .any(|&(start, end)| symbol.line <= end && start <= symbol.end_line)
        })
        .collect();
    hit.iter()
        .copied()
        .filter(|&s| {
            !hit.iter()
                .any(|&other| other != s && all[other as usize].parent == s)
        })
        .collect()
}

fn definitions(facts: &Facts, identities: &[Option<String>]) -> Option<Vec<Definition>> {
    facts
        .symbols
        .iter()
        .enumerate()
        .map(|(index, symbol)| {
            let mut parents = Vec::new();
            let mut current = symbol.parent;
            let mut remaining = facts.symbols.len();
            while let Some(parent) = current {
                if remaining == 0 {
                    return None;
                }
                remaining -= 1;
                let enclosing = facts.symbols.get(parent as usize)?;
                parents.push(enclosing.name.as_str());
                current = enclosing.parent;
            }
            if !symbol.owner.is_empty() && parents.first() != Some(&symbol.owner.as_str()) {
                parents.insert(0, &symbol.owner);
            }
            parents.reverse();
            parents.push(&symbol.name);
            Some(Definition {
                key: DefinitionKey {
                    kind: symbol.kind(),
                    parts: parents.into_iter().map(str::to_owned).collect(),
                },
                flags: symbol.flags,
                signature: identities.get(index).cloned().flatten(),
                line: symbol.line,
                matched: false,
                ambiguous: false,
                local: false,
                signature_changed: false,
            })
        })
        .collect()
}

fn revision_file(
    root: &Path,
    rev: &str,
    path: &str,
    file_type: Option<FileType>,
    keep_content: bool,
) -> Result<Option<RevisionFile>, AppError> {
    let Some(content) = git::show(root, rev, path) else {
        return Ok(None);
    };
    // A file deleted since REV has no current record; classify it from its REV content.
    let classification = crate::domain::role::classify(&crate::domain::role::FileFacts {
        path,
        size: content.len() as u64,
        head: Some(&content[..content.len().min(crate::domain::role::HEAD_LEN)]),
        cpp_sibling: false,
    });
    let file_type = file_type.or(classification.file_type);
    let record = CorpusRecord {
        path: path.to_owned(),
        size: content.len() as u64,
        mtime_ns: 0,
        identity: None,
        hash: 0,
        role: crate::domain::role::Role::Src,
        file_type,
    };
    let parsed = extract::extract_with_identities(&record, &content);
    let facts = parsed.facts;
    Ok(Some(RevisionFile {
        path: path.to_owned(),
        file_type,
        role: classification.role,
        package: facts.package.clone(),
        symbols: definitions(&facts, &parsed.identities).ok_or_else(|| {
            AppError::Invalid(format!("invalid revision definitions in '{path}'"))
        })?,
        content: if keep_content { content } else { Vec::new() },
        incomplete: parsed.incomplete,
    }))
}

fn collect_changes(
    index: &Index,
    store: &crate::infra::store::Store,
    root: &Path,
    rev: &str,
    diffs: Vec<git::FileDiff>,
) -> Result<Vec<Change>, AppError> {
    let mut shards = HashMap::new();
    let mut changes = Vec::new();
    let records = store
        .read_files()
        .map_err(|error| AppError::Invalid(format!("cannot read index file records: {error}")))?;
    for diff in diffs {
        let file = (!diff.deleted)
            .then(|| {
                index
                    .derived
                    .files()
                    .iter()
                    .position(|file| index.string(file.path) == diff.path)
            })
            .flatten();
        if !diff.deleted && file.is_none() {
            continue;
        }
        let current = if let Some(file) = file {
            let record = &index.derived.files()[file];
            let verify_empty = matches!(
                record.role(),
                crate::domain::role::Role::Src
                    | crate::domain::role::Role::Test
                    | crate::domain::role::Role::Gen
            ) && record.file_type().is_some_and(FileType::is_code);
            let source = if record.symbol_count > 0 || verify_empty {
                let source = records
                    .binary_search_by(|entry| entry.path.as_str().cmp(diff.path.as_str()))
                    .ok()
                    .and_then(|position| records.get(position))
                    .ok_or_else(|| {
                        AppError::Invalid(format!("index file record missing for '{}'", diff.path))
                    })?;
                if source.role != record.role()
                    || source.file_type != record.file_type()
                    || !crate::domain::role::needs_content(&source.path, source.size)
                {
                    return Err(AppError::Invalid(format!(
                        "index file record disagrees for '{}'",
                        diff.path
                    )));
                }
                Some(source)
            } else {
                None
            };
            let facts = if record.symbol_count == 0 {
                None
            } else {
                let source = source.ok_or_else(|| {
                    AppError::Invalid(format!("index file record missing for '{}'", diff.path))
                })?;
                let shard = crate::infra::store::shard_of(&diff.path);
                if let std::collections::hash_map::Entry::Vacant(entry) = shards.entry(shard) {
                    let entries = store.read_shard(shard).map_err(|error| {
                        AppError::RecoverFacts(format!(
                            "cannot read index facts for '{}': {error}; run 'xcalygraph index'",
                            diff.path
                        ))
                    })?;
                    entry.insert(entries);
                }
                Some(
                    shards
                        .get(&shard)
                        .and_then(|entries: &Vec<(String, u128, Facts)>| {
                            entries.iter().find(|(path, content_hash, _)| {
                                path == &diff.path && *content_hash == source.hash
                            })
                        })
                        .map(|(_, _, facts)| facts)
                        .ok_or_else(|| {
                            AppError::RecoverFacts(format!(
                                "index facts for '{}' do not match the source; run 'xcalygraph index'",
                                diff.path
                            ))
                        })?,
                )
            };
            let (symbols, hash, incomplete) = if facts.is_some() || verify_empty {
                let source = source.ok_or_else(|| {
                    AppError::Invalid(format!("index file record missing for '{}'", diff.path))
                })?;
                let content = std::fs::read(root.join(&diff.path))
                    .map_err(AppError::io(format!("read source '{}'", diff.path)))?;
                if xxhash_rust::xxh3::xxh3_128(&content) != source.hash {
                    return Err(AppError::Invalid(format!(
                        "source changed during diff analysis: '{}'",
                        diff.path
                    )));
                }
                let parsed = extract::extract_with_identities(source, &content);
                let symbols = match facts {
                    Some(facts)
                        if parsed.facts.symbols == facts.symbols
                            && parsed.facts.package == facts.package =>
                    {
                        definitions(facts, &parsed.identities).ok_or_else(|| {
                            AppError::Invalid(format!("index facts disagree for '{}'", diff.path))
                        })?
                    }
                    None if parsed.facts.symbols.is_empty() => Vec::new(),
                    _ => {
                        return Err(AppError::Invalid(format!(
                            "index facts disagree for '{}'; run 'xcalygraph index'",
                            diff.path
                        )));
                    }
                };
                (symbols, Some(source.hash), parsed.incomplete)
            } else {
                (Vec::new(), None, false)
            };
            if symbols.len() != record.symbol_count as usize {
                return Err(AppError::Invalid(format!(
                    "index facts disagree for '{}'; run 'xcalygraph index'",
                    diff.path
                )));
            }
            if let Some(facts) = facts {
                for (offset, symbol) in facts.symbols.iter().enumerate() {
                    let derived = index
                        .derived
                        .symbols()
                        .get(record.first_symbol as usize + offset)
                        .ok_or_else(|| {
                            AppError::Invalid(format!(
                                "index facts disagree for '{}'; run 'xcalygraph index'",
                                diff.path
                            ))
                        })?;
                    let parent = match symbol.parent {
                        Some(parent) if (parent as usize) < facts.symbols.len() => {
                            record.first_symbol.checked_add(parent)
                        }
                        Some(_) => None,
                        None => Some(u32::MAX),
                    };
                    let special = symbol.kind() == SymKind::Heading
                        || (symbol.kind() == SymKind::Op && symbol.name.contains(' '));
                    if index.string(derived.name) != symbol.name
                        || derived.kind() != symbol.kind()
                        || derived.line != symbol.line
                        || derived.end_line != symbol.end_line
                        || (!special && index.string(derived.signature) != symbol.signature)
                        || Some(derived.parent) != parent
                    {
                        return Err(AppError::Invalid(format!(
                            "index facts disagree for '{}'; run 'xcalygraph index'",
                            diff.path
                        )));
                    }
                }
            }
            Some(CurrentFile {
                file: file as u32,
                path: diff.path.clone(),
                file_type: record.file_type(),
                package: facts.and_then(|facts| facts.package.clone()),
                hash,
                symbols,
                incomplete,
            })
        } else {
            None
        };
        let previous = if diff.untracked || crate::domain::role::is_sensitive(&diff.path) {
            None
        } else {
            revision_file(
                root,
                rev,
                &diff.path,
                current.as_ref().and_then(|file| file.file_type),
                diff.deleted,
            )?
        };
        if previous.is_some() || current.is_some() {
            changes.push(Change {
                diff,
                previous,
                current,
            });
        }
    }
    Ok(changes)
}

fn match_local(previous: &mut [Definition], current: &mut [Definition]) {
    let mut groups: BTreeMap<DefinitionKey, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (index, old) in previous.iter().enumerate() {
        groups.entry(old.key.clone()).or_default().0.push(index);
    }
    for (index, new) in current.iter().enumerate() {
        groups.entry(new.key.clone()).or_default().1.push(index);
    }
    for (old_indices, new_indices) in groups.values() {
        for &old_index in old_indices {
            let Some(signature) = previous[old_index].signature.as_deref() else {
                continue;
            };
            if old_indices
                .iter()
                .filter(|&&index| previous[index].signature.as_deref() == Some(signature))
                .take(2)
                .count()
                != 1
            {
                continue;
            }
            let mut candidates = new_indices
                .iter()
                .copied()
                .filter(|&index| current[index].signature.as_deref() == Some(signature));
            let (Some(new_index), None) = (candidates.next(), candidates.next()) else {
                continue;
            };
            previous[old_index].matched = true;
            current[new_index].matched = true;
            current[new_index].local = true;
        }
        let old_left: Vec<usize> = old_indices
            .iter()
            .copied()
            .filter(|&index| !previous[index].matched)
            .collect();
        let new_left: Vec<usize> = new_indices
            .iter()
            .copied()
            .filter(|&index| !current[index].matched)
            .collect();
        if old_left.len() == 1 && new_left.len() == 1 {
            let changed = previous[old_left[0]]
                .signature
                .as_ref()
                .zip(current[new_left[0]].signature.as_ref())
                .map(|(old, new)| old != new);
            previous[old_left[0]].matched = true;
            let new = &mut current[new_left[0]];
            new.matched = true;
            new.local = true;
            new.signature_changed = changed == Some(true);
            new.ambiguous = changed.is_none();
        } else if !old_left.is_empty() && !new_left.is_empty() {
            for index in old_left {
                previous[index].ambiguous = true;
            }
            for index in new_left {
                current[index].ambiguous = true;
            }
        }
    }
}

fn same_scope(previous: &RevisionFile, current: &CurrentFile) -> bool {
    if previous.file_type != current.file_type
        || previous.package.is_none()
        || previous.package != current.package
    {
        return false;
    }
    matches!(
        previous.file_type,
        Some(
            FileType::Go
                | FileType::Java
                | FileType::Kotlin
                | FileType::Scala
                | FileType::Groovy
                | FileType::CSharp
                | FileType::VbNet
                | FileType::Php
        )
    ) && crate::resolve::modules::parent_dir(&previous.path)
        == crate::resolve::modules::parent_dir(&current.path)
}

fn uncertain_scope(previous: &RevisionFile, current: &CurrentFile) -> bool {
    previous.file_type == current.file_type
        && previous.package.is_some()
        && previous.package == current.package
        && matches!(
            previous.file_type,
            Some(
                FileType::Java
                    | FileType::Kotlin
                    | FileType::Scala
                    | FileType::Groovy
                    | FileType::CSharp
                    | FileType::VbNet
                    | FileType::Php
            )
        )
        && crate::resolve::modules::parent_dir(&previous.path)
            != crate::resolve::modules::parent_dir(&current.path)
}

fn similar_content(previous: &[u8], current: &[u8]) -> bool {
    let largest = previous.len().max(current.len());
    if largest < 32 {
        return false;
    }
    let shared = previous
        .split(|&byte| byte == b'\n')
        .zip(current.split(|&byte| byte == b'\n'))
        .filter(|(old, new)| old.len() > 2 && old == new)
        .fold(0usize, |sum, (line, _)| sum.saturating_add(line.len() + 1));
    shared.saturating_mul(4) >= largest.saturating_mul(3)
}

fn file_scoped(file_type: Option<FileType>) -> bool {
    matches!(
        file_type,
        Some(
            FileType::TypeScript
                | FileType::JavaScript
                | FileType::Vue
                | FileType::Svelte
                | FileType::Astro
                | FileType::Python
                | FileType::Rust
                | FileType::Bash
                | FileType::C
                | FileType::Cpp
                | FileType::ObjC
                | FileType::Elixir
                | FileType::Erlang
                | FileType::Lua
                | FileType::Zig
                | FileType::Julia
                | FileType::OCaml
                | FileType::Fortran
                | FileType::Pascal
                | FileType::Lisp
                | FileType::Solidity
                | FileType::PowerShell
                | FileType::Robot
                | FileType::Dart
        )
    )
}

fn match_moves(
    changes: &mut [Change],
    root: &Path,
    rev: &str,
) -> Result<Vec<(usize, usize, usize, usize)>, AppError> {
    let mut candidates = Vec::new();
    let mut uncertain = Vec::new();
    let mut content = HashMap::new();
    let mut comparisons = HashMap::new();
    let has_deletion = changes
        .iter()
        .any(|change| change.diff.deleted && change.previous.is_some());
    let has_addition = changes
        .iter()
        .any(|change| change.previous.is_none() && change.current.is_some());
    let renames = if has_deletion && has_addition {
        git::renames(root, rev).ok_or_else(|| {
            AppError::Invalid(format!("git rename detection against '{rev}' unavailable"))
        })?
    } else {
        Vec::new()
    };
    let rename_pairs: std::collections::HashSet<(&str, &str)> = renames
        .iter()
        .map(|(old, new)| (old.as_str(), new.as_str()))
        .collect();
    let mut by_key: BTreeMap<DefinitionKey, Vec<(usize, usize)>> = BTreeMap::new();
    for (file, change) in changes.iter().enumerate() {
        if let Some(current) = &change.current {
            for (symbol, definition) in current
                .symbols
                .iter()
                .enumerate()
                .filter(|(_, definition)| !definition.matched && !definition.ambiguous)
            {
                by_key
                    .entry(definition.key.clone())
                    .or_default()
                    .push((file, symbol));
            }
        }
    }
    for (old_file, change) in changes.iter().enumerate() {
        let Some(previous) = &change.previous else {
            continue;
        };
        for (old_symbol, old) in previous
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, symbol)| !symbol.matched && !symbol.ambiguous)
        {
            let Some(matches) = by_key.get(&old.key) else {
                continue;
            };
            for &(new_file, new_symbol) in matches {
                let next = &changes[new_file];
                let Some(current) = &next.current else {
                    continue;
                };
                if previous.path == current.path || previous.file_type != current.file_type {
                    continue;
                }
                let new = &current.symbols[new_symbol];
                let renamed =
                    rename_pairs.contains(&(previous.path.as_str(), current.path.as_str()));
                let possible = file_scoped(previous.file_type)
                    && change.diff.deleted
                    && next.previous.is_none()
                    && old
                        .signature
                        .as_ref()
                        .zip(new.signature.as_ref())
                        .is_some_and(|(old, new)| old == new);
                let (identical, similar) = if !renamed
                    && !same_scope(previous, current)
                    && change.diff.deleted
                    && next.previous.is_none()
                {
                    let pair = (old_file, new_file);
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        comparisons.entry(pair)
                    {
                        let bytes = match content.entry(current.path.clone()) {
                            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                let bytes = std::fs::read(root.join(&current.path)).map_err(
                                    AppError::io(format!("cannot read '{}'", current.path)),
                                )?;
                                if Some(xxhash_rust::xxh3::xxh3_128(&bytes)) != current.hash {
                                    return Err(AppError::Invalid(format!(
                                        "source changed during diff analysis: '{}'",
                                        current.path
                                    )));
                                }
                                entry.insert(bytes)
                            }
                        };
                        let equal = *bytes == previous.content;
                        entry.insert((equal, !equal && similar_content(&previous.content, bytes)));
                    }
                    comparisons
                        .get(&pair)
                        .map_or((false, false), |&(equal, similar)| {
                            (equal, possible && similar)
                        })
                } else {
                    (false, false)
                };
                if same_scope(previous, current) || renamed || identical || similar {
                    candidates.push((old_file, old_symbol, new_file, new_symbol));
                } else if uncertain_scope(previous, current) || possible {
                    uncertain.push((old_file, old_symbol, new_file, new_symbol));
                }
            }
        }
    }
    let mut old_counts = HashMap::new();
    let mut new_counts = HashMap::new();
    for &(old_file, old_symbol, new_file, new_symbol) in &candidates {
        *old_counts.entry((old_file, old_symbol)).or_insert(0usize) += 1;
        *new_counts.entry((new_file, new_symbol)).or_insert(0usize) += 1;
    }
    let mut moves = Vec::new();
    for &(old_file, old_symbol, new_file, new_symbol) in &candidates {
        let unique_old = old_counts.get(&(old_file, old_symbol)) == Some(&1);
        let unique_new = new_counts.get(&(new_file, new_symbol)) == Some(&1);
        if let Some(previous) = &mut changes[old_file].previous {
            if unique_old && unique_new {
                previous.symbols[old_symbol].matched = true;
            } else {
                previous.symbols[old_symbol].ambiguous = true;
            }
        }
        if let Some(current) = &mut changes[new_file].current {
            if unique_old && unique_new {
                current.symbols[new_symbol].matched = true;
            } else {
                current.symbols[new_symbol].ambiguous = true;
            }
        }
        if unique_old && unique_new {
            moves.push((old_file, old_symbol, new_file, new_symbol));
        }
    }
    for (old_file, old_symbol, new_file, new_symbol) in uncertain {
        let old_matched = changes[old_file]
            .previous
            .as_ref()
            .is_some_and(|file| file.symbols[old_symbol].matched);
        let new_matched = changes[new_file]
            .current
            .as_ref()
            .is_some_and(|file| file.symbols[new_symbol].matched);
        if old_matched || new_matched {
            continue;
        }
        if let Some(previous) = &mut changes[old_file].previous {
            previous.symbols[old_symbol].ambiguous = true;
        }
        if let Some(current) = &mut changes[new_file].current {
            current.symbols[new_symbol].ambiguous = true;
        }
    }
    Ok(moves)
}

fn changed_targets(index: &Index, filter: &Filter, changes: &[Change]) -> Vec<Target> {
    let mut targets = Vec::new();
    for change in changes {
        let Some(current) = &change.current else {
            continue;
        };
        if current.incomplete {
            continue;
        }
        let record = &index.derived.files()[current.file as usize];
        if !filter.file(index, record) {
            continue;
        }
        let symbols: Vec<u32> =
            (record.first_symbol..record.first_symbol + record.symbol_count).collect();
        let file_node = index.symbol_count() + current.file;
        if change.previous.is_none() {
            let seeds: Vec<u32> = symbols
                .iter()
                .copied()
                .filter(|&node| filter.symbol(index, &index.derived.symbols()[node as usize]))
                .collect();
            targets.push(Target {
                node: file_node,
                note: format!("new {}", seeds.len()),
                seeds,
            });
            continue;
        }
        let hit = innermost_hits(index, filter, &symbols, &change.diff.ranges);
        for &node in &hit {
            let definition = &current.symbols[(node - record.first_symbol) as usize];
            let note = if definition.ambiguous && filter.exact {
                ""
            } else if definition.ambiguous {
                "uncertain"
            } else if !definition.local {
                "new"
            } else if definition.signature_changed
                && matches!(index.node(node), Node::Symbol(symbol) if symbol.exported != 0)
            {
                "sig"
            } else {
                ""
            };
            targets.push(Target {
                node,
                note: note.to_owned(),
                seeds: Vec::new(),
            });
        }
        if hit.is_empty() {
            targets.push(Target {
                node: file_node,
                note: String::new(),
                seeds: Vec::new(),
            });
        }
    }
    targets
}

fn render(
    index: &Index,
    filter: &Filter,
    targets: &[Target],
    diff: DiffOutput<'_>,
    options: &ImpactOptions,
) -> String {
    let DiffOutput {
        removed,
        moved,
        survivors,
        incomplete,
    } = diff;
    let mut writer = Writer::new(options.json, options.budget);
    let incomplete_current: BTreeSet<&str> = incomplete
        .iter()
        .filter_map(|(path, side)| (*side == "current").then_some(*path))
        .collect();
    let target_nodes: BTreeSet<u32> = targets
        .iter()
        .flat_map(|target| std::iter::once(target.node).chain(target.seeds.iter().copied()))
        .collect();
    // Reverse BFS over dependent edges, ordered by depth, then rank. A dependent is exact only
    // when some shortest chain to a target consists of exact edges; within one depth an exact
    // chain wins over an inferred one found earlier.
    let mut depth_of: HashMap<u32, usize> = target_nodes.iter().map(|&node| (node, 0)).collect();
    let mut exact_of: HashMap<u32, bool> = target_nodes.iter().map(|&node| (node, true)).collect();
    let mut frontier: Vec<u32> = target_nodes.iter().copied().collect();
    let mut affected: Vec<(usize, u32, bool)> = Vec::new();
    let mut tests: BTreeMap<u32, bool> = BTreeMap::new();
    for depth in 1..=options.depth.max(1) {
        let mut layer = Vec::new();
        // Surviving references to removed definitions are inferred direct dependents.
        if depth == 1 {
            for &node in survivors {
                if incomplete_current.contains(index.path_of(node)) {
                    continue;
                }
                if let std::collections::hash_map::Entry::Vacant(slot) = depth_of.entry(node) {
                    slot.insert(1);
                    exact_of.insert(node, false);
                    layer.push(node);
                }
            }
        }
        for &node in &frontier {
            let chain_exact = exact_of[&node];
            for edge in index.derived.incoming(node) {
                if incomplete_current.contains(index.path_of(edge.node)) {
                    continue;
                }
                if edge.kind() == EdgeKind::Tests {
                    if filter.edge(edge) && filter.includes(index, edge.node) {
                        let exact = chain_exact && edge.is_exact();
                        if !filter.exact || exact {
                            tests
                                .entry(edge.node)
                                .and_modify(|previous| *previous |= exact)
                                .or_insert(exact);
                        }
                    }
                    continue;
                }
                if !AFFECTING.contains(&edge.kind()) || !filter.edge(edge) {
                    continue;
                }
                let exact = chain_exact && edge.is_exact();
                match depth_of.get(&edge.node) {
                    Some(&seen) if seen < depth => {}
                    Some(_) => {
                        if exact {
                            exact_of.insert(edge.node, true);
                        }
                    }
                    None => {
                        depth_of.insert(edge.node, depth);
                        exact_of.insert(edge.node, exact);
                        layer.push(edge.node);
                    }
                }
            }
        }
        let mut next = Vec::new();
        for node in layer {
            if is_test(index, node) {
                let exact = exact_of[&node];
                if filter.includes(index, node) && (!filter.exact || exact) {
                    tests
                        .entry(node)
                        .and_modify(|previous| *previous |= exact)
                        .or_insert(exact);
                }
                continue;
            }
            if filter.node(index, node) {
                affected.push((depth, node, exact_of[&node]));
            }
            next.push(node);
        }
        frontier = next;
    }
    let rank = |node: u32| match index.node(node) {
        Node::Symbol(symbol) => symbol.rank,
        Node::File(_) => 0.0,
    };
    affected.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(rank(b.1).total_cmp(&rank(a.1)))
            .then(a.1.cmp(&b.1))
    });
    // Reserve package-level commands before any notices, targets, dependents or tests.
    let commands = run_lines(index, options.root, &tests);
    let run_record = |command: &str| -> (String, String) {
        let text = format!("r {command}");
        let record = super::output::json_object(&[
            ("t", Value::Str("edge")),
            ("rel", Value::Str("run")),
            ("name", Value::Str(command)),
        ]);
        (text, record)
    };
    let run_size = |command: &str| -> usize {
        let (text, record) = run_record(command);
        if options.json {
            record.len()
        } else {
            text.len() + 1
        }
    };
    let wanted: usize = commands
        .iter()
        .take(RUN_LINES)
        .map(|(_, compact)| run_size(compact))
        .sum();
    let reserved = writer.reserve(
        wanted.min(
            options
                .budget
                .saturating_mul(super::output::BYTES_PER_TOKEN)
                / 2,
        ),
    );
    write_changes(index, &mut writer, targets, removed, moved, incomplete);
    for (shown, (depth, node, exact)) in affected.iter().enumerate() {
        if shown >= AFFECTED_LINES {
            more(&mut writer, affected.len() - shown, "affected");
            break;
        }
        let label = index.label(*node);
        let path = index.path_of(*node);
        let line = index.line_of(*node);
        let mark = if *exact { "" } else { "?" };
        writer.line(
            &format!("< {depth} {label} {path}:{line}{mark}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("in")),
                ("name", Value::Str(&label)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(line))),
                ("n", Value::Int(*depth as u64)),
                (
                    "conf",
                    Value::Str(if *exact { "exact" } else { "inferred" }),
                ),
            ],
        );
    }
    for (shown, (&test, &exact)) in tests.iter().enumerate() {
        if shown >= TEST_LINES {
            more(&mut writer, tests.len() - shown, "tests");
            break;
        }
        let label = index.label(test);
        let path = index.path_of(test);
        let line = index.line_of(test);
        let mark = if exact { "" } else { "?" };
        writer.line(
            &format!("t {label} {path}:{line}{mark}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("test")),
                ("name", Value::Str(&label)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(line))),
                ("conf", Value::Str(if exact { "exact" } else { "inferred" })),
            ],
        );
    }
    writer.release(reserved);
    let shown_commands = commands.len().min(RUN_LINES);
    for (shown, (precise, compact)) in commands.iter().enumerate() {
        if shown >= RUN_LINES {
            more(&mut writer, commands.len() - shown, "run commands");
            break;
        }
        // The precise command only while every later command still fits in its short form;
        // otherwise the package-level one; a command that fits in neither form is counted.
        let later: usize = commands[shown + 1..shown_commands]
            .iter()
            .map(|(_, compact)| run_size(compact))
            .sum();
        let command = if run_size(precise) + later <= writer.remaining() {
            precise
        } else {
            compact
        };
        writer.line(
            &format!("r {command}"),
            &[
                ("t", Value::Str("edge")),
                ("rel", Value::Str("run")),
                ("name", Value::Str(command)),
            ],
        );
    }
    writer.finish()
}

fn write_changes(
    index: &Index,
    writer: &mut Writer,
    targets: &[Target],
    removed: &[RevisionLocation<'_>],
    moved: &[Move<'_>],
    incomplete: &[(&str, &str)],
) {
    for (shown, &(path, side)) in incomplete.iter().enumerate() {
        if shown >= CHANGED_LINES {
            more(writer, incomplete.len() - shown, "incomplete files");
            break;
        }
        let message = format!("incomplete {side}");
        writer.line(
            &format!("! {message} {path}"),
            &[
                ("t", Value::Str("note")),
                ("name", Value::Str(&message)),
                ("path", Value::Str(path)),
            ],
        );
    }
    // Code changes first; config, docs and other files after.
    let mut ordered: Vec<&Target> = targets.iter().collect();
    ordered.sort_by_key(|target| !matches!(index.node(target.node), Node::Symbol(_)));
    for (shown, target) in ordered.iter().enumerate() {
        if shown >= CHANGED_LINES {
            more(writer, ordered.len() - shown, "changed");
            break;
        }
        let label = index.label(target.node);
        let path = index.path_of(target.node);
        let line = index.line_of(target.node);
        let mut text = format!("~ {label} {path}:{line}");
        if !target.note.is_empty() {
            text.push(' ');
            text.push_str(&target.note);
            if target.note == "uncertain" {
                text.push('?');
            }
        }
        writer.line(
            &text,
            &[
                ("t", Value::Str("sym")),
                ("rel", Value::Str("chg")),
                ("name", Value::Str(&label)),
                ("path", Value::Str(path)),
                ("line", Value::Int(u64::from(line))),
                ("kind", Value::Str(&target.note)),
                (
                    "conf",
                    Value::Str(if target.note == "uncertain" {
                        "inferred"
                    } else {
                        ""
                    }),
                ),
            ],
        );
    }
    for previous in removed {
        let kind = if previous.ambiguous {
            "uncertain"
        } else {
            "removed"
        };
        let marker = if previous.ambiguous { "?" } else { "" };
        writer.line(
            &format!(
                "~ {} {}:{} {kind}{marker}",
                previous.name, previous.path, previous.line
            ),
            &[
                ("t", Value::Str("sym")),
                ("rel", Value::Str("chg")),
                ("name", Value::Str(&previous.name)),
                ("path", Value::Str(previous.path)),
                ("line", Value::Int(u64::from(previous.line))),
                ("kind", Value::Str(kind)),
                (
                    "conf",
                    Value::Str(if previous.ambiguous { "inferred" } else { "" }),
                ),
            ],
        );
    }
    for movement in moved {
        let previous = &movement.previous;
        let signature = if movement.signature_changed {
            " sig?"
        } else {
            ""
        };
        writer.line(
            &format!(
                "~ {} {}:{} -> {}:{} moved?{signature}",
                previous.name, previous.path, previous.line, movement.path, movement.line,
            ),
            &[
                ("t", Value::Str("sym")),
                ("rel", Value::Str("chg")),
                ("name", Value::Str(&previous.name)),
                ("path", Value::Str(previous.path)),
                ("line", Value::Int(u64::from(previous.line))),
                ("kind", Value::Str("moved")),
                ("new_path", Value::Str(movement.path)),
                ("new_line", Value::Int(u64::from(movement.line))),
                ("conf", Value::Str("inferred")),
                (
                    "sig_change",
                    Value::Str(if movement.signature_changed {
                        "changed"
                    } else {
                        ""
                    }),
                ),
            ],
        );
    }
}

fn more(writer: &mut Writer, count: usize, what: &str) {
    if !writer.line(
        &format!("+{count} more {what}"),
        &[
            ("t", Value::Str("more")),
            ("name", Value::Str(what)),
            ("n", Value::Int(count as u64)),
        ],
    ) {
        writer.skip(count.saturating_sub(1));
    }
}

fn is_test(index: &Index, node: u32) -> bool {
    match index.node(node) {
        Node::Symbol(symbol) => symbol.flags & FLAG_TEST != 0,
        Node::File(file) => file.role() == crate::domain::role::Role::Test,
    }
}

/// Commands that run exactly the selected tests: Go per package, Rust per crate, JVM test classes
/// per Gradle or Maven project, test files otherwise (section 7.5).
/// Run commands as (precise, package-level): the second is shorter and still correct, running
/// the whole package, crate, project or suite; it replaces the first when the budget is short.
fn run_lines(index: &Index, root: &Path, tests: &BTreeMap<u32, bool>) -> Vec<(String, String)> {
    let mut go: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut rust: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // (build tool, project directory) -> test classes.
    let mut jvm: BTreeMap<(JvmBuild, String), BTreeSet<String>> = BTreeMap::new();
    // .NET project directory -> fully qualified test names.
    let mut dotnet: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // SwiftPM package directory -> test classes.
    let mut swift: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // Runner prefix -> test file paths (RSpec, Minitest, ExUnit, Dart), PHPUnit test names.
    let mut by_path: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut phpunit: BTreeSet<String> = BTreeSet::new();
    let mut zig = false;
    let mut files: BTreeSet<String> = BTreeSet::new();
    let paths: Vec<&str> = index
        .derived
        .files()
        .iter()
        .map(|file| index.string(file.path))
        .collect();
    for &test in tests.keys() {
        let path = index.path_of(test);
        let runner = if path.ends_with("_spec.rb") {
            Some("bundle exec rspec")
        } else if path.ends_with(".rb") {
            Some("bundle exec ruby -Itest")
        } else if path.ends_with(".exs") {
            Some("mix test")
        } else if path.ends_with(".dart") {
            Some("dart test")
        } else {
            None
        };
        if let Some(runner) = runner {
            by_path.entry(runner).or_default().insert(path.to_owned());
            continue;
        }
        if path.ends_with(".zig") {
            zig = true;
            continue;
        }
        if path.ends_with(".php")
            && let Node::Symbol(symbol) = index.node(test)
            && index.string(symbol.name).starts_with("test")
        {
            phpunit.insert(index.string(symbol.name).to_owned());
            continue;
        }
        let Node::Symbol(symbol) = index.node(test) else {
            files.insert(index.path_of(test).to_owned());
            continue;
        };
        let path = index.path_of(test);
        let name = index.string(symbol.name).to_owned();
        match index.file_of(test).file_type() {
            Some(FileType::Go) if symbol.kind() == SymKind::Fn && name.starts_with("Test") => {
                let dir = parent(path);
                let module = nearest(&paths, dir, "go.mod").unwrap_or_default();
                let relative = dir
                    .strip_prefix(module.as_str())
                    .unwrap_or(dir)
                    .trim_start_matches('/');
                go.entry((module, relative.to_owned()))
                    .or_default()
                    .insert(name);
            }
            Some(FileType::Rust) if symbol.kind() == SymKind::Fn => {
                let manifest = nearest(&paths, parent(path), "Cargo.toml");
                let crate_name = manifest
                    .and_then(|dir| {
                        let file = if dir.is_empty() {
                            "Cargo.toml".to_owned()
                        } else {
                            format!("{dir}/Cargo.toml")
                        };
                        let text = std::fs::read_to_string(root.join(file)).ok()?;
                        extract::manifest::parse("Cargo.toml", &text)?.module
                    })
                    .unwrap_or_default();
                rust.entry(crate_name).or_default().insert(name);
            }
            Some(FileType::Java | FileType::Kotlin | FileType::Scala | FileType::Groovy)
                if symbol.parent != u32::MAX =>
            {
                let class = index.qualified(&index.derived.symbols()[symbol.parent as usize]);
                let dir = parent(path);
                let build = ["build.gradle", "build.gradle.kts"]
                    .iter()
                    .find_map(|manifest| nearest(&paths, dir, manifest))
                    .map(|project| (JvmBuild::Gradle, project))
                    .or_else(|| {
                        nearest(&paths, dir, "pom.xml").map(|project| (JvmBuild::Maven, project))
                    });
                match build {
                    Some(key) => {
                        jvm.entry(key).or_default().insert(class.into_owned());
                    }
                    None => {
                        files.insert(path.to_owned());
                    }
                }
            }
            Some(FileType::Swift) if symbol.parent != u32::MAX => {
                let class = index
                    .string(index.derived.symbols()[symbol.parent as usize].name)
                    .to_owned();
                match nearest(&paths, parent(path), "Package.swift") {
                    Some(package) => {
                        swift.entry(package).or_default().insert(class);
                    }
                    None => {
                        files.insert(path.to_owned());
                    }
                }
            }
            Some(FileType::CSharp | FileType::VbNet) => {
                let qualified = index.qualified(symbol).into_owned();
                match nearest_project(&paths, parent(path)) {
                    Some(project) => {
                        dotnet.entry(project).or_default().insert(qualified);
                    }
                    None => {
                        files.insert(path.to_owned());
                    }
                }
            }
            _ => {
                files.insert(path.to_owned());
            }
        }
    }
    let mut lines = Vec::new();
    for ((module, package), names) in go {
        let names: Vec<String> = names.into_iter().collect();
        let package = if package.is_empty() {
            ".".to_owned()
        } else {
            format!("./{package}")
        };
        let prefix = if module.is_empty() {
            "go".to_owned()
        } else {
            format!("go -C {module}")
        };
        // Beyond a handful of names the whole package is the shorter and safer command.
        let whole = format!("{prefix} test {package}");
        if names.len() > RUN_NAMES {
            lines.push((whole.clone(), whole));
        } else {
            lines.push((format!("{whole} -run '^({})$'", names.join("|")), whole));
        }
    }
    for (crate_name, names) in rust {
        let names: Vec<String> = names.into_iter().collect();
        let package = if crate_name.is_empty() {
            String::new()
        } else {
            format!(" -p {crate_name}")
        };
        let whole = format!("cargo test{package}");
        if names.len() > RUN_NAMES {
            lines.push((whole.clone(), whole));
        } else {
            lines.push((format!("{whole} -- {}", names.join(" ")), whole));
        }
    }
    for ((build, project), classes) in jvm {
        let classes: Vec<String> = classes.into_iter().collect();
        lines.push(match build {
            JvmBuild::Gradle => {
                let project = if project.is_empty() {
                    String::new()
                } else {
                    format!(" -p {project}")
                };
                let filters: Vec<String> = classes
                    .iter()
                    .map(|class| format!("--tests {class}"))
                    .collect();
                (
                    format!("./gradlew{project} test {}", filters.join(" ")),
                    format!("./gradlew{project} test"),
                )
            }
            JvmBuild::Maven => {
                let project = if project.is_empty() {
                    String::new()
                } else {
                    format!(" -f {project}/pom.xml")
                };
                (
                    format!("mvn{project} test -Dtest={}", classes.join(",")),
                    format!("mvn{project} test"),
                )
            }
        });
    }
    for (package, classes) in swift {
        let package = if package.is_empty() {
            String::new()
        } else {
            format!(" --package-path {package}")
        };
        let classes: Vec<String> = classes.into_iter().collect();
        lines.push((
            format!("swift test{package} --filter '{}'", classes.join("|")),
            format!("swift test{package}"),
        ));
    }
    for (project, names) in dotnet {
        let project = if project.is_empty() {
            String::new()
        } else {
            format!(" {project}")
        };
        let filter: Vec<String> = names
            .iter()
            .map(|name| format!("FullyQualifiedName~{name}"))
            .collect();
        lines.push((
            format!("dotnet test{project} --filter \"{}\"", filter.join("|")),
            format!("dotnet test{project}"),
        ));
    }
    for (runner, paths) in by_path {
        let paths: Vec<String> = paths.into_iter().collect();
        if runner == "bundle exec ruby -Itest" {
            // Minitest runs one file per process; no shorter command is equally correct.
            lines.extend(paths.iter().map(|path| {
                let line = format!("{runner} {path}");
                (line.clone(), line)
            }));
        } else {
            lines.push((format!("{runner} {}", paths.join(" ")), runner.to_owned()));
        }
    }
    if !phpunit.is_empty() {
        let names: Vec<String> = phpunit.into_iter().collect();
        lines.push((
            format!("vendor/bin/phpunit --filter '{}'", names.join("|")),
            "vendor/bin/phpunit".to_owned(),
        ));
    }
    if zig {
        lines.push(("zig build test".to_owned(), "zig build test".to_owned()));
    }
    lines.extend(files.into_iter().map(|file| (file.clone(), file)));
    lines
}

/// Directory of the nearest `.csproj`, `.vbproj` or `.fsproj` at or above `dir`.
fn nearest_project(paths: &[&str], dir: &str) -> Option<String> {
    let mut current = dir.to_owned();
    loop {
        let prefix = if current.is_empty() {
            String::new()
        } else {
            format!("{current}/")
        };
        let start = paths.partition_point(|path| *path < prefix.as_str());
        let found = paths[start..]
            .iter()
            .take_while(|path| path.starts_with(&prefix))
            .any(|path| {
                let rest = &path[prefix.len()..];
                !rest.contains('/')
                    && [".csproj", ".vbproj", ".fsproj"]
                        .iter()
                        .any(|suffix| rest.ends_with(suffix))
            });
        if found {
            return Some(current);
        }
        if current.is_empty() {
            return None;
        }
        current = parent(&current).to_owned();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum JvmBuild {
    Gradle,
    Maven,
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// Directory of the nearest `manifest` at or above `dir`.
fn nearest(paths: &[&str], dir: &str, manifest: &str) -> Option<String> {
    let mut current = dir.to_owned();
    loop {
        let candidate = if current.is_empty() {
            manifest.to_owned()
        } else {
            format!("{current}/{manifest}")
        };
        if paths.binary_search(&candidate.as_str()).is_ok() {
            return Some(current);
        }
        if current.is_empty() {
            return None;
        }
        current = parent(&current).to_owned();
    }
}

#[cfg(test)]
mod tests {
    use super::definitions;
    use crate::domain::facts::{Facts, SymKind, Symbol};

    #[test]
    fn corrupt_symbol_parents_are_rejected_without_panicking_or_looping() {
        for parent in [0, 1, u32::MAX] {
            let facts = Facts {
                symbols: vec![Symbol {
                    kind: Some(SymKind::Fn),
                    name: "item".to_owned(),
                    parent: Some(parent),
                    ..Symbol::default()
                }],
                ..Facts::default()
            };
            assert!(definitions(&facts, &[]).is_none(), "parent {parent}");
        }
    }
}
