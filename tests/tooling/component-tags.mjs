import { root, env as buildEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory, copyProject } from "../../scripts/build.mjs";
// Native input contracts, explicit content projection, and wrapper-free lifetime
// behavior in an independent consumer. The legacy authoring suite runs separately.
import assert from "node:assert/strict";
import { readFile, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { chromium, firefox, webkit, expect } from "@playwright/test";

const scratch = await temporaryDirectory("fusor-component-tags-");
const cli = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
const env = { ...buildEnv, CARGO_NET_OFFLINE: "true", CARGO_TARGET_DIR: join(root, "tests/fixtures/component-tags/target"), };
const run = args => exec(cli, args, { cwd: scratch, env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
let server, browser;
const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 30_000, interval: 40, process: server });
try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--offline", "--locked"], { cwd: root, timeout: 180_000 });
  await copyProject(join(root, "tests/fixtures/component-tags"), scratch);
  const manifest = await readFile(join(scratch, "Cargo.toml"), "utf8");
  await writeFile(join(scratch, "Cargo.toml"), manifest.replaceAll("../../../crates/", `${root}/crates/`));
  await run(["check", "--offline"]);
  const entry = join(scratch, "web/index.html"), source = await readFile(entry, "utf8");
  for (const [before, after, diagnostic] of [
    ['count="{{ state.count.clone() }}"', 'count="{{ 42_u32 }}"', /mismatched types/],
    ['count="{{ state.count.clone() }}"', 'unknown="{{ state.count.clone() }}"', /no field named `unknown`/],
    ['count="{{ state.count.clone() }}"', '', /missing field `count`/],
    ['<CounterAlias ', '<MissingCounter ', /cannot find type `MissingCounter`/],
    ['rust:content="body"', 'rust:content="missing"', /no field named `missing`/],
    ['<Row count="{{ state.count.clone() }}">', '<Row>', /missing field `count`/],
    ['<Row count="{{ state.count.clone() }}">', '<Row count="{{ 42_u32 }}">', /mismatched types/],
    ['<Row count="{{ state.count.clone() }}">', '<Row unknown="{{ state.count.clone() }}">', /no field named `unknown`/],
  ]) {
    let broken = source.replace(before, after);
    if (after.includes("MissingCounter")) broken = broken.replace("</CounterAlias>", "</MissingCounter>");
    await writeFile(entry, broken);
    await assert.rejects(run(["check", "--offline", "--locked"]), error => {
      assert.match(error.stderr, diagnostic);
      assert.match(error.stderr, /HTML binding at web[\/]index\.html:/);
      return true;
    });
  }
  await writeFile(entry, source);
  // Derive diagnostics point to the authored Rust field; no HTML parser owns
  // or scans these annotations. Check actual rustc failures, not expansion text.
  const widgets = join(scratch, "src/widgets.rs");
  const widgetSource = await readFile(widgets, "utf8");
  for (const [replacement, diagnostic] of [
    ["count: Signal<i32>,", /field needs #\[input\] or #\[local/],
    ["#[input] #[local(init = signal(0))] count: Signal<i32>,", /choose exactly one/],
    ["#[local(init = 42_u32)] count: Signal<i32>,", /mismatched types/],
  ]) {
    await writeFile(widgets, widgetSource.replace("#[input]\n    count: Signal<i32>,", replacement));
    await assert.rejects(run(["check", "--offline", "--locked"]), error => {
      assert.match(error.stderr, diagnostic);
      assert.match(error.stderr, /src[\/]widgets\.rs:/);
      return true;
    });
  }
  await writeFile(widgets, widgetSource);
  // Cargo notices membership changes in an already discovered directory.
  const added = join(scratch, "web/components/added.html");
  await writeFile(added, '<template rust:component="Added"><p>unused template</p></template>');
  const expanded = await run(["expand", "--module", "web/components/added.html", "--offline", "--locked"]);
  assert.match(expanded.stdout, /Added/);
  await rm(added);
  await run(["build", "--debug", "--offline", "--locked"]);
  console.log("PASS: native discovery, aliases, PascalCase Button, typed values, missing/unknown fields, and HTML diagnostics");
  const port = await reservePort();
  server = startProcess(cli, ["preview", "--port", String(port), "--offline", "--locked"], { cwd: scratch, env });

  await waitFor(() => server.output.includes("Ctrl+C to stop."), "preview startup");
  const refresh = await readFile(join(root, "crates/fusor-cli/src/dev/refresh.js"), "utf8");
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    browser = await ({ chromium, firefox, webkit }[name]).launch(name === "chromium" ? { channel: process.env.PLAYWRIGHT_CHANNEL || undefined } : {});
    const page = await browser.newPage(), errors = [], consoleErrors = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text()); });
    await page.goto(`http://127.0.0.1:${port}/`);
    await expect(page.locator("main > .counter")).toHaveCount(2);
    await page.evaluate(async () => {
      const boot = document.querySelector('script[type="module"]').src;
      window.client = await import(new URL("./pkg/app.js", boot).href);
      window.first = document.querySelector('.counter[data-label="first"]');
    });
    const metrics = () => page.evaluate(() => Array.from(window.client.metrics()));
    assert.deepEqual(await metrics(), [2, 0, 2]);
    // One native event has separate reactive batches for each registered
    // listener. Text bind installs first; select bind installs after its options
    // and therefore after the authored change listener. Source attribute order
    // does not reverse those existing compiler/runtime decisions.
    const eventTrace = await page.evaluate(() => {
      window.client.take_event_trace();
      const text = document.querySelector('#event-text');
      text.value = 'edited';
      text.dispatchEvent(new Event('input'));
      const input = window.client.take_event_trace();
      const select = document.querySelector('#event-select');
      select.value = 'b';
      select.dispatchEvent(new Event('change'));
      return { input, change: window.client.take_event_trace() };
    });
    assert.deepEqual(eventTrace, {
      input: 'text:edited|input:edited|pulse:2',
      change: 'change:a|pulse:4|select:b',
    });
    console.log(`PASS (${name}): text/select bind-handler ordering and separate listener batches, with one effect flush for repeated handler writes`);
    await expect(page.locator(".rust-button")).toHaveText("Rust component");
    await expect(page.locator("tbody > tr > td")).toHaveText("0");
    assert.deepEqual(await page.locator("tbody").evaluate(node => [...node.children].map(child => child.localName)), ["tr"]);
    assert.equal(await page.locator("counteralias, panel, row").count(), 0);
    await expect(page.locator(".caller-title")).toHaveText(["caller title", "caller title"]);
    await expect(page.locator(".projected > .badge")).toHaveText(["outer", "outer"]);
    await expect(page.locator(".nested .badge")).toHaveText(["inner", "inner", "inner", "inner"]);
    await page.locator('.counter[data-label="first"] button').click();
    await page.evaluate(() => window.client.set_count(7));
    await expect(page.locator(".counter .count")).toHaveText(["7", "7"]);
    await expect(page.locator(".counter .initial")).toHaveText(["0", "0"]);
    await expect(page.locator(".counter .clicks")).toHaveText(["1", "0"]);
    assert.equal(await page.evaluate(() => window.first === document.querySelector('.counter[data-label="first"]')), true);
    assert.deepEqual(await metrics(), [2, 0, 2]);
    await page.evaluate(() => { window.detached = document.querySelector('.projected-button'); window.client.show(false); });
    await expect(page.locator(".projected")).toHaveCount(0);
    await page.evaluate(() => window.detached.click());
    await expect(page.locator("tbody > tr > td")).toHaveText("7");
    await page.evaluate(() => { window.client.set_count(9); window.client.show(true); });
    await expect(page.locator(".projected-count")).toHaveText(["9", "9"]);
    await expect(page.locator('.counter[data-label="first"] .initial')).toHaveText("9");
    assert.deepEqual(await metrics(), [3, 1, 3]);
    await page.locator('.counter[data-label="first"] button').click();
    await page.evaluate(() => { window.first = document.querySelector('.counter[data-label="first"]'); window.client.reset(1, true); });
    await expect(page.locator('.counter[data-label="first"] .clicks')).toHaveText("1");
    assert.equal(await page.evaluate(() => window.first === document.querySelector('.counter[data-label="first"]')), true);
    assert.deepEqual(await metrics(), [4, 2, 3]);
    await page.evaluate(() => window.client.reset(2, false));
    await expect(page.locator('.counter[data-label="first"] .clicks')).toHaveText("0");
    assert.deepEqual(await metrics(), [5, 3, 4]);
    // Cached exact copies are accepted; corrupt projection anchors force full
    // validation, preserve the old mounted panel, and dispose the failed attempt.
    await page.evaluate(() => {
      window.panel = document.querySelector('main > .panel');
      for (const template of document.querySelectorAll('template[data-fusor-component]')) {
        if (!template.content.querySelector('.projected')) continue;
        const walker = document.createTreeWalker(template.content, NodeFilter.SHOW_COMMENT);
        let node;
        while ((node = walker.nextNode())) if (node.data.startsWith('/fusor:mount:')) {
          window.anchor = node; window.originalAnchor = node.data; node.data = '/fusor:mount:9999'; break;
        }
      }
      window.client.reset_panel(1);
    });
    assert.equal(await page.evaluate(() => window.panel === document.querySelector('main > .panel')), true);
    await expect(page.locator(".badge")).toHaveCount(6);
    await page.evaluate(() => { window.anchor.data = window.originalAnchor; window.client.reset_panel(2); });
    assert.equal(await page.evaluate(() => window.panel === document.querySelector('main > .panel')), false);
    await expect(page.locator(".badge")).toHaveCount(6);
    // Refresh treats managed sibling ranges as opaque and still patches both
    // live content instances and inert declarations for future mounts.
    await page.evaluate(async code => {
      const { patchDocument } = await import(`data:text/javascript;base64,${btoa(code)}`);
      const source = await (await fetch('/')).text();
      const before = new DOMParser().parseFromString(source, 'text/html');
      const after = new DOMParser().parseFromString(source.replace('Authored components', 'Edited components').replaceAll('class="projected"', 'class="projected updated"'), 'text/html');
      window.first = document.querySelector('.counter[data-label="first"]');
      patchDocument(before, after);
    }, refresh);
    await expect(page.locator("h1")).toHaveText("Edited components");
    await expect(page.locator(".projected.updated")).toHaveCount(2);
    assert.equal(await page.evaluate(() => window.first === document.querySelector('.counter[data-label="first"]')), true);
    await page.evaluate(() => window.client.unmount());
    await expect(page.locator(".counter, .projected, .badge, tbody > tr")).toHaveCount(0);
    assert.deepEqual(await metrics(), [5, 5, 4]);
    assert.deepEqual(errors, []);
    assert.equal(consoleErrors.length, 2, consoleErrors.join("\n"));
    assert.match(consoleErrors[0], /expected construction failure/);
    assert.match(consoleErrors[1], /template mismatch.*component/);
    console.log(`PASS (${name}): wrapper-free roots/tables, signal sharing, retained state, caller scope, receiving context, content reuse, rollback, cache validation, refresh, disposal`);
    // The live document parses noscript differently from DOMParser. Unchanged
    // fallback markup must not turn an otherwise compatible edit into a reload.
    const fallback = await browser.newPage();
    const fallbackSource = `<!doctype html><html><head><title>Refresh</title></head><body><h1>Original</h1><noscript><P class='fallback'>Enable &amp; use JavaScript</P></noscript><template data-fusor-component="42"><!--fusor:mount:0--><!--/fusor:mount:0--></template></body></html>`;
    await fallback.setContent(fallbackSource);
    const fallbackResult = await fallback.evaluate(async ({ code, source }) => {
      const { patchDocument } = await import(`data:text/javascript;base64,${btoa(code)}`);
      const before = document.cloneNode(true), heading = document.querySelector('h1');
      const after = new DOMParser().parseFromString(source.replace('Original', 'Updated'), 'text/html');
      patchDocument(before, after);
      const changedFallback = new DOMParser().parseFromString(source.replace('Original', 'Unpublished').replace('Enable', 'Turn on'), 'text/html');
      let failure;
      try { patchDocument(after, changedFallback); } catch (error) { failure = error.message; }
      const changedFragment = new DOMParser().parseFromString(source.replace('Original', 'Unpublished').replaceAll('mount:0', 'mount:1'), 'text/html');
      let fragmentFailure;
      try { patchDocument(after, changedFragment); } catch (error) { fragmentFailure = error.message; }
      return [before.querySelector('noscript').firstChild.nodeType, after.querySelector('noscript').firstChild.nodeType,
        heading.textContent, heading === document.querySelector('h1'), failure, fragmentFailure];
    }, { code: refresh, source: fallbackSource });
    assert.deepEqual(fallbackResult, [3, 1, 'Updated', true, 'noscript markup changed', 'component root structure changed']);
    await fallback.close();
    console.log(`PASS (${name}): unchanged noscript/fragments preserve static refresh; changed fallback/fragment rejects the full patch`);
    await browser.close(); browser = undefined;
  }
  // An unused missing association can pass rustc. The actual loader export
  // check must turn an omitted entry macro into a visible initialization error.
  const appRust = join(scratch, "src/app.rs");
  await writeFile(appRust, (await readFile(appRust, "utf8")).replace('fusor::template!("web/index.html");', ''));
  await run(["build", "--debug", "--offline", "--locked"]);
  browser = await chromium.launch({ channel: process.env.PLAYWRIGHT_CHANNEL || undefined });
  const missing = await browser.newPage();
  await missing.addInitScript(() => document.addEventListener('fusor:error', event => { window.startupError = String(event.detail); }));
  await missing.goto(`http://127.0.0.1:${port}/`);
  await expect.poll(() => missing.evaluate(() => window.startupError || '')).toMatch(/generated startup is absent.*template!/);
  await expect(missing.locator(".counter")).toHaveCount(0);
  console.log("PASS: omitted entry association produces an explicit loader error");
} finally {
  await browser?.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
