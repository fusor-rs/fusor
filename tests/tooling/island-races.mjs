import assert from "node:assert/strict";
// Real network acquisition and the production registry; no mocked lifecycle.
export async function islandRaces(browser, origin) {
  async function fresh() {
    const context = await browser.newContext();
    const page = await context.newPage();
    await page.goto(origin);
    await page.waitForFunction(
      () => globalThis.__fusor_islands?.inspect().instances.length === 3,
    );
    return { context, page };
  }
  // Failed installation must leave no acquired listeners or registry, including
  // a failure from MutationObserver.observe after listener registration.
  {
    const { context, page } = await fresh();
    const result = await page.evaluate(async () => {
      const boot = document.querySelector('script[type="module"][src$="/boot.js"]').src;
      const { install } = await import(new URL("registry.js", boot).href);
      const manifest = await (await fetch(new URL("manifest.json", boot))).json();
      __fusor_islands.destroy();
      const composing = globalThis.__fusor_composing;
      const add = document.addEventListener, remove = document.removeEventListener;
      const listeners = new Set();
      document.addEventListener = function (type, callback, options) {
        listeners.add(callback); return add.call(this, type, callback, options);
      };
      document.removeEventListener = function (type, callback, options) {
        listeners.delete(callback); return remove.call(this, type, callback, options);
      };
      const failures = [];
      try {
        for (const corrupt of [
          value => { Object.values(value.units)[0].javascript = "relative.js"; },
          value => { Object.values(value.units)[0].entries = []; },
          value => { Object.values(value.units)[0].entries[0].props_schema = null; },
        ]) {
          for (let attempt = 0; attempt < 2; attempt++) {
            const invalid = structuredClone(manifest); corrupt(invalid);
            let code;
            try { install(invalid); } catch (error) { code = error.code; }
            failures.push([code, listeners.size, !!globalThis.__fusor_islands, globalThis.__fusor_composing === composing]);
          }
        }
        let postAcquisitionFailed = false;
        try { install(manifest, { root: {} }); } catch { postAcquisitionFailed = true; }
        const afterFailure = [postAcquisitionFailed, listeners.size, !!globalThis.__fusor_islands, globalThis.__fusor_composing === composing];
        const registry = install(manifest, { root: document.createDocumentFragment() });
        const activeListeners = listeners.size;
        registry.destroy();
        return { failures, afterFailure, activeListeners, disposed: [listeners.size, !!globalThis.__fusor_islands] };
      } finally {
        document.addEventListener = add; document.removeEventListener = remove;
      }
    });
    assert.deepEqual(result.failures, Array.from({ length: 6 }, () => ["protocol-mismatch", 0, false, true]));
    assert.deepEqual(result.afterFailure, [true, 0, false, true]);
    assert.equal(result.activeListeners, 3);
    assert.deepEqual(result.disposed, [0, false]);
    await context.close();
  }
  // One canceled caller must not cancel another caller of the same instance.
  {
    const { context, page } = await fresh();
    let release;
    let acquired;
    const seen = new Promise((resolve) => (acquired = resolve));
    await page.route("**/cart/unit_bg.wasm", async (route) => {
      await new Promise((resolve) => {
        release = resolve;
        acquired();
      });
      await route.continue();
    });
    await page.evaluate(() => {
      const one = __fusor_islands.activate("cart-one");
      globalThis.first = one.promise.catch((error) => error.code);
      globalThis.second = __fusor_islands.activate("cart-one").promise;
      one.cancel();
    });
    await seen;
    assert.equal(await page.evaluate(() => first), "cancelled");
    await page
      .locator("#cart-one input[name=quantity]")
      .fill("slow-network edit");
    release();
    await page.evaluate(() => second);
    assert.equal(
      await page.locator("#cart-one input[name=quantity]").inputValue(),
      "slow-network edit",
    );
    await page.locator("#cart-one button[type=submit]").click();
    assert.equal(
      await page.locator("#cart-one .submissions").textContent(),
      "1",
    );
    assert.equal(
      await page.evaluate(
        () =>
          __fusor_islands
            .inspect()
            .instances.find((item) => item.id === "cart-one").waiters,
      ),
      0,
    );
    await context.close();
  }
  // A transient Wasm response can be retried at the same immutable URL.
  {
    const { context, page } = await fresh();
    let attempts = 0;
    await page.route("**/cart/unit_bg.wasm", (route) =>
      ++attempts === 1
        ? route.fulfill({ status: 503, body: "deliberate transient failure" })
        : route.continue(),
    );
    assert.equal(
      await page.evaluate(() =>
        __fusor_islands.activate("cart-one").promise.then(
          () => "",
          (error) => error.code,
        ),
      ),
      "load-failed",
    );
    assert.equal(
      await page.getAttribute("#cart-one", "data-fusor-status"),
      "failed",
    );
    await page.evaluate(() => __fusor_islands.retry("cart-one").promise);
    assert.equal(
      await page.getAttribute("#cart-one", "data-fusor-status"),
      "active",
    );
    assert.equal(attempts, 2);
    await context.close();
  }
  // Removal during acquisition invalidates every late attachment path.
  {
    const { context, page } = await fresh();
    const cancelledLoad = page.waitForEvent("requestfailed", {
      predicate: request => request.url().endsWith("/cart/unit_bg.wasm"),
    });
    let release, acquired;
    const seen = new Promise((resolve) => (acquired = resolve));
    await page.route("**/cart/unit_bg.wasm", async (route) => {
      await new Promise((resolve) => {
        release = resolve;
        acquired();
      });
      // Detaching the final claimant cancels the intercepted request.
      await route.continue().catch(() => {});
    });
    await page.evaluate(() => {
      globalThis.waiting = __fusor_islands
        .activate("cart-one")
        .promise.then(
          () => "",
          (error) => error.code,
        );
    });
    await seen;
    await page.evaluate(() => {
      globalThis.detached = document.querySelector("#cart-one");
      detached.remove();
    });
    assert.equal(await page.evaluate(() => waiting), "cancelled");
    release();
    await cancelledLoad;
    assert(
      await page.evaluate(
        () =>
          !__fusor_islands
            .inspect()
            .instances.some((item) => item.id === "cart-one") &&
          detached.dataset.fusorStatus !== "active",
      ),
    );
    await context.close();
  }
  // Fallbacks still activate when optional browser schedulers are unavailable.
  {
    const context = await browser.newContext();
    await context.addInitScript(() => {
      globalThis.IntersectionObserver = undefined;
      globalThis.requestIdleCallback = undefined;
    });
    const page = await context.newPage();
    await page.goto(origin);
    await page.waitForFunction(() => globalThis.__fusor_islands);
    await page.evaluate(() => {
      for (const policy of ["visible", "idle"]) {
        const host = document.querySelector("#cart-one").cloneNode(true);
        host.id = "fallback-" + policy;
        host.dataset.fusorActivate = policy;
        document.querySelector("main").append(host);
      }
    });
    for (const policy of ["visible", "idle"])
      await page.waitForFunction(
        (id) => document.getElementById(id)?.dataset.fusorStatus === "active",
        "fallback-" + policy,
      );
    await context.close();
  }
  // A page with expired assets must fail, not silently load a new generation.
  {
    const { context, page } = await fresh();
    await page.route("**/cart/unit_bg.wasm", (route) =>
      route.fulfill({ status: 404, body: "expired generation" }),
    );
    assert.equal(
      await page.evaluate(() =>
        __fusor_islands.activate("cart-one").promise.then(
          () => "",
          (error) => error.code,
        ),
      ),
      "load-failed",
    );
    assert.equal(
      await page.locator("#cart-one input[name=quantity]").inputValue(),
      "1",
    );
    assert.equal(
      await page.getAttribute("#cart-one", "data-fusor-status"),
      "failed",
    );
    await context.close();
  }
  // Diagnostic cold/warm activation observations, separate from artificial delays.
  const { context, page } = await fresh();
  const latency = await page.evaluate(async () => {
    const initial = performance
      .getEntriesByType("resource")
      .map((entry) => ({
        url: new URL(entry.name).pathname,
        transferSize: entry.transferSize,
        encodedBodySize: entry.encodedBodySize,
      }));
    let start = performance.now();
    await __fusor_islands.activate("cart-one").promise;
    const cold = performance.now() - start;
    start = performance.now();
    await __fusor_islands.activate("cart-two").promise;
    const warm = performance.now() - start;
    return {
      coldMs: cold,
      warmMs: warm,
      initial,
      after: performance
        .getEntriesByType("resource")
        .map((entry) => ({
          url: new URL(entry.name).pathname,
          transferSize: entry.transferSize,
          encodedBodySize: entry.encodedBodySize,
        })),
    };
  });
  await context.close();
  return latency;
}
