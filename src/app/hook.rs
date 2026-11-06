//! `xcalygraph hook EVENT`: context for agent hosts. Reads the host's JSON from stdin, prints the
//! host's JSON or nothing, and always exits 0: a hook must never break or slow its host.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::query::Filter;
use super::query::find;
use super::query::impact::{self, ImpactOptions};
use super::query::output::json_string;
use super::refresh::Mode;
use super::workspace::Workspace;
use crate::cli::Options;
use crate::extract::json;

const MAX_INPUT: u64 = 1 << 20;
/// Prompt bytes that count: a pasted log must not cost a search over its whole text.
const MAX_PROMPT: usize = 2 << 10;
const PROMPT_BUDGET: usize = 200;
const EDIT_BUDGET: usize = 120;
/// Well below the one second `install --hooks` writes: an answer that comes late is dropped.
const PROMPT_DEADLINE: Duration = Duration::from_millis(150);
const EDIT_DEADLINE: Duration = Duration::from_millis(800);

static OUTPUT_CLAIMED: AtomicBool = AtomicBool::new(false);

/// Ends the process silently at the deadline, unless the answer has started: no partial JSON.
fn start_watchdog(deadline: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(deadline);
        if !OUTPUT_CLAIMED.swap(true, Ordering::SeqCst) {
            std::process::exit(0);
        }
    });
}

pub fn run(event: &str) -> String {
    let (hook_event, deadline) = match event {
        "prompt" => ("UserPromptSubmit", PROMPT_DEADLINE),
        "edit" => ("PostToolUse", EDIT_DEADLINE),
        _ => return String::new(),
    };
    start_watchdog(deadline);
    let context = read_input().and_then(|input| match event {
        "prompt" => prompt_context(&input),
        _ => edit_context(&input),
    });
    let Some(context) = context else {
        return String::new();
    };
    if OUTPUT_CLAIMED.swap(true, Ordering::SeqCst) {
        // The watchdog won and is ending the process.
        std::thread::sleep(Duration::from_secs(1));
        return String::new();
    }
    format!(
        "{{\"hookSpecificOutput\":{{\"hookEventName\":\"{hook_event}\",\"additionalContext\":{}}}}}\n",
        json_string(&context)
    )
}

fn read_input() -> Option<json::Value> {
    let mut text = String::new();
    std::io::stdin()
        .take(MAX_INPUT)
        .read_to_string(&mut text)
        .ok()?;
    json::parse_strict(&text)
}

fn workspace_of(input: &json::Value) -> Option<Workspace> {
    let options = Options {
        root: input
            .get("cwd")
            .and_then(json::Value::as_str)
            .map(PathBuf::from),
        ..Options::default()
    };
    Workspace::resolve(&options).ok()
}

/// The start of a prompt, cut at a character boundary.
fn bounded(prompt: &str) -> &str {
    let mut end = prompt.len().min(MAX_PROMPT);
    while !prompt.is_char_boundary(end) {
        end -= 1;
    }
    &prompt[..end]
}

/// Read-only: a stale or missing index stays silent, the prompt path never builds or writes.
fn prompt_context(input: &json::Value) -> Option<String> {
    let prompt = bounded(input.get("prompt")?.as_str()?);
    let workspace = workspace_of(input)?;
    let (workspace, _, index) = super::prepare_workspace(workspace, Mode::ReadOnly).ok()?;
    let filter = Filter::new(&Options::default()).ok()?;
    let hits = find::search(&index, &filter, prompt);
    if !find::is_decided(&index, &hits, prompt) {
        return None;
    }
    let root = workspace.root.clone();
    Some(find::render_answer(
        &index,
        &filter,
        &root,
        &hits,
        prompt,
        false,
        PROMPT_BUDGET,
    ))
}

/// Refreshes after the edit and reports only what reaches beyond the changed code.
fn edit_context(input: &json::Value) -> Option<String> {
    let workspace = workspace_of(input)?;
    let (workspace, _, index) = super::prepare_workspace(workspace, Mode::Write).ok()?;
    let filter = Filter::new(&Options::default()).ok()?;
    let options = ImpactOptions {
        root: &workspace.root,
        store: &workspace.store,
        depth: 1,
        json: false,
        budget: EDIT_BUDGET,
        history: None,
    };
    let text = impact::render_diff(&index, &filter, "HEAD", &options).ok()?;
    // A changed contract with someone depending on it; a body edit keeps its callers working.
    let contract = text.lines().any(|line| {
        line.starts_with("~ ")
            && [" sig", " sig?", " removed", " moved?"]
                .iter()
                .any(|mark| line.ends_with(mark))
    });
    let reaches = contract
        && text
            .lines()
            .any(|line| line.starts_with("< ") || line.starts_with("t "));
    reaches.then_some(text)
}
