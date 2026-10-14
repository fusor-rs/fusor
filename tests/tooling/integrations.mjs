import { root, suiteEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory, copyProject, independentManifest } from "../../scripts/build.mjs";
// Independent consumer: real CodeMirror/Chart.js, shared reads, and development rebuilds.
import assert from "node:assert/strict";
import { readFile, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { chromium, firefox, webkit, expect } from "@playwright/test";

const scratch = await temporaryDirectory("fusor-integrations-");
const executable = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
const env = { ...suiteEnv, CARGO_NET_OFFLINE: "true", npm_config_cache: join(root, "target/npm-cache"), };
let server, browser, url;
const cli = args => exec(executable, args, { cwd: scratch, env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 120_000, interval: 30, process: server });
const version = async () => (await fetch(`${url}__fusor/version`)).text();
async function edit(file, source) {
  const before = await version();
  await writeFile(join(scratch, file), source);
  await waitFor(async () => await version() !== before, `publish ${file}`);
  return [before, await version()];
}
try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--offline", "--locked"], { timeout: 180_000 });
  await copyProject(join(root, "examples/integrations"), scratch);
  let manifest = await independentManifest(await readFile(join(scratch, "Cargo.toml"), "utf8"));
  // Keep custom asset-hook coverage separate from automatic npm module bundling.
  manifest = manifest.replace('[package.metadata.fusor]', '[package.metadata.fusor]\nassets-build = ["node", "build-assets.mjs"]');
  const buildScript = 'import { readFile, writeFile } from "node:fs/promises";\nconst file = new URL("./public/hook.txt", import.meta.url);\nif (await readFile(file, "utf8").catch(() => "") !== "fixture asset") await writeFile(file, "fixture asset");\n';
  await writeFile(join(scratch, "build-assets.mjs"), buildScript);
  await writeFile(join(scratch, "Cargo.toml"), manifest);
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: scratch });
  await exec(process.platform === "win32" ? "npm.cmd" : "npm", ["ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--cache", join(root, "target/npm-cache")], { cwd: scratch, timeout: 120_000 });
  const entry = await readFile(join(scratch, "web/index.html"), "utf8");
  const panel = await readFile(join(scratch, "web/panel.html"), "utf8");
  const css = await readFile(join(scratch, "public/app.css"), "utf8");
  const library = await readFile(join(scratch, "src/lib.rs"), "utf8");

  // Component inputs remain type-checked by the independent consumer’s rustc.
  const invalid = panel.replace('count="{{ state.count.clone() }}"', 'count="{{ state.text.clone() }}"');
  await writeFile(join(scratch, "web/panel.html"), invalid);
  await assert.rejects(cli(["check", "--offline"]), error => {
    assert.match(error.stderr, /mismatched types/);
    assert.match(error.stderr.replaceAll("\\", "/"), /web\/panel.html:\d+:/); return true;
  });
  await writeFile(join(scratch, "web/panel.html"), panel);
  await cli(["check", "--offline", "--locked"]);
  const port = await reservePort();
  url = `http://127.0.0.1:${port}/lab/`;
  server = startProcess(executable, ["dev", "--port", String(port), "--offline", "--locked"], { cwd: scratch, env });

  await waitFor(() => server.output.includes("Ctrl+C to stop."), "dev startup");

  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    browser = await ({ chromium, firefox, webkit }[name]).launch(name === "chromium" ? { channel: process.env.PLAYWRIGHT_CHANNEL || undefined } : {});
    const page = await browser.newPage(), errors = [], consoleErrors = [], requests = [], pending = new Map();
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") consoleErrors.push(message); });
    await page.route("**/lab/api/items/*", async route => {
      const id = Number(route.request().url().split("/").at(-1)); requests.push(id);
      if (id === 2) pending.set(id, route);
      else await route.fulfill({ status: 200, contentType: "text/plain", body: `Item ${id}` });
    });
    await page.goto(url);
    await expect(page.locator(".cm-editor")).toHaveCount(2);
    await expect(page.locator(".chart canvas")).toHaveCount(2);
    await expect(page.locator(".data")).toHaveText(["Item 1", "Item 1"]);
    assert.deepEqual(requests, [1], "two independently mounted views share one request");
    const counts = () => page.evaluate(() => ({ ...FusorWidgets.counts }));
    assert.deepEqual(await counts(), { created: 4, destroyed: 0, observers: 4, callbacks: 0 });
    await page.locator("#draft").fill("A preserved draft");
    await page.locator("#increment").click();
    await page.evaluate(() => {
      window.identity = [document.querySelector("main"), document.querySelector(".cm-editor"), document.querySelector("canvas")];
      const input = document.querySelector("#draft"); input.focus(); input.setSelectionRange(2, 7);
    });
    // Native JS components conservatively reload on development edits. Rust-only
    // fast refresh is covered by dev.mjs; do not promise state preservation here.
    const originalGeneration = (await version()).split(":")[0];
    let changedEntry = entry.replace("Owned integrations</h1>", "Native integrations</h1>");
    await edit("web/index.html", changedEntry);
    await expect(page.locator("h1")).toHaveText("Native integrations");
    await expect(page.locator("#count")).toHaveText("0");
    assert.notEqual((await version()).split(":")[0], originalGeneration);
    await edit("web/panel.html", panel.replace("Shared item</h2>", "Cached item</h2>"));
    await expect(page.locator(".heading")).toHaveText(["Cached item", "Cached item"]);
    await edit("public/app.css", `${css}\nbody { background-color: rgb(241, 242, 243); }\n`);
    await expect(page.locator("body")).toHaveCSS("background-color", "rgb(241, 242, 243)");
    await expect(page.locator(".data")).toHaveText(["Item 1", "Item 1"]);
    console.log(`PASS (${name}): native JavaScript builds reload changed HTML, component templates and CSS`);

    requests.length = 0;
    await page.reload();
    await expect(page.locator(".data")).toHaveText(["Item 1", "Item 1"]);
    await expect(page.locator(".cm-editor")).toHaveCount(2);
    assert.deepEqual(requests, [1]);
    assert.deepEqual(await counts(), { created: 4, destroyed: 0, observers: 4, callbacks: 0 });
    await page.locator("#draft").fill("A preserved draft");
    await page.locator("#increment").click();
    await expect(page.locator(".cm-content")).toHaveText(["A preserved draft", "A preserved draft"]);
    await expect(page.locator("#count")).toHaveText("1");
    assert.equal((await counts()).created, 4, "input updates retain library instances");
    await page.locator("#toggle-first").click();
    await expect(page.locator(".cm-editor")).toHaveCount(1);
    assert.equal((await counts()).observers, 2);
    await page.locator("#toggle-first").click();
    await expect(page.locator(".heading")).toHaveText(["Cached item", "Cached item"]);
    assert.deepEqual(requests, [1], "fresh data survived a remount");
    await page.locator("#next").click(); await waitFor(() => pending.has(2), "delayed request");
    await page.locator("#next").click();
    await expect(page.locator(".data")).toHaveText(["Item 3", "Item 3"]);
    await pending.get(2).fulfill({ status: 200, body: "STALE" }).catch(() => {});
    await expect(page.locator(".data")).toHaveText(["Item 3", "Item 3"]);
    assert.deepEqual(requests, [1, 2, 3]);
    await page.locator("#refresh").click();
    await expect.poll(() => requests.filter(id => id === 3).length).toBe(2);
    await expect(page.locator(".data")).toHaveText(["Item 3", "Item 3"]);
    await page.locator("#fail").click();
    await expect(page.locator(".cm-editor")).toHaveCount(1);
    assert.equal((await counts()).observers, 3);
    assert.equal(consoleErrors.length, 1);
    // Firefox's console text omits Error.message; inspect the thrown value.
    assert.match(await consoleErrors.splice(0)[0].args()[0].evaluate(error => error.message), /expected widget setup failure/);
    await expect(page.locator("#draft")).toHaveValue("A preserved draft");
    await page.locator("#fail").click();
    await expect(page.locator(".cm-editor")).toHaveCount(2);
    for (let i = 0; i < 10; i++) {
      await page.locator("#toggle-first").click(); await page.locator("#toggle-first").click();
    }
    await page.evaluate(async () => {
      const boot = document.querySelector("script[data-fusor-revision]").src;
      const app = await import(new URL("./pkg/app.js", boot)); app.unmount(); FusorWidgets.lateCallback();
    });
    const disposed = await counts();
    assert.equal(disposed.created, disposed.destroyed); assert.equal(disposed.observers, 0);
    assert.deepEqual(errors, []); assert.deepEqual(consoleErrors, []);
    console.log(`PASS (${name}): real widgets update without recreation; failure, teardown, observer cleanup, late callbacks, query sharing and stale results`);

    // Structural edits also reload and recreate library instances.
    changedEntry = changedEntry.replace("Native integrations</h1>", "<strong>Native integrations</strong></h1>");
    await edit("web/index.html", changedEntry);
    await expect(page.locator("h1 > strong")).toHaveText("Native integrations");
    await expect(page.locator(".cm-editor")).toHaveCount(2);
    assert.notEqual((await version()).split(":")[0], originalGeneration);

    // A Rust edit requires a new compiled generation and a fresh application.
    changedEntry = changedEntry.replace("*n += 1", "*n += 2");
    await edit("web/index.html", changedEntry);
    const nextGeneration = (await version()).split(":")[0];
    await expect.poll(() => page.locator("script[data-fusor-revision]").getAttribute("src")).toContain(nextGeneration);
    await expect(page.locator(".cm-editor")).toHaveCount(2);
    assert.notEqual((await version()).split(":")[0], originalGeneration);
    await page.locator("#increment").click(); await expect(page.locator("#count")).toHaveText("2");
    const good = await version(), logStart = server.output.length;
    await writeFile(join(scratch, "web/index.html"), changedEntry.replace("state.count.get()", "state.count.does_not_exist()"));
    await waitFor(() => server.output.slice(logStart).includes("Keeping the last successful build"), "failed Rust build");
    assert.equal(await version(), good); await expect(page.locator("#count")).toHaveText("2");
    await edit("web/index.html", entry);
    await expect(page.locator("h1")).toHaveText("Owned integrations");
    await edit("web/panel.html", panel);
    await edit("public/app.css", css);
    const beforeFailure = await version(), hookLog = server.output.length;
    await writeFile(join(scratch, "build-assets.mjs"), `${buildScript}\nprocess.exit(7);\n`);
    await waitFor(() => server.output.slice(hookLog).includes("Keeping the last successful build"), "failed asset hook");
    assert.equal(await version(), beforeFailure);
    await edit("build-assets.mjs", buildScript);
    console.log(`PASS (${name}): Rust changes compile, compiler errors preserve the live app, and asset-build failures preserve published output`);

    // Native includes read bytes at compile time, even when Rust tokens agree.
    await edit("src/lib.rs", `${library}\npub fn compiled_css() -> &'static str { include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/public/app.css")) }\n`);
    await edit("web/index.html", entry.replace('<p id="intro">', '<p id="intro" title="{{ crate::compiled_css() }}">'));
    await expect(page.locator("#intro")).toHaveAttribute("title", css);
    const withInclude = (await version()).split(":")[0];
    const includedCss = `${css}\n/* This must reach compiled Rust too. */\n`;
    await edit("public/app.css", includedCss);
    assert.notEqual((await version()).split(":")[0], withInclude, "native file includes require Cargo");
    await expect(page.locator("#intro")).toHaveAttribute("title", includedCss);
    await edit("web/index.html", entry);
    await edit("src/lib.rs", library);
    await edit("public/app.css", css);
    console.log(`PASS (${name}): include_str! inputs rebuild native Rust when only the included asset changes`);
    await browser.close(); browser = undefined;
  }
  await stopProcess(server); server = undefined;
  await cli(["build", "--offline", "--locked"]);
  const built = JSON.parse(await readFile(join(scratch, "dist/.fusor-output.json"), "utf8"));
  const boot = await readFile(join(scratch, `dist/__fusor/${built.generation}/boot.js`), "utf8");
  assert(!boot.includes("watch(") && !boot.includes("refresh.js"));
  assert(!Object.hasOwn(built, "rust_signature"));
  console.log("PASS: optimized output excludes development refresh and uses reproducible bundled npm assets under /lab/");

} finally {
  if (browser) await browser.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
