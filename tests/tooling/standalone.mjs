import { root, env as buildEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory, copyProject } from "../../scripts/build.mjs";
// Exercise the installed-style executable against independent Cargo workspaces.
// Browser checks cover observable ownership, retained identity and failed mounts.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { chromium, firefox, webkit } from "@playwright/test";

const scratch = await temporaryDirectory("fusor-standalone-");
const executable = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
const env = {
  ...buildEnv, RUSTUP_TOOLCHAIN: process.env.RUSTUP_TOOLCHAIN || "stable",
  CARGO_NET_OFFLINE: "true",
  CARGO_TARGET_DIR: join(root, "target/standalone-tests"),
};
let server;
let browser;

async function cli(args, cwd = scratch) {
  return exec(executable, args, { cwd, env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
}

const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 120_000, interval: 100, process: server });

async function stop() {
  await stopProcess(server);
  server = undefined;
}

async function serve(app, command) {
  const port = await reservePort();
  server = startProcess(executable, [command, "--port", String(port), "--offline", "--locked"], {
    cwd: app, env,
  });

  await waitFor(() => server.output.includes("Ctrl+C to stop."), `${command} ready`);
  return `http://127.0.0.1:${port}`;
}

async function connectClient(page) {
  await page.waitForFunction(() => document.querySelector("#first .counter"));
  await page.evaluate(async () => {
    const boot = document.querySelector('script[type="module"]').src;
    window.client = await import(new URL("./pkg/app.js", boot).href);
  });
}

try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--locked", "--offline"], { cwd: root, timeout: 180_000 });
  const scaffold = join(scratch, "my-app");
  await cli(["new", scaffold, "--framework-path", root]);
  await assert.rejects(cli(["new", scaffold, "--framework-path", root]), /destination already exists/);
  await assert.rejects(cli(["new", join(scratch, "fn"), "--framework-path", root]), /lowercase Cargo package name/);
  await mkdir(join(scaffold, "dist"));
  await writeFile(join(scaffold, "dist/keep.txt"), "authored file");
  await cli(["check", "--offline"], scaffold);
  await assert.rejects(cli(["build", "--debug", "--offline"], scaffold), /not owned by fusor/);
  assert.equal(await readFile(join(scaffold, "dist/keep.txt"), "utf8"), "authored file");
  await rm(join(scaffold, "dist"), { recursive: true });
  await cli(["build", "--debug", "--offline", "--locked"], scaffold);
  const scaffoldBase = await serve(scaffold, "serve");
  browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || undefined });
  let page = await browser.newPage();
  await page.goto(scaffoldBase);
  await page.locator(".counter").first().waitFor();
  assert.equal(await page.locator(".counter").count(), 2);
  await page.getByRole("button", { name: "Increment" }).first().click();
  await waitFor(async () => (await page.locator("main > p").last().textContent()).includes("1"), "scaffold signal update");
  assert.match(await page.locator(".counter").first().textContent(), /Clicks here: 1/);
  assert.match(await page.locator(".counter").last().textContent(), /Clicks here: 0/);
  await browser.close(); browser = undefined;
  await stop();
  console.log("PASS: new/check/build/serve work outside the repository with shared inputs and independent component state");

  // Exercise the still-supported reverse-registration API after verifying the
  // new scaffold's discovered native templates above.
  await writeFile(join(scaffold, "src/app.rs"), (await readFile(join(scaffold, "src/app.rs"), "utf8")).replace('fusor::template!("web/index.html");', 'fusor::bindings!(app);'));
  await writeFile(join(scaffold, "web/index.html"), (await readFile(join(scaffold, "web/index.html"), "utf8")).replace('</head>', '<script type="text/rust" src="../src/app.rs" rust:module="crate::app"></script>\n</head>'));
  await writeFile(join(scaffold, "src/lib.rs"), 'mod app;\nmod counter;\ninclude!(env!("FUSOR_MODULE"));\n');
  const nativePath = join(scaffold, "src/app.rs");
  const nativeSource = await readFile(nativePath, "utf8");
  const nativeHtmlPath = join(scaffold, "web/index.html");
  const nativeHtml = await readFile(nativeHtmlPath, "utf8");
  await writeFile(join(scaffold, "src/wrong.rs"), nativeSource);
  const nativeBase = await serve(scaffold, "dev");
  browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || undefined });
  page = await browser.newPage();
  await page.goto(nativeBase);
  await page.locator(".counter").first().waitFor();
  await page.getByRole("button", { name: "Increment" }).first().click();
  const nativeBoot = await page.locator('script[type="module"]').getAttribute("src");
  await writeFile(nativeHtmlPath, nativeHtml.replace("Rust, inside HTML.", "Native Rust, reactive HTML."));
  await waitFor(async () => (await page.locator("h1").textContent()) === "Native Rust, reactive HTML.", "external module static refresh");
  assert.equal(await page.locator('script[type="module"]').getAttribute("src"), nativeBoot, server.output);
  assert.match(await page.locator(".counter").first().textContent(), /Clicks here: 1/);
  await writeFile(nativePath, nativeSource.replace("signal(0)", "signal(7)"));
  await waitFor(async () => (await page.locator('script[type="module"]').getAttribute("src")) !== nativeBoot, "native source rebuild");
  await waitFor(async () => (await page.locator("main > p").last().textContent()).includes("7"), "native source reloaded");
  const validNativeBoot = await page.locator('script[type="module"]').getAttribute("src");
  server.output = "";
  await writeFile(nativeHtmlPath, nativeHtml.replace("../src/app.rs", "../src/wrong.rs"));
  await waitFor(() => server.output.includes("Keeping the last successful build."), "source registration requires Cargo");
  assert.match(server.output, /external Rust source mismatch/);
  assert.equal(await page.locator('script[type="module"]').getAttribute("src"), validNativeBoot);
  await writeFile(nativeHtmlPath, nativeHtml);
  await waitFor(async () => (await page.locator("h1").textContent()) === "Rust, inside HTML.", "registration correction recovers");
  await browser.close(); browser = undefined;
  await stop();
  console.log("PASS: external modules preserve static refresh/state, rebuild on native edits, and reject mismatched registration without publishing");

  const app = join(scratch, "application");
  await copyProject(join(root, "tests/fixtures/application"), app, ["target", "site", ".fusor"]);
  const manifestPath = join(app, "Cargo.toml");
  const manifest = (await readFile(manifestPath, "utf8")).replace(/"\.\.\/\.\.\/\.\.\/crates\/([^"]+)"/g,
    (_, crate) => JSON.stringify(join(root, "crates", crate)));
  await writeFile(manifestPath, manifest);
  const expanded = await cli(["expand", "--module", "counter", "--offline"], app);
  assert.match(expanded.stdout, /pub struct Counter/);
  await cli(["check", "--offline"], app);

  const entryPath = join(app, "web/index.html");
  const entry = await readFile(entryPath, "utf8");
  for (const [before, after, expected] of [
    ['count="{{ state.count.clone() }}" fail="{{ false }}"', 'count="{{ signal(String::new()) }}" fail="{{ false }}"', /mismatched types/],
    ['rust:if="state.visible.get()"', 'rust:if="state.count.get()"', /mismatched types/],
    ['<Panel count="{{ state.count.clone() }}" fail="{{ false }}"></Panel>', '<self::App></self::App>', /FromInputs|TemplateComponent/],
  ]) {
    assert.ok(entry.includes(before));
    const broken = entry.replace(before, after);
    const expectedLine = broken.slice(0, broken.indexOf(after)).split("\n").length;
    await writeFile(entryPath, broken);
    await assert.rejects(cli(["check", "--offline", "--locked"], app), (error) => {
      assert.match(error.stderr, expected);
      assert.ok(error.stderr.includes(`HTML binding at web/index.html:${expectedLine}:`)
        || error.stderr.includes(`HTML binding at web\\index.html:${expectedLine}:`), error.stderr);
      return true;
    });
  }
  await writeFile(entryPath, entry);
  console.log("PASS: rustc checks child constructor inputs, conditional booleans and template-only children at their HTML declarations");

  await cli(["build", "--debug", "--offline", "--locked"], app);
  const base = await serve(app, "serve");
  const url = `${base}/tools/issues/`;
  assert.equal((await fetch(`${base}/tools/issues`, { redirect: "manual" })).status, 308);
  for (const path of ["/", "/tools/issues/Cargo.toml", "/tools/issues/web/index.html", "/tools/issues/.fusor-output.json", "/tools/issues/%2e%2e/Cargo.toml", "/tools/issues/missing.css"]) {
    assert.equal((await fetch(`${base}${path}`)).status, 404, path);
  }
  assert.equal((await fetch(url, { method: "POST" })).status, 405);
  assert.equal(await (await fetch(url, { method: "HEAD" })).text(), "");
  for (const engine of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    browser = await ({ chromium, firefox, webkit })[engine].launch(engine === "chromium" ? { channel: process.env.PLAYWRIGHT_CHANNEL || undefined } : {});
    page = await browser.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
    await page.goto(url);
    await connectClient(page);
    assert.equal(await page.evaluate(() => window.client.live()), 2);
    assert.equal(await page.evaluate(() => window.client.live_static()), 2);
    assert.equal(await page.locator("h1").textContent(), "Reusable HTML");
    assert.match(await page.evaluate(() => {
      try { window.client.recursive_mount(); return "no error"; }
      catch (error) { return String(error); }
    }), /component nesting exceeds 128/);
    const scriptUrl = await page.locator('script[type="module"]').getAttribute("src");
    const wasm = await fetch(new URL("pkg/app_bg.wasm", base + scriptUrl));
    assert.equal(wasm.headers.get("content-type"), "application/wasm");
    // Consume the large debug response before continuing: an unread fetch body
    // can leave the local server backpressured while the next browser connects.
    assert.deepEqual([...new Uint8Array(await wasm.arrayBuffer()).slice(0, 4)], [0, 97, 115, 109]);
    await page.evaluate(() => {
      window.retained = document.querySelector("#first .counter");
      window.oldButton = window.retained.querySelector("button");
    });
    await page.locator("#first .increment").click();
    assert.equal(await page.locator("#total").textContent(), "1");
    assert.equal(await page.locator("#first .clicks").textContent(), "1");
    assert.equal(await page.locator("#second .clicks").textContent(), "0");
    assert.deepEqual(await page.locator(".shared").allTextContents(), ["1", "1"]);
    await page.locator("#first input").fill("draft");
    await page.locator("#first input").evaluate((n) => n.setSelectionRange(2, 2));
    await page.evaluate(() => window.client.set_count(9));
    assert.equal(await page.locator("#first input").inputValue(), "draft");
    assert.equal(await page.locator("#first .label").textContent(), "draft");
    assert.equal(await page.locator("#first input").evaluate((n) => n.selectionStart), 2);
    assert.equal(await page.locator("#first input").evaluate((n) => document.activeElement === n), true);
    assert.equal(await page.evaluate(() => window.retained === document.querySelector("#first .counter")), true);
    await page.locator("#fail").click();
    assert.equal(await page.evaluate(() => window.retained === document.querySelector("#first .counter")), true);
    assert.equal(await page.evaluate(() => window.client.live()), 2);
    assert.deepEqual(errors.splice(0), ["expected constructor failure"]);
    await page.locator("#recover").click();
    assert.equal(await page.evaluate(() => window.retained === document.querySelector("#first .counter")), false);
    assert.equal(await page.locator("#first .clicks").textContent(), "0");
    assert.equal(await page.evaluate(() => window.client.live()), 2);
    await page.evaluate(() => window.oldButton.click());
    assert.equal(await page.locator("#total").textContent(), "9");
    await page.locator("#toggle").click();
    assert.equal(await page.locator("#first .counter").count(), 0);
    assert.equal(await page.evaluate(() => window.client.live()), 1);
    assert.equal(await page.evaluate(() => window.client.live_static()), 1);
    await page.locator("#toggle").click();
    assert.equal(await page.evaluate(() => window.client.live()), 2);
    await page.evaluate(() => window.client.unmount());
    assert.equal(await page.locator(".counter").count(), 0);
    assert.equal(await page.evaluate(() => window.client.live()), 0);
    assert.equal(await page.evaluate(() => window.client.live_static()), 0);
    await page.evaluate(() => { window.client.set_count(10); window.client.mount(); });
    assert.equal(await page.locator("#total").textContent(), "10");
    assert.equal(await page.evaluate(() => window.client.live()), 2);
    assert.deepEqual(errors, []);
    await browser.close(); browser = undefined;
    console.log(`PASS (${engine}): nested instances retain DOM/focus/drafts, replace by key, preserve old children on failure and fully dispose`);
  }
  await stop();

  const devBase = await serve(app, "dev");
  const devUrl = `${devBase}/tools/issues/`;
  const version = async () => (await fetch(`${devUrl}__fusor/version`)).text();
  browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || undefined });
  page = await browser.newPage();
  await page.goto(devUrl);
  await connectClient(page);
  const labelPath = join(app, "web/components/label.html");
  const label = await readFile(labelPath, "utf8");
  const oldBoot = await page.locator('script[type="module"]').getAttribute("src");
  let previousVersion = await version();
  await writeFile(labelPath, label.replace("{{ state.text.get() }}", '{{ format!("Value: {}", state.text.get()) }}'));
  await waitFor(async () => (await version()) !== previousVersion, "component file rebuild");
  await waitFor(async () => (await page.locator("#first .label").textContent()) === "Value: edit me", "automatic browser reload");
  assert.equal((await fetch(devBase + oldBoot)).status, 200);
  console.log("PASS: component edits rebuild and reload; the previous immutable generation remains available");

  previousVersion = await version();
  server.output = "";
  await writeFile(labelPath, label.replace("state.text.get()", "state.missing.get()"));
  await waitFor(() => server.output.includes("Keeping the last successful build."), "mapped component error");
  const labelLine = label.slice(0, label.indexOf("state.text.get()")).split("\n").length;
  assert(server.output.replaceAll("\\", "/").includes(`HTML binding at web/components/label.html:${labelLine}:`), server.output);
  assert.equal(await version(), previousVersion);
  await page.reload();
  await connectClient(page);
  assert.equal(await page.locator("#first .label").textContent(), "Value: edit me");
  await writeFile(labelPath, label);
  await waitFor(async () => (await version()) !== previousVersion, "recovery after component error");
  console.log("PASS: errors map to the correct HTML module and keep the last successful site; fixing them recovers");

  previousVersion = await version();
  await writeFile(join(app, "public/app.css"), "body { font-family: monospace; }\n");
  await waitFor(async () => (await version()) !== previousVersion, "asset edit rebuild");
  assert.match(await (await fetch(`${devUrl}app.css`)).text(), /monospace/);
  // A release build must coexist with the live development server, including
  // when the configured release directory has a custom name.
  const development = await readFile(join(app, ".fusor/dev/.fusor-output.json"), "utf8");
  const devVersion = await version();
  await cli(["build", "--offline", "--locked"], app);
  assert.equal(await readFile(join(app, ".fusor/dev/.fusor-output.json"), "utf8"), development);
  const production = await readFile(join(app, "site/.fusor-output.json"), "utf8");
  const generation = JSON.parse(production).generation;
  assert.ok(!(await readFile(join(app, "site/__fusor", generation, "boot.js"), "utf8")).includes("setInterval"));
  await writeFile(join(app, "public/app.css"), "body { font-family: monospace; color: navy; }\n");
  await waitFor(async () => (await version()) !== devVersion, "watcher processes the next authored edit");
  const [devGeneration, devRevision] = devVersion.split(":");
  assert.equal(await version(), `${devGeneration}:${Number(devRevision) + 1}`,
    "only the authored CSS refresh may advance the development revision");
  assert.equal(await readFile(join(app, "site/.fusor-output.json"), "utf8"), production);
  await browser.close(); browser = undefined;
  await stop();
  console.log("PASS: concurrent custom release output preserves the development generation and stays outside its watcher");
  console.log("PASS: asset changes are watched, and release output has no development client");
} finally {
  if (browser) await browser.close();
  await stop();
  await rm(scratch, { recursive: true, force: true });
}
