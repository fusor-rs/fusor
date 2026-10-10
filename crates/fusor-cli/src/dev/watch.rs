//! Cheapest first: patch HTML and CSS in place, recompile, or report the
//! failure and keep serving.
use super::{refresh, sources};
use crate::{
    context::Context,
    error::{Error, Result},
    reporter::Reporter,
    workspace::Project,
};
use std::{
    sync::{Arc, RwLock, mpsc},
    thread,
    time::Duration,
};

const INTERVAL: Duration = Duration::from_millis(150);

pub(crate) struct Watcher {
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
    reporter: Reporter,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        // A disconnected channel means the watcher has already exited.
        let _ = self.stop.send(());
        if self
            .thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            self.reporter.fail("the source watcher panicked");
        }
    }
}

/// Keeps `current` pointed at the project the server answers from.
pub(crate) fn start(
    cx: &Context,
    project: Project,
    current: Arc<RwLock<Project>>,
) -> Result<Watcher> {
    let previous = sources::snapshot(cx, &project)?;
    let reporter = cx.reporter.clone();
    let cx = cx.clone();
    let (stop, stopped) = mpsc::channel();
    let thread = thread::spawn(move || watch(cx, project, current, stopped, previous));
    Ok(Watcher {
        stop,
        thread: Some(thread),
        reporter,
    })
}

fn watch(
    cx: Context,
    mut project: Project,
    current: Arc<RwLock<Project>>,
    stopped: mpsc::Receiver<()>,
    mut previous: sources::Snapshot,
) {
    while matches!(
        stopped.recv_timeout(INTERVAL),
        Err(mpsc::RecvTimeoutError::Timeout)
    ) {
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
        match refresh::try_refresh(&cx, &project, &changed, &mut previous) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                report(&cx, &project, &error);
                continue;
            }
        }
        match rebuild(&cx, &project, &mut previous) {
            Ok(next) => {
                project = next;
                // Keep the pre-build source snapshot: an edit made while Cargo
                // ran must trigger another build.
                *current.write().expect("preview state") = project.clone();
            }
            Err(error) => report(&cx, &project, &error),
        }
    }
}

fn rebuild(cx: &Context, project: &Project, watched: &mut sources::Snapshot) -> Result<Project> {
    let next = project.rediscover(cx)?;
    if next.config.base_path != project.config.base_path {
        return Err(Error::project("base-path changed")
            .remedy("restart `fusor dev` to serve from the new URL"));
    }
    crate::commands::build::run(
        cx,
        &next,
        crate::pipeline::BuildMode::Development,
        Some(watched),
    )?;
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
