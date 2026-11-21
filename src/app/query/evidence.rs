//! A bounded evidence bundle for a named configuration key or environment variable.
//! Definitions and documents are retained source sites; tests use graph relations.

use std::collections::BTreeMap;

use super::find::Hit;
use super::output::{OutputFact, Value, Writer, json_object};
use super::{Filter, Index, Node, impact, show};
use crate::domain::facts::SymKind;
use crate::domain::filetype::contains_word;
use crate::domain::graph::SiteKind;

const MAX_BUNDLES: usize = 2;
const RANKED_HITS: usize = 32;

struct Site<'a> {
    path: &'a str,
    line: u32,
    exact: bool,
}

struct Test<'a> {
    path: &'a str,
    line: u32,
    indirect: bool,
    exact: bool,
    reader_path: Option<&'a str>,
    intermediary_path: Option<&'a str>,
    intermediary_name: Option<std::borrow::Cow<'a, str>>,
    call_line: Option<u32>,
    command: Option<String>,
}

struct Bundle<'a> {
    name: &'a str,
    definition: Option<Site<'a>>,
    read: Option<Site<'a>>,
    test: Option<Test<'a>>,
    document: Option<Site<'a>>,
}

impl OutputFact for Bundle<'_> {
    fn text(&self) -> String {
        let mut text = format!("~ {}", self.name);
        for (role, marker, site) in [
            (
                "definition",
                if self.definition.as_ref().is_some_and(|site| !site.exact) {
                    "<definition?"
                } else {
                    "<definition"
                },
                self.definition.as_ref().map(|site| (site.path, site.line)),
            ),
            (
                "read",
                "<read",
                self.read.as_ref().map(|site| (site.path, site.line)),
            ),
            (
                "doc",
                "<doc?",
                self.document.as_ref().map(|site| (site.path, site.line)),
            ),
        ] {
            match site {
                Some((path, line)) => text.push_str(&format!(" {marker} {path}:{line}")),
                None => text.push_str(&format!(" -{role}")),
            }
        }
        match &self.test {
            Some(test) => {
                text.push_str(&format!(
                    " <t{}{} {}:{}",
                    if test.indirect { "2" } else { "" },
                    if test.exact { "" } else { "?" },
                    test.path,
                    test.line
                ));
                if let Some(name) = &test.intermediary_name {
                    text.push_str(&format!(
                        " <via {} {}:{}",
                        name,
                        test.intermediary_path.unwrap_or(""),
                        test.call_line.unwrap_or(0),
                    ));
                }
            }
            None => text.push_str(" -t"),
        }
        if let Some(command) = self.test.as_ref().and_then(|test| test.command.as_ref()) {
            text.push_str(&format!(" <run {command}"));
        }
        text
    }

    fn json_line(&self) -> String {
        let mut fields = vec![
            ("t", Value::Str("evidence")),
            ("name", Value::Str(self.name)),
        ];
        let mut roles = Vec::new();
        for (role, site) in [
            ("definition", self.definition.as_ref()),
            ("read", self.read.as_ref()),
        ] {
            if let Some(site) = site {
                roles.push(vec![
                    ("role", Value::Str(role)),
                    ("path", Value::Str(site.path)),
                    ("line", Value::Int(u64::from(site.line))),
                    (
                        "confidence",
                        Value::Str(if site.exact { "exact" } else { "inferred" }),
                    ),
                ]);
            }
        }
        if let Some(test) = &self.test {
            let mut role = vec![
                ("role", Value::Str("test")),
                (
                    "relation",
                    Value::Str(if test.indirect { "t2" } else { "t" }),
                ),
                ("path", Value::Str(test.path)),
                ("line", Value::Int(u64::from(test.line))),
                (
                    "confidence",
                    Value::Str(if test.exact { "exact" } else { "inferred" }),
                ),
            ];
            if let Some(name) = &test.intermediary_name {
                role.push(("via", Value::Str(name)));
                role.push(("reader", Value::Str(test.reader_path.unwrap_or(""))));
                role.push((
                    "intermediary_path",
                    Value::Str(test.intermediary_path.unwrap_or("")),
                ));
                role.push((
                    "call_line",
                    Value::Int(u64::from(test.call_line.unwrap_or(0))),
                ));
            }
            roles.push(role);
            if let Some(command) = &test.command {
                roles.push(vec![
                    ("role", Value::Str("run")),
                    ("path", Value::Str(test.path)),
                    ("line", Value::Int(u64::from(test.line))),
                    (
                        "confidence",
                        Value::Str(if test.exact { "exact" } else { "inferred" }),
                    ),
                    ("command", Value::Str(command)),
                ]);
            }
        }
        if let Some(site) = &self.document {
            roles.push(vec![
                ("role", Value::Str("document")),
                ("relation", Value::Str("doc")),
                ("path", Value::Str(site.path)),
                ("line", Value::Int(u64::from(site.line))),
                ("confidence", Value::Str("inferred")),
            ]);
        }
        fields.push(("roles", Value::Objects(&roles)));
        let missing = [
            (self.definition.is_none(), "definition"),
            (self.read.is_none(), "read"),
            (self.test.is_none(), "test"),
            (self.document.is_none(), "document"),
        ]
        .into_iter()
        .filter(|(missing, _)| *missing)
        .map(|(_, role)| role.to_owned())
        .collect::<Vec<_>>();
        if !missing.is_empty() {
            fields.push(("unavailable", Value::Strings(&missing)));
        }
        json_object(&fields)
    }
}

fn names(query: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for raw in query
        .split(|character: char| !(character.is_alphanumeric() || matches!(character, '_' | '.')))
    {
        let word = raw.trim_matches('.');
        let valid = if word.contains('.') {
            word.split('.').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
        } else {
            word.len() >= 3
                && word.bytes().any(|byte| byte.is_ascii_uppercase())
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        };
        if valid && !found.contains(&word) {
            found.push(word);
        }
        if found.len() == MAX_BUNDLES {
            break;
        }
    }
    found
}

fn bundles<'a>(index: &'a Index, filter: &Filter, query: &'a str, hits: &[Hit]) -> Vec<Bundle<'a>> {
    let rank = |node| {
        hits.iter()
            .take(RANKED_HITS)
            .position(|hit| hit.node == node)
            .unwrap_or(usize::MAX)
    };
    let mut found = Vec::new();
    for name in names(query) {
        let sites = index.derived.sites();
        let start = sites.partition_point(|site| index.string(site.name) < name);
        let sites = &sites[start..];
        let envs = index.derived.envs();
        let start = envs.partition_point(|env| index.string(env.name) < name);
        let envs = envs[start..]
            .iter()
            .take_while(|env| index.string(env.name) == name);
        let mut definitions = envs
            .clone()
            .filter(|env| env.source != 0 && filter.includes(index, env.node));
        let reader = envs
            .clone()
            .filter(|env| env.source == 0 && filter.includes(index, env.node))
            .min_by_key(|env| (rank(env.node), index.path_of(env.node), env.line));
        let mut config = sites
            .iter()
            .take_while(|site| index.string(site.name) == name)
            .filter(|site| {
                site.kind() == Some(SiteKind::Definition) && filter.includes(index, site.node)
            });
        let definition = if envs.count() != 0 {
            definitions
                .next()
                .filter(|_| definitions.next().is_none())
                .map(|env| (env.node, env.line))
        } else {
            config
                .next()
                .filter(|_| config.next().is_none())
                .map(|site| (site.node, site.line))
        }
        .filter(|_| !filter.exact || reader.is_none())
        .map(|(node, line)| Site {
            path: index.path_of(node),
            line,
            exact: reader.is_none(),
        });
        let read = reader.map(|env| Site {
            path: index.path_of(env.node),
            line: env.line,
            exact: true,
        });
        if definition.is_none() && read.is_none() {
            continue;
        }
        let document = (!filter.exact)
            .then(|| {
                let mentions = sites
                    .iter()
                    .take_while(|site| index.string(site.name) == name)
                    .filter(|site| {
                        site.kind() == Some(SiteKind::Document) && filter.includes(index, site.node)
                    })
                    .map(|site| (site.node, site.line));
                let headings = hits.iter().take(RANKED_HITS).filter_map(|hit| {
                    let Node::Symbol(symbol) = index.node(hit.node) else {
                        return None;
                    };
                    (symbol.kind() == SymKind::Heading
                        && contains_word(index.string(symbol.name), name))
                    .then_some((hit.node, symbol.line))
                });
                mentions
                    .chain(headings)
                    .min_by_key(|&(node, line)| (rank(node), index.path_of(node), line))
            })
            .flatten()
            .map(|(node, line)| Site {
                path: index.path_of(node),
                line,
                exact: false,
            })
            .or_else(|| {
                if filter.exact || reader.is_none() {
                    return None;
                }
                index
                    .derived
                    .symbols()
                    .iter()
                    .enumerate()
                    .filter(|(_, symbol)| {
                        symbol.kind() == crate::domain::facts::SymKind::Heading
                            && filter.symbol(index, symbol)
                            && contains_word(index.string(symbol.name), name)
                    })
                    .min_by_key(|(node, _)| index.path_of(*node as u32))
                    .map(|(node, symbol)| Site {
                        path: index.path_of(node as u32),
                        line: symbol.line,
                        exact: false,
                    })
            });
        let reader = reader.map(|env| env.node);
        let test = reader.and_then(|reader| {
            show::tests_of(index, filter, reader)
                .into_iter()
                .min_by_key(|link| {
                    (
                        !link.exact,
                        link.caller.is_some(),
                        index.path_of(link.edge.node),
                        link.edge.line,
                    )
                })
                .map(
                    |show::TestLink {
                         edge: test,
                         caller,
                         exact,
                     }| Test {
                        path: index.path_of(test.node),
                        line: test.line,
                        indirect: caller.is_some(),
                        exact,
                        reader_path: Some(index.path_of(reader)),
                        intermediary_path: caller.map(|edge| index.path_of(edge.node)),
                        intermediary_name: caller.map(|edge| index.short_label(edge.node)),
                        call_line: caller.map(|edge| edge.line),
                        command: impact::run_lines(index, &BTreeMap::from([(test.node, exact)]))
                            .into_iter()
                            .next()
                            .map(|(command, _)| command),
                    },
                )
        });
        found.push(Bundle {
            name,
            definition,
            read,
            test,
            document,
        });
    }
    found
}

pub(super) fn write_bundles(
    index: &Index,
    filter: &Filter,
    writer: &mut Writer,
    query: &str,
    hits: &[Hit],
) {
    for bundle in bundles(index, filter, query, hits) {
        writer.fact(&bundle);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_are_bounded() {
        assert_eq!(
            super::names("APP_TOKEN APP_TOKEN OTHER_KEY"),
            ["APP_TOKEN", "OTHER_KEY"]
        );
        assert!(super::names("app_token _ 123").is_empty());
    }
}
