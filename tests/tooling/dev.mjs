import { root, env as buildEnv, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory, copyProject } from "../../scripts/build.mjs";
// Compile edits made inside HTML and verify the development loop in isolation.
import assert from "node:assert/strict";
import { readFile, writeFile, rename, rm } from "node:fs/promises";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { chromium } from "@playwright/test";

const env = { ...buildEnv, CARGO_NET_OFFLINE: "true", CARGO_TARGET_DIR: join(root, "target/dev-tests") };
const scratch = await temporaryDirectory("fusor-html-");
const port = await reservePort();
const base = `http://127.0.0.1:${port}`;
let server;
let browser;

const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 60_000, interval: 150, process: server });

const version = async () => (await fetch(`${base}/__fusor/version`)).text();
const hashWasm = async () => {
  const generation = (await version()).split(":")[0];
  return createHash("sha256").update(Buffer.from(await (await fetch(`${base}/__fusor/${generation}/pkg/app_bg.wasm`)).arrayBuffer())).digest("hex");
};

try {
  for (const path of ["Cargo.toml", "Cargo.lock", ".cargo", "crates", "examples", "apps", "benchmarks/workloads"]) {
    await copyProject(join(root, path), join(scratch, path));
  }
  server = startProcess("cargo", ["run", "--locked", "-p", "fusor-cli", "--bin", "fusor", "--", "dev", "-p", "fusor-playground", "--offline", "--port", String(port)], {
    cwd: scratch,
    env,
  });

  await waitUntil(() => server.output.includes("watching for changes"), "initial build from HTML", { timeout: 180_000, interval: 150, process: server });
  browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || undefined });
  const page = await browser.newPage();
  // Bindings are empty until the Wasm mounts, and a click before then is lost.
  const mounted = (selector, text) => page.waitForFunction(([selector, text]) =>
    document.querySelector(selector)?.textContent === text, [selector, text], { timeout: 30_000 });
  // Let the development client reload; a second goto can race that navigation.
  // The generated loader identifies the exact published document to await.
  const waitForPublishedPage = async () => {
    if (page.url() === "about:blank") await page.goto(base);
    const generation = (await version()).split(":")[0];
    await page.waitForFunction(generation =>
      document.querySelector('script[type="module"][src$="/boot.js"]')?.src.includes(`/${generation}/`),
    generation, { timeout: 60_000 });
    await page.waitForLoadState("load");
  };
  const errors = [], reloadReasons = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", message => {
    if (message.text().startsWith("fusor reload:")) reloadReasons.push(message.text());
  });
  await waitForPublishedPage();
  await page.waitForSelector('[data-ready="true"]');
  assert.equal(await page.locator("#count").textContent(), "0");
  await page.waitForTimeout(900); // Establish the development client's version.

  const path = join(scratch, "examples/playground/web/index.html");
  // An in-place write exposes a truncated file to the watcher. If generation
  // then reads the completed save, the stale snapshot queues another refresh
  // after the browser has already shown that edit. Publish complete saves only.
  const saveHtml = async html => {
    const pending = join(scratch, ".fusor-dev-edit");
    await writeFile(pending, html);
    await rename(pending, path);
  };
  const source = await readFile(path, "utf8");
  const edited = source.replace("let count = signal(0_i32);", "let count = signal(7_i32);");
  assert.notEqual(edited, source);
  const firstVersion = await version();
  const reload = page.waitForEvent("load");
  await saveHtml(edited);
  await waitFor(async () => (await version()) !== firstVersion, "HTML Rust edit to rebuild");
  await reload;
  await page.waitForSelector('[data-ready="true"]');
  assert.equal(await page.locator("#count").textContent(), "7");
  assert.equal(await page.locator("#doubled").textContent(), "14");
  assert.equal(await page.locator('script[type="text/rust"]').count(), 0);
  console.log("PASS: Rust edited inside HTML recompiles, reloads, and changes browser state");

  // Playground metadata now participates in the CLI's static refresh contract.
  await page.locator("#increment").click();
  assert.equal(await page.locator("#count").textContent(), "8");
  await page.evaluate(() => { globalThis.retainedCounter = document.querySelector("#count"); });
  const beforeStatic = await version(), staticWasm = await hashWasm();
  const staticEdit = edited.replace('<h1 id="hero-title">', '<h1 id="hero-title" title="Refreshed">');
  assert.notEqual(staticEdit, edited);
  await saveHtml(staticEdit);
  await waitFor(async () => await page.locator("#hero-title").getAttribute("title") === "Refreshed", "playground static refresh");
  assert.equal((await version()).split(":")[0], beforeStatic.split(":")[0]);
  assert.equal(await hashWasm(), staticWasm);
  assert.equal(await page.locator("#count").textContent(), "8", reloadReasons.join("\n"));
  assert(await page.evaluate(() => retainedCounter === document.querySelector("#count")));
  await saveHtml(edited);
  await waitFor(async () => await page.locator("#hero-title").getAttribute("title") === null, "restoring static HTML");
  assert.equal(await page.locator("#count").textContent(), "8");
  console.log("PASS: playground static HTML refresh preserves Wasm, state and native node identity");

  const goodVersion = await version();
  const goodWasm = await hashWasm();
  const counterTag = '<script type="text/rust" id="counter-rust">';
  const blockStart = edited.indexOf(counterTag);
  assert.notEqual(blockStart, -1);
  const bodyStart = blockStart + counterTag.length;
  const broken = edited.slice(0, bodyStart)
    + '\nconst EXPECTED_HTML_FAILURE: i32 = "not an integer";\n'
    + edited.slice(bodyStart);
  const errorLine = broken.slice(0, broken.indexOf("const EXPECTED_HTML_FAILURE")).split("\n").length;
  server.output = "";
  await saveHtml(broken);
  await waitFor(() => server.output.includes("Keeping the last successful build."), "expected compiler failure");
  assert.equal(await version(), goodVersion);
  assert.equal(await hashWasm(), goodWasm);
  assert.ok(server.output.includes(`examples/playground/web/index.html:${errorLine}:`), server.output);
  assert.ok(server.output.includes("mismatched types"), server.output);
  console.log("PASS: rustc errors report the HTML line and preserve the last working build");

  await saveHtml(edited);
  await waitFor(async () => (await version()) !== goodVersion, "recovery after fixing HTML Rust");
  console.log("PASS: fixing the Rust inside HTML recovers the development server");

  const beforeBindingError = await version();
  const bindingWasm = await hashWasm();
  const outputBinding = '<output id="doubled">{{ state.doubled.get() }}</output>';
  const badBinding = edited.replace(outputBinding,
    '<output id="doubled">{{ state.missing_field.get() }}</output>');
  assert.notEqual(badBinding, edited);
  const bindingLine = badBinding.slice(0, badBinding.indexOf("state.missing_field")).split("\n").length;
  server.output = "";
  await saveHtml(badBinding);
  await waitFor(() => server.output.includes("Keeping the last successful build."), "binding expression compiler error");
  assert.equal(await version(), beforeBindingError);
  assert.equal(await hashWasm(), bindingWasm);
  assert.ok(server.output.includes(`HTML binding at examples/playground/web/index.html:${bindingLine}:`), server.output);
  assert.ok(server.output.includes("no field `missing_field`"), server.output);
  console.log("PASS: Rust expressions on HTML elements are checked by rustc with HTML source locations");

  const changedBinding = edited.replace(outputBinding,
    '<output id="doubled">{{ state.doubled.get() + 1 }}</output>');
  await saveHtml(changedBinding);
  await waitFor(async () => (await version()) !== beforeBindingError, "HTML interpolation edit to rebuild");
  await waitForPublishedPage();
  await page.waitForSelector('[data-ready="true"]');
  assert.equal(await page.locator("#doubled").textContent(), "15");
  console.log("PASS: changing an HTML interpolation rebuilds its reactive Wasm binding");

  // A complete HTML-authored application that relies on none of the
  // playground's Rust definitions.
  const minimal = `<!doctype html>
<html lang="en">
  <head><meta charset="utf-8"><title>Rust in HTML</title></head>
  <body>
    <App state="{{ Counter { count: signal(0) } }}">
<section id="counter">
      <output>{{ state.count.get() }}</output>
      <button on:click="state.count.update(|n| *n += 1)">Increment</button>
    </section>
</App>

    <script type="text/rust">
use fusor::prelude::*;

struct Counter {
    count: Signal<i32>,
}
    </script>
  </body>
</html>`;
  const beforeMinimal = await version();
  await saveHtml(minimal);
  await waitFor(async () => (await version()) !== beforeMinimal, "minimal HTML example to compile");
  await waitForPublishedPage();
  await mounted("#counter output", "0");
  await page.locator("#counter button").click();
  assert.equal(await page.locator("#counter output").textContent(), "1");
  console.log("PASS: a standalone HTML application compiles and runs");

  const fixture = minimal.replace("</section>", `
      <input id="property" value="Count: {{ state.count.get() }}">
      <p id="entity">{{ if state.count.get() &lt; 2 { "small &amp; safe" } else { "large" } }}</p>
      <p id="delimiter">{{ format!("{} {}", state.count.get(), r#"}}"#) }}</p>
      <button id="reset" disabled="{{ state.count.get() == 0 }}" on:click="state.count.set(0)">Reset</button>
    </section>`);
  const beforeFixture = await version();
  await saveHtml(fixture);
  await waitFor(async () => (await version()) !== beforeFixture, "binding semantics fixture");
  await waitForPublishedPage();
  await mounted("#counter output", "0");
  assert.equal(await page.locator("#entity").textContent(), "small & safe");
  assert.equal(await page.locator("#delimiter").textContent(), "0 }}");
  assert.equal(await page.locator("#reset").isDisabled(), true);
  await page.locator("#property").fill("User edit");
  await page.locator("#counter button").first().click();
  assert.equal(await page.locator("#property").inputValue(), "Count: 1");
  assert.equal(await page.locator("#reset").isDisabled(), false);
  await page.locator("#counter button").first().click();
  assert.equal(await page.locator("#entity").textContent(), "large");
  assert.equal(await page.locator("#delimiter").textContent(), "2 }}");
  console.log("PASS: HTML entities, raw Rust strings, live input properties, and boolean removal work in Wasm");

  const templateMarkup = minimal.match(/<section[\s\S]*?<\/section>/)[0]
    .replace(' rust:component="Counter"', '');
  const templateScript = minimal.match(/<script type="text\/rust">[\s\S]*?<\/script>/)[0]
    .replace('</script>', 'struct App;\nstruct CounterInputs {}\nimpl fusor::dom::FromInputs for Counter { type Error = fusor::dom::JsValue; type Inputs = CounterInputs; fn from_inputs(_: Self::Inputs, _: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> { Ok(Self { count: signal(0) }) } }\n</script>');
  const templatePage = `<!doctype html><html><head><title>Template component</title></head>
    <body><App state="{{ App }}"><div id="host"><Counter></Counter></div></App><template rust:component="Counter">
    ${templateMarkup}${templateScript}</template></body></html>`;
  const beforeTemplate = await version();
  await saveHtml(templatePage);
  await waitFor(async () => (await version()) !== beforeTemplate, "Rust authored inside a template");
  await waitForPublishedPage();
  await mounted("#host output", "0");
  await page.locator("#host button").click();
  assert.equal(await page.locator("#host output").textContent(), "1");
  assert.equal(await page.locator('script[type="module"][src$="/boot.js"]').count(), 1);
  console.log("PASS: Rust authored inside an HTML template gets an active module loader and reactive cloned markup");

  const beforeTypeError = await version();
  server.output = "";
  await saveHtml(fixture.replace('disabled="{{ state.count.get() == 0 }}"', 'disabled="{{ state.count.get() }}"'));
  await waitFor(() => server.output.includes("Keeping the last successful build."), "boolean binding type error");
  assert.equal(await version(), beforeTypeError);
  assert.ok(server.output.includes("expected `bool`, found `i32`"), server.output);
  assert.ok(server.output.includes("HTML binding at examples/playground/web/index.html:"), server.output);
  assert.deepEqual(errors, []);
  console.log("PASS: HTML boolean properties reject non-boolean Rust expressions at compile time");
} catch (error) {
  console.error(server?.output || "Development server did not start");
  throw error;
} finally {
  if (browser) await browser.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
