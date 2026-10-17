import { root, suiteEnv as env, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory } from "../../scripts/build.mjs";
import assert from "node:assert/strict";
import { mkdir, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { chromium } from "@playwright/test";

const scratch = await realpath(await temporaryDirectory("fusor-libraries-"));
const app = join(scratch, "app");
const library = join(scratch, "panel");
const manifest = join(app, "Cargo.toml");
const executable = join(root, "target/debug", process.platform === "win32" ? "fusor.exe" : "fusor");
const offline = { ...env, CARGO_NET_OFFLINE: "true" };
const cli = (args, options = {}) => exec(executable, args, { cwd: app, env: offline, timeout: 300_000, ...options });
const crate = name => JSON.stringify(join(root, "crates", name));
let server, browser;

const write = async (path, contents) => {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, contents);
};
const style = (page, selector, property) =>
  page.locator(selector).first().evaluate((element, property) => getComputedStyle(element)[property], property);

const panelTemplate = heading => `<template rust:component="Panel">
  <section class="panel tracking-widest" class:skew-y-3="state.active.get()">
    <h2>${heading}</h2>
    <button class="toggle" on:click="state.active.update(|active| *active = !*active)">Toggle</button>
  </section>
</template>
`;
const appPage = title => `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Libraries</title><link rel="icon" href="data:,"></head>
<body>
  <App state="{{ App::new() }}">
    <main>
      <h1 id="title">${title}</h1>
      <Router>
        <Route path="/">
          <Panel></Panel>
          <div id="slot" rust:slot="state.slot.clone()"></div>
        </Route>
        <Route path="/panel/*"><PanelPages></PanelPages></Route>
        <Route path="/:chosen/*"><div id="chosen" rust:slot="state.pages.clone()"></div></Route>
      </Router>
    </main>
  </App>
</body>
</html>
`;

async function scaffold() {
  await exec(executable, ["new", app, "--framework-path", root, "--skip-install", "--yes"], { cwd: scratch, env, timeout: 120_000 });
  await rm(join(app, "src/counter.rs"));
  await rm(join(app, "web/components"), { recursive: true });
  await write(join(scratch, "Cargo.toml"), `[workspace]\nresolver = "3"\nmembers = ["app", "panel"]\n`);
  const generated = await readFile(manifest, "utf8");
  await write(manifest, generated
    .slice(0, generated.indexOf("[profile.release]"))
    .replace("[workspace]\n\n", "")
    .replace('wasm-bindgen =', `fusor-router = { path = ${crate("fusor-router")}, features = ["browser"] }\npanel = { path = "../panel" }\nwasm-bindgen =`)
    .replace('base-path = "/"', 'base-path = "/"\nhistory-fallback = ["/panel", "/chosen"]\ntailwind = "web/app.css"'));
  await write(join(app, "web/app.css"), '@import "tailwindcss";\n');
  await write(join(app, "src/lib.rs"), "mod app;\n");
  await write(join(app, "src/app.rs"), `use fusor::prelude::*;
use panel::{Panel, PanelPages};

struct App {
    slot: Content,
    pages: Content,
}

impl App {
    fn new() -> Self {
        Self {
            slot: Content::new(|owner| Panel::new(owner)),
            pages: Content::new(|_| PanelPages {}),
        }
    }
}

fusor::template!("web/index.html");
`);
  await write(join(app, "web/index.html"), appPage("Shell"));

  await write(join(library, "Cargo.toml"), `[package]
name = "panel"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"

[dependencies]
fusor-core = { path = ${crate("fusor-core")}, features = ["dom"] }
fusor-router = { path = ${crate("fusor-router")}, features = ["browser"] }

[build-dependencies]
fusor-build = { path = ${crate("fusor-build")} }

[package.metadata.fusor]
library = true
`);
  await write(join(library, "build.rs"), "fn main() -> Result<(), fusor_build::BuildError> {\n    fusor_build::compile_library()\n}\n");
  await write(join(library, "src/lib.rs"), "mod pages;\nmod panel;\n\npub use pages::PanelPages;\npub use panel::Panel;\n");
  await write(join(library, "src/panel.rs"), `use fusor::prelude::*;

#[derive(FromInputs)]
pub struct Panel {
    #[local(init = signal(false))]
    active: Signal<bool>,
}

impl Panel {
    pub fn new(_owner: OwnerHandle) -> Self {
        Self { active: signal(false) }
    }
}

fusor::template!("web/components/panel.html");
`);
  await write(join(library, "src/pages.rs"), `use fusor::prelude::*;

#[derive(FromInputs)]
pub struct PanelPages {}

fusor::template!("web/components/pages.html");
`);
  await write(join(library, "web/components/panel.html"), panelTemplate("Panel"));
  await write(join(library, "web/components/pages.html"), `<template rust:component="PanelPages">
  <section class="pages">
    <Router>
      <Route path="settings"><p id="panel-settings">Panel settings</p></Route>
      <Route fallback><p id="panel-home">Panel home</p></Route>
    </Router>
  </section>
</template>
`);
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: scratch, env });
}

async function checkPage(tab, url) {
  await tab.goto(`${url}/`);
  await tab.waitForFunction(() => document.querySelectorAll(".panel").length === 2);
  assert.equal(await tab.locator("#slot .panel").count(), 1, "the slot mounts the library's Content");
  assert.equal(await style(tab, ".panel", "letterSpacing"), "1.6px", "the library's static class is compiled");
  assert.equal(await style(tab, ".panel", "transform"), "none");
  await tab.locator(".toggle").first().click();
  await tab.waitForFunction(() => getComputedStyle(document.querySelector(".panel")).transform !== "none");
  await tab.goto(`${url}/panel/settings`);
  await tab.waitForSelector("#panel-settings");
  await tab.goto(`${url}/panel/elsewhere`);
  await tab.waitForSelector("#panel-home");
  // A library's Router in Content chosen at run time, under a captured /* route.
  await tab.goto(`${url}/chosen/settings`);
  await tab.waitForSelector("#chosen #panel-settings");
  await tab.goto(`${url}/chosen`);
  await tab.waitForSelector("#chosen #panel-home");
}

try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--offline", "--locked"], { cwd: root, env, timeout: 300_000 });
  await scaffold();
  // Preparation may download the pinned Tailwind binary; builds never do.
  await exec(executable, ["install", "--manifest-path", manifest], { cwd: app, env, timeout: 600_000 });
  await cli(["build", "--manifest-path", manifest]);
  const generation = (await readdir(join(app, "dist/__fusor")))[0];
  const [stylesheet] = (await readdir(join(app, "dist/__fusor", generation, "pkg"))).filter(name => name.endsWith(".css"));
  const css = await readFile(join(app, "dist/__fusor", generation, "pkg", stylesheet), "utf8");
  assert.match(css, /\.tracking-widest/, "Tailwind scans the library's HTML");
  assert.match(css, /\.skew-y-3/, "Tailwind receives the library's class: bindings");

  browser = await chromium.launch(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
  const tab = await browser.newPage();
  const errors = [];
  tab.on("pageerror", error => errors.push(error.message));
  const previewPort = await reservePort();
  server = startProcess(executable, ["preview", "--manifest-path", manifest, "--port", String(previewPort)], { cwd: app, env: offline });
  await waitUntil(() => server.output.includes(`localhost:${previewPort}`), "preview startup", { timeout: 60_000, process: server });
  await checkPage(tab, `http://127.0.0.1:${previewPort}`);
  await stopProcess(server);
  server = undefined;

  const devPort = await reservePort();
  server = startProcess(executable, ["dev", "--manifest-path", manifest, "--offline", "--port", String(devPort)], { cwd: app, env: offline });
  await waitUntil(() => server.output.includes("Ctrl+C to stop."), "development server startup", { timeout: 300_000, process: server });
  await checkPage(tab, `http://127.0.0.1:${devPort}`);
  await tab.goto(`http://127.0.0.1:${devPort}/`);
  await tab.waitForFunction(() => document.querySelectorAll(".panel").length === 2);
  await tab.evaluate(() => { window.keptState = true; });
  await writeFile(join(app, "web/index.html"), appPage("Edited shell"));
  await tab.waitForFunction(() => document.querySelector("#title")?.textContent === "Edited shell", null, { timeout: 120_000 });
  assert.equal(await tab.evaluate(() => window.keptState), true, "an application edit keeps the library's templates and the page state");
  await writeFile(join(library, "web/components/panel.html"), panelTemplate("Edited panel"));
  await tab.waitForFunction(() => document.querySelectorAll(".panel h2")[1]?.textContent === "Edited panel", null, { timeout: 300_000 });
  assert.deepEqual(errors, []);
  console.log("Component libraries: templates, Content slots, nested routes and Tailwind classes from a library crate, with application refresh and library rebuilds in fusor dev, passed");
} finally {
  await browser?.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
