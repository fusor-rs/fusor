import { root, env as buildEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory } from "../../scripts/build.mjs";
import assert from "node:assert/strict";
import { readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { chromium } from "@playwright/test";

const scratch = await realpath(await temporaryDirectory("fusor-tailwind-"));
const app = join(scratch, "tailwind-app");
const manifest = join(app, "Cargo.toml");
const executable = join(root, "target/debug", process.platform === "win32" ? "fusor.exe" : "fusor");
const env = { ...buildEnv, CARGO_TARGET_DIR: join(root, "target") };
const offline = { ...env, CARGO_NET_OFFLINE: "true" };
const cli = (args, options = {}) => exec(executable, args, { cwd: app, env: offline, timeout: 300_000, ...options });
const brand = "rgb(79, 70, 229)", idle = "rgb(100, 116, 139)", ring = "rgb(244, 63, 94)";
const stylesheet = `@import "tailwindcss";

@theme {
  --color-brand: #4f46e5;
  --color-idle: #64748b;
}
`;
let server, browser;

const style = (page, selector, property) =>
  page.locator(selector).evaluate((element, property) => getComputedStyle(element)[property], property);

try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--offline", "--locked"], { cwd: root, env, timeout: 300_000 });
  await exec(executable, ["new", app, "--framework-path", root, "--skip-install", "--yes"], { cwd: scratch, env, timeout: 120_000 });
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: app, env });
  await writeFile(manifest, (await readFile(manifest, "utf8")).replace('base-path = "/"', 'base-path = "/"\ntailwind = "web/app.css"'));
  await writeFile(join(app, "web/app.css"), stylesheet);
  await writeFile(join(app, "src/app.rs"), `use crate::counter::Counter;
use fusor::prelude::*;

struct App {
    count: Signal<i32>,
    active: Signal<bool>,
}

impl App {
    fn new() -> Self {
        Self { count: signal(0), active: signal(false) }
    }

    fn tone(&self) -> &'static str {
        if self.active.get() { "text-brand" } else { "text-idle" }
    }
}

fusor::template!("web/index.html");
`);
  const html = join(app, "web/index.html");
  const page = (title) => `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Tailwind</title><link rel="icon" href="data:,"></head>
<body>
  <App state="{{ App::new() }}">
    <main>
      <h1 id="title" class="${title}">Title</h1>
      <button id="toggle" class="rounded px-2" class:bg-brand="state.active.get()" class:md:px-8="state.active.get()" class:shadow-[0_0_0_3px_#f43f5e]="state.active.get()" on:click="state.active.update(|active| *active = !*active)">Toggle</button>
      <p id="status" class="{{ state.tone() }}">Status</p>
      <Counter count="{{ state.count.clone() }}"></Counter>
    </main>
  </App>
</body>
</html>
`;
  await writeFile(html, page("text-2xl"));
  await writeFile(join(app, "web/components/counter.html"), `<template rust:component="Counter">
  <section id="counter" class:underline="state.clicks.get() > 0">
    <button id="increment" on:click="{ state.count.update(|n| *n += 1); state.clicks.update(|n| *n += 1); }">Increment</button>
    <p>{{ state.count.get() }}</p>
  </section>
</template>
`);

  // Preparation may download the pinned binary; builds never do.
  await exec(executable, ["install", "--manifest-path", manifest], { cwd: app, env, timeout: 600_000 });
  await cli(["build", "--manifest-path", manifest]);
  const generation = (await readdir(join(app, "dist/__fusor")))[0];
  const built = (await readdir(join(app, "dist/__fusor", generation, "pkg"))).filter(name => /^tailwind-[0-9a-f]{16}\.css$/.test(name));
  assert.equal(built.length, 1, "the build publishes one content-named stylesheet");
  assert.match(await readFile(join(app, "dist/index.html"), "utf8"), new RegExp(`<link rel="stylesheet" href="/__fusor/${generation}/pkg/${built[0]}">`));

  const previewPort = await reservePort();
  server = startProcess(executable, ["preview", "--manifest-path", manifest, "--port", String(previewPort)], { cwd: app, env: offline });
  await waitUntil(() => server.output.includes(`127.0.0.1:${previewPort}`), "preview startup", { timeout: 60_000, process: server });
  browser = await chromium.launch(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
  const tab = await browser.newPage({ viewport: { width: 1024, height: 768 } });
  const errors = [];
  tab.on("pageerror", error => errors.push(error.message));
  await tab.goto(`http://127.0.0.1:${previewPort}/`);
  await tab.waitForFunction(() => document.querySelector("#status")?.className === "text-idle");
  assert.equal(await style(tab, "#title", "fontSize"), "24px");
  assert.equal(await style(tab, "#status", "color"), idle);
  assert.equal(await style(tab, "#toggle", "paddingLeft"), "8px");
  await tab.click("#toggle");
  await tab.waitForFunction(() => document.querySelector("#status")?.className === "text-brand");
  assert.equal(await style(tab, "#toggle", "backgroundColor"), brand);
  assert.equal(await style(tab, "#toggle", "paddingLeft"), "32px");
  assert.match(await style(tab, "#toggle", "boxShadow"), new RegExp(`${ring.replace(/[()]/g, "\\$&")} 0px 0px 0px 3px`));
  assert.equal(await style(tab, "#status", "color"), brand);
  assert.equal(await style(tab, "#counter", "textDecorationLine"), "none");
  await tab.click("#increment");
  await tab.waitForFunction(() => getComputedStyle(document.querySelector("#counter")).textDecorationLine === "underline");
  await stopProcess(server);
  server = undefined;

  await writeFile(join(app, "web/app.css"), `${stylesheet}.broken { @apply not-a-utility; }\n`);
  const failure = await cli(["build", "--manifest-path", manifest]).then(() => null, error => error);
  assert(failure, "an invalid stylesheet fails the build");
  assert.match(failure.message, /Cannot apply unknown utility class `not-a-utility`/);
  assert.doesNotMatch(failure.message, /\u001b\[/, "Tailwind's color codes are removed");
  await writeFile(join(app, "web/app.css"), stylesheet);

  const devPort = await reservePort();
  server = startProcess(executable, ["dev", "--manifest-path", manifest, "--offline", "--port", String(devPort)], { cwd: app, env: offline });
  await waitUntil(() => server.output.includes("Ctrl+C to stop."), "development server startup", { timeout: 300_000, process: server });
  await tab.goto(`http://127.0.0.1:${devPort}/`);
  await tab.waitForFunction(() => document.querySelector("#status")?.className === "text-idle");
  await tab.evaluate(() => { window.keptState = true; });
  // Each edit adds a utility the previous stylesheet lacks. The second one
  // proves an earlier refresh leaves the stylesheet link patchable.
  for (const [title, size] of [["text-5xl", "48px"], ["text-6xl", "60px"]]) {
    await writeFile(html, page(title));
    await tab.waitForFunction(size => getComputedStyle(document.querySelector("#title")).fontSize === size, size, { timeout: 120_000 });
    assert.equal(await tab.evaluate(() => window.keptState), true, `${title} refreshed without a reload`);
  }
  await writeFile(join(app, "web/app.css"), stylesheet.replace("#64748b", "#0f766e"));
  await tab.waitForFunction(() => getComputedStyle(document.querySelector("#status")).color === "rgb(15, 118, 110)", null, { timeout: 120_000 });
  assert.equal(await tab.evaluate(() => window.keptState), true, "a stylesheet edit refreshed without a reload");
  assert.deepEqual(errors, []);
  console.log("Tailwind CSS: pinned standalone binary, content-named stylesheet, class: bindings with variants and arbitrary values, classes chosen in Rust, readable compile errors and live refresh passed");
} finally {
  await browser?.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
