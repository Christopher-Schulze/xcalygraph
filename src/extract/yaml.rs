//! Shared YAML syntax, decoded scalar values and source-backed mapping paths.

use yaml_rust2::Yaml;
use yaml_rust2::parser::{Event, Parser, Tag};
use yaml_rust2::scanner::TScalarStyle;

use crate::domain::facts::{Coverage, ValueKind};

pub(super) struct Entry {
    pub path: Vec<String>,
    pub key: String,
    pub value: String,
    pub kind: Option<ValueKind>,
    pub line: u32,
    pub end: u32,
    pub document: u32,
}

enum Frame {
    Mapping {
        path: Vec<String>,
        key: Option<(String, u32)>,
    },
    Sequence {
        path: Vec<String>,
    },
}

struct Position {
    path: Vec<String>,
    key: String,
    line: u32,
}

fn value_position(frames: &mut [Frame], line: u32) -> Option<Position> {
    match frames.last_mut() {
        Some(Frame::Mapping { path, key }) => {
            let (key, line) = key.take()?;
            Some(Position {
                path: path.clone(),
                key,
                line,
            })
        }
        Some(Frame::Sequence { path }) => {
            let mut path = path.clone();
            path.push("[]".to_owned());
            Some(Position {
                path,
                key: String::new(),
                line,
            })
        }
        None => Some(Position {
            path: Vec::new(),
            key: String::new(),
            line,
        }),
    }
}

/// Complex keys, aliases and custom tags remain explicitly unsupported.
pub(super) fn parse(text: &str) -> (Vec<Entry>, Coverage) {
    let mut parser = Parser::new_from_str(text);
    let mut entries: Vec<Entry> = Vec::new();
    let mut frames = Vec::new();
    let mut coverage = Coverage::Complete;
    let mut document: u32 = 0;
    loop {
        let Ok((event, marker)) = parser.next_token() else {
            coverage = coverage.max(Coverage::ParseErrors);
            break;
        };
        let line = u32::try_from(marker.line()).unwrap_or(u32::MAX);
        match event {
            Event::StreamEnd => break,
            Event::DocumentStart => {
                document = document.saturating_add(1);
                frames.clear();
            }
            Event::Scalar(value, style, _, tag) => {
                if let Some(Frame::Mapping { key, .. }) = frames.last_mut()
                    && key.is_none()
                {
                    if tag.as_ref().is_some_and(|tag| {
                        tag.handle != "tag:yaml.org,2002:" || tag.suffix != "str"
                    }) {
                        coverage = coverage.max(Coverage::NoGrammar);
                        break;
                    }
                    *key = Some((value, line));
                    continue;
                }
                let Some(position) = value_position(&mut frames, line) else {
                    coverage = coverage.max(Coverage::NoGrammar);
                    break;
                };
                let kind = scalar_kind(&value, style, tag.as_ref());
                if kind.is_none() {
                    coverage = coverage.max(Coverage::NoGrammar);
                }
                entries.push(Entry {
                    path: position.path,
                    key: position.key,
                    value: if kind.is_some() { value } else { String::new() },
                    kind,
                    line: position.line,
                    end: line,
                    document,
                });
            }
            event @ (Event::MappingStart(_, _) | Event::SequenceStart(_, _)) => {
                let mapping = matches!(event, Event::MappingStart(_, _));
                let tag = match &event {
                    Event::MappingStart(_, tag) | Event::SequenceStart(_, tag) => tag.as_ref(),
                    _ => None,
                };
                if tag.is_some_and(|tag| {
                    tag.handle != "tag:yaml.org,2002:"
                        || tag.suffix != if mapping { "map" } else { "seq" }
                }) {
                    coverage = coverage.max(Coverage::NoGrammar);
                    break;
                }
                let root = frames.is_empty();
                let Some(position) = value_position(&mut frames, line) else {
                    coverage = coverage.max(Coverage::NoGrammar);
                    break;
                };
                let mut path = position.path.clone();
                if !position.key.is_empty() {
                    path.push(position.key.clone());
                }
                if !root {
                    entries.push(Entry {
                        path: position.path,
                        key: position.key,
                        value: String::new(),
                        kind: Some(if mapping {
                            ValueKind::Object
                        } else {
                            ValueKind::Array
                        }),
                        line: position.line,
                        end: line,
                        document,
                    });
                }
                frames.push(if mapping {
                    Frame::Mapping { path, key: None }
                } else {
                    Frame::Sequence { path }
                });
            }
            Event::MappingEnd | Event::SequenceEnd => {
                if frames.pop().is_none() {
                    coverage = coverage.max(Coverage::ParseErrors);
                    break;
                }
            }
            Event::Alias(_) => {
                coverage = coverage.max(Coverage::NoGrammar);
                let Some(position) = value_position(&mut frames, line) else {
                    break;
                };
                entries.push(Entry {
                    path: position.path,
                    key: position.key,
                    value: String::new(),
                    kind: None,
                    line: position.line,
                    end: line,
                    document,
                });
            }
            Event::Nothing | Event::StreamStart | Event::DocumentEnd => {}
        }
    }
    let last = u32::try_from(text.lines().count()).unwrap_or(u32::MAX);
    let mut open: Vec<usize> = Vec::new();
    for index in 0..entries.len() {
        while open
            .last()
            .is_some_and(|&prior| entries[prior].path.len() >= entries[index].path.len())
        {
            if let Some(prior) = open.pop() {
                entries[prior].end = entries[prior]
                    .line
                    .max(entries[index].line.saturating_sub(1));
            }
        }
        open.push(index);
    }
    for index in open {
        entries[index].end = entries[index].line.max(last);
    }
    (entries, coverage)
}

fn scalar_kind(value: &str, style: TScalarStyle, tag: Option<&Tag>) -> Option<ValueKind> {
    if let Some(tag) = tag {
        if tag.handle != "tag:yaml.org,2002:" {
            return None;
        }
        return match tag.suffix.as_str() {
            "str" => Some(ValueKind::String),
            "bool"
                if matches!(
                    value,
                    "true" | "True" | "TRUE" | "false" | "False" | "FALSE"
                ) =>
            {
                Some(ValueKind::Bool)
            }
            "int" if matches!(Yaml::from_str(value), Yaml::Integer(_)) => Some(ValueKind::Number),
            "float" if matches!(Yaml::from_str(value), Yaml::Integer(_) | Yaml::Real(_)) => {
                Some(ValueKind::Number)
            }
            "null" if matches!(value, "" | "~" | "null" | "Null" | "NULL") => Some(ValueKind::Null),
            _ => None,
        };
    }
    if style != TScalarStyle::Plain {
        return Some(ValueKind::String);
    }
    if matches!(value, "Null" | "NULL") {
        return Some(ValueKind::Null);
    }
    Some(match Yaml::from_str(value) {
        Yaml::Integer(_) | Yaml::Real(_) => ValueKind::Number,
        Yaml::Boolean(_) => ValueKind::Bool,
        Yaml::Null => ValueKind::Null,
        Yaml::String(_) | Yaml::Array(_) | Yaml::Hash(_) | Yaml::Alias(_) | Yaml::BadValue => {
            ValueKind::String
        }
    })
}
