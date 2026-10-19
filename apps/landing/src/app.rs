use crate::{
    code::CodeBlock,
    greeting::Greeting,
    project::{GROUPS, MADE_WITH, ProjectId, Shape},
};
use fusor::prelude::*;

const INSTALL_UNIX: &str = "curl -fsSL https://fusor.build/install.sh | sh";
const INSTALL_WINDOWS: &str = "irm https://fusor.build/install.ps1 | iex";

struct App {
    project: Signal<ProjectId>,
    flipped: Signal<bool>,
    install_windows: Signal<bool>,
    copy_label: Signal<&'static str>,
}

impl App {
    fn new() -> Self {
        Self {
            project: signal(ProjectId::Fusor),
            flipped: signal(false),
            install_windows: signal(
                web_sys::window()
                    .and_then(|window| window.navigator().user_agent().ok())
                    .is_some_and(|agent| agent.contains("Windows")),
            ),
            copy_label: signal("Copy command"),
        }
    }

    // A project with nothing on its back lands face up.
    fn select(&self, project: ProjectId) {
        fusor::batch(|| {
            self.project.set(project);
            if project.project().sources.is_empty() {
                self.flipped.set(false);
            }
        });
    }

    fn select_install(&self, windows: bool) {
        self.install_windows.set(windows);
        self.copy_label.set("Copy command");
    }

    fn install_command(&self) -> &'static str {
        if self.install_windows.get() {
            INSTALL_WINDOWS
        } else {
            INSTALL_UNIX
        }
    }

    fn copy_install(&self) {
        let label = self.copy_label.clone();
        let platform = self.install_windows.clone();
        let copied_platform = platform.get();
        let Some(window) = web_sys::window() else {
            return;
        };
        let promise = window
            .navigator()
            .clipboard()
            .write_text(self.install_command());
        wasm_bindgen_futures::spawn_local(async move {
            let copied = wasm_bindgen_futures::JsFuture::from(promise).await.is_ok();
            if platform.get() == copied_platform {
                label.set(if copied { "Copied!" } else { "Select to copy" });
            }
        });
    }
}

fusor::template!("web/index.html");
