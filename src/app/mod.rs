//! Use cases: one per command.

pub mod error;
pub mod indexes;
pub mod install;
pub mod progress;
pub mod query;
pub mod refresh;
pub mod update;
pub mod workspace;

use crate::cli::{CommandName, Invocation};
use error::AppError;
use refresh::Mode;
use workspace::Workspace;

/// Result lines of `install` and `update`: on a live terminal the progress lines already showed
/// every step, so only `!` notices (PATH remediation, conflicts) and the update summary remain.
fn result_lines(progress: &progress::Progress, out: String) -> String {
    if !progress.is_live() {
        return out;
    }
    out.lines()
        .filter(|line| {
            line.starts_with('!')
                || line.starts_with("updated ")
                || line.starts_with("update available")
        })
        .map(|line| format!("{line}\n"))
        .collect()
}

/// Runs a command that needs the index and returns its stdout text.
pub fn run(invocation: &Invocation) -> Result<String, AppError> {
    let options = &invocation.options;
    match invocation.command {
        CommandName::Version => return Ok(version()),
        CommandName::Install => {
            let home = install::home()?;
            let mut progress = progress::Progress::stderr();
            let options = install::Options {
                binary: !options.skill_only,
                link: !options.no_link,
            };
            let out = install::install(&home, options, &mut progress)?;
            return Ok(result_lines(&progress, out));
        }
        CommandName::Update => {
            let mut progress = progress::Progress::stderr();
            let out = update::update(options.check, &mut progress)?;
            return Ok(result_lines(&progress, out));
        }
        CommandName::Ls => return indexes::list(options),
        CommandName::Clean => return indexes::clean(options, &invocation.args),
        _ => {}
    }
    let workspace = Workspace::resolve(&invocation.options)?;
    match invocation.command {
        CommandName::Index => index(&workspace),
        CommandName::Help
        | CommandName::Version
        | CommandName::Install
        | CommandName::Update
        | CommandName::Ls
        | CommandName::Clean => Ok(String::new()),
        command => {
            let mode = if options.read_only {
                Mode::ReadOnly
            } else {
                Mode::Write
            };
            refresh::refresh(&workspace, mode)?;
            if mode == Mode::Write {
                workspace.store.mark_used();
            }
            let index = open_index(&workspace, mode)?;
            let filter = query::Filter::new(options)?;
            run_query(
                &index,
                &filter,
                command,
                &invocation.args,
                options,
                &workspace,
            )
        }
    }
}

/// Opens the derived image and, only for writable operations, repairs it once from facts.
fn open_index(workspace: &Workspace, mode: Mode) -> Result<query::Index, AppError> {
    match query::open(&workspace.store) {
        Ok(index) => Ok(index),
        Err(_) if mode == Mode::Write => {
            refresh::rederive(workspace)?;
            query::open(&workspace.store)
        }
        Err(error) => Err(error),
    }
}

fn run_query(
    index: &query::Index,
    filter: &query::Filter,
    command: CommandName,
    args: &[String],
    options: &crate::cli::Options,
    workspace: &Workspace,
) -> Result<String, AppError> {
    let root = workspace.root.as_path();
    let json = options.json;
    let budget = |default: usize| options.budget.unwrap_or(default);
    match command {
        CommandName::Map => Ok(query::map::render(
            index,
            filter,
            &workspace.root.file_name().map_or_else(
                || "root".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            ),
            json,
            budget(2500),
        )),
        CommandName::Find => {
            let terms = args.join(" ");
            let hits = query::find::search(index, filter, &terms);
            if hits.is_empty() {
                return Err(AppError::NoMatch(format!("no match for '{terms}'")));
            }
            Ok(query::find::render(index, &hits, json, budget(600)))
        }
        CommandName::Ctx => {
            let terms = args.join(" ");
            query::ctx::render(index, filter, &terms, json, budget(2000))
                .ok_or_else(|| AppError::NoMatch(format!("no match for '{terms}'")))
        }
        CommandName::Show => {
            let node = query::names::resolve_one(index, filter, &args[0], json)?;
            let show = query::show::ShowOptions {
                depth: options.depth.unwrap_or(1),
                all: options.all,
                json,
                budget: budget(1200),
            };
            Ok(query::show::render(index, filter, node, &show))
        }
        CommandName::Impact => {
            let impact = query::impact::ImpactOptions {
                root,
                depth: options.depth.unwrap_or(3),
                json,
                budget: budget(1500),
            };
            if options.diff {
                if args.len() > 1 {
                    return Err(AppError::Invalid(
                        "impact --diff takes at most one REV".to_owned(),
                    ));
                }
                let rev = args.first().map_or("HEAD", String::as_str);
                return query::impact::render_diff(index, filter, rev, &impact);
            }
            if args.is_empty() {
                return Err(AppError::Invalid(
                    "impact needs NAME... or --diff [REV]".to_owned(),
                ));
            }
            let targets = query::impact::named(index, filter, args, json)?;
            Ok(query::impact::render_names(index, filter, targets, &impact))
        }
        CommandName::Audit => {
            let history = audit_history(root, &workspace.store, options.read_only);
            Ok(query::audit::render(
                index,
                filter,
                root,
                history.as_ref(),
                json,
                budget(2000),
            ))
        }
        CommandName::Path => {
            let from = query::names::resolve_one(index, filter, &args[0], json)?;
            let to = query::names::resolve_one(index, filter, &args[1], json)?;
            query::path::render(index, filter, from, to, json, budget(400)).ok_or_else(|| {
                AppError::NoMatch(format!("no path from '{}' to '{}'", args[0], args[1]))
            })
        }
        CommandName::Index
        | CommandName::Ls
        | CommandName::Clean
        | CommandName::Help
        | CommandName::Version
        | CommandName::Install
        | CommandName::Update => Ok(String::new()),
    }
}

/// Git history analyses cached by HEAD in the `git` store file; computed from the last 1000
/// non-merge commits on a miss and stored unless read-only. `None` without git.
fn audit_history(
    root: &std::path::Path,
    store: &crate::infra::store::Store,
    read_only: bool,
) -> Option<crate::domain::history::History> {
    let head = crate::infra::git::head(root)?;
    if let Some(history) = store.read_history(&head) {
        return Some(history);
    }
    let commits = crate::infra::git::history(root, 1000)?;
    let history = crate::domain::history::analyze(&commits);
    if !read_only
        && let Ok(_lock) =
            crate::infra::lock::WriterLock::acquire(&store.dir, crate::infra::lock::WAIT)
    {
        let _ = store.write_history(&head, &history);
    }
    Some(history)
}

/// Binary version, index format and installed skill version; a mismatch adds one notice line.
fn version() -> String {
    let mut out = format!(
        "xcalygraph {}\nindex format {}\n",
        install::VERSION,
        crate::infra::store::container::FORMAT_VERSION
    );
    if let Ok(home) = install::home() {
        match install::installed_version(&home) {
            Some(skill) => out.push_str(&format!("skill {skill}\n")),
            None => out.push_str("skill not installed\n"),
        }
        out.push_str(&install::mismatch_line(&home));
    }
    out
}

fn index(workspace: &Workspace) -> Result<String, AppError> {
    let report = refresh::refresh(workspace, Mode::Write)?;
    open_index(workspace, Mode::Write)?;
    workspace.store.mark_used();
    Ok(format!(
        "files {} changed {} deleted {}\n",
        report.files, report.changed, report.deleted
    ))
}
