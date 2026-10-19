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
  viewport: { width: 1440, height: 900 },
  reducedMotion: "reduce",
});
const page = await context.newPage();
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
const index = page.getByRole("navigation", { name: "Projects" });
// An index entry's accessible name starts with the project's name.
const entry = (name) => index.getByRole("button", { name: new RegExp(`^${name} `) });
const world = page.locator(".world");
const frame = page.locator(".stage-frame");
const front = page.locator(".stage-front");
const back = page.locator(".stage-back");
const caption = page.locator(".bench-caption");
const turn = page.getByRole("button", { name: "How it’s made", exact: true });
// Every page leads with Hughie's waitlist, in his own night.
const expectHughieStrip = async () => {
  const strip = page.getByRole("link", { name: /^Hughie\. The AI that never leaves your house\./ });
  await expect(strip).toHaveAttribute("href", "https://hughie.si/");
  await expect(strip).toHaveAttribute("data-project", "hughie");
};
// Name, data-project, world, frame shape, and the files on the back of a tool.
const projects = [
  ["fusor", "fusor", "day", "browser", ["web/components/greeting.html", "src/greeting.rs"]],
  ["HyperCMD", "hypercmd", "night", "terminal", ["shown/hypercmd/browser.html"]],
  ["hyperpane", "hyperpane", "day", "phone", ["shown/hyperpane/run.sh"]],
  ["Hughie", "hughie", "night", "face", []],
  ["SQL Bomb", "sql-bomb", "night", "terminal", []],
];
// Hobby projects live on their own page, not in the index.
const hobbies = ["spotify-tui", "supabase-tui"];
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
    rgb(getComputedStyle(element.closest(".stage-back")).backgroundColor),
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
  assert.equal(await page.title(), "Fusor — An open source lab working in Rust");
  // Link previews show the lab's card, which the site publishes at the URL they name.
  const card = new URL(await page.locator('meta[property="og:image"]').getAttribute("content"));
  assert.equal(card.origin, "https://fusor.build");
  const published = await page.request.get(base + card.pathname);
  assert.equal(published.headers()["content-type"], "image/png");
  assert.equal(await page.locator('meta[name="twitter:card"]').getAttribute("content"), "summary_large_image");
  await expectHughieStrip();
  // The index lists the tools, then the things made with them; fusor is on stage first.
  await expect(index.getByRole("heading")).toHaveText(["Tools", "Things"]);
  await expect(index.locator(".index-name")).toHaveText(projects.map(([name]) => name));
  await expect(entry("fusor")).toHaveAttribute("aria-pressed", "true");
  await expect(index.locator('[aria-pressed="true"]')).toHaveCount(1);
  // Every enabled button shows the pointer, from the index to the install bar.
  assert.deepEqual(
    await page
      .locator("button:not(:disabled)")
      .evaluateAll((buttons) => [...new Set(buttons.map((button) => getComputedStyle(button).cursor))]),
    ["pointer"],
  );
  await expect(index.locator("time")).toHaveCount(0);

  // The browser on stage greets whoever types, from a Rust String shown as Rust prints it.
  const name = front.getByLabel("What should Rust call you?");
  await name.fill("  Ana ");
  await expect(front.getByText("Hello, Ana.", { exact: true })).toBeVisible();
  await expect(front.getByText('self.name = "  Ana "', { exact: true })).toBeVisible();
  await name.fill("   ");
  await expect(front.getByText("Hello.", { exact: true })).toBeVisible();
  await page.screenshot({ path: `${artifacts}desktop.png` });

  // Each project takes the stage: the page takes on its world, the frame its shape.
  // A tool turns over to the exact files that make its front; a thing does not turn.
  for (const [project, key, colors, shape, paths] of projects) {
    await entry(project).click();
    await expect(entry(project)).toHaveAttribute("aria-pressed", "true");
    await expect(index.locator('[aria-pressed="true"]')).toHaveCount(1);
    await expect(world).toHaveAttribute("data-project", key);
    await expect(world).toHaveAttribute("data-world", colors);
    await expect(frame).toHaveAttribute("data-shape", shape);
    await expect(caption.getByText(project, { exact: true })).toBeVisible();
    await page.screenshot({ path: `${artifacts}desktop-${key}.png` });
    if (paths.length === 0) {
      await expect(turn).toBeHidden();
      continue;
    }
    await turn.click();
    await expect(frame).toHaveAttribute("data-flipped", "true");
    await expect(front).toHaveAttribute("inert", "");
    const shown = back.locator("pre code");
    await expect(shown).toHaveCount(paths.length);
    for (const [position, path] of paths.entries()) {
      const source = await readFile(new URL(`../${path}`, import.meta.url), "utf8");
      await expect.poll(() => shown.nth(position).textContent()).toBe(source);
      const contrast = await shown.nth(position).evaluate(syntaxContrast);
      assert.ok(contrast.minimum >= 4.5, `${path}: readable syntax contrast`);
      const response = await context.request.get(`${base}/source/${path}.txt`);
      assert.equal(response.status(), 200);
      assert.equal(await response.text(), source, `${path}: published copy`);
    }
    await turn.click();
    await expect(frame).toHaveAttribute("data-flipped", "false");
    await expect(back).toHaveAttribute("inert", "");
  }
  // A greeting that left the stage comes back fresh.
  await entry("fusor").click();
  await expect(name).toHaveValue("");
  // Choosing a thing while a tool is turned over lands it face up.
  await entry("HyperCMD").click();
  await turn.click();
  await expect(frame).toHaveAttribute("data-flipped", "true");
  await entry("Hughie").click();
  await expect(frame).toHaveAttribute("data-flipped", "false");
  await expect(front).not.toHaveAttribute("inert", "");

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
    await readFile(new URL("../../docs/content/navigation.json", import.meta.url), "utf8"),
  );
  const slugs = new Set(docs.map((page) => `/docs/${page.slug}`));
  slugs.add("/docs/showcase");
  // The header's Blog link reaches the blog mounted in the same site.
  await expect(page.getByRole("link", { name: "Blog", exact: true })).toHaveAttribute("href", "/blog/");
  const blog = await context.request.get(`${base}/blog/`);
  assert.equal(blog.status(), 200);
  assert.match(await blog.text(), /From the lab\./);
  for (const [project] of projects) {
    await entry(project).click();
    for (const href of await page
      .locator('a[href^="/docs/"]')
      .evaluateAll((links) => links.map((link) => link.getAttribute("href")))) {
      assert.ok(slugs.has(href), `Documentation link exists: ${href}`);
    }
    for (const width of [320, 390, 768, 1024, 1440]) {
      await page.setViewportSize({ width, height: 844 });
      assert.equal(
        await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
        true,
        `${project}: no overflow at ${width}px`,
      );
    }
  }

  // The hobby projects have their own page, reached from beside the index.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.getByRole("link", { name: "Made with fusor, for fun", exact: true }).click();
  await expect(page).toHaveURL(`${base}/made-with-fusor`);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Made with fusor.");
  await expect(index).toHaveCount(0);
  await expectHughieStrip();
  const hobbyNames = page.locator("main li h2 span:first-child");
  await expect(hobbyNames).toHaveText(hobbies);
  // Each tile names the lab's tool it is built with.
  for (const hobby of hobbies) {
    const tile = page.locator("main li", { has: page.getByText(hobby, { exact: true }) });
    await expect(tile.locator(".tool-badge")).toHaveText("Built with HyperCMD");
    await expect(tile.locator(".tool-badge img")).toHaveAttribute("src", "./lab/hypercmd-mark.svg");
    await expect(tile.getByText("Hobby project", { exact: true })).toBeVisible();
    await expect(tile.getByRole("link", { name: "Source ↗" })).toHaveAttribute(
      "href",
      `https://github.com/fusor-rs/${hobby}`,
    );
  }
  await page.screenshot({ path: `${artifacts}made-with-fusor.png`, fullPage: true });
  for (const width of [320, 390, 768, 1440]) {
    await page.setViewportSize({ width, height: 844 });
    assert.equal(
      await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth),
      true,
      `Made with fusor: no overflow at ${width}px`,
    );
  }
  // It also loads directly at its own address.
  await page.goto(`${base}/made-with-fusor`);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Made with fusor.");
  await page.getByRole("link", { name: "The lab", exact: true }).click();
  await expect(index).toBeVisible();

  // On phones the object comes first and the index is a dial beneath it.
  await page.setViewportSize({ width: 390, height: 844 });
  await entry("fusor").click();
  const stageBounds = await page.locator(".bench-stage").boundingBox();
  const indexBounds = await index.boundingBox();
  assert.ok(stageBounds.y < indexBounds.y, "Keep the object above the index on phones");
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({ path: `${artifacts}mobile.png`, fullPage: true });
  await entry("hyperpane").focus();
  await page.keyboard.press("Enter");
  await expect(world).toHaveAttribute("data-project", "hyperpane");
  assert.deepEqual(errors, [], "No uncaught browser errors");
  console.log(
    `${engine.name()}: the Hughie strip, index, worlds and stage shapes for ${projects.length} projects, the Made with fusor page, the greeting, exact sources behind each tool with readable contrast, install commands, documentation links, responsive layouts and keyboard selection passed.`,
  );
} finally {
  await browser.close();
  await stopProcess(server);
}
