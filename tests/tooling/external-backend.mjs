// Public consumer contract: execute native generated code, compile the same
// component for the browser, execute it across a package boundary, and translate rustc diagnostics through the
// versioned backend output manifest and source-map contracts.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { promisify } from "node:util";
import { chromium, firefox, webkit, expect } from "@playwright/test";
import { reservePort, startProcess, stopProcess, waitFor } from "../../scripts/build.mjs";

const exec = promisify(execFile);
const root = process.cwd();
const fixture = join(root, "tests/fixtures/external-backend");
const scratch = await mkdtemp(join(tmpdir(), "fusor-external-backend-"));
const cargo = process.env.CARGO || "cargo";
const env = { ...process.env, CARGO_TARGET_DIR: join(root, "target/external-backend-tests") };
const manifest = join(scratch, "Cargo.toml");
let server, browser;

async function browserConsumer() {
  const app = join(scratch, "browser");
  await mkdir(join(app, "src"), { recursive: true });
  await mkdir(join(app, "web"));
  const dep = name => JSON.stringify(join(root, "crates", name));
  await writeFile(join(app, "Cargo.toml"), `[package]
name="external-backend-browser"
version="0.0.0"
edition="2024"
[workspace]
[lib]
crate-type=["cdylib","rlib"]
[dependencies]
fusor-core={path=${dep("fusor-core")},features=["dom","derive"]}
fusor-components={path=${dep("fusor-components")},features=["browser"]}
fixture-components={path="../components",features=["browser"]}
wasm-bindgen="=0.2.117"
[build-dependencies]
fusor-build={path=${dep("fusor-build")}}
[package.metadata.fusor]
entry="web/index.html"
`);
  await writeFile(join(app, "build.rs"), "fn main()->Result<(),fusor_build::BuildError>{fusor_build::compile_app()}\n");
  await writeFile(join(app, "src/lib.rs"), `use fusor::{FromInputs, Signal, signal};
use fixture_components::{Panel, Row};
use std::{cell::Cell, rc::Rc};
#[derive(FromInputs)] struct Local {}
struct App { visible: Signal<bool>, number: Signal<i32>, checked: Signal<bool>, observed: Signal<i32>, rows: Signal<Vec<Row>>, cleanups: Rc<Cell<usize>> }
impl App { fn new() -> Self { Self { visible: signal(true), number: signal(1), checked: signal(false), observed: signal(0), rows: signal(Vec::new()), cleanups: Rc::new(Cell::new(0)) } } }
fusor::template!("web/index.html");
`);
  // Local and the independently compiled Panel both have component ID zero.
  await writeFile(join(app, "web/index.html"), `<!doctype html><html><head><title>Library consumer</title></head><body>
<template rust:component="Local"><p id="local">Application template</p></template>
<App state="{{ App::new() }}"><main><Local></Local><Panel title='{{ "Library".to_owned() }}' visible="{{ state.visible.clone() }}" rows="{{ state.rows.clone() }}" cleanups="{{ state.cleanups.clone() }}" number="{{ state.number.clone() }}" checked="{{ state.checked.clone() }}" observed="{{ state.observed.clone() }}"></Panel></main></App>
</body></html>`);
  const cli = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
  await exec(cargo, ["build", "-p", "fusor-cli", "--offline", "--locked"], { cwd: root, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
  await exec(cargo, ["generate-lockfile", "--offline"], { cwd: app, env });
  await exec(cli, ["build", "--debug", "--offline", "--locked"], { cwd: app, env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
  const port = await reservePort();
  server = startProcess(cli, ["preview", "--port", String(port), "--offline", "--locked"], { cwd: app, env });
  await waitFor(() => server.output.includes("Ctrl+C to stop."), "library preview startup", { timeout: 30_000, process: server });
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    browser = await ({ chromium, firefox, webkit })[name].launch(name === "chromium" && process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
    const page = await browser.newPage(), errors = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await page.goto(`http://127.0.0.1:${port}/`);
    await expect(page.locator("#local")).toHaveText("Application template");
    await expect(page.locator("#increment")).toHaveText("Library: 0");
    await page.locator("#increment").click();
    await expect(page.locator("#increment")).toHaveText("Library: 1");
    await expect(page.locator("#projected")).toHaveText("Library");
    await page.locator(".child button:not(.remove)").click();
    await expect(page.locator(".child button:not(.remove)")).toHaveText("Library clicks 1");
    await page.locator(".child .remove").click();
    await expect(page.locator(".child")).toHaveCount(0);
    assert.deepEqual(errors, []);
    console.log(`PASS (${name}): independently compiled library templates, flat bindings, projected children and self-removal with colliding document IDs`);
    await browser.close(); browser = undefined;
  }
  await stopProcess(server); server = undefined;
}

async function run(verb, extra = []) {
  let success = true;
  let output;
  try {
    output = await exec(cargo, [verb, "--offline", "--locked", "--manifest-path", manifest, ...extra], {
      env, timeout: 180_000, maxBuffer: 16 * 1024 * 1024,
    });
  } catch (error) {
    if (error.killed || typeof error.code !== "number") throw error;
    success = false;
    output = error;
  }
  return { success, ...output };
}

async function compile(extra = []) {
  const output = await run("check", ["--message-format=json", ...extra]);
  const messages = output.stdout.trim().split("\n").filter(Boolean).map(JSON.parse);
  const built = messages.find((message) => message.reason === "build-script-executed" && message.package_id.includes("fixture-components"));
  const errors = messages.filter((message) => message.reason === "compiler-message" && message.message.level === "error").map((message) => message.message);
  return { ...output, errors, out: built?.out_dir };
}

function spans(value) {
  if (!value || typeof value !== "object") return [];
  return (value.file_name && value.line_start ? [value] : []).concat(Object.values(value).flatMap(spans));
}

function portablePath(value) {
  return value.replace(/^\\\\\?\\/, "").replaceAll("\\", "/");
}

try {
  await cp(fixture, scratch, { recursive: true, filter: (path) => basename(path) !== "target" });
  for (const relative of ["Cargo.toml", "components/Cargo.toml", "renderer/Cargo.toml", "compiler/Cargo.toml"]) {
    const file = join(scratch, relative);
    const text = await readFile(file, "utf8");
    await writeFile(file, text.replace(/"(?:\.\.\/){3,4}crates\/([^"]+)"/g,
      (_, name) => JSON.stringify(join(root, "crates", name))));
  }
  const tests = await run("test", ["--workspace"]);
  assert.equal(tests.success, true, tests.stderr);
  const tree = await run("tree", ["-e", "normal", "--prefix", "none"]);
  assert.equal(tree.success, true, tree.stderr);
  assert.doesNotMatch(tree.stdout, /(?:^|\n)(?:web-sys|js-sys|wasm-bindgen) v/);
  console.log("PASS: generated native code executes through public APIs without browser runtime dependencies");

  const portableWasm = await compile(["--target", "wasm32-unknown-unknown"]);
  assert.equal(portableWasm.success, true, portableWasm.stderr + JSON.stringify(portableWasm.errors));
  const browser = await compile(["--features", "browser", "--target", "wasm32-unknown-unknown"]);
  assert.equal(browser.success, true, browser.stderr + JSON.stringify(browser.errors));
  assert.ok((await readFile(join(browser.out, "fusor_templates/ui/panel.html.rs"), "utf8")).includes("Component"));
  assert.ok((await readFile(join(browser.out, "fusor_backends/memory/ui/panel.html.rs"), "utf8")).includes("memory_renderer"));
  console.log("PASS: the component-owning crate compiles one HTML source for both browser and independent Wasm backends");
  await browserConsumer();

  const htmlPath = join(scratch, "components/ui/panel.html");
  const good = await readFile(htmlPath, "utf8");
  for (const [before, after, code] of [
    ["{{ state.title }}: {{ state.count.get() }}", "{{ state.missing_title }}: {{ state.count.get() }}", "E0609"],
    ['key="{{ |row| row.id }}"', 'key="{{ |row| row.missing_id }}"', "E0609"],
    ['<Frame title="Receiver">', '<Frame nonexistent="Receiver">', "E0560"],
    ["{{ number.get() }}", "{{ number.get().missing_field }}", "E0610"],
    ['bind="state.number"', 'bind="state.rows"', "E0277"],
    ['state.title, team.team, member.id', 'state.title, team.team, member.missing_id', "E0609"],
    ['state.prefix, result.as_str(), again.as_str()', 'state.prefix, result.missing_method(), again.as_str()', "E0599"],
  ]) {
    assert.ok(good.includes(before));
    const broken = good.replace(before, after);
    const expectedLine = broken.slice(0, broken.indexOf(after)).split("\n").length;
    await writeFile(htmlPath, broken);
    const result = await compile();
    assert.equal(result.success, false, after);
    const diagnostic = result.errors.find((error) => error.code?.code === code);
    assert.ok(diagnostic, result.stderr + JSON.stringify(result.errors));
    const output = JSON.parse(await readFile(join(result.out, "fusor_backends/memory/manifest.json"), "utf8"));
    assert.equal(output.version, 1);
    assert.equal(output.namespace, "memory");
    const generated = output.sources.find((source) => portablePath(source.template) === "ui/panel.html");
    assert.ok(generated);
    const lines = (await readFile(generated.source_map, "utf8")).trimEnd().split("\n");
    assert.equal(lines.shift(), "fusor-source-map-v1");
    const mappings = lines.map((line) => line.split("\t").map(Number));
    const origins = spans(diagnostic).filter((span) => portablePath(span.file_name) === portablePath(generated.rust))
      .flatMap((span) => mappings.filter(([start, end]) => start <= span.line_start && span.line_start < end));
    assert.ok(origins.some((entry) => entry[2] === expectedLine), JSON.stringify({ expectedLine, origins, diagnostic }));
    console.log(`PASS: ${code} maps to authored ${generated.template}:${expectedLine}`);
  }
  for (const [before, after, message] of [
    ["<p>Static &amp; complete</p>", "<canvas></canvas>", "does not support element <canvas>"],
    ["<p>Static &amp; complete</p>", '<input type="file">', "supports only text, number and checkbox"],
    ['id="checked" type="checkbox"', 'id="checked" type="checkbox" on:input="state.count.set(0)"', "directly supported control"],
    ['on:click="state.count.update', 'on:keydown="state.count.update', 'does not support Event("keydown")'],
    ['<section id="panel"', '<section on:click="state.count.set(0)" id="panel"', "bubbling is unsupported"],
    ['<p id="selection">', '<input bind="state.selection"><p id="selection">', "support display bindings only"],
  ]) {
    assert.ok(good.includes(before));
    const broken = good.replace(before, after);
    const line = broken.slice(0, broken.indexOf(after)).split("\n").length;
    await writeFile(htmlPath, broken);
    const result = await compile();
    assert.equal(result.success, false);
    assert.ok(result.stderr.includes(message), result.stderr);
    assert.match(result.stderr, new RegExp(`panel\\.html:${line}:\\d+:`));
    console.log(`PASS: unsupported feature rejected at authored line ${line}`);
  }
} finally {
  await browser?.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
