import assert from "node:assert/strict";
import { mkdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium, webkit } from "playwright";
import { expect } from "@playwright/test";
import { root, startProcess, stopProcess, waitFor, reservePort } from "../../../scripts/build.mjs";

const engine = process.env.BROWSER === "webkit" ? webkit : chromium;
const artifacts = fileURLToPath(
  new URL(`../test-results/${engine.name()}/`, import.meta.url),
);
await mkdir(artifacts, { recursive: true });
// Serves the assembled site from `fusor build --site`, so /docs/ and
// /benchmarks/ links resolve exactly as they will when deployed.
const port = await reservePort();
const server = startProcess(
  "cargo",
  ["run", "--locked", "-q", "-p", "fusor-cli", "--bin", "fusor", "--", "preview", "dist", "--port", String(port)],
  { cwd: root },
);
await waitFor(() => server.output.includes("Ready"), "site preview", {
  timeout: 120_000,
  process: server,
});
const base = `http://127.0.0.1:${port}`;
const browser = await engine.launch();
const context = await browser.newContext({
  viewport: { width: 1440, height: 1050 },
  reducedMotion: "reduce",
});
const page = await context.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
const picker = page.getByRole("group", { name: "Choose an example" });
const select = (name) =>
  picker.getByRole("button", { name, exact: true }).click();
const output = page.locator(".counter output");
const code = page.locator(".source-content pre code");
const examples = [
  ["Counter", "counter"],
  ["Live search", "search"],
  ["Keyed lists", "keyed_list"],
  ["Async data", "async_data"],
];
const asyncReady = async () => {
  await expect(page.locator(".issue-result")).not.toHaveAttribute(
    "aria-busy",
    "true",
  );
  await expect(page.locator('[data-field="title"]')).not.toHaveText("");
};
// Runs in the page: distinct syntax colors and their worst contrast ratio.
const syntaxContrast = (element) => {
  const rgb = (value) =>
    value
      .match(/[\d.]+/g)
      .slice(0, 3)
      .map(Number);
  const luminance = (color) =>
    color.reduce((sum, channel, index) => {
      const value = channel / 255;
      return (
        sum +
        [0.2126, 0.7152, 0.0722][index] *
          (value <= 0.04045
            ? value / 12.92
            : ((value + 0.055) / 1.055) ** 2.4)
      );
    }, 0);
  const bg = luminance(
    rgb(
      getComputedStyle(element.closest(".source-panel")).backgroundColor,
    ),
  );
  const colors = [
    ...new Set(
      [...element.querySelectorAll(".syntax-token")].map(
        (token) => getComputedStyle(token).color,
      ),
    ),
  ];
  return {
    colors: colors.length,
    minimum: Math.min(
      ...colors.map((color) => {
        const fg = luminance(rgb(color));
        return (Math.max(fg, bg) + 0.05) / (Math.min(fg, bg) + 0.05);
      }),
    ),
  };
};

try {
  await page.goto(base);
  // The smallest example opens first, with its HTML file shown.
  await expect(output).toHaveText("0");
  await expect(
    picker.getByRole("button", { name: "Counter", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await expect(
    page.getByRole("button", { name: "counter.html", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  // Template syntax in the source key renders literally, not as a binding.
  await expect(page.locator(".source-key dt code")).toHaveText([
    "rust:component",
    "{{ … }}",
    "on:click",
    "bind",
    "template!(…)",
  ]);
  assert.equal(
    await page.title(),
    "fusor — A framework for Rust and HTML",
  );
  await page.screenshot({ path: `${artifacts}desktop.png`, fullPage: true });
  await page.screenshot({ path: `${artifacts}desktop-fold.png` });

  // Every displayed file must remain byte-for-byte identical to the real source.
  for (const [label, stem] of examples) {
    await select(label);
    for (const language of ["html", "rs"]) {
      await page
        .getByRole("button", { name: `${stem}.${language}`, exact: true })
        .click();
      const path =
        language === "html" ? `web/components/${stem}.html` : `src/${stem}.rs`;
      const source = await readFile(
        new URL(`../${path}`, import.meta.url),
        "utf8",
      );
      assert.equal(
        await code.textContent(),
        source,
        `${stem}.${language}: complete source`,
      );
      await expect(
        page.getByRole("link", { name: "Open the displayed source file" }),
      ).toHaveAttribute("href", `./source/${path}.txt`);
      const response = await context.request.get(`${base}/source/${path}.txt`);
      assert.equal(response.status(), 200);
      assert.match(response.headers()["content-type"], /^text\/plain/);
      assert.equal(await response.text(), source);
      const contrast = await code.evaluate(syntaxContrast);
      assert.ok(
        contrast.colors >= 3,
        `${stem}.${language}: multiple syntax colors`,
      );
      assert.ok(
        contrast.minimum >= 4.5,
        `${stem}.${language}: readable syntax contrast`,
      );
      assert.equal(
        await page
          .locator(".source-content pre")
          .evaluate((pre) => pre.scrollTop),
        0,
      );
    }
  }

  // The same editor shows the page that hosts the selected example, from real
  // source. It mounts and imports only that example's component.
  const componentFiles = page.getByRole("group", { name: "Component files" });
  const pageFiles = page.getByRole("group", { name: "Page files" });
  const tags = {
    counter: "Counter",
    search: "LiveSearch",
    keyed_list: "KeyedList",
    async_data: "AsyncData",
  };
  const checkPage = async (stem, name) => {
    const path =
      name === "index.html"
        ? `host/${stem}/web/index.html`
        : `host/${stem}/src/app.rs`;
    await expect(
      pageFiles.getByRole("button", { name, exact: true }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(
      page.locator('.source-tabs button[aria-pressed="true"]'),
    ).toHaveCount(1);
    const source = await readFile(new URL(`../${path}`, import.meta.url), "utf8");
    assert.equal(await code.textContent(), source, `${path}: complete page source`);
    await expect(
      page.getByRole("link", { name: "Open the displayed source file" }),
    ).toHaveAttribute("href", `./source/${path}.txt`);
    const response = await context.request.get(`${base}/source/${path}.txt`);
    assert.equal(response.status(), 200);
    assert.match(response.headers()["content-type"], /^text\/plain/);
    assert.equal(await response.text(), source);
    const contrast = await code.evaluate(syntaxContrast);
    assert.ok(contrast.colors >= 3, `${path}: multiple syntax colors`);
    assert.ok(contrast.minimum >= 4.5, `${path}: readable syntax contrast`);
    const tag = tags[stem];
    assert.ok(
      name === "index.html"
        ? source.includes(`<${tag}></${tag}>`)
        : source.includes(`use crate::${stem}::${tag};`),
      `${path}: uses ${tag}`,
    );
    for (const other of Object.values(tags).filter((other) => other !== tag)) {
      assert.ok(!source.includes(other), `${path}: does not reference ${other}`);
    }
  };
  await select("Counter");
  let open = "index.html";
  await pageFiles.getByRole("button", { name: open, exact: true }).click();
  for (const [label, stem] of examples) {
    // Changing examples keeps the open page file, now showing this example's page.
    await select(label);
    await expect(page.locator(".page-tag")).toHaveText(`<${tags[stem]}>`);
    await checkPage(stem, open);
    open = open === "index.html" ? "app.rs" : "index.html";
    await pageFiles.getByRole("button", { name: open, exact: true }).click();
    await checkPage(stem, open);
  }
  // From a component file, choosing another example opens that component's HTML.
  await componentFiles
    .getByRole("button", { name: "async_data.rs", exact: true })
    .click();
  await select("Counter");
  await expect(
    componentFiles.getByRole("button", { name: "counter.html", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");

  await select("Live search");
  const query = page.getByLabel("Find a guide");
  await query.fill("  TYPED  ");
  await expect(page.locator(".search-results li")).toHaveCount(2);
  await expect(page.locator(".result-count")).toHaveText("2 of 4 guides");
  await page.getByRole("button", { name: "search.rs", exact: true }).click();
  await expect(query).toHaveValue("  TYPED  ");
  // Viewing the host page's source leaves the running component intact.
  for (const name of ["index.html", "app.rs"]) {
    await pageFiles.getByRole("button", { name, exact: true }).click();
    await expect(query).toHaveValue("  TYPED  ");
    await expect(page.locator(".search-results li")).toHaveCount(2);
  }
  await componentFiles
    .getByRole("button", { name: "search.html", exact: true })
    .click();
  await page.screenshot({ path: `${artifacts}search-rust.png` });
  await query.fill("<script>alert(1)</script>");
  await expect(page.locator(".search-results li")).toHaveCount(0);
  await expect(
    page.getByText("No matching guides.", { exact: true }),
  ).toBeVisible();
  await query.fill("");
  await expect(page.locator(".search-results li")).toHaveCount(4);

  await select("Keyed lists");
  await page
    .getByLabel("Note for row 1", { exact: true })
    .fill("Keep this draft");
  await page.evaluate(() => {
    window.originalRow = document.querySelector('[data-row="1"] input');
  });
  await page
    .getByRole("button", { name: "Reverse order", exact: true })
    .click();
  assert.deepEqual(
    await page
      .locator(".keyed-rows li")
      .evaluateAll((rows) => rows.map((row) => row.dataset.row)),
    ["3", "2", "1"],
  );
  await expect(page.getByLabel("Note for row 1", { exact: true })).toHaveValue(
    "Keep this draft",
  );
  assert.equal(
    await page.evaluate(
      () =>
        window.originalRow === document.querySelector('[data-row="1"] input'),
    ),
    true,
  );
  await page.getByRole("button", { name: "Add row", exact: true }).click();
  await expect(page.locator(".keyed-rows li")).toHaveCount(4);
  await page.getByRole("button", { name: "Remove row 2", exact: true }).click();
  await expect(page.locator(".keyed-rows li")).toHaveCount(3);
  await expect(page.getByLabel("Note for row 1", { exact: true })).toHaveValue(
    "Keep this draft",
  );
  await page.screenshot({ path: `${artifacts}keyed-list.png` });
  for (const id of [1, 3, 4])
    await page
      .getByRole("button", { name: `Remove row ${id}`, exact: true })
      .click();
  await expect(
    page.getByText("No rows. Add one to start again.", { exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Reverse order", exact: true })
    .isDisabled()
    .then((disabled) => assert.ok(disabled));
  await page.getByRole("button", { name: "Add row", exact: true }).click();
  await expect(
    page.getByLabel("Note for row 5", { exact: true }),
  ).toBeVisible();

  await select("Async data");
  await asyncReady();
  await expect(page.locator(".issue-result")).toHaveAttribute(
    "data-issue",
    "12",
  );
  const titleReady = page.waitForResponse(
    (response) => response.url().endsWith("/34-title.txt") && response.ok(),
  );
  await page.getByRole("button", { name: "Issue #34", exact: true }).click();
  await titleReady;
  await expect(page.locator(".issue-result")).toHaveAttribute(
    "data-issue",
    "12",
  );
  await expect(page.locator('[data-field="title"]')).toHaveText(
    "Add keyboard shortcuts",
  );
  await expect(page.locator('[data-field="status"]')).toHaveText("Open");
  await asyncReady();
  await expect(page.locator(".issue-result")).toHaveAttribute(
    "data-issue",
    "34",
  );
  await expect(page.locator('[data-field="title"]')).toHaveText(
    "Support nested routes",
  );
  await expect(page.locator('[data-field="status"]')).toHaveText("Closed");
  await page.screenshot({ path: `${artifacts}async-data.png` });

  // Delay an obsolete request beyond the latest selection; it must not publish.
  let started, release;
  const oldStarted = new Promise((resolve) => {
    started = resolve;
  });
  const oldResponse = new Promise((resolve) => {
    release = resolve;
  });
  const staleRoute = async (route) => {
    started();
    await oldResponse;
    await route.fulfill({ status: 200, body: "Stale response" });
  };
  await page.route("**/12-status.txt", staleRoute);
  await page.getByRole("button", { name: "Issue #12", exact: true }).click();
  await oldStarted;
  await page.getByRole("button", { name: "Issue #34", exact: true }).click();
  await asyncReady();
  release();
  await page.unrouteAll({ behavior: "wait" });
  await expect(page.locator(".issue-result")).toHaveAttribute(
    "data-issue",
    "34",
  );
  await expect(page.locator('[data-field="status"]')).toHaveText("Closed");

  await page.route("**/12-status.txt", (route) =>
    route.fulfill({ status: 503, body: "Test failure" }),
  );
  const failedRead = page.waitForResponse(
    (response) =>
      response.url().endsWith("/12-status.txt") && response.status() === 503,
  );
  await page.getByRole("button", { name: "Issue #12", exact: true }).click();
  await failedRead;
  await expect(page.locator(".issue-result")).toHaveAttribute(
    "data-issue",
    "34",
  );
  await expect(page.locator('[data-field="status"]')).toHaveText("Closed");
  await page.unrouteAll({ behavior: "wait" });
  await page.getByRole("button", { name: "Issue #34", exact: true }).click();
  await asyncReady();
  await page.getByRole("button", { name: "Issue #12", exact: true }).click();
  await asyncReady();
  await expect(page.locator(".issue-result")).toHaveAttribute(
    "data-issue",
    "12",
  );

  // Switching away during a request disposes that example's work.
  await page.getByRole("button", { name: "Issue #34", exact: true }).click();
  await select("Counter");
  await expect(output).toHaveText("0");
  await page.evaluate(() => {
    window.counterElement = document.querySelector(".counter output");
    window.counterText = window.counterElement.firstChild;
  });
  const increment = page.getByRole("button", {
    name: "Increment",
    exact: true,
  });
  for (let n = 0; n < 5; n++) await increment.click();
  await increment.focus();
  await page.keyboard.press("Enter");
  await expect(output).toHaveText("6");
  assert.equal(
    await page.evaluate(() => {
      const output = document.querySelector(".counter output");
      return (
        output === window.counterElement &&
        output.firstChild === window.counterText
      );
    }),
    true,
  );
  await page.getByRole("button", { name: "counter.rs", exact: true }).click();
  await expect(output).toHaveText("6");
  await page.getByRole("button", { name: "Reset", exact: true }).click();
  await expect(output).toHaveText("0");
  await expect(page.locator(".async-demo")).toHaveCount(0);
  await select("Async data");
  await asyncReady();
  await expect(page.locator(".issue-result")).toHaveAttribute(
    "data-issue",
    "12",
  );

  if (engine === chromium) {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  }
  const platforms = page.getByRole("group", { name: "Installation platform" });
  for (const [platform, command] of [
    ["Windows", "irm https://fusor.build/install.ps1 | iex"],
    ["macOS / Linux", "curl -fsSL https://fusor.build/install.sh | sh"],
  ]) {
    const button = platforms.getByRole("button", { name: platform, exact: true });
    await button.click();
    await expect(button).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator(".install-command code")).toHaveText(command);
    const copy = page.getByRole("button", { name: "Copy command", exact: true });
    await expect(copy).toBeVisible();
    if (engine === chromium) {
      await copy.click();
      await expect(page.getByRole("button", { name: "Copied!", exact: true })).toBeVisible();
      assert.equal(await page.evaluate(() => navigator.clipboard.readText()), command);
    }
  }
  const docs = JSON.parse(
    await readFile(
      new URL("../../docs/content/pages.json", import.meta.url),
      "utf8",
    ),
  );
  const slugs = new Set(docs.map((page) => `/docs/${page.slug}`));
  slugs.add("/docs/showcase");
  for (const [label] of examples) {
    await select(label);
    for (const href of await page
      .locator('a[href^="/docs/"]')
      .evaluateAll((links) => links.map((link) => link.getAttribute("href")))) {
      assert.ok(slugs.has(href), `Documentation link exists: ${href}`);
    }
    for (const width of [320, 390, 701, 768, 1024, 1440]) {
      await page.setViewportSize({ width, height: 844 });
      assert.equal(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
        true,
        `${label}: no overflow at ${width}px`,
      );
      // The panel clips overflow, so check each file tab and link fits inside it.
      const clipped = await page.evaluate(() => {
        const panel = document
          .querySelector(".source-panel")
          .getBoundingClientRect();
        return [...document.querySelectorAll(".source-tabs button, .source-tabs a")]
          .filter((control) => {
            const box = control.getBoundingClientRect();
            return (
              box.left < panel.left - 1 ||
              box.right > panel.right + 1 ||
              box.top < panel.top - 1 ||
              box.bottom > panel.bottom + 1
            );
          })
          .map((control) => control.textContent.trim() || control.ariaLabel);
      });
      assert.deepEqual(clipped, [], `${label}: source tabs fit at ${width}px`);
    }
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await select("Live search");
  const previewBounds = await page.locator(".preview-panel").boundingBox();
  const sourceBounds = await page.locator(".source-panel").boundingBox();
  assert.ok(
    previewBounds.y < sourceBounds.y,
    "Keep the live result above source on phones",
  );
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: `${artifacts}mobile.png`, fullPage: true });
  await page.screenshot({ path: `${artifacts}mobile-fold.png` });
  await page.getByLabel("Find a guide").fill("state");
  await expect(page.locator(".search-results li")).toHaveCount(1);
  await picker
    .getByRole("button", { name: "Keyed lists", exact: true })
    .focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByRole("region", { name: "Keyed list example" }),
  ).toBeVisible();
  assert.deepEqual(errors, [], "No uncaught browser errors");
  console.log(
    `${engine.name()}: all four examples, eight component and eight page source files, syntax contrast, keyed DOM identity, automatic async coherence/supersession/recovery/disposal, keyboard interaction, and responsive layouts passed.`,
  );
} finally {
  await browser.close();
  await stopProcess(server);
}
