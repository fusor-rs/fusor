// An independent consumer of the exact published Three.js showcase files.
// Two real renderers verify ownership and keyed retention without changing the hero UI.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createServer } from "node:http";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { extname, join, resolve, sep } from "node:path";
import { chromium, firefox, webkit, expect } from "@playwright/test";

const exec = promisify(execFile);
const root = process.cwd();
const scratch = await mkdtemp(join(tmpdir(), "fusor-docs-libraries-"));
const suffix = process.platform === "win32" ? ".exe" : "";
const cli = join(root, `target/debug/fusor${suffix}`);
const env = { ...process.env, CARGO_NET_OFFLINE: "true", CARGO_TARGET_DIR: join(root, "target") };
let browser, server;
try {
  for (const folder of ["src/demos", "web/demos", "public"]) await mkdir(join(scratch, folder), { recursive: true });
  for (const file of ["src/demos/threejs.rs", "web/demos/threejs.html", "web/demos/threejs.js", "package.json", "package-lock.json"]) {
    await cp(join(root, "apps/docs", file), join(scratch, file));
  }
  await cp(join(root, "apps/docs/node_modules"), join(scratch, "node_modules"), { recursive: true, dereference: true });
  await cp(join(root, "apps/docs/public/showcase.css"), join(scratch, "public/showcase.css"));
  const rustPath = (name) => JSON.stringify(join(root, "crates", name).replaceAll("\\", "/"));
  await writeFile(join(scratch, "Cargo.toml"), `[package]
name = "docs-library-consumer"
version = "0.0.0"
edition = "2024"
rust-version = "1.85"
[workspace]
[lib]
crate-type = ["cdylib", "rlib"]
[dependencies]
fusor-core = { path = ${rustPath("fusor-core")}, features = ["dom", "javascript"] }
fusor-components = { path = ${rustPath("fusor-components")}, features = ["browser"] }
wasm-bindgen = "=0.2.117"
web-sys = { version = "=0.3.94", features = ["HtmlInputElement"] }
[build-dependencies]
fusor-build = { path = ${rustPath("fusor-build")} }
[package.metadata.fusor]
templates = ["web/demos"]
assets = "public"
`);
  await writeFile(join(scratch, "build.rs"), 'fn main() -> Result<(), fusor_build::BuildError> { fusor_build::compile_app() }\n');
  await writeFile(join(scratch, "src/lib.rs"), `mod app;
mod demos;
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn stop() -> Result<(), wasm_bindgen::JsValue> { fusor::dom::application::unmount() }
`);
  await writeFile(join(scratch, "src/demos/mod.rs"), "pub mod threejs;\n");
  await writeFile(join(scratch, "src/app.rs"), `use fusor::{Signal, signal};
use crate::demos::threejs::ThreeJs;
struct App { order: Signal<Vec<u32>> }
impl App { fn new() -> Self { Self { order: signal(vec![1, 2]) } } }
fusor::template!("web/index.html");
`);
  await writeFile(join(scratch, "web/index.html"), `<!doctype html><html><head><meta charset="utf-8"><link rel="stylesheet" href="/showcase.css"><style>body{margin:0;background:#0a1220;font-family:system-ui}main>button{margin:12px;padding:10px}.instances{display:grid;grid-template-columns:1fr 1fr}.garden-scene{height:380px}.garden-controls{padding:12px}.garden-heading h3{font-size:36px}</style></head><body>
<App state="{{ App::new() }}"><main>
<button id="reverse" on:click="state.order.update(|order| order.reverse())">Reverse</button>
<button id="remove" on:click="state.order.update(|order| order.retain(|id| *id != 1))">Remove first garden</button>
<button id="clear" on:click="state.order.set(Vec::new())">Clear</button>
<div class="instances"><ForEach items="{{ state.order.get() }}" key="{{ |id| *id }}"><section data-instance="{{ item.get() }}"><ThreeJs></ThreeJs></section></ForEach></div>
</main></App></body></html>`);
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: scratch });
  await exec(cli, ["build", "--debug", "--offline"], { cwd: scratch, env, timeout: 180_000, maxBuffer: 16 * 1024 * 1024 });
  const dist = join(scratch, "dist");
  server = createServer(async (request, response) => {
    const pathname = new URL(request.url, "http://localhost").pathname;
    if (pathname === "/favicon.ico") { response.writeHead(204).end(); return; }
    const file = resolve(dist, "." + (pathname === "/" ? "/index.html" : pathname));
    if (!file.startsWith(dist + sep)) { response.writeHead(404).end(); return; }
    try {
      response.setHeader("content-type", ({ ".js": "text/javascript", ".wasm": "application/wasm", ".html": "text/html", ".css": "text/css" })[extname(file)] || "text/plain");
      response.end(await readFile(file));
    } catch { response.writeHead(404).end(); }
  });
  await new Promise(done => server.listen(0, "127.0.0.1", done));
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    console.log(`Checking ${name}: two independent Three.js renderers`);
    browser = await ({ chromium, firefox, webkit })[name].launch({
      ...(name === "chromium" && process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {}),
      // Linux Firefox needs Xvfb (provided by CI) for these real WebGL 2 renderers.
      headless: !(name === "firefox" && process.platform === "linux"),
    });
    const page = await browser.newPage({ viewport: { width: 1500, height: 950 } });
    const errors = [];
    const recordError = (message) => {
      errors.push(message);
      console.error(`[${name}] ${message}`);
    };
    page.on("pageerror", error => recordError(error.message));
    page.on("console", message => { if (message.type() === "error") recordError(message.text()); });
    await page.addInitScript(() => {
      window.libraryTest = { contexts: [], frames: new Set(), observers: new Set(), listeners: new Set() };
      const get = HTMLCanvasElement.prototype.getContext;
      HTMLCanvasElement.prototype.getContext = function(type, ...args) {
        const context = get.call(this, type, ...args);
        if (context && /^(webgl|webgl2)$/.test(type) && !libraryTest.contexts.includes(context)) libraryTest.contexts.push(context);
        return context;
      };
      const request = requestAnimationFrame, cancel = cancelAnimationFrame;
      window.requestAnimationFrame = callback => {
        const id = request(time => { libraryTest.frames.delete(id); callback(time); });
        libraryTest.frames.add(id); return id;
      };
      window.cancelAnimationFrame = id => { libraryTest.frames.delete(id); cancel(id); };
      const Observer = ResizeObserver;
      window.ResizeObserver = class extends Observer {
        observe(...args) { libraryTest.observers.add(this); return super.observe(...args); }
        disconnect() { libraryTest.observers.delete(this); return super.disconnect(); }
      };
      const add = HTMLCanvasElement.prototype.addEventListener, remove = HTMLCanvasElement.prototype.removeEventListener;
      HTMLCanvasElement.prototype.addEventListener = function(type, listener, options) {
        libraryTest.listeners.add(listener); return add.call(this, type, listener, options);
      };
      HTMLCanvasElement.prototype.removeEventListener = function(type, listener, options) {
        libraryTest.listeners.delete(listener); return remove.call(this, type, listener, options);
      };
    });
    const url = `http://127.0.0.1:${server.address().port}/`;
    await page.goto(url);
    const first = page.locator('[data-instance="1"]'), second = page.locator('[data-instance="2"]');
    // Both renderers compile shaders and environment maps using software WebGL
    // on CI. Their startup can exceed the default five-second assertion timeout.
    await expect(page.locator(".library-status")).toHaveText(
      ["Three.js ready · a living Rust connection", "Three.js ready · a living Rust connection"],
      { timeout: 30_000 },
    );
    await expect.poll(() => page.evaluate(() => libraryTest.contexts.length)).toBe(2);
    await expect.poll(() => page.evaluate(() => libraryTest.frames.size)).toBe(2);
    await page.evaluate(() => {
      window.firstCanvas = document.querySelector('[data-instance="1"] canvas');
      window.secondCanvas = document.querySelector('[data-instance="2"] canvas');
    });
    await first.getByRole("slider", { name: "Bloom", exact: true }).focus();
    await page.keyboard.press("Home");
    await expect(first.locator(".garden-bloom output")).toHaveText("20%");
    await expect(second.locator(".garden-bloom output")).toHaveText("72%");
    await first.getByRole("button", { name: "Ember palette", exact: true }).click();
    await expect(second.getByRole("button", { name: "Aurora palette", exact: true })).toHaveAttribute("aria-pressed", "true");
    await first.locator("canvas").focus();
    await page.keyboard.press("ArrowRight");
    await expect(first.locator(".garden-selection")).toHaveText("PETAL 01 / SELECTED IN RUST");
    await expect(second.locator(".garden-selection")).toHaveText("CLICK A RIBBON TO EXPLORE");
    await page.locator("#reverse").click();
    await expect(page.locator(".instances > section").first()).toHaveAttribute("data-instance", "2");
    assert(await page.evaluate(() => firstCanvas === document.querySelector('[data-instance="1"] canvas') && secondCanvas === document.querySelector('[data-instance="2"] canvas')));
    assert.equal(await page.evaluate(() => libraryTest.contexts.length), 2, "Rust updates and keyed moves do not reconstruct renderers");
    await first.getByRole("button", { name: "Pause", exact: false }).click();
    await expect.poll(() => page.evaluate(() => libraryTest.frames.size)).toBe(1);
    await page.locator("#remove").click();
    await expect(first).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => libraryTest.contexts.filter(context => !context.isContextLost()).length)).toBe(1);
    assert(await page.evaluate(() => secondCanvas === document.querySelector('[data-instance="2"] canvas')));
    assert.equal(await page.evaluate(() => libraryTest.observers.size), 1);
    await second.getByRole("slider", { name: "Bloom", exact: true }).focus();
    await page.keyboard.press("End");
    await expect(second.locator(".garden-bloom output")).toHaveText("100%");
    await second.locator("canvas").focus();
    await page.keyboard.press("ArrowRight");
    await expect(second.locator(".garden-selection")).toHaveText("PETAL 01 / SELECTED IN RUST");
    await page.evaluate(async () => {
      const loader = document.querySelector('script[type="module"][src]').src;
      const app = await import(new URL("./pkg/app.js", loader).href);
      app.stop();
    });
    await expect.poll(() => page.evaluate(() => libraryTest.frames.size)).toBe(0);
    await expect.poll(() => page.evaluate(() => libraryTest.observers.size)).toBe(0);
    await expect.poll(() => page.evaluate(() => libraryTest.listeners.size)).toBe(0);
    await expect.poll(() => page.evaluate(() => libraryTest.contexts.every(context => context.isContextLost()))).toBe(true);

    // A deferred library import must not create a renderer after both owners leave.
    let release, requested;
    const gate = new Promise(done => { release = done; });
    const started = new Promise(done => { requested = done; });
    await page.route("**/three.module-*.js", async route => { requested(); await gate; await route.continue(); });
    await page.goto(url);
    await started;
    await page.locator("#clear").click();
    await expect(page.locator(".demo-threejs")).toHaveCount(0);
    const finished = page.waitForResponse(response => /three\.module-.*\.js$/.test(response.url()));
    release();
    await (await finished).finished();
    await page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
    assert.equal(await page.evaluate(() => libraryTest.contexts.length), 0, "late imports cannot revive disposed owners");
    assert.equal(await page.evaluate(() => libraryTest.frames.size), 0);
    assert.deepEqual(errors, []);
    console.log(`PASS ${name}: exact Three.js showcase source, two independent Rust inputs/events, retained keyed renderers, survivor lifetime, GPU/listener/frame disposal, and cancelled async setup`);
    await browser.close(); browser = null;
  }
} finally {
  await browser?.close();
  if (server) { server.closeAllConnections(); await new Promise(done => server.close(done)); }
  await rm(scratch, { recursive: true, force: true });
}
