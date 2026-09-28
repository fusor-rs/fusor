import { test, expect } from "@playwright/test";
import { collectBrowserErrors } from "./errors.js";

let browserErrors = [];
test.beforeEach(async ({ page }) => {
  browserErrors = collectBrowserErrors(page);
  await page.goto("/");
  await expect(page.locator("#playground")).toHaveAttribute(
    "data-ready",
    "true",
  );
});

test.afterEach(() => expect(browserErrors).toEqual([]));

test("Rust events update signals, derived values, and boolean attributes", async ({
  page,
}) => {
  await expect(page.locator("#count")).toHaveText("0");
  await expect(page.locator("#reset")).toBeDisabled();
  await page.getByRole("button", { name: "Increase counter" }).click();
  await expect(page.locator("#count")).toHaveText("1");
  await expect(page.locator("#doubled")).toHaveText("2");
  await expect(page.locator("#parity")).toHaveText("Odd");
  await page.locator("#step").fill("5");
  await expect(page.locator("#step-value")).toHaveText("5");
  await page.getByRole("button", { name: "Decrease counter" }).click();
  await expect(page.locator("#count")).toHaveText("-4");
  await expect(page.locator("#doubled")).toHaveText("-8");
  await expect(page.locator("#parity")).toHaveText("Even");
  await page.locator("#reset").click();
  await expect(page.locator("#count")).toHaveText("0");
  await expect(page.locator("#reset")).toBeDisabled();
});

test("two-way input treats markup as text and preserves the cursor", async ({
  page,
}) => {
  const input = page.locator("#greeting-input");
  const text = "<img src=x onerror=alert(1)>";
  await input.fill(text);
  await expect(page.locator("#greeting")).toHaveText(text);
  await expect(page.locator("#greeting img")).toHaveCount(0);
  await input.evaluate((node) => node.setSelectionRange(3, 3));
  await input.press("X");
  await expect(page.locator("#greeting")).toHaveText(
    "<imXg src=x onerror=alert(1)>",
  );
  expect(await input.evaluate((node) => node.selectionStart)).toBe(4);
  await input.fill("");
  await expect(page.locator("#greeting")).toHaveText("Hello, browser.");
});

test("interpolations update individual text nodes and mixed attributes", async ({ page }) => {
  await page.evaluate(() => {
    const note = document.querySelector("#counter-note");
    window.bindingNodes = {
      count: document.querySelector("#count").firstChild,
      strong: note.querySelector("strong"),
      dot: note.querySelector(".live-dot"),
      children: [...note.childNodes],
    };
  });
  await page.locator("#increment").click();
  await page.locator("#increment").click();
  await expect(page.locator("#count")).toHaveAttribute("title", "Count: 2; double: 4");
  await expect(page.locator("#counter-note")).toHaveText(/Count: 2 · Same DOM\./);
  expect(await page.evaluate(() => {
    const note = document.querySelector("#counter-note");
    const old = window.bindingNodes;
    return old.count?.nodeType === Node.TEXT_NODE
      && old.count === document.querySelector("#count").firstChild
      && document.querySelector("#count").childNodes.length === 1
      && old.strong === note.querySelector("strong")
      && old.dot === note.querySelector(".live-dot")
      && old.children.length === note.childNodes.length
      && old.children.every((node, i) => node === note.childNodes[i]);
  })).toBe(true);
  // Runtime HTML contains generated markers, not executable binding strings.
  expect(await page.locator("#playground").evaluate((root) =>
    [...root.querySelectorAll("*")].flatMap((node) => [...node.attributes])
      .filter((attr) => /^(rust|on|bind|class):/.test(attr.name)).length,
  )).toBe(0);
});

test("a two-way checkbox controls conditional Rust reads in HTML", async ({ page }) => {
  await page.locator("#greeting-input").fill("Custom greeting");
  await expect(page.locator("#greeting")).toHaveText("Custom greeting");
  await page.locator("#greeting-default").check();
  await expect(page.locator("#greeting")).toHaveText("Hello, browser.");
  await page.locator("#greeting-input").fill("A later greeting");
  await expect(page.locator("#greeting")).toHaveText("Hello, browser.");
  await page.locator("#greeting-default").uncheck();
  await expect(page.locator("#greeting")).toHaveText("A later greeting");
});

test("tasks support creation, completion, filtering, and removal", async ({
  page,
}) => {
  const rows = page.locator("#task-list li");
  await expect(rows).toHaveCount(3);
  await page.locator("#task-input").fill("  Ship Rust to the browser  ");
  await page.locator("#task-input").press("Enter");
  await expect(rows).toHaveCount(4);
  await expect(page.locator("#task-input")).toHaveValue("");
  await expect(page.locator("#add-task")).toBeDisabled();
  const task = rows.filter({ hasText: "Ship Rust to the browser" });
  await task.getByRole("checkbox").check();
  await expect(task).toHaveClass(/is-done/);
  await expect(page.locator("#task-progress")).toHaveText("3 of 4 complete");
  await page.locator("#filter-active").click();
  await expect(rows).toHaveCount(1);
  await expect(page.locator("#filter-active")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  // Removing a row while its own change handler runs must release it safely.
  await rows.first().getByRole("checkbox").click();
  await expect(rows).toHaveCount(0);
  await expect(page.locator("#empty-tasks")).toBeVisible();
  await page.locator("#filter-done").click();
  await expect(rows).toHaveCount(4);
  await page
    .getByRole("button", {
      name: "Delete Ship Rust to the browser",
      exact: true,
    })
    .click();
  await expect(rows).toHaveCount(3);
  await page.locator("#clear-completed").click();
  await expect(rows).toHaveCount(0);
  await expect(page.locator("#task-progress")).toHaveText("0 of 0 complete");
  await expect(page.locator("#clear-completed")).toBeDisabled();
  await expect(page.locator("#progress-bar")).toHaveAttribute(
    "style",
    "width: 0%",
  );
});

test("keyed reconciliation retains row identity and focus", async ({
  page,
}) => {
  const result = await page.evaluate(() => {
    const original = document.querySelector('[data-task-id="2"]');
    const checkbox = original.querySelector("input");
    checkbox.focus();
    document.querySelector("#reverse-tasks").click();
    return {
      sameNode: original === document.querySelector('[data-task-id="2"]'),
      order: [...document.querySelectorAll("[data-task-id]")].map(
        (row) => row.dataset.taskId,
      ),
      focused: document.activeElement === checkbox,
    };
  });
  expect(result).toEqual({
    sameNode: true,
    order: ["2", "1", "0"],
    focused: true,
  });
});

test("unmount detaches listeners, including detached rows, and remount is clean", async ({
  page,
}) => {
  await page.locator("#increment").click();
  await page.evaluate(async () => {
    window.detachedRow = document.querySelector('[data-task-id="2"]');
    window.detachedInput = document.querySelector("#greeting-input");
    const app = await import(new URL("pkg/app.js", document.querySelector('script[type="module"][src$="/boot.js"]').src).href);
    app.unmount();
  });
  await page.locator("#increment").click();
  await expect(page.locator("#count")).toHaveText("1");
  await expect(page.locator("#greeting-input")).toHaveCount(0);
  await page.evaluate(async () => {
    const app = await import(new URL("pkg/app.js", document.querySelector('script[type="module"][src$="/boot.js"]').src).href);
    app.mount();
    // Old DOM nodes no longer have live callbacks into dropped Rust scopes.
    window.detachedRow.querySelector("[data-remove]").click();
    window.detachedInput.value = "Unmounted";
    window.detachedInput.dispatchEvent(new Event("input"));
    delete window.detachedRow;
    delete window.detachedInput;
  });
  await expect(page.locator("#count")).toHaveText("0");
  await expect(page.locator("#task-list li")).toHaveCount(3);
  await expect(page.locator("#greeting")).toHaveText("Hello, browser.");
  await expect(page.locator(".input-card")).toHaveCount(1);
  await page.locator("#increment").click();
  await expect(page.locator("#count")).toHaveText("1");
});

test("native string arguments preserve live attributes and exact listener removal", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const app = await import(new URL("pkg/app.js", document.querySelector('script[type="module"][src$="/boot.js"]').src).href);
    app.unmount();
    const add = EventTarget.prototype.addEventListener;
    const remove = EventTarget.prototype.removeEventListener;
    const attribute = Element.prototype.getAttribute;
    const active = new Map(), names = new Set();
    let added = 0, removed = 0, reads = 0;
    Element.prototype.getAttribute = function (name) {
      if (typeof name !== "string") throw Error("attribute name must remain a string");
      reads++;
      return attribute.call(this, name);
    };
    EventTarget.prototype.addEventListener = function (name, callback, ...options) {
      if (typeof name !== "string") throw Error("event name must remain a string");
      names.add(name);
      const listeners = active.get(this) || [];
      listeners.push([name, callback]);
      active.set(this, listeners);
      added++;
      return add.call(this, name, callback, ...options);
    };
    EventTarget.prototype.removeEventListener = function (name, callback, ...options) {
      const listeners = active.get(this) || [];
      const index = listeners.findIndex(([type, fn]) => type === name && fn === callback);
      if (index < 0) throw Error("removal changed its target, event name or callback");
      listeners.splice(index, 1);
      removed++;
      return remove.call(this, name, callback, ...options);
    };
    try {
      app.mount();
      document.querySelector("#greeting-input").value = "日本語😀<&\"";
      document.querySelector("#greeting-input").dispatchEvent(new Event("input"));
      if (document.querySelector("#greeting").textContent !== "日本語😀<&\"") throw Error("dynamic Unicode changed");
      app.unmount();
      return { added, removed, reads, names: [...names], remaining: [...active.values()].flat().length };
    } finally {
      EventTarget.prototype.addEventListener = add;
      EventTarget.prototype.removeEventListener = remove;
      Element.prototype.getAttribute = attribute;
    }
  });
  expect(result.added).toBeGreaterThan(0);
  expect(result.removed).toBe(result.added);
  expect(result.remaining).toBe(0);
  expect(result.reads).toBeGreaterThan(0);
  expect(result.names).toEqual(expect.arrayContaining(["click", "input", "change", "submit"]));
});

test("compiled bindings retain native nodes after their lookup metadata is removed", async ({ page }) => {
  await page.evaluate(async () => {
    const app = await import(new URL("pkg/app.js", document.querySelector('script[type="module"][src$="/boot.js"]').src).href);
    app.unmount();
    const query = Element.prototype.querySelector;
    const matches = Element.prototype.matches;
    const rejectBindingSelector = (selector) => {
      if (/data-fusor-(node|text)/.test(selector)) throw new Error("a compiled binding used a CSS selector");
    };
    Element.prototype.querySelector = function (selector) {
      rejectBindingSelector(selector);
      return query.call(this, selector);
    };
    Element.prototype.matches = function (selector) {
      rejectBindingSelector(selector);
      return matches.call(this, selector);
    };
    try {
      app.mount();
    } finally {
      Element.prototype.querySelector = query;
      Element.prototype.matches = matches;
    }
    for (const node of document.querySelectorAll("[data-fusor-node]")) node.removeAttribute("data-fusor-node");
    for (const node of document.querySelectorAll("[data-fusor-text]")) node.removeAttribute("data-fusor-text");
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_COMMENT);
    const anchors = [];
    while (walker.nextNode()) {
      if (/^\/?fusor:/.test(walker.currentNode.data)) anchors.push(walker.currentNode);
    }
    for (const anchor of anchors) anchor.remove();
    window.nativeCount = document.querySelector("#count").firstChild;
  });
  await page.locator("#increment").click();
  await expect(page.locator("#count")).toHaveText("1");
  await expect(page.locator("#count")).toHaveAttribute("title", "Count: 1; double: 2");
  expect(await page.evaluate(() => window.nativeCount === document.querySelector("#count").firstChild)).toBe(true);
  await page.locator("#greeting-input").fill("Native handles");
  await expect(page.locator("#greeting")).toHaveText("Native handles");
  await page.locator("#task-input").fill("A new template instance");
  await page.locator("#task-input").press("Enter");
  await expect(page.locator("#task-list li")).toHaveCount(4);
});

for (const damage of [
  "missing marker", "noncanonical marker", "unknown marker", "duplicate marker",
  "extra text", "element child", "comment child", "old schema",
]) {
  test(`sole-child text rejects ${damage} without installing bindings`, async ({ page }) => {
    await page.locator("#increment").click();
    const result = await page.evaluate(async damage => {
      const app = await import(new URL("pkg/app.js", document.querySelector('script[type="module"][src$="/boot.js"]').src).href);
      app.unmount();
      const root = document.querySelector(".counter-card");
      const backup = root.cloneNode(true);
      const output = root.querySelector("#count"), text = output.firstChild;
      if (text?.nodeType !== Node.TEXT_NODE || output.childNodes.length !== 1)
        throw Error("fixture did not compile to a sole text child");
      const id = output.getAttribute("data-fusor-text");
      if (id === null) throw Error("missing direct text marker");
      switch (damage) {
        case "missing marker": output.removeAttribute("data-fusor-text"); break;
        case "noncanonical marker": output.setAttribute("data-fusor-text", `0${id}`); break;
        case "unknown marker": output.setAttribute("data-fusor-text", "999999"); break;
        case "duplicate marker": root.querySelector("#parity").setAttribute("data-fusor-text", id); break;
        case "extra text": output.append(document.createTextNode("extra")); break;
        case "element child": output.append(document.createElement("b")); break;
        case "comment child": output.append(document.createComment("ordinary comment")); break;
        case "old schema": root.setAttribute("data-fusor-version", "1"); break;
      }
      let error = "mount unexpectedly succeeded";
      try { app.mount(); } catch (problem) { error = String(problem); }
      root.querySelector("#increment").click();
      const unchanged = text.data === "1" && text === output.firstChild;
      root.replaceWith(backup);
      app.mount();
      return { error, unchanged };
    }, damage);
    expect(result.error).toContain("template mismatch");
    expect(result.unchanged).toBe(true);
    await expect(page.locator("#count")).toHaveText("0");
    await page.locator("#increment").click();
    await expect(page.locator("#count")).toHaveText("1");
  });
}

for (const [damage, expected] of [
  ["version", "schema version"],
  ["missing element", "missing element"],
  ["duplicate element", "duplicate element"],
  ["wrong element type", "must be <input>"],
  ["missing text anchor", "missing text start"],
  ["duplicate text anchor", "duplicate text start"],
  ["unexpected text content", "unexpected nodes in text slot"],
  ["invalid identifier", "canonical unsigned template identifier"],
  ["duplicate component", "requires exactly one root"],
]) {
  test(`mount rejects ${damage} before installing component bindings`, async ({ page }) => {
    await page.locator("#increment").click();
    const result = await page.evaluate(async (damage) => {
      const app = await import(new URL("pkg/app.js", document.querySelector('script[type="module"][src$="/boot.js"]').src).href);
      app.unmount();
      const root = document.querySelector(".counter-card");
      const backup = root.cloneNode(true);
      // Mixed content retains anchors even when sole-child text is specialized.
      const note = root.querySelector("#counter-note");
      const start = [...note.childNodes].find(node => node.nodeType === Node.COMMENT_NODE && /^fusor:\d+$/.test(node.data));
      if (!start) throw Error("missing mixed-content test anchor");
      let duplicate;
      switch (damage) {
        case "version": root.setAttribute("data-fusor-version", "999"); break;
        case "missing element": root.querySelector("#step").removeAttribute("data-fusor-node"); break;
        case "duplicate element": root.append(root.querySelector("#increment").cloneNode(true)); break;
        case "wrong element type": {
          const input = root.querySelector("#step");
          const div = document.createElement("div");
          for (const attribute of input.attributes) div.setAttribute(attribute.name, attribute.value);
          input.replaceWith(div);
          break;
        }
        case "missing text anchor": start.remove(); break;
        case "duplicate text anchor": note.append(start.cloneNode()); break;
        case "unexpected text content": start.after(document.createElement("span")); break;
        case "invalid identifier": root.querySelector("#step").setAttribute("data-fusor-node", "01"); break;
        case "duplicate component": duplicate = root.cloneNode(true); root.after(duplicate); break;
      }
      let error = "mount unexpectedly succeeded";
      try { app.mount(); } catch (problem) { error = String(problem); }
      root.querySelector("#increment").click();
      const countAfterFailure = root.querySelector("#count").textContent;
      duplicate?.remove();
      root.replaceWith(backup);
      app.mount();
      return { error, countAfterFailure };
    }, damage);
    expect(result.error).toContain("template mismatch");
    expect(result.error).toContain(expected);
    expect(result.countAfterFailure).toBe("1");
    await expect(page.locator("#count")).toHaveText("0");
    await page.locator("#increment").click();
    await expect(page.locator("#count")).toHaveText("1");
  });
}

test("the document fits its viewport and the Wasm artifact is served correctly", async ({
  page,
  request,
}, testInfo) => {
  const sizes = await page.evaluate(() => ({
    viewport: innerWidth,
    content: document.documentElement.scrollWidth,
  }));
  expect(sizes.content).toBeLessThanOrEqual(sizes.viewport);
  const wasmUrl = await page.evaluate(() => new URL("pkg/app_bg.wasm", document.querySelector('script[type="module"][src$="/boot.js"]').src).href);
  const response = await request.get(wasmUrl);
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toBe("application/wasm");
  expect((await response.body()).subarray(0, 4)).toEqual(
    Buffer.from([0, 97, 115, 109]),
  );
  await page.screenshot({
    path: testInfo.outputPath("playground.png"),
    fullPage: true,
  });
});

test("native text comparison preserves Unicode, skips equal text and repairs external edits", async ({ page }) => {
  await page.locator("#greeting-input").fill("🦀 café e\u0301 <b>plain text</b>");
  await expect(page.locator("#greeting")).toHaveText("🦀 café e\u0301 <b>plain text</b>");
  await expect(page.locator("#greeting b")).toHaveCount(0);
  await page.locator("#step").fill("2");
  const result = await page.evaluate(() => {
    const output = document.querySelector("#parity");
    const node = [...output.childNodes].find(node => node.nodeType === Node.TEXT_NODE && node.data === "Even");
    if (!node) throw Error("missing live parity text");
    const observer = new MutationObserver(() => {});
    observer.observe(output, { characterData: true, subtree: true });
    const increase = document.querySelector('[aria-label="Increase counter"]');
    increase.click(); // New signal value, same displayed parity.
    const equalWrites = observer.takeRecords().length;
    node.data = "external edit";
    observer.takeRecords();
    increase.click();
    const repairWrites = observer.takeRecords().length;
    observer.disconnect();
    return { equalWrites, repairWrites, text: node.data, retained: node.parentNode === output };
  });
  expect(result).toEqual({ equalWrites: 0, repairWrites: 1, text: "Even", retained: true });
});
