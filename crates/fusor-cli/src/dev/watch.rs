//! Cheapest first: patch HTML and CSS in place, recompile, or report the
//! failure and keep serving.
use super::{refresh, sources};
use crate::{
    context::Context,
    error::{Error, Result},
    workspace::Project,
};
use std::{
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

const INTERVAL: Duration = Duration::from_millis(150);

/// Keeps `current` pointed at the project the server answers from.
pub(crate) fn start(cx: &Context, project: Project, current: Arc<RwLock<Project>>) -> Result {
    let mut previous = sources::snapshot(cx, &project)?;
    let cx = cx.clone();
    thread::spawn(move || {
        let mut project = project;
        loop {
            thread::sleep(INTERVAL);
            let next = match sources::snapshot(&cx, &project) {
                Ok(next) => next,
                Err(error) => {
                    cx.reporter.warn(format!("watching sources: {error}"));
                    continue;
                }
            };
            if next == previous {
                continue;
            }
            let changed: Vec<_> = next
                .keys()
                .chain(previous.keys())
                .filter(|path| next.get(*path) != previous.get(*path))
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            previous = next;
            cx.reporter.blank();
            cx.reporter.step(format!("Changed {}", changes(&changed)));
            match refresh::try_refresh(&cx, &project, &changed) {
                Ok(true) => continue,
                Ok(false) => {}
                Err(error) => {
                    report(&cx, &project, &error);
                    continue;
                }
            }
            match rebuild(&cx, &project) {
                Ok(next) => {
                    project = next;
                    // Keep the pre-build snapshot: an edit made while Cargo
                    // ran must trigger another build.
                    *current.write().expect("preview state") = project.clone();
                }
                Err(error) => report(&cx, &project, &error),
            }
        }
    });
    Ok(())
}

fn rebuild(cx: &Context, project: &Project) -> Result<Project> {
    let next = project.rediscover(cx)?;
    if next.config.base_path != project.config.base_path {
        return Err(Error::project("base-path changed")
            .remedy("restart `fusor dev` to serve from the new URL"));
    }
    crate::commands::build::run(cx, &next, crate::pipeline::BuildMode::Development)?;
    Ok(next)
}

fn report(cx: &Context, project: &Project, error: &Error) {
    cx.reporter.fail(format!(
        "{} failed to build. Keeping the last successful build.\n{error}",
        project.name
    ));
}

/// `web/app.html`, or `web/app.html and 2 more`, relative to where `dev` runs.
fn changes(changed: &[std::path::PathBuf]) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let Some(first) = changed.first() else {
        return "sources".into();
    };
    let first = first.strip_prefix(&cwd).unwrap_or(first).display();
    match changed.len() {
        1 => first.to_string(),
        n => format!("{first} and {} more", n - 1),
    }
}
