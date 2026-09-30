import { readFile, mkdir } from "node:fs/promises";
import assert from "node:assert/strict";
import { expect } from "@playwright/test";

export async function checkHighlighting(page, origin) {
  await page.goto(origin + "project-structure");
  await expect(page.locator("h1")).toHaveText("Project structure");
  for (const dark of [false, true]) {
    if (
      (await page.locator(".site").getAttribute("class")).includes("dark") !==
      dark
    )
      await page.getByRole("button", { name: "Toggle color theme" }).click();
    for (const id of ["configuration", "app-state", "entry", "workspace"]) {
      const palette = await page
        .locator(`#${id} .code-block`)
        .evaluate((block) => {
          const rgb = (value) =>
            value
              .match(/[\d.]+/g)
              .slice(0, 3)
              .map(Number);
          const luminance = (value) =>
            rgb(value).reduce((sum, channel, i) => {
              const c = channel / 255;
              return (
                sum +
                [0.2126, 0.7152, 0.0722][i] *
                  (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
              );
            }, 0);
          const bg = luminance(getComputedStyle(block).backgroundColor);
          const colors = [
            ...new Set(
              [...block.querySelectorAll(".syntax-token")].map(
                (token) => getComputedStyle(token).color,
              ),
            ),
          ];
          return colors.map((color) => {
            const fg = luminance(color);
            return {
              color,
              contrast: (Math.max(bg, fg) + 0.05) / (Math.min(bg, fg) + 0.05),
            };
          });
        });
      assert(palette.length >= 2, `${id} should have syntax colors`);
      assert(
        palette.every(({ contrast }) => contrast >= 4.49),
        `${id} has low-contrast tokens: ${JSON.stringify(palette)}`,
      );
    }
  }
  await page.getByRole("button", { name: "Toggle color theme" }).click();
}

export async function checkShowcase(page, origin, browserName, demos) {
  await page.addInitScript(() => {
    const start = window.setInterval.bind(window);
    const stop = window.clearInterval.bind(window);
    window.demoIntervalsForTest = new Set();
    window.demoFramesForTest = new Set();
    const requestFrame = window.requestAnimationFrame.bind(window);
    const cancelFrame = window.cancelAnimationFrame.bind(window);
    window.requestAnimationFrame = (callback) => {
      const id = requestFrame((time) => {
        window.demoFramesForTest.delete(id);
        callback(time);
      });
      window.demoFramesForTest.add(id);
      return id;
    };
    window.cancelAnimationFrame = (id) => {
      window.demoFramesForTest.delete(id);
      cancelFrame(id);
    };
    window.demoObserversForTest = new Set();
    const NativeObserver = window.ResizeObserver;
    window.ResizeObserver = class extends NativeObserver {
      observe(...args) { window.demoObserversForTest.add(this); return super.observe(...args); }
      disconnect() { window.demoObserversForTest.delete(this); return super.disconnect(); }
    };
    window.demoContextsForTest = [];
    const getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function(type, ...args) {
      const context = getContext.call(this, type, ...args);
      if (context && /^(webgl|webgl2)$/.test(type) && !window.demoContextsForTest.includes(context)) {
        window.demoContextsForTest.push(context);
      }
      return context;
    };
    window.setInterval = (...args) => {
      const id = start(...args);
      window.demoIntervalsForTest.add(id);
      return id;
    };
    window.clearInterval = (id) => {
      window.demoIntervalsForTest.delete(id);
      stop(id);
    };
  });
  await page.setViewportSize({ width: 1440, height: 1050 });
  await page.emulateMedia({ reducedMotion: "reduce" });
  const libraryRequests = [];
  const recordLibrary = (request) => {
    if (/\/(?:auto|three[^/]*|RoomEnvironment)-[^/]+\.js(?:\?|$)/.test(request.url())) libraryRequests.push(request.url());
  };
  page.on("request", recordLibrary);
  await page.goto(origin);
  await page
    .locator(".topbar")
    .getByRole("link", { name: "Showcase", exact: true })
    .click();
  await expect(page.locator(".showcase-card")).toHaveCount(demos.length);
  await expect(page).toHaveURL(origin + "showcase");
  assert.deepEqual(libraryRequests, [], "Rust-only docs/gallery routes must not load library chunks");
  await page.evaluate(() => {
    window.showcaseNavigationMarker = true;
  });
  if (browserName === "chromium") {
    await mkdir("target/screenshots", { recursive: true });
    await screenshot(page, "target/screenshots/showcase-desktop.png");
    await page.getByRole("button", { name: "Toggle color theme" }).click();
    await screenshot(page, "target/screenshots/showcase-dark.png");
    await page.getByRole("button", { name: "Toggle color theme" }).click();
  }

  for (const demo of demos) {
    await page.locator(`.showcase-card[data-demo="${demo.slug}"]`).click();
    await expect(page.locator("h1")).toHaveText(demo.title);
    assert(
      await page.evaluate(() => window.showcaseNavigationMarker),
      "demo navigation must stay in the Rust app",
    );
    await expect(page.locator(".demo-canvas > *")).toHaveCount(1);
    for (const [language, folder, extension] of [
      ["Rust", "src", "rs"],
      ["HTML", "web", "html"],
      ...(demo.javascript ? [["JavaScript", "web", "js"]] : []),
    ]) {
      await page
        .locator(".source-switch")
        .getByRole("button", { name: language, exact: true })
        .click();
      const source = await readFile(
        `apps/docs/${folder}/demos/${demo.slug}.${extension}`,
        "utf8",
      );
      await expect(page.locator(".demo-source code").first()).toHaveText(
        source,
        { useInnerText: false },
      );
      assert.equal(
        await page.locator(".demo-source pre code").textContent(),
        source,
        "highlighting must preserve every byte of source text",
      );
      assert((await page.locator(".demo-source .syntax-token").count()) > 10);
      const raw = await page.locator(".source-controls a").getAttribute("href");
      const response = await page.request.get(new URL(raw, origin).href);
      assert.equal(response.status(), 200);
      assert.equal(await response.text(), source);
    }
    await page
      .locator(".source-switch")
      .getByRole("button", { name: "Rust", exact: true })
      .click();

    const canvas = page.locator(".demo-canvas");
    if (demo.slug === "chartjs") {
      await expect(canvas.locator(".library-status")).toHaveText("Chart.js ready · data lives in Rust");
      await expect(canvas.locator(".chart-total output")).toHaveText("538");
      assert(libraryRequests.some(url => /\/auto-/.test(url)), "opening the chart downloads Chart.js lazily");
      const chart = canvas.locator("canvas");
      const box = await chart.boundingBox();
      await chart.click({ position: { x: box.width * 0.55, y: box.height * 0.4 } });
      await expect(canvas.locator(".chart-selection")).toContainText("visits");
      // The native chart click sets Rust selection, which also updates day buttons.
      await expect(canvas.locator('.chart-days button[aria-pressed="true"]')).toHaveCount(1);
      await canvas.getByRole("button", { name: "Mon", exact: true }).click();
      await expect(canvas.locator(".chart-selection")).toHaveText("Monday · 42 visits");
      await canvas.getByRole("button", { name: "+12 visits", exact: true }).click();
      await expect(canvas.locator(".chart-selection")).toHaveText("Monday · 54 visits");
      await expect(canvas.locator(".chart-total output")).toHaveText("550");
      await canvas.getByRole("button", { name: "Smooth curve", exact: true }).click();
      await expect(canvas.getByRole("button", { name: "Smooth curve", exact: true })).toHaveAttribute("aria-pressed", "false");
      if (browserName === "chromium") await canvas.locator(".demo-chartjs").screenshot({ path: "target/screenshots/showcase-chartjs.png" });
      await canvas.getByRole("button", { name: "Next week", exact: false }).click();
      await expect(canvas.locator(".chart-total output")).toHaveText("571");
      await page.getByRole("button", { name: "Reset demo" }).click();
      await expect(canvas.locator(".library-status")).toContainText("Chart.js ready");
      await expect(canvas.locator(".chart-total output")).toHaveText("538");
      await expect.poll(() => page.evaluate(() => window.demoObserversForTest.size)).toBe(1);
      await page.setViewportSize({ width: 390, height: 844 });
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      if (browserName === "chromium") await canvas.locator(".demo-chartjs").screenshot({ path: "target/screenshots/showcase-chartjs-mobile.png" });
      await page.setViewportSize({ width: 1440, height: 1050 });
    } else if (demo.slug === "threejs") {
      await expect(canvas.locator(".library-status")).toHaveText("Three.js ready · reduced motion");
      await expect.poll(() => page.evaluate(() => window.demoFramesForTest.size)).toBe(0);
      const garden = canvas.locator("canvas");
      const initial = await garden.screenshot();
      assert(libraryRequests.some(url => /\/three\.module-/.test(url)), "opening the garden downloads Three.js lazily");
      if (browserName === "chromium") await canvas.locator(".demo-threejs").screenshot({ path: "target/screenshots/showcase-threejs.png" });
      const bounds = await garden.boundingBox();
      await garden.click({ position: { x: bounds.width * 0.55, y: bounds.height * 0.42 } });
      await expect(canvas.locator(".garden-selection")).toContainText("SELECTED IN RUST");
      const picked = Number((await canvas.locator(".garden-selection").textContent()).match(/PETAL (\d+)/)[1]);
      await garden.focus();
      await garden.press("ArrowRight");
      await expect(canvas.locator(".garden-selection")).toHaveText(`PETAL ${String(picked % 24 + 1).padStart(2, "0")} / SELECTED IN RUST`);
      await canvas.getByRole("button", { name: "Ember palette", exact: true }).click();
      await expect(canvas.getByRole("button", { name: "Ember palette", exact: true })).toHaveAttribute("aria-pressed", "true");
      await canvas.getByRole("slider", { name: "Bloom", exact: false }).focus();
      await page.keyboard.press("Home");
      await expect(canvas.locator(".garden-bloom output")).toHaveText("20%");
      assert(!initial.equals(await garden.screenshot()), "Rust palette/bloom changes must alter rendered pixels");
      await page.keyboard.press("End");
      await expect(canvas.locator(".garden-bloom output")).toHaveText("100%");
      await canvas.getByRole("button", { name: "Pause", exact: false }).click();
      await page.emulateMedia({ reducedMotion: "no-preference" });
      await expect.poll(() => page.evaluate(() => window.demoFramesForTest.size)).toBe(0);
      await canvas.getByRole("button", { name: "Resume", exact: false }).click();
      await expect.poll(() => page.evaluate(() => window.demoFramesForTest.size)).toBe(1);
      await canvas.getByRole("button", { name: "Pause", exact: false }).click();
      await expect.poll(() => page.evaluate(() => window.demoFramesForTest.size)).toBe(0);
      await page.emulateMedia({ reducedMotion: "reduce" });
      if (browserName === "chromium") await canvas.locator(".demo-threejs").screenshot({ path: "target/screenshots/showcase-threejs-ember.png" });
      await page.getByRole("button", { name: "Reset demo" }).click();
      await expect(canvas.locator(".library-status")).toHaveText("Three.js ready · reduced motion");
      await expect(canvas.locator(".garden-bloom output")).toHaveText("72%");
      await expect.poll(() => page.evaluate(() => window.demoContextsForTest.filter(context => !context.isContextLost()).length)).toBe(1);
      await page.setViewportSize({ width: 390, height: 844 });
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      if (browserName === "chromium") await canvas.locator(".demo-threejs").screenshot({ path: "target/screenshots/showcase-threejs-mobile.png" });
      await page.setViewportSize({ width: 1440, height: 1050 });
    } else if (demo.slug === "comparison") {
      const independent = canvas.locator(".independent-panel");
      const coherent = canvas.locator(".coherent-panel");
      await expect(independent.getByRole("status")).toHaveText("Up to date");
      await expect(coherent.getByRole("status")).toHaveText("Up to date");
      await expect(
        coherent.locator(".coherent-result small").first(),
      ).toHaveText("Notebook");

      // Hold both stock responses so a busy browser cannot miss the intermediate view.
      let releaseStock;
      const stockGate = new Promise((resolve) => {
        releaseStock = resolve;
      });
      const stockUrl = "**/demo-data/Sketchbook-stock.txt";
      await page.route(stockUrl, async (route) => {
        await stockGate;
        await route.continue();
      });
      try {
        await canvas
          .getByRole("button", { name: "Sketchbook", exact: true })
          .click();
        await expect(
          independent.locator('[data-field="price"] small'),
        ).toHaveText("Sketchbook");
        await expect(
          independent.locator('[data-field="price"] strong'),
        ).toHaveText("$36");
        await expect(
          independent.locator('[data-field="stock"] small'),
        ).toHaveText("Still showing Notebook");
        await expect(
          independent.locator('[data-field="stock"] strong'),
        ).toHaveText("8");
        await expect(independent.getByRole("status")).toHaveText(
          "Price ready. Stock loading…",
        );
        await expect(coherent.getByRole("status")).toHaveText(
          "Waiting for both…",
        );
        await expect(
          coherent.locator(".coherent-result small").first(),
        ).toHaveText("Notebook");
        await expect(coherent.locator(".comparison-field strong")).toHaveText([
          "$24",
          "8",
        ]);
        if (browserName === "chromium")
          await screenshot(
            page,
            "target/screenshots/showcase-comparison-pending.png",
          );
      } finally {
        releaseStock();
      }
      await expect(independent.getByRole("status")).toHaveText("Up to date");
      await expect(
        independent.locator('[data-field="stock"] small'),
      ).toHaveText("Sketchbook");
      await expect(
        independent.locator('[data-field="stock"] strong'),
      ).toHaveText("3");
      await expect(coherent.getByRole("status")).toHaveText("Up to date");
      await expect(
        coherent.locator(".coherent-result small").first(),
      ).toHaveText("Sketchbook");
      await expect(coherent.locator(".comparison-field strong")).toHaveText([
        "$36",
        "3",
      ]);
      await page.unroute(stockUrl);

      await canvas
        .getByRole("button", { name: "Notebook", exact: true })
        .click();
      await expect(independent.getByRole("status")).toHaveText("Loading…");
      await expect(coherent.getByRole("status")).toHaveText(
        "Waiting for both…",
      );
      await expect(
        coherent.locator(".coherent-result small").first(),
      ).toHaveText("Sketchbook");
      await canvas
        .getByRole("button", { name: "Sketchbook", exact: true })
        .click();
      await expect(independent.getByRole("status")).toHaveText("Up to date");
      await expect(independent.locator(".independent-result small")).toHaveText(
        ["Sketchbook", "Sketchbook"],
      );
      await expect(coherent.getByRole("status")).toHaveText("Up to date");
      await expect(
        coherent.locator(".coherent-result small").first(),
      ).toHaveText("Sketchbook");
      await page.getByRole("button", { name: "Reset demo" }).click();
      await expect(
        canvas.getByRole("button", { name: "Notebook", exact: true }),
      ).toHaveAttribute("aria-pressed", "true");
      await expect(coherent.getByRole("status")).toHaveText("Up to date");
      await expect(
        coherent.locator(".coherent-result small").first(),
      ).toHaveText("Notebook");
      await page.setViewportSize({ width: 390, height: 844 });
      assert(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      );
      if (browserName === "chromium")
        await screenshot(
          page,
          "target/screenshots/showcase-comparison-mobile.png",
        );
      await page.setViewportSize({ width: 1440, height: 1050 });
    } else if (demo.slug === "reactive") {
      await canvas.getByLabel("Workspace name").fill("<Rust & HTML>");
      await expect(canvas.locator("h3")).toHaveText("<Rust & HTML>");
      await canvas.getByRole("button", { name: "Add a seat" }).click();
      await expect(canvas.locator(".demo-total output")).toContainText("$72");
      await canvas.getByRole("button", { name: "Off", exact: true }).click();
      await expect(canvas.locator(".demo-total output")).toContainText("$60");
      await page.getByRole("button", { name: "Reset demo" }).click();
      await expect(canvas.getByLabel("Workspace name")).toHaveValue(
        "Acme Studio",
      );
      await expect(canvas.locator(".demo-total output")).toContainText("$48");
    } else if (demo.slug === "keyed") {
      const note = canvas.locator('[data-book="1"] input');
      await note.fill("Keep this note with the Rust book.");
      await note.evaluate((node) => {
        node.dataset.retained = "yes";
      });
      await canvas.getByRole("button", { name: "Reverse order" }).click();
      await expect(canvas.locator(".reading-list li").last()).toHaveAttribute(
        "data-book",
        "1",
      );
      await expect(note).toHaveValue("Keep this note with the Rust book.");
      await expect(note).toHaveAttribute("data-retained", "yes");
      await canvas
        .getByRole("button", {
          name: "Remove Programming WebAssembly",
          exact: true,
        })
        .click();
      await expect(canvas.locator(".reading-list li")).toHaveCount(2);
      await canvas.getByRole("button", { name: "Add a book" }).click();
      await expect(canvas.locator('[data-book="4"]')).toBeVisible();
    } else if (demo.slug === "lifecycle") {
      await expect
        .poll(() => page.evaluate(() => window.demoIntervalsForTest.size))
        .toBe(1);
      await expect(canvas.locator(".timer-value output")).not.toHaveText("00");
      await canvas
        .getByRole("button", { name: "Remove panel", exact: true })
        .click();
      const stopped = await canvas.locator(".timer-value output").textContent();
      await expect(canvas.getByRole("log")).toContainText(
        "Disposed → timer stopped",
      );
      assert.equal(
        await page.evaluate(() => window.demoIntervalsForTest.size),
        0,
      );
      await page.waitForTimeout(1200);
      await expect(canvas.locator(".timer-value output")).toHaveText(stopped);
      await canvas
        .getByRole("button", { name: "Mount panel", exact: true })
        .click();
      await expect(canvas.locator(".timer-value output")).toHaveText("00");
      assert.equal(
        await page.evaluate(() => window.demoIntervalsForTest.size),
        1,
      );
    } else if (demo.slug === "loading") {
      await expect(canvas.getByRole("status")).toHaveText("Kyoto is ready");
      await canvas.getByRole("button", { name: "Lisbon", exact: true }).click();
      await expect(canvas.getByRole("status")).toHaveText("Loading Lisbon…");
      await expect(canvas.locator(".field-note h3")).toHaveText("Kyoto");
      await canvas
        .getByRole("button", { name: "Reykjavik", exact: true })
        .click();
      await expect(canvas.getByRole("status")).toHaveText("Reykjavik is ready");
      await expect(canvas.locator(".field-note h3")).toHaveText("Reykjavik");
      await expect(canvas.locator(".field-note")).toContainText("old harbor");
      await canvas.getByRole("button", { name: "Simulate an error" }).click();
      await expect(canvas.getByRole("status")).toContainText(
        "simulated connection error",
      );
      await expect(canvas.locator(".field-note h3")).toHaveText("Reykjavik");
      await canvas.getByRole("button", { name: "Reload / retry" }).click();
      await expect(canvas.getByRole("status")).toHaveText("Reykjavik is ready");
    } else if (demo.slug === "coherent") {
      await expect(canvas.getByRole("status")).toHaveText(
        "Complete view published",
      );
      await expect(canvas.locator(".product-card h3")).toHaveText("Notebook");
      await canvas
        .getByRole("button", { name: "Sketchbook", exact: true })
        .click();
      await expect(canvas.getByRole("status")).toContainText(
        "Preparing Sketchbook",
      );
      await page.waitForTimeout(550);
      await expect(canvas.locator(".product-card h3")).toHaveText("Notebook");
      await expect(canvas.locator(".product-value strong")).toHaveText([
        "$24",
        "8",
      ]);
      await expect(canvas.getByRole("status")).toHaveText(
        "Complete view published",
      );
      await expect(canvas.locator(".product-card h3")).toHaveText("Sketchbook");
      await expect(canvas.locator(".product-value strong")).toHaveText([
        "$36",
        "3",
      ]);
    } else if (demo.slug === "search") {
      const box = canvas.getByRole("searchbox", { name: "Search a million books" });
      await expect(box).toBeVisible({ timeout: 30000 });
      const summary = canvas.locator(".search-meta strong");
      await expect(summary).toHaveText("1,000,000 books");
      await box.pressSequentially("moon", { delay: 20 });
      await expect(summary).not.toHaveText("1,000,000 books");
      await expect(summary).not.toHaveText("Searching…");
      const titles = await canvas.locator(".search-title").allTextContents();
      assert(titles.length > 0 && titles.every((title) => /moon/i.test(title)), "every result matches");
      await expect(canvas.locator(".search-title .hit").first()).toHaveText(/^moon$/i);
      await expect(canvas.locator(".search-facts")).toContainText("bytes");
      const scifi = canvas.locator(".search-genres button", { hasText: "Science fiction" });
      await scifi.click();
      await expect(scifi).toHaveAttribute("aria-pressed", "true");
      await expect(canvas.locator(".search-genre").first()).toHaveText("Science fiction");
      assert((await canvas.locator(".search-genre").allTextContents()).every((genre) => genre === "Science fiction"));
      await scifi.click();
      await box.fill("zzqx");
      await expect(summary).toHaveText("0 books");
      await expect(canvas.locator(".search-none")).toBeVisible();
    } else if (demo.slug === "game") {
      const status = canvas.locator(".game-status");
      const engine = canvas.locator(".cell.engine");
      await expect(status).toHaveText(/Your move/, { timeout: 30000 });
      await canvas.getByRole("radio", { name: "Quick" }).check();
      await canvas.getByRole("button", { name: "Drop a disc in column 4" }).click();
      await expect(canvas.locator(".cell.you")).toHaveCount(1);
      await expect(engine).toHaveCount(1, { timeout: 10000 });
      await expect(status).toHaveText("Your move.");
      await expect(canvas.locator(".game-stats")).toContainText("moves ahead");
      // Move now settles a long search at once with the best move so far.
      await canvas.getByRole("radio", { name: "Deep" }).check();
      await canvas.getByRole("button", { name: "Drop a disc in column 3" }).click();
      await expect(status).toContainText("thinking");
      await expect(canvas.locator(".rating.known").first()).toBeVisible();
      const asked = Date.now();
      await canvas.getByRole("button", { name: "Move now" }).click();
      await expect(engine).toHaveCount(2);
      assert(Date.now() - asked < 2000, "Move now must not wait for the whole think time");
      await canvas.getByRole("button", { name: "Undo" }).click();
      await expect(canvas.locator(".cell.you")).toHaveCount(1);
      await expect(engine).toHaveCount(1);
      await canvas.getByRole("radio", { name: "Quick" }).check();
      await canvas.getByRole("button", { name: "Engine starts" }).click();
      await expect(canvas.locator(".cell.you")).toHaveCount(0);
      await expect(engine).toHaveCount(1, { timeout: 15000 });
      await expect(status).toHaveText(/Your move/);
    } else if (demo.slug === "fractal") {
      const status = canvas.locator(".fractal-status");
      await expect(status).toHaveText(/^Painted 160 tiles/, { timeout: 60000 });
      assert(await page.evaluate(() => crossOriginIsolated), "the docs are served cross-origin isolated");
      await expect(canvas.locator(".fractal-notice")).toBeHidden();
      const counts = await canvas.locator(".painter").allTextContents();
      assert.equal(counts.reduce((sum, text) => sum + Number(text.split("·")[1]), 0), 160, "each tile is painted once");
      await canvas.getByRole("button", { name: /^Race/ }).click();
      await expect(canvas.locator(".fractal-race-result")).toContainText("faster", { timeout: 60000 });
      // Zooming mid-picture stops the tiles of the old view.
      await canvas.getByRole("button", { name: "1", exact: true }).click();
      await expect(status).toContainText("Painting with 1 thread");
      await canvas.locator(".fractal-tile").nth(88).click();
      await expect(canvas.locator(".fractal-depth")).toHaveText("3× zoom");
      await expect(canvas.locator(".fractal-stopped")).toContainText("stopped");
      await expect(status).toHaveText(/^Painted 160 tiles/, { timeout: 60000 });
      // Without isolation there is no shared memory: one ordinary worker paints.
      const plain = await page.context().browser().newContext();
      await plain.route("**/*", async (route) => {
        const response = await route.fetch();
        const headers = { ...response.headers() };
        delete headers["cross-origin-opener-policy"];
        delete headers["cross-origin-embedder-policy"];
        await route.fulfill({ response, headers });
      });
      const other = await plain.newPage();
      await other.goto(origin + "showcase/fractal");
      await expect(other.locator(".fractal-notice")).toContainText("isn’t cross-origin isolated");
      await expect(other.locator(".fractal-status")).toHaveText(/^Painted 160 tiles in \d+ ms with 1 thread$/, { timeout: 60000 });
      await expect(other.getByRole("button", { name: /^Race/ })).toBeDisabled();
      await plain.close();
    } else if (demo.slug === "context") {
      await canvas.getByRole("button", { name: "Violet", exact: true }).click();
      await expect(canvas.locator(".context-badge h3")).toHaveText("Violet");
      await expect(canvas.locator(".context-badge")).toHaveClass(/violet/);
      await canvas.getByRole("button", { name: "Forest", exact: true }).click();
      await expect(canvas.locator(".context-badge h3")).toHaveText("Forest");
    }

    if (browserName === "chromium" && demo.slug === "reactive") {
      await screenshot(page, "target/screenshots/showcase-demo.png");
      await page.getByRole("button", { name: "Toggle color theme" }).click();
      await screenshot(page, "target/screenshots/showcase-demo-dark.png");
      await page.getByRole("button", { name: "Toggle color theme" }).click();
    }
    const guide = await page.locator(".demo-guide a").getAttribute("href");
    assert.equal(guide, demo.guide);
    await page.getByRole("link", { name: "All examples" }).click();
    await expect(page.locator(".showcase-card")).toHaveCount(demos.length);
    assert.equal(
      await page.evaluate(() => window.demoIntervalsForTest.size),
      0,
      "leaving a demo must dispose its timers",
    );
    if (demo.javascript) {
      await expect.poll(() => page.evaluate(() => window.demoFramesForTest.size)).toBe(0);
      await expect.poll(() => page.evaluate(() => window.demoObserversForTest.size)).toBe(0);
      await expect.poll(() => page.evaluate(() => window.demoContextsForTest.every(context => context.isContextLost()))).toBe(true);
    }
  }

  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator(".showcase-card")).toHaveCount(demos.length);
  assert(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  );
  if (browserName === "chromium")
    await screenshot(page, "target/screenshots/showcase-mobile.png");
  await page.locator('.showcase-card[data-demo="reactive"]').click();
  await page.getByRole("button", { name: "Add a seat" }).click();
  await expect(page.locator(".demo-total output")).toContainText("$72");
  await page
    .locator(".source-switch")
    .getByRole("button", { name: "HTML", exact: true })
    .click();
  assert(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  );
  if (browserName === "chromium")
    await screenshot(page, "target/screenshots/showcase-demo-mobile.png");
  await page.goBack();
  await expect(page.locator(".showcase-card")).toHaveCount(demos.length);
  await page.getByRole("button", { name: "Toggle navigation" }).click();
  await page
    .locator(".sidebar")
    .getByRole("link", { name: "Introduction", exact: true })
    .click();
  await expect(page.locator("h1")).toHaveText("Build for the web. Write Rust.");
  await expect(page.locator(".sidebar")).not.toBeVisible();
  await page.goto(origin + "showcase/loading");
  await expect(page.locator("h1")).toHaveText("An async field guide");
  await expect(page.locator(".request-status")).toHaveText("Kyoto is ready");
  page.off("request", recordLibrary);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto(origin + "showcase/missing");
  await expect(page.locator("h1")).toHaveText("Page not found");
}

async function screenshot(page, path) {
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: "instant" }));
  await page.screenshot({ path, fullPage: true, animations: "disabled" });
}
