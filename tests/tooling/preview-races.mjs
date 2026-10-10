import assert from "node:assert/strict";

export async function previewRaces(browser, origin) {
  async function fresh() {
    const context = await browser.newContext();
    const page = await context.newPage();
    const requests = [];
    await page.route("**/api/**", (route) => {
      requests.push(route);
    });
    await page.goto(origin);
    await page.waitForFunction(() => globalThis.__fusor_islands);
    await page.evaluate(() => {
      globalThis.preview = document.querySelector("#designer .preview");
    });
    return { context, page, requests };
  }
  async function reached(requests, n) {
    for (let i = 0; i < 500; i++) {
      if (requests.length >= n) return;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    throw Error(
      `Expected ${n} independent read requests, saw ${requests.length}`,
    );
  }
  const value = (route) =>
    route.request().url().includes("/price/") ? "price-A" : "stock-A";
  // Failure before the first coherent publication keeps native HTML. Retry
  // constructs a fresh prepared scope while reusing the already initialized unit.
  {
    const { context, page, requests } = await fresh();
    await page.evaluate(() => {
      globalThis.activation = __fusor_islands
        .activate("designer")
        .promise.then(
          () => "",
          (error) => error.code,
        );
    });
    await reached(requests, 2);
    assert(
      await page.evaluate(
        () => document.querySelector("#designer .preview") === preview,
      ),
    );
    assert.equal(await page.locator("#designer .designer").count(), 0);
    await requests[0].fulfill({
      status: 503,
      body: "initial read deliberately failed",
    });
    assert.equal(await page.evaluate(() => activation), "binding-failed");
    const causal = await page.evaluate(async () => {
      await __fusor_islands.activate('cart-one').promise;
      const script = performance.getEntriesByType('resource').find(item => /\/cart\/unit.js$/.test(item.name));
      const controller = await import(script.name);
      try {
        await controller.exercise_control('activate');
        return '';
      } catch (error) {
        return String(error);
      }
    });
    assert.match(causal, /initial coherent view failed:/);
    assert.match(causal, /503/);
    assert(
      await page.evaluate(
        () => document.querySelector("#designer .preview") === preview,
      ),
    );
    await requests[1]
      .fulfill({ status: 200, body: value(requests[1]) })
      .catch(() => {});
    // The native interaction button is also an explicit retry action.
    await page.locator("#open-designer").click();
    await reached(requests, 4);
    for (const route of requests.slice(2))
      await route.fulfill({ status: 200, body: value(route) });
    await page.waitForFunction(
      () => document.querySelector("#designer").dataset.fusorStatus === "active",
    );
    assert.equal(await page.locator("#designer .selection").textContent(), "A");
    assert.equal(await page.locator("#designer .preview").count(), 0);
    await context.close();
  }
  // Cancel and immediately restart. The old attempt's rejection must never
  // dispose the new attempt, even though both refer to the same HTML instance.
  {
    const { context, page, requests } = await fresh();
    await page.evaluate(() => {
      globalThis.first = __fusor_islands.activate("designer");
      globalThis.cancelled = first.promise.then(
        () => "",
        (error) => error.code,
      );
    });
    await reached(requests, 2);
    await page.evaluate(() => {
      first.cancel();
      globalThis.next = __fusor_islands.activate("designer").promise;
    });
    assert.equal(await page.evaluate(() => cancelled), "cancelled");
    await reached(requests, 4);
    assert(
      await page.evaluate(
        () => document.querySelector("#designer .preview") === preview,
      ),
    );
    for (const route of requests.slice(2))
      await route.fulfill({ status: 200, body: value(route) });
    await page.evaluate(() => next);
    for (const route of requests.slice(0, 2))
      await route.fulfill({ status: 200, body: "obsolete" }).catch(() => {});
    assert.equal(await page.locator("#designer .designer").count(), 1);
    assert.equal(
      await page.locator("#designer .price").textContent(),
      "price-A",
    );
    assert.equal(
      await page.getAttribute("#designer", "data-fusor-status"),
      "active",
    );
    await context.close();
  }
  // Removal while data is pending releases the scope and forbids late attachment.
  {
    const { context, page, requests } = await fresh();
    await page.evaluate(() => {
      globalThis.pending = __fusor_islands
        .activate("designer")
        .promise.then(
          () => "",
          (error) => error.code,
        );
    });
    await reached(requests, 2);
    await page.evaluate(() => {
      globalThis.removed = document.querySelector("#designer");
      removed.remove();
    });
    assert.equal(await page.evaluate(() => pending), "cancelled");
    for (const route of requests)
      await route.fulfill({ status: 200, body: value(route) }).catch(() => {});
    assert(
      await page.evaluate(
        () =>
          removed.querySelector(".preview") === preview &&
          !removed.querySelector(".designer"),
      ),
    );
    assert(
      await page.evaluate(
        () =>
          !__fusor_islands
            .inspect()
            .instances.some((instance) => instance.id === "designer"),
      ),
    );
    await context.close();
  }
}
