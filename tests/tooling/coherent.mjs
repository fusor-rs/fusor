import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { chromium, firefox, webkit } from "playwright";
import assert from "node:assert/strict";
import { observeFetch } from "../../scripts/observe-fetch.mjs";
import { observeConsoleErrors } from "../../scripts/observe-console.mjs";
const root = join(process.cwd(), "examples/coherent/dist");
const requests = [];
const server = createServer(async (req, res) => {
  if (req.url === "/favicon.ico") {
    res.writeHead(204);
    res.end();
    return;
  }
  if (req.url.startsWith("/api/")) {
    requests.push({ url: req.url, res });
    return;
  }
  try {
    const path = req.url === "/" ? "/index.html" : req.url;
    const data = await readFile(join(root, path));
    res.setHeader(
      "content-type",
      path.endsWith(".wasm")
        ? "application/wasm"
        : path.endsWith(".js")
          ? "text/javascript"
          : "text/html",
    );
    res.end(data);
  } catch {
    res.writeHead(404);
    res.end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
async function wait(fn) {
  const deadline = Date.now() + 10000;
  while (!fn()) {
    if (Date.now() > deadline) throw Error("timeout");
    await new Promise((r) => setTimeout(r, 20));
  }
}
let browser;
try {
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(
    ",",
  )) {
    requests.length = 0;
    const type = { chromium, firefox, webkit }[name];
    browser = await type.launch(
      name === "chromium" && process.env.PLAYWRIGHT_CHANNEL
        ? { channel: process.env.PLAYWRIGHT_CHANNEL }
        : {},
    );
    const page = await browser.newPage();
    const fetches = await observeFetch(page, "/api/");
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    page.on("console", (m) => {
      if (m.type() === "error") errors.push(m.text());
    });
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await wait(() => requests.length === 2);
    assert.equal(await page.locator("#mounts").textContent(), "0");
    const complete = (kind, product, code = 200) => {
      const request = requests.findLast(
        (r) => r.url === `/api/${kind}/${product}`,
      );
      assert(request);
      request.res.writeHead(code);
      request.res.end(`${kind}-${product}`);
    };
    complete("price", "A");
    await fetches.consumed("/api/price/A");
    assert.equal(await page.locator("#selection").textContent(), " · ");
    complete("stock", "A");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert.equal(await page.locator("#selection").textContent(), "A · en");
    assert.equal(await page.locator(".price").textContent(), "price-A");
    assert.equal(await page.locator(".price-child").textContent(), "price-A");
    assert.equal(await page.locator("#mounts").textContent(), "2");
    await page.evaluate(
      () => (globalThis.oldPrice = document.querySelector(".price")),
    );
    await page.click("#b");
    await wait(() => requests.length === 4);
    await page.evaluate(
      () => (globalThis.oldRow = document.querySelector('[data-id="2"]')),
    );
    await page.click("#rows");
    await wait(() => requests.length === 6);
    assert.equal(await page.locator("#mounts").textContent(), "2");
    assert.equal(await page.locator("#items").textContent(), "onetwo");
    assert.deepEqual(await page.locator("#items > .row-tail").evaluateAll(nodes => nodes.map(node => node.dataset.row)), ["1", "2"]);
    assert.equal(await page.locator(".coherent-panel + .panel-tail").count(), 2);
    assert.deepEqual(await page.locator("#inline-items > li").allTextContents(), ["0:one", "1:two"]);
    await page.evaluate(() => document.querySelector("#inside").click());
    assert.equal(await page.locator("#clicks").textContent(), "0");
    assert.equal(await page.locator("#selection").textContent(), "A · en");
    await page.click("#c");
    await wait(() => requests.length === 8);
    complete("stock", "C");
    await fetches.consumed("/api/stock/C");
    assert.equal(await page.locator("#selection").textContent(), "A · en");
    complete("price", "C");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert.equal(await page.locator(".stock").textContent(), "stock-C");
    assert.equal(await page.locator(".price-child").textContent(), "price-C");
    assert.equal(await page.locator(".price-child").getAttribute("title"), "price-C");
    assert.equal(await page.locator("#selection").textContent(), "C · en");
    assert(
      await page.evaluate(
        () => globalThis.oldPrice === document.querySelector(".price"),
      ),
    );
    assert(
      await page.evaluate(
        () => globalThis.oldRow === document.querySelector('[data-id="2"]'),
      ),
    );
    assert.equal(await page.locator("#items").textContent(), "secondthird");
    assert.deepEqual(await page.locator("#items > .row-tail").evaluateAll(nodes => nodes.map(node => node.dataset.row)), ["2", "3"]);
    assert.equal(await page.locator(".coherent-panel + .panel-tail").count(), 2);
    assert.deepEqual(await page.locator("#inline-items > li").allTextContents(), ["0:second", "1:third"]);
    assert.equal(await page.locator("#mounts").textContent(), "3");
    await page.click("#inside");
    assert.equal(await page.locator("#clicks").textContent(), "1");
    // Errors retain the last complete view. Retry keeps the successful sibling.
    await page.click("#a");
    await wait(() => requests.length === 10);
    complete("price", "A");
    complete("stock", "A", 503);
    await page.locator("#status").filter({ hasText: "Error" }).waitFor();
    assert.equal(await page.locator("#selection").textContent(), "C · en");
    await page.click("#retry");
    await wait(() => requests.length === 11);
    assert.equal(requests.at(-1).url, "/api/stock/A");
    complete("stock", "A");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    await page.click("#inert");
    assert(await page.locator("#product").evaluate((node) => node.inert));
    await page.click("#inert");
    assert(!(await page.locator("#product").evaluate((node) => node.inert)));
    // Programmatic selection lets us test focus restoration without clicking away.
    await page.locator("#inside").focus();
    await page.evaluate(() => document.querySelector("#b").click());
    await wait(() => requests.length === 13);
    complete("price", "B");
    complete("stock", "B");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert.equal(
      await page.evaluate(() => document.activeElement.id),
      "inside",
    );
    await page.evaluate(() => document.querySelector("#c").click());
    await wait(() => requests.length === 15);
    await page.locator("#retry").focus();
    complete("price", "C");
    complete("stock", "C");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert.equal(await page.evaluate(() => document.activeElement.id), "retry");
    // A keyed replacement stays detached until its async read is ready.
    await page.evaluate(() => globalThis.oldStock = document.querySelector(".stock"));
    const beforeReset = requests.length;
    await page.click("#reset-stock");
    await wait(() => requests.length === beforeReset + 1);
    assert(await page.evaluate(() => globalThis.oldStock === document.querySelector(".stock")));
    complete("stock", "C");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert(await page.evaluate(() => globalThis.oldStock !== document.querySelector(".stock")));
    // Hiding during a new pending replacement cancels it and removes only stock.
    await page.click("#reset-stock");
    await wait(() => requests.length === beforeReset + 2);
    const cancelled = requests.at(-1);
    await page.click("#toggle-stock");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert.equal(await page.locator(".stock").count(), 0);
    assert.equal(await page.locator(".price").textContent(), "price-C");
    cancelled.res.end("stale stock");
    await page.click("#toggle-stock");
    await wait(() => requests.length === beforeReset + 3);
    assert.equal(await page.locator(".stock").count(), 0);
    complete("stock", "C");
    await page.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert.equal(await page.locator(".stock").textContent(), "stock-C");
    assert.equal(await page.locator(".price-child").textContent(), "price-C");
    assert.equal(await page.locator(".price-child").getAttribute("title"), "price-C");
    assert.deepEqual(
      errors.filter((error) => !error.includes("503")),
      [],
    );
    // Real Prepared::apply rollback: a later attribute write fails after the
    // authored inert flag and selection text were patched. Structural apply
    // failures occur afterward and do not promise this patch rollback contract.
    const rollback = await browser.newPage();
    const rollbackErrors = await observeConsoleErrors(rollback);
    const beforeRollback = requests.length;
    await rollback.goto(`http://127.0.0.1:${server.address().port}`);
    await wait(() => requests.length === beforeRollback + 2);
    await rollback.evaluate(() => {
      const setAttribute = Element.prototype.setAttribute;
      let failedRollback = false;
      globalThis.patchTrace = [];
      globalThis.restoreAttributes = () => { Element.prototype.setAttribute = setAttribute; };
      Element.prototype.setAttribute = function (name, value) {
        if (this.id === "product" && name === "inert") {
          patchTrace.push(["inert", document.querySelector("#selection").textContent]);
          if (!failedRollback && patchTrace.some(([kind]) => kind === "failure")) {
            failedRollback = true;
            throw new Error("deliberate coherent rollback failure");
          }
        }
        if (this.classList.contains("price-child") && name === "title") {
          patchTrace.push(["failure", document.querySelector("#selection").textContent]);
          throw new Error("deliberate later coherent patch failure");
        }
        return setAttribute.call(this, name, value);
      };
    });
    await rollback.click("#inert");
    await wait(() => requests.length === beforeRollback + 4);
    complete("price", "A");
    complete("stock", "A");
    await rollback.locator("#status").filter({ hasText: "Faulted" }).waitFor().catch(async error => {
      console.error(await rollback.evaluate(() => ({ status: document.querySelector("#status").textContent, trace: patchTrace, mounts: document.querySelector("#mounts").textContent })));
      throw error;
    });
    assert.match(await rollback.locator("#status").textContent(), /deliberate later coherent patch failure/);
    const trace = await rollback.evaluate(() => patchTrace);
    assert(trace.some(([kind, text]) => kind === "failure" && text === "A · en"), JSON.stringify(trace));
    assert.deepEqual(trace.at(-1), ["inert", " · "]);
    assert.equal(rollbackErrors.filter(error => error.includes("deliberate coherent rollback failure")).length, 1);
    assert.equal(await rollback.locator("#selection").textContent(), " · ");
    assert.equal(await rollback.locator("#mounts").textContent(), "0");
    assert.equal(await rollback.locator("#items > li").count(), 0);
    assert(await rollback.locator("#product").evaluate(node => node.inert && node.getAttribute("aria-busy") === "true"));
    await rollback.evaluate(() => document.querySelector("#inside").click());
    assert.equal(await rollback.locator("#clicks").textContent(), "0");
    await rollback.evaluate(() => restoreAttributes());
    await rollback.click("#retry");
    await rollback.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert.equal(await rollback.locator("#selection").textContent(), "A · en");
    assert.equal(await rollback.locator("#mounts").textContent(), "2");
    assert(await rollback.locator("#product").evaluate(node => node.inert && !node.hasAttribute("aria-busy")));
    await rollback.click("#inert");
    assert(!(await rollback.locator("#product").evaluate(node => node.inert)));
    await rollback.click("#inside");
    assert.equal(await rollback.locator("#clicks").textContent(), "1");
    await rollback.evaluate(() => {
      const root = document.querySelector("#product");
      const remove = root.removeAttribute;
      root.removeAttribute = function (name) {
        if (name === "aria-busy") throw Error("deliberate overlay removal failure");
        return remove.call(this, name);
      };
      globalThis.restoreOverlay = () => { root.removeAttribute = remove; };
    });
    const beforeRemoval = requests.length;
    await rollback.click("#b");
    await wait(() => requests.length === beforeRemoval + 2);
    complete("price", "B");
    complete("stock", "B");
    await rollback.locator("#status").filter({ hasText: "Ready" }).waitFor();
    await wait(() => rollbackErrors.some(error => error.includes("deliberate overlay removal failure")));
    assert.equal(rollbackErrors.filter(error => error.includes("deliberate overlay removal failure")).length, 1);
    const beforeOverlay = requests.length;
    await rollback.click("#c");
    await wait(() => requests.length === beforeOverlay + 2);
    assert(await rollback.locator("#product").evaluate(node => node.inert && node.getAttribute("aria-busy") === "true"));
    await rollback.evaluate(() => restoreOverlay());
    complete("price", "C");
    complete("stock", "C");
    await rollback.locator("#status").filter({ hasText: "Ready" }).waitFor();
    assert(!(await rollback.locator("#product").evaluate(node => node.inert || node.hasAttribute("aria-busy"))));
    await rollback.close();
    console.log(
      "PASS",
      name,
      "coherent siblings, cancellation, retained nodes, real patch rollback, inert restoration and deferred activation",
    );
    await browser.close();
    browser = null;
  }
} finally {
  await browser?.close();
  server.closeAllConnections();
  await new Promise((r) => server.close(r));
}
