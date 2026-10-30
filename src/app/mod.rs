//! Use cases: one per command.

pub mod error;
pub mod query;
pub mod refresh;
pub mod workspace;

use crate::cli::{CommandName, Invocation};
use error::AppError;
use refresh::Mode;
use workspace::Workspace;

/// Runs a command that needs the index and returns its stdout text.
pub fn run(invocation: &Invocation) -> Result<String, AppError> {
    let workspace = Workspace::resolve(&invocation.options)?;
    let options = &invocation.options;
    match invocation.command {
        CommandName::Index => index(&workspace, options.drop),
        CommandName::Help | CommandName::Version => Ok(String::new()),
        command => {
            let mode = if options.read_only {
                Mode::ReadOnly
            } else {
                Mode::Write
            };
            refresh::refresh(&workspace, mode)?;
            let index = query::open(&workspace.store)?;
            let filter = query::Filter::new(options)?;
            let root_name = workspace.root.file_name().map_or_else(
                || "root".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            run_query(
                &index,
                &filter,
                command,
                &invocation.args,
                options,
                &root_name,
            )
        }
    }
}

fn run_query(
    index: &query::Index,
    filter: &query::Filter,
    command: CommandName,
    args: &[String],
    options: &crate::cli::Options,
    root_name: &str,
) -> Result<String, AppError> {
    let json = options.json;
    let budget = |default: usize| options.budget.unwrap_or(default);
    match command {
        CommandName::Map => Ok(query::map::render(
            index,
            filter,
            root_name,
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
        CommandName::Path => {
            let from = query::names::resolve_one(index, filter, &args[0], json)?;
            let to = query::names::resolve_one(index, filter, &args[1], json)?;
            query::path::render(index, filter, from, to, json, budget(400)).ok_or_else(|| {
                AppError::NoMatch(format!("no path from '{}' to '{}'", args[0], args[1]))
            })
        }
        CommandName::Index | CommandName::Help | CommandName::Version => Ok(String::new()),
    }
}

fn index(workspace: &Workspace, drop: bool) -> Result<String, AppError> {
    if drop {
        let dropped = workspace.store.drop_all().map_err(AppError::io(format!(
            "drop index {}",
            workspace.store.dir.display()
        )))?;
        return Ok(if dropped {
            "index dropped\n"
        } else {
            "no index\n"
        }
        .to_owned());
    }
    let report = refresh::refresh(workspace, Mode::Write)?;
    Ok(format!(
        "files {} changed {} deleted {}\n",
        report.files, report.changed, report.deleted
    ))
}
