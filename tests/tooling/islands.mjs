// Production-built independent Wasm units, native HTML, and a controlled API.
import assert from "node:assert/strict";
import { islandRaces } from "./island-races.mjs";
import { previewRaces } from "./preview-races.mjs";
import { createServer } from "node:http";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
import { chromium, firefox, webkit } from "playwright";
import { observeFetch } from "../../scripts/observe-fetch.mjs";
const root = resolve("examples/islands/site/dist");
const requests = [],
  api = [];
let buys = 0;
const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost");
  requests.push(url.pathname);
  if (url.pathname === "/favicon.ico") {
    res.writeHead(204);
    res.end();
    return;
  }
  if (url.pathname === "/buy") {
    buys++;
    req.resume();
    res.end("Native form received");
    return;
  }
  if (url.pathname.startsWith("/api/")) {
    api.push({ url: url.pathname, res });
    return;
  }
  const path = resolve(
    root,
    "." + (url.pathname === "/" ? "/index.html" : url.pathname),
  );
  if (!path.startsWith(root + sep)) {
    res.writeHead(404);
    res.end();
    return;
  }
  try {
    const data = await readFile(path);
    res.setHeader(
      "content-type",
      {
        ".js": "text/javascript",
        ".wasm": "application/wasm",
        ".json": "application/json",
        ".html": "text/html",
      }[extname(path)] || "application/octet-stream",
    );
    res.setHeader(
      "cache-control",
      url.pathname.startsWith("/__fusor/")
        ? "public,max-age=31536000,immutable"
        : "no-store",
    );
    res.end(data);
  } catch {
    res.writeHead(404);
    res.end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
async function wait(read, label) {
  const until = Date.now() + 15_000;
  while (!(await read())) {
    if (Date.now() > until) throw Error(`Timed out: ${label}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
}
let browser;
const latencies = {};
try {
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(
    ",",
  )) {
    browser = await { chromium, firefox, webkit }[name].launch(
      name === "chromium" && process.env.PLAYWRIGHT_CHANNEL
        ? { channel: process.env.PLAYWRIGHT_CHANNEL }
        : {},
    );
    const native = await browser.newContext({ javaScriptEnabled: false });
    const fallback = await native.newPage();
    const beforeBuy = buys;
    await fallback.goto(origin);
    assert.equal(
      await fallback.locator("#cart-one .product-id").textContent(),
      "Product 9007199254740993",
    );
    assert.deepEqual(await fallback.locator("#cart-one .inline-lines > li").allTextContents(), ["0:1", "1:2"]);
    assert.deepEqual(await fallback.locator("#cart-one .note-label").allTextContents(), ["Nested note"]);
    await fallback.locator("#cart-one input[name=quantity]").fill("6");
    await fallback.locator("#cart-one button[type=submit]").click();
    await wait(() => buys === beforeBuy + 1, "one native submission");
    await native.close();
    const context = await browser.newContext();
    const page = await context.newPage();
    const fetches = await observeFetch(page, "/api/");
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    requests.length = 0;
    api.length = 0;
    await page.goto(origin);
    await page.waitForFunction(
      () => globalThis.__fusor_islands?.inspect().instances.length === 3,
    );
    assert(
      !requests.some(
        (path) => path.includes("/cart/") || path.includes("/designer/"),
      ),
    );
    await page.evaluate(() => {
      globalThis.nativeInput = document.querySelector(
        "#cart-one input[name=quantity]",
      );
    });
    await page.locator("#cart-one input[name=quantity]").fill("7");
    await page.locator("#cart-one textarea").fill("Edited before activation");
    await page.locator("#cart-one .nested-input").fill("Child edited");
    await page.locator("#cart-one .line-input").first().fill("Row edited");
    await page.evaluate(() => {
      globalThis.nativeNoteLabel = document.querySelector("#cart-one .note-label");
      globalThis.nativeChild = document.querySelector(
        "#cart-one .nested-input",
      );
      globalThis.nativeRow = document.querySelector("#cart-one .line-input");
      globalThis.nativeInlineRow = document.querySelector("#cart-one .inline-lines > li");
    });
    await page.locator("#cart-one input[type=checkbox]").check();
    // The server writes the bound select and radio state before any code loads.
    assert.equal(await page.locator("#cart-one select[name=size]").inputValue(), "medium");
    assert(await page.locator("#cart-one input[name=wrap][value=plain]").isChecked());
    await page.locator("#cart-one select[name=size]").selectOption("large");
    await page.locator("#cart-one input[name=wrap][value=paper]").check();
    await page.evaluate(
      () => globalThis.__fusor_islands.prefetch("cart-one").promise,
    );
    assert.equal(
      await page.getAttribute("#cart-one", "data-fusor-status"),
      "dormant",
    );
    assert.equal(api.length, 0);
    assert(!requests.some((path) => path.includes("/designer/")));
    const cartLoads = requests.filter((path) => path.includes("/cart/")).length;
    assert(cartLoads >= 2);
    await page.locator("#cart-one input[name=quantity]").focus();
    await page.evaluate(() => {
      const add = document.addEventListener;
      globalThis.compositionWaiterAttached = false;
      document.addEventListener = function (type, listener, options) {
        add.call(this, type, listener, options);
        if (type === "compositionend" && options === true) {
          document.addEventListener = add;
          globalThis.compositionWaiterAttached = true;
        }
      };
      const input = document.querySelector("#cart-one input[name=quantity]");
      input.dispatchEvent(
        new CompositionEvent("compositionstart", { bubbles: true }),
      );
      globalThis.activating = __fusor_islands.activate("cart-one").promise;
    });
    await page.waitForFunction(() => compositionWaiterAttached);
    assert.equal(
      await page.getAttribute("#cart-one", "data-fusor-status"),
      "requested",
    );
    await page.evaluate(() => {
      const input = document.querySelector("#cart-one input[name=quantity]");
      input.value = "7あ";
      input.dispatchEvent(
        new CompositionEvent("compositionend", { bubbles: true }),
      );
      return globalThis.activating;
    });
    assert.equal(
      await page.getAttribute("#cart-one", "data-fusor-status"),
      "active",
    );
    assert(
      await page.evaluate(
        () =>
          globalThis.nativeInput ===
          document.querySelector("#cart-one input[name=quantity]"),
      ),
    );
    assert.equal(
      await page.locator("#cart-one input[name=quantity]").inputValue(),
      "7あ",
    );
    assert(
      await page.evaluate(
        () =>
          nativeNoteLabel === document.querySelector("#cart-one .note-label") &&
          nativeChild === document.querySelector("#cart-one .nested-input") &&
          nativeRow === document.querySelector("#cart-one .line-input") &&
          nativeInlineRow === document.querySelector("#cart-one .inline-lines > li"),
      ),
    );
    assert.equal(
      await page.locator("#cart-one .nested-input").inputValue(),
      "Child edited",
    );
    assert.equal(
      await page.locator("#cart-one .line-value").first().textContent(),
      "Row edited",
    );
    assert.equal(
      await page.locator("#cart-one .draft").textContent(),
      "7あ · Edited before activation · true · large · paper",
    );
    assert.equal(await page.locator("#cart-one select[name=size]").inputValue(), "large");
    assert(!(await page.locator("#cart-one input[name=wrap][value=plain]").isChecked()));
    await page.locator("#cart-one select[name=size]").selectOption("small");
    await page.locator("#cart-one input[name=wrap][value=plain]").check();
    assert.equal(
      await page.locator("#cart-one .draft").textContent(),
      "7あ · Edited before activation · true · small · plain",
    );
    assert.equal(
      await page.locator("#cart-one .product-id").textContent(),
      "Product 9007199254740993",
    );
    await page.locator("#cart-one button[type=submit]").click();
    assert.equal(
      await page.locator("#cart-one .submissions").textContent(),
      "1",
    );
    assert.equal(buys, beforeBuy + 1);
    // A server value that matches no option leaves the first one showing,
    // which activation must not adopt as an edit.
    await page.evaluate(() => {
      for (const option of document.querySelectorAll("#cart-two select[name=size] option"))
        option.removeAttribute("selected");
    });
    await page.evaluate(
      () => globalThis.__fusor_islands.activate("cart-two").promise,
    );
    assert.equal(
      await page.locator("#cart-two input[name=quantity]").inputValue(),
      "2",
    );
    assert.equal(await page.locator("#cart-two select[name=size]").inputValue(), "medium");
    assert.equal(
      await page.locator("#cart-two .submissions").textContent(),
      "0",
    );
    assert.equal(
      requests.filter((path) => path.includes("/cart/")).length,
      cartLoads,
    );
    // Exercise the actual Rust future and owner cancellation paths in an active unit.
    await page.evaluate(async () => {
      const script = performance
        .getEntriesByType("resource")
        .find((item) => /\/cart\/unit.js$/.test(item.name));
      const unit = (globalThis.controllerUnit = await import(script.name));
      unit.exercise_waiter(false);
      unit.exercise_waiter(true);
      await new Promise((resolve) => setTimeout(resolve, 20));
      if (
        __fusor_islands
          .inspect()
          .instances.find((item) => item.id === "designer").waiters !== 0
      )
        throw Error("Rust waiter leaked");
    });
    const restoration = await page.evaluate(async () => {
      const host = document.createElement('div');
      document.body.append(host);
      try {
        await controllerUnit.exercise_preview_restoration(host);
        throw Error('expected preview commit rejection');
      } catch (error) {
        return { message: error.message, causes: error.cause?.map(cause => typeof cause === 'string' ? cause : cause.name) };
      } finally {
        host.remove();
      }
    });
    assert.match(restoration.message, /island commit failed:.*fallback restoration failed:/);
    assert.deepEqual(restoration.causes, ['component disposed during activation', 'NotFoundError']);
    await page.evaluate(() => controllerUnit.exercise_messages());
    await page.evaluate(() => controllerUnit.exercise_control("prefetch"));
    assert.equal(api.length, 0);
    assert.equal(
      await page.getAttribute("#designer", "data-fusor-status"),
      "dormant",
    );
    assert.equal(
      await page.evaluate(() => controllerUnit.exercise_status()),
      "Dormant",
    );
    await page.evaluate(() => {
      globalThis.originalPreview = document.querySelector("#designer .preview");
      globalThis.designerActivation = controllerUnit.exercise_control("activate");
    });
    // A concurrent keyboard policy request joins the prepared activation.
    await page.locator("#open-designer").focus();
    await page.keyboard.press("Enter");
    await wait(() => api.length === 2, "designer descendant reads");
    assert(
      requests.filter(
        (path) => path.includes("/designer/") && !path.startsWith("/api/"),
      ).length >= 2,
    );
    assert.equal(await page.locator("#designer .designer").count(), 0);
    assert(
      await page.evaluate(
        () => document.querySelector("#designer .preview") === originalPreview,
      ),
    );
    const price = api.find((request) => request.url.includes("/price/"));
    price.res.end("price-A");
    await fetches.consumed(price.url);
    assert.equal(await page.locator("#designer .designer").count(), 0);
    assert(
      await page.evaluate(
        () => document.querySelector("#designer .preview") === originalPreview,
      ),
    );
    api.find((request) => request.url.includes("/stock/")).res.end("stock-A");
    await page
      .locator("#designer .status")
      .filter({ hasText: "Ready" })
      .waitFor();
    assert.equal(await page.locator("#designer .selection").textContent(), "A");
    await page.evaluate(() => designerActivation);
    assert.equal(await page.locator("#designer .preview").count(), 0);
    assert.equal(
      await page.evaluate(() => controllerUnit.exercise_status()),
      "Active",
    );
    // Retrying an active island joins its completed activation.
    await page.evaluate(() => controllerUnit.exercise_control("retry"));
    assert.equal(await page.locator("#designer .designer").count(), 1);
    // A same-document move preserves the registration. Reusing an ID does not.
    await page.evaluate(async () => {
      const host = document.querySelector("#cart-two");
      globalThis.oldToken = __fusor_islands.lookup(
        "cart-two",
        host.dataset.fusorIsland,
        host.dataset.fusorSchema,
      );
      const main = document.querySelector("main");
      const moved = new Promise(resolve => {
        const observer = new MutationObserver(() => {
          observer.disconnect();
          resolve();
        });
        observer.observe(main, { childList: true });
      });
      main.append(host);
      await moved;
    });
    assert.equal(
      await page.evaluate(() => __fusor_islands.status(oldToken)),
      "active",
    );
    await page.evaluate(() => {
      const host = document.querySelector("#cart-two");
      globalThis.copy = host.cloneNode(true);
      host.remove();
    });
    await page.waitForFunction(() => {
      try {
        __fusor_islands.status(oldToken);
        return false;
      } catch (error) {
        return error.code === "stale-instance";
      }
    });
    await page.evaluate(() => document.querySelector("main").append(copy));
    await page.waitForFunction(
      () => document.querySelector("#cart-two").dataset.fusorStatus === "dormant",
    );
    await page.evaluate(
      () => __fusor_islands.activate("cart-two").promise,
    );
    // All automatic policies use the same loaded unit and independent instance state.
    for (const policy of ["load", "visible", "idle"]) {
      await page.evaluate((policy) => {
        const host = document.querySelector("#cart-two").cloneNode(true);
        host.id = "policy-" + policy;
        host.dataset.fusorActivate = policy;
        document.querySelector("main").prepend(host);
        if (policy === "visible") host.scrollIntoView();
      }, policy);
      await page.waitForFunction(
        (id) => document.getElementById(id)?.dataset.fusorStatus === "active",
        "policy-" + policy,
      );
    }
    // A failed descendant contract must leave every existing native node intact.
    await page.evaluate(() => {
      const host = document.querySelector("#cart-two").cloneNode(true);
      host.id = "invalid-child";
      host.dataset.fusorActivate = "manual";
      host.querySelector("input[name=quantity]").value = "Edited while dormant";
      host.querySelector("li").dataset.fusorKey = "wrong";
      globalThis.failedDraft = host.querySelector(".draft").textContent;
      globalThis.failedChild = host.querySelector(".nested-input");
      document.querySelector("main").append(host);
    });
    await page.waitForFunction(
      () =>
        document.querySelector("#invalid-child").dataset.fusorStatus === "dormant",
    );
    assert.equal(
      await page.evaluate(async () => {
        try {
          await __fusor_islands.activate("invalid-child").promise;
          return false;
        } catch (error) {
          return error.code === "binding-failed";
        }
      }),
      true,
    );
    assert(
      await page.evaluate(
        () =>
          document.querySelector("#invalid-child .draft").textContent ===
            failedDraft &&
          document.querySelector("#invalid-child .nested-input") ===
            failedChild,
      ),
    );
    assert.deepEqual(errors, []);
    console.log(
      `PASS ${name}: native forms, independent units, prefetch without activation, exact u64 props, IME/input adoption, shared code/independent state, combined coherent island`,
    );
    await context.close();
    await previewRaces(browser, origin);
    console.log(
      `PASS ${name}: initial preview retention, first-read error/retry, cancellation/restart and removal during prepared reads`,
    );
    latencies[name] = await islandRaces(browser, origin);
    console.log(
      `PASS ${name}: independent cancellation, delayed edits, retry, removal during fetch, scheduler fallbacks, expired generation`,
    );
    await browser.close();
    browser = null;
  }
  await mkdir("test-results", { recursive: true });
  await writeFile(
    "test-results/selective-activation-latency.json",
    JSON.stringify(
      {
        generatedAt: new Date().toISOString(),
        method:
          "One cold first-instance and warm second-instance observation per browser; localhost, uncompressed assets, no throttling. Diagnostic only, not statistically sampled benchmark results.",
        browsers: latencies,
      },
      null,
      2,
    ) + "\n",
  );
} finally {
  await browser?.close();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
