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
    let mode = if options.read_only && invocation.command != CommandName::Index {
        Mode::ReadOnly
    } else {
        Mode::Write
    };
    let (workspace, report, index) = prepare_workspace(workspace, mode)?;
    if mode == Mode::Write {
        workspace.store.mark_used();
    }
    match invocation.command {
        CommandName::Index => Ok(format!(
            "files {} changed {} deleted {}\n",
            report.files, report.changed, report.deleted
        )),
        CommandName::Help
        | CommandName::Version
        | CommandName::Install
        | CommandName::Update
        | CommandName::Ls
        | CommandName::Clean => Ok(String::new()),
        command => {
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

/// Selects a local index only when an implicit shared index is actually unusable.
fn prepare_workspace(
    workspace: Workspace,
    mode: Mode,
) -> Result<(Workspace, refresh::Report, query::Index), AppError> {
    match prepare_index(&workspace, mode) {
        Ok((report, index)) => Ok((workspace, report, index)),
        Err(error) => {
            let retry = if mode == Mode::Write {
                error.is_index_location_failure()
            } else {
                matches!(
                    error,
                    AppError::Stale(_) | AppError::Invalid(_) | AppError::IndexIo(..)
                )
            };
            let Some(local) = workspace.local_fallback().filter(|_| retry) else {
                return Err(error);
            };
            match prepare_index(&local, mode) {
                Ok((report, index)) => {
                    if mode == Mode::Write {
                        eprintln!(
                            "xcalygraph: index in {} (shared index unavailable); add {}/ to .gitignore",
                            local.root.join(workspace::LOCAL_DIR).display(),
                            workspace::LOCAL_DIR
                        );
                    }
                    Ok((local, report, index))
                }
                Err(local_error)
                    if mode == Mode::ReadOnly
                        && matches!(
                            local_error,
                            AppError::Stale(_) | AppError::Invalid(_) | AppError::IndexIo(..)
                        ) =>
                {
                    Err(error)
                }
                Err(local_error) => Err(local_error),
            }
        }
    }
}

fn prepare_index(
    workspace: &Workspace,
    mode: Mode,
) -> Result<(refresh::Report, query::Index), AppError> {
    if mode == Mode::Write {
        workspace.prepare_base()?;
    }
    let report = refresh::refresh(workspace, mode)?;
    // Building a new index is the moment a shared base grows: clear what is left of gone roots.
    if report.rebuilt
        && let Some(base) = &workspace.default_base
    {
        indexes::prune_orphans(base);
    }
    let index = open_index(workspace, mode)?;
    Ok((report, index))
}

/// Opens the derived image and, only for writable operations, repairs it once from facts.
/// Under `--read-only` a missing or invalid image needs a rebuild, so it is stale (exit 5).
fn open_index(workspace: &Workspace, mode: Mode) -> Result<query::Index, AppError> {
    match query::open(&workspace.store) {
        Ok(index) => Ok(index),
        Err(AppError::Invalid(_)) if mode == Mode::Write => {
            refresh::rederive(workspace)?;
            query::open(&workspace.store)
        }
        Err(AppError::Invalid(_)) => Err(AppError::Stale("index corrupt; run 'xcalygraph index'")),
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
    let ambiguity = budget(query::names::AMBIGUITY_BUDGET).min(query::names::AMBIGUITY_BUDGET);
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
            Ok(query::find::render_answer(
                index,
                filter,
                root,
                &hits,
                &terms,
                json,
                budget(600),
            ))
        }
        CommandName::Ctx => {
            let terms = args.join(" ");
            query::ctx::render(index, filter, &terms, json, budget(2000))
                .ok_or_else(|| AppError::NoMatch(format!("no match for '{terms}'")))
        }
        CommandName::Show => {
            let node = query::names::resolve_one(index, filter, &args[0], json, ambiguity)?;
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
                store: &workspace.store,
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
                return match query::impact::render_diff(index, filter, rev, &impact) {
                    Err(AppError::RecoverFacts(_)) if options.read_only => Err(AppError::Stale(
                        "index facts damaged; run 'xcalygraph index'",
                    )),
                    Err(AppError::RecoverFacts(_)) => {
                        refresh::rederive(workspace)?;
                        let recovered = open_index(workspace, Mode::Write)?;
                        query::impact::render_diff(&recovered, filter, rev, &impact)
                    }
                    result => result,
                };
            }
            if args.is_empty() {
                return Err(AppError::Invalid(
                    "impact needs NAME... or --diff [REV]".to_owned(),
                ));
            }
            let targets = query::impact::named(index, filter, args, json, ambiguity)?;
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
            let from = query::names::resolve_one(index, filter, &args[0], json, ambiguity)?;
            let to = query::names::resolve_one(index, filter, &args[1], json, ambiguity)?;
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

/// Git history analyses cached by HEAD and analysis version; computed from the last 1000
/// non-merge commits on a miss and stored unless read-only. `None` without git.
fn audit_history(
    root: &std::path::Path,
    store: &crate::infra::store::Store,
    read_only: bool,
) -> Option<crate::domain::history::History> {
    let head = format!("2:{}", crate::infra::git::head(root)?);
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

#[cfg(test)]
mod tests {
    mod support {
        include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/mod.rs"));
    }

    use std::error::Error;
    use std::fs;

    use super::audit_history;
    use crate::domain::history::History;
    use crate::infra::{git, store::Store, testdir::TestDir};

    #[test]
    fn history_test_does_not_inherit_a_foreign_git_repository() -> Result<(), Box<dyn Error>> {
        let fixture = TestDir::new("history-foreign-environment")?;
        let foreign = fixture.path().join("foreign");
        fs::create_dir(&foreign)?;
        let initialized = support::command("git", fixture.path())
            .args(["init", "-q"])
            .arg(&foreign)
            .status()?;
        assert!(initialized.success());
        let configuration = foreign.join(".git/config");
        let before = fs::read(&configuration)?;
        let index = foreign.join("foreign-index");
        let output = support::command(std::env::current_exe()?, fixture.path())
            .args([
                "--exact",
                "app::tests::audit_recomputes_old_history_cache_without_writing_in_read_only_mode",
            ])
            .env("GIT_DIR", foreign.join(".git"))
            .env("GIT_WORK_TREE", &foreign)
            .env("GIT_INDEX_FILE", &index)
            .output()?;
        assert!(!index.exists(), "foreign Git index was created");
        assert_eq!(fs::read(configuration)?, before);
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn audit_recomputes_old_history_cache_without_writing_in_read_only_mode()
    -> Result<(), Box<dyn Error>> {
        let foreign_git = std::env::vars_os().any(|(name, value)| {
            let name = name.to_string_lossy();
            match name.as_ref() {
                "GIT_CONFIG_GLOBAL" => value != "/dev/null",
                "GIT_CONFIG_NOSYSTEM" => value != "1",
                _ => name.starts_with("GIT_"),
            }
        });
        let configured = std::env::var_os("GIT_CONFIG_GLOBAL").is_some_and(|v| v == "/dev/null")
            && std::env::var_os("GIT_CONFIG_NOSYSTEM").is_some_and(|v| v == "1");
        if std::env::var_os("XCALYGRAPH_HISTORY_TEST_CHILD").is_none() || foreign_git || !configured
        {
            let fixture = TestDir::new("history-isolated-child")?;
            let output = support::command(std::env::current_exe()?, fixture.path())
                .args(["--exact", "app::tests::audit_recomputes_old_history_cache_without_writing_in_read_only_mode"])
                .env("XCALYGRAPH_HISTORY_TEST_CHILD", "1")
                .output()?;
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Ok(());
        }
        let dir = TestDir::new("history-cache-version")?;
        let root = dir.path().join("repo");
        dir.write("repo/ü.go", "package sample\nfunc One() {}\n")?;
        for args in [
            &["init", "-q"][..],
            &["add", "."],
            &["commit", "-q", "-m", "initial"],
        ] {
            let status = support::command("git", dir.path())
                .arg("-C")
                .arg(&root)
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .status()?;
            assert!(status.success(), "git {args:?}");
        }
        let store = Store::new(dir.path().join("index"));
        fs::create_dir_all(&store.dir)?;
        let head = git::head(&root).ok_or("missing HEAD")?;
        store.write_history(&head, &History::default())?;
        let before = fs::read(store.dir.join("git"))?;
        let history = audit_history(&root, &store, true).ok_or("missing read-only history")?;
        assert_eq!(history.churn.get("ü.go"), Some(&1));
        assert_eq!(fs::read(store.dir.join("git"))?, before);
        let history = audit_history(&root, &store, false).ok_or("missing writable history")?;
        assert_eq!(history.churn.get("ü.go"), Some(&1));
        assert_ne!(fs::read(store.dir.join("git"))?, before);
        Ok(())
    }
}
