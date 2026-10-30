//! Use cases: one per command.

pub mod error;
pub mod install;
pub mod query;
pub mod refresh;
pub mod update;
pub mod workspace;

use crate::cli::{CommandName, Invocation};
use error::AppError;
use refresh::Mode;
use workspace::Workspace;

/// Runs a command that needs the index and returns its stdout text.
pub fn run(invocation: &Invocation) -> Result<String, AppError> {
    let options = &invocation.options;
    match invocation.command {
        CommandName::Version => return Ok(version()),
        CommandName::Install => {
            let home = install::home()?;
            return install::install(&home, options.link.is_some());
        }
        CommandName::Update => {
            let mut out = update::update(options.check)?;
            if let Ok(home) = install::home() {
                out.push_str(&install::mismatch_line(&home));
            }
            return Ok(out);
        }
        _ => {}
    }
    let workspace = Workspace::resolve(&invocation.options)?;
    match invocation.command {
        CommandName::Index => index(&workspace, options.drop),
        CommandName::Help | CommandName::Version | CommandName::Install | CommandName::Update => {
            Ok(String::new())
        }
        command => {
            let mode = if options.read_only {
                Mode::ReadOnly
            } else {
                Mode::Write
            };
            refresh::refresh(&workspace, mode)?;
            let index = match query::open(&workspace.store) {
                Ok(index) => index,
                // A corrupt derived file (checksum) is rebuilt from the facts once.
                Err(_) if mode == Mode::Write => {
                    refresh::rederive(&workspace)?;
                    query::open(&workspace.store)?
                }
                Err(error) => return Err(error),
            };
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
                &workspace.root,
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
    root: &std::path::Path,
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
        CommandName::Path => {
            let from = query::names::resolve_one(index, filter, &args[0], json)?;
            let to = query::names::resolve_one(index, filter, &args[1], json)?;
            query::path::render(index, filter, from, to, json, budget(400)).ok_or_else(|| {
                AppError::NoMatch(format!("no path from '{}' to '{}'", args[0], args[1]))
            })
        }
        CommandName::Index
        | CommandName::Help
        | CommandName::Version
        | CommandName::Install
        | CommandName::Update => Ok(String::new()),
    }
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
