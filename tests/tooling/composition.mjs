import { root, suiteEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory, copyProject } from "../../scripts/build.mjs";
// The example becomes an independent consumer: no generated code or private API.
import assert from "node:assert/strict";
import { readFile, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { chromium, firefox, webkit, expect } from "@playwright/test";

const scratch = await temporaryDirectory("fusor-composition-");
const executable = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
const env = {
  ...suiteEnv,
  CARGO_NET_OFFLINE: "true",
};
let server, browser;
const cli = args => exec(executable, args, { cwd: scratch, env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 30_000, interval: 25, process: server });
try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--locked", "--offline"], { timeout: 180_000 });
  await copyProject(join(root, "examples/composition"), scratch);
  const dependency = name => JSON.stringify(join(root, "crates", name));
  await writeFile(join(scratch, "Cargo.toml"), `[package]
name = "composition-consumer"
version = "0.1.0"
edition = "2024"
[workspace]
[lib]
crate-type = ["cdylib", "rlib"]
[dependencies]
fusor-core = { path = ${dependency("fusor-core")}, features = ["dom"] }
fusor-async = { path = ${dependency("fusor-async")}, features = ["browser"] }
wasm-bindgen = "=0.2.117"
[build-dependencies]
fusor-build = { path = ${dependency("fusor-build")} }
[package.metadata.fusor]
entry = "web/index.html"
output = "dist"
[package.metadata.fusor.components]
layout = "web/layout.html"
card = "web/card.html"
`);
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: scratch });
  await cli(["check", "--offline"]);
  // Authored Rust names must not be shadowed by the compiler's slot temporaries.
  const layoutPath = join(scratch, "web/layout.html");
  const layout = await readFile(layoutPath, "utf8");
  await writeFile(layoutPath, layout
    .replace('<script type="text/rust">', '<script type="text/rust">\nfn content() {}')
    .replace('rust:key="state.reset.get()"', 'rust:key="{ content(); state.reset.get() }"'));
  await cli(["check", "--offline", "--locked"]);
  await writeFile(layoutPath, layout);
  for (const [file, before, after, diagnostic, binding] of [
    ["layout", 'rust:slot="state.body.get()"', 'rust:slot="42_u32"', /From<u32>|Into<Option<Content>>/, true],
    ["layout", 'rust:if="state.visible.get()"', 'rust:if="state.reset.get()"', /mismatched types/, true],
    ["index", 'owner.provide::<Theme>(signal("root".into()))', 'owner.provide::<Theme>(42_u32)', /mismatched types/, false],
    ["index", 'Content::new(|_| -> Card', 'Content::new(|_| -> App', /TemplateComponent/, false],
  ]) {
    const path = join(scratch, `web/${file}.html`);
    const original = await readFile(path, "utf8");
    assert(original.includes(before));
    const broken = original.replace(before, after);
    const line = broken.slice(0, broken.indexOf(after)).split("\n").length;
    await writeFile(path, broken);
    await assert.rejects(cli(["check", "--offline", "--locked"]), error => {
      assert.match(error.stderr, diagnostic);
      const diagnosticText = error.stderr.replaceAll("\\", "/");
      assert(diagnosticText.includes(`web/${file}.html:${line}:`), error.stderr);
      if (binding) assert(diagnosticText.includes(`HTML binding at web/${file}.html:${line}:`), error.stderr);
      return true;
    });
    await writeFile(path, original);
  }
  console.log("PASS: independent Cargo consumer; wrong slot, condition, context and component types map to authored HTML");
  await cli(["build", "--offline", "--locked"]);
  const port = await reservePort();
  server = startProcess(executable, ["preview", "--port", String(port), "--offline", "--locked"], { cwd: scratch, env });

  await waitFor(() => server.output.includes("Ctrl+C to stop."), "preview startup");
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    browser = await ({ chromium, firefox, webkit }[name]).launch(name === "chromium" ? { channel: process.env.PLAYWRIGHT_CHANNEL || undefined } : {});
    const page = await browser.newPage();
    const errors = [], consoleErrors = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text()); });
    await page.goto(`http://127.0.0.1:${port}/`);
    await expect(page.locator(".card")).toHaveCount(2);
    await page.evaluate(async () => {
      const boot = document.querySelector('script[type="module"]').src;
      window.client = await import(new URL("./pkg/app.js", boot).href);
    });
    const metrics = () => page.evaluate(() => Array.from(window.client.metrics()));
    await expect.poll(metrics).toEqual([2, 0, 2, 0, 0, 1]);
    await expect(page.locator(".theme")).toHaveText(["amber", "violet"]);
    await expect(page.locator(".caption")).toHaveText(["Group 0", "Group 0"]);
    await page.locator("#first .increment").click();
    await expect(page.locator(".shared")).toHaveText(["1", "1"]);
    await expect(page.locator(".clicks")).toHaveText(["1", "0"]);
    assert.equal((await metrics())[5], 2, "one computation shared by four bindings, even when output stays equal");
    await page.locator("#first input").fill("My local note");
    await page.evaluate(() => {
      window.first = document.querySelector("#first .card");
      window.second = document.querySelector("#second .card");
      window.oldButton = window.first.querySelector("button");
      const input = window.first.querySelector("input"); input.focus(); input.setSelectionRange(2, 5);
      document.querySelector("#restore").click(); // Same factory identity.
      document.querySelector("#ten").click();
    });
    await expect(page.locator(".caption")).toHaveText(["Group 1", "Group 1"]);
    assert.equal((await metrics())[5], 3);
    assert.deepEqual(await page.evaluate(() => {
      const input = window.first.querySelector("input");
      return [window.first === document.querySelector("#first .card"), window.second === document.querySelector("#second .card"),
        document.activeElement === input, input.selectionStart, input.selectionEnd];
    }), [true, true, true, 2, 5]);
    await page.locator("#theme").click();
    await expect(page.locator(".theme")).toHaveText(["green", "violet"]);
    assert.equal((await metrics())[5], 3, "context updates do not recompute an unrelated memo");
    const before = await metrics();
    await page.locator("#fail").click();
    assert.deepEqual(await metrics(), before, "factory reads are untracked");
    await page.locator("#reset").click();
    assert.equal(await page.evaluate(() => window.first === document.querySelector("#first .card")), true);
    await expect(page.locator("#first input")).toHaveValue("My local note");
    const failed = await metrics();
    assert.deepEqual(failed, [before[0] + 1, before[1] + 1, ...before.slice(2)]);
    assert.deepEqual(consoleErrors.splice(0), ["expected content failure"]);
    await page.locator("#recover").click();
    assert.deepEqual(await metrics(), failed, "an untracked factory change does not retry until a tracked input changes");
    await page.locator("#reset").click();
    await expect.poll(async () => (await metrics())[4]).toBe(before[4] + 1);
    assert.deepEqual(await page.evaluate(() => [window.first.isConnected, window.second === document.querySelector("#second .card")]), [false, true]);
    await expect(page.locator("#first input")).toHaveValue("");
    await page.evaluate(() => window.oldButton.click());
    await expect(page.locator("#total")).toHaveText("11");
    console.log(`PASS (${name}): lazy content, nearest typed context, shared memo, retained focus and independent local state; failed replacement starts no async work`);

    const active = await metrics();
    await page.locator("#toggle").click();
    await expect(page.locator("#first .body")).toBeEmpty();
    await expect(page.locator("#second .card")).toHaveCount(1);
    await expect.poll(async () => (await metrics())[4]).toBe(active[4] + 1);
    assert.equal((await metrics())[3], active[3] + 1);
    await page.locator("#toggle").click();
    await expect(page.locator(".theme")).toHaveText(["green", "violet"]);
    await page.evaluate(() => { window.first = document.querySelector("#first .card"); });
    await page.locator("#replace").click();
    assert.deepEqual(await page.evaluate(() => [window.first.isConnected, window.second.isConnected]), [false, false]);
    await page.locator("#clear").click();
    await expect(page.locator(".card")).toHaveCount(0);
    await expect.poll(async () => { const m = await metrics(); return [m[0] - m[1], m[2] - m[3], m[2] - m[4]]; }).toEqual([0, 0, 0]);
    await page.locator("#restore").click();
    for (let i = 0; i < 10; i++) {
      await page.locator("#toggle").click(); await page.locator("#toggle").click();
    }
    await page.evaluate(() => window.client.unmount());
    await expect(page.locator(".card")).toHaveCount(0);
    await expect.poll(async () => { const m = await metrics(); return [m[0] - m[1], m[2] - m[3], m[2] - m[4]]; }).toEqual([0, 0, 0]);
    await page.evaluate(() => window.client.start());
    await expect.poll(metrics).toEqual([2, 0, 2, 0, 0, 1]);
    await expect(page.locator(".theme")).toHaveText(["amber", "violet"]);
    assert.deepEqual(errors, []); assert.deepEqual(consoleErrors, []);
    console.log(`PASS (${name}): key/factory replacement, optional content, conditional teardown, resource cancellation, repeated remounts and released component/future captures`);
    await browser.close(); browser = undefined;
  }
} finally {
  if (browser) await browser.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
