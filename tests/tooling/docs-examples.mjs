// Compile the documented files in independent applications, then exercise what
// the guides tell a newcomer to observe. No framework internals are inspected.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createServer } from "node:http";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { resolve, join, extname, sep } from "node:path";
import { chromium, firefox, webkit, expect } from "@playwright/test";
import { env as buildEnv, root } from "../../scripts/build.mjs";

const exec = promisify(execFile);
const env = {
  ...buildEnv, RUSTUP_TOOLCHAIN: process.env.RUSTUP_TOOLCHAIN || "stable",
  CARGO_TARGET_DIR: resolve(root, "target/docs-examples"),
};
const tutorial = resolve(root, "apps/docs/tutorial");
const guides = JSON.parse(
  await readFile(resolve(root, "apps/docs/content/pages.json"), "utf8"),
);
// Copy the actual published blocks, including inline module declarations.
async function copyGuideFiles(slug, app, files) {
  const guide = guides.find((page) => page.slug === slug);
  for (const [id, file] of Object.entries(files)) {
    const section = guide.sections.find((section) => section.id === id);
    const code = section.source
      ? await readFile(resolve(root, "apps/docs", section.source), "utf8")
      : section.code;
    await mkdir(resolve(app, file, ".."), { recursive: true });
    await writeFile(resolve(app, file), code);
  }
}
await mkdir(resolve(root, "target"), { recursive: true });
const scratch = await mkdtemp(resolve(root, "target/docs-lessons-"));
const cli = resolve(
  root,
  "target/debug",
  `fusor${process.platform === "win32" ? ".exe" : ""}`,
);
let activeDist;
let browser;
const server = createServer(async (request, response) => {
  const pathname = decodeURIComponent(
    new URL(request.url, "http://localhost").pathname,
  );
  const file = resolve(
    activeDist,
    extname(pathname) ? pathname.slice(1) : "index.html",
  );
  if (!file.startsWith(activeDist + sep)) {
    response.writeHead(404).end();
    return;
  }
  try {
    const bytes = await readFile(file);
    response.setHeader(
      "content-type",
      {
        ".html": "text/html",
        ".js": "text/javascript",
        ".wasm": "application/wasm",
        ".css": "text/css",
        ".txt": "text/plain",
      }[extname(file)] || "application/octet-stream",
    );
    response.end(bytes);
  } catch {
    response.writeHead(404).end("Not found");
  }
});
const run = (program, args, options = {}) =>
  exec(program, args, {
    cwd: root,
    env,
    timeout: 300_000,
    maxBuffer: 8 * 1024 * 1024,
    ...options,
  });
const frameworkCli = (args) => run(cli, args);
try {
  await run("cargo", ["build", "-p", "fusor-cli", "--locked"], {
    env: buildEnv,
  });
  for (const lesson of ["components", "content", "reader", "manual-inputs"]) {
    const app = join(scratch, lesson);
    await frameworkCli(["new", app, "--framework-path", root, "--skip-install"]);
    await copyGuideFiles("components", app, {
      "typed-inputs": "src/counter.rs",
      "counter-template": "web/components/counter.html",
      "parent-state": "src/app.rs",
      "component-tags": "web/index.html",
      modules: "src/lib.rs",
    });
    if (lesson === "content") {
      await copyGuideFiles("nested-content", app, {
        "panel-state": "src/panel.rs",
        "panel-template": "web/components/panel.html",
        modules: "src/lib.rs",
        parent: "src/app.rs",
        "explicit-content": "web/index.html",
      });
    }
    if (lesson === "manual-inputs") {
      await copyGuideFiles("components", app, {
        "manual-inputs": "src/counter.rs",
      });
    }
    if (lesson === "reader") {
      await copyGuideFiles("async-data", app, {
        resource: "src/reader.rs",
        display: "web/reader.html",
      });
      const guide = guides.find((page) => page.slug === "async-data");
      const manifest = join(app, "Cargo.toml");
      const dependency = guide.sections.find((section) => section.id === "dependency").code
        // Test the guide against this checkout while its public snippet uses crates.io.
        .replace("{", `{ path = ${JSON.stringify(join(root, "crates/fusor-async"))},`);
      await writeFile(manifest, (await readFile(manifest, "utf8"))
        .replace("[dependencies]\n", `[dependencies]\n${dependency}\n`)
        + "\n" + guide.sections.find((section) => section.id === "adapt").code + "\n");
      const lib = join(app, "src/lib.rs");
      await writeFile(lib, (await readFile(lib, "utf8"))
        + '\nmod reader;\ninclude!(env!("FUSOR_MODULE"));\n');
      const state = join(app, "src/app.rs");
      await writeFile(state, "use crate::reader::Reader;\n" + (await readFile(state, "utf8"))
        .replace("struct App {", "struct App {\n    selected_id: Signal<u32>,\n    show_reader: Signal<bool>,")
        .replace("Self { count: signal(0) }", "Self { count: signal(0), selected_id: signal(1), show_reader: signal(true) }"));
      const html = join(app, "web/index.html");
      await writeFile(html, (await readFile(html, "utf8"))
        .replace("</main>", guide.sections.find((section) => section.id === "mount").code + "\n</main>"));
      await mkdir(join(app, "public/data"), { recursive: true });
      for (const id of [1, 2])
        await cp(join(tutorial, `public/data/${id}.txt`), join(app, `public/data/${id}.txt`));
    }
    await run("cargo", ["generate-lockfile", "--offline", "--manifest-path", join(app, "Cargo.toml")]);
    await frameworkCli(["build", "--manifest-path", join(app, "Cargo.toml")]);
    console.log(`PASS build: copied ${lesson} guide in a standalone application`);
  }
  for (const lesson of ["basics", "forms", "routing", "context", "mounting", "foreach", "app", "control-flow"]) {
    const app = join(scratch, lesson);
    await frameworkCli(["new", app, "--framework-path", root, "--skip-install"]);
    const files =
      lesson !== "routing"
        ? { "app.rs": "src/app.rs", "index.html": "web/index.html" }
        : {
            "app.rs": "src/app.rs",
            "pages.rs": "src/pages.rs",
            "lib.rs": "src/lib.rs",
            "index.html": "web/index.html",
            "pages.html": "web/components/pages.html",
          };
    for (const [from, to] of Object.entries(files))
      await cp(join(tutorial, "lessons", lesson, from), join(app, to));
    if (lesson === "control-flow") {
      await cp(join(tutorial, "lessons/control-flow/dashboard.html"), join(app, "web/components/dashboard.html"));
      const htmlFile = join(app, "web/index.html");
      const original = await readFile(htmlFile, "utf8");
      for (const [invalid, diagnostic] of [
        [original.replace(/<Case pattern="Session::Guest">[\s\S]*?<\/Case>/, ""), /non-exhaustive patterns/],
        [original.replace("Session::Authenticated { user }", "Session::Authenticated { missing }"), /does not have a field named/],
        [original.replace("state.show_help.get()", "42_u32"), /mismatched types/],
      ]) {
        await writeFile(htmlFile, invalid);
        await assert.rejects(
          run("cargo", ["check", "--offline", "--target", "wasm32-unknown-unknown", "--manifest-path", join(app, "Cargo.toml")]),
          error => diagnostic.test(error.stderr),
        );
      }
      await writeFile(htmlFile, original);
    }
    if (lesson === "basics") {
      const reset = guides.find((page) => page.slug === "reactivity")
        .sections.find((section) => section.id === "batch").code;
      const html = join(app, "web/index.html");
      await writeFile(html, (await readFile(html, "utf8")).replace("</main>", reset + "\n</main>"));
    }
    if (lesson === "forms") {
      // A bound checkbox whose own change removes it.
      const html = join(app, "web/index.html");
      await writeFile(html, (await readFile(html, "utf8")).replace("</main>",
        '<If condition="{{ !state.newsletter.get() }}"><label><input type="checkbox" bind="state.newsletter"> Hide after subscribing</label></If>\n</main>'));
    }
    if (lesson === "routing") {
      const manifest = join(app, "Cargo.toml");
      let text = await readFile(manifest, "utf8");
      text = text.replace(
        "[dependencies]\n",
        `[dependencies]\nfusor-router = { path = ${JSON.stringify(join(root, "crates/fusor-router"))}, features = ["browser"] }\n`,
      );
      text = text.replace(
        'base-path = "/"',
        'base-path = "/"\nhistory-fallback = ["/articles", "/missing"]',
      );
      await writeFile(manifest, text);
    }
    await run("cargo", ["generate-lockfile", "--offline", "--manifest-path", join(app, "Cargo.toml")]);
    await frameworkCli(["build", "--manifest-path", join(app, "Cargo.toml")]);
    console.log(
      `PASS build: documented ${lesson} files in a generated standalone application`,
    );
  }
  await frameworkCli([
    "build",
    "--manifest-path",
    join(tutorial, "Cargo.toml"),
    "--locked",
  ]);
  console.log(
    "PASS build: component, ownership, resource and coherent-view companion",
  );
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(
    ",",
  )) {
    browser = await { chromium, firefox, webkit }[name].launch(
      name === "chromium" && process.env.PLAYWRIGHT_CHANNEL
        ? { channel: process.env.PLAYWRIGHT_CHANNEL }
        : {},
    );
    const context = await browser.newContext();
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));

    for (const lesson of ["components", "content", "manual-inputs"]) {
      activeDist = join(scratch, lesson, "dist");
      await page.goto(origin);
      const counters = page.locator("section.counter");
      await expect(counters).toHaveCount(2);
      if (lesson === "content") {
        await expect(page.getByRole("heading", { name: "Shared counters" })).toBeVisible();
        await expect(page.locator(".panel .panel-content > section.counter")).toHaveCount(2);
      } else {
        await expect(page.locator("main > section.counter")).toHaveCount(2);
      }
      await expect(counters.locator("p")).toHaveText([
        "Shared: 0 · Clicks here: 0", "Shared: 0 · Clicks here: 0",
      ]);
      await counters.nth(0).getByRole("button", { name: "Increment" }).click();
      await expect(counters.locator("p")).toHaveText([
        "Shared: 1 · Clicks here: 1", "Shared: 1 · Clicks here: 0",
      ]);
      await expect(page.locator("output")).toHaveText("1");
      await counters.nth(1).getByRole("button", { name: "Increment" }).click();
      await expect(counters.locator("p")).toHaveText([
        "Shared: 2 · Clicks here: 1", "Shared: 2 · Clicks here: 1",
      ]);
      await expect(page.locator("output")).toHaveText("2");
      console.log(`PASS ${name}: ${lesson} guide renders and shares state as documented`);
    }

    activeDist = join(scratch, "mounting/dist");
    await page.goto(origin);
    await expect(page.locator("#counter-host > section p")).toHaveText("Count: 0");
    await page.getByRole("button", { name: "Add one", exact: true }).click();
    await expect(page.locator("#counter-host > section p")).toHaveText("Count: 1");
    await page.evaluate(async () => {
      const boot = document.querySelector('script[type="module"][src]');
      if (!boot) throw new Error("Missing generated Wasm boot script");
      const wasm = await import(new URL("./pkg/app.js", boot.src).href);
      wasm.unmount_counter();
    });
    await expect(page.locator("#counter-host > section")).toHaveCount(0);
    await expect(page.locator("#counter-host")).toHaveCount(1);
    console.log(`PASS ${name}: Rust mounting inserts, retains and removes the owned child`);

    activeDist = join(scratch, "reader/dist");
    await page.goto(origin);
    await expect(page.locator(".reader [role=status]")).toHaveText("Loaded issue 1");
    await page.getByRole("button", { name: "Select issue 2", exact: true }).click();
    await expect(page.locator(".reader [role=status]")).toHaveText("Loaded issue 2");
    await page.getByRole("button", { name: "Toggle reader" }).click();
    await expect(page.locator(".reader")).toHaveCount(0);
    console.log(`PASS ${name}: documented reader adaptation loads and unmounts in a generated app`);

    activeDist = join(scratch, "basics/dist");
    await page.goto(origin);
    await expect(
      page.getByRole("heading", { name: "Hello, Ada" }),
    ).toBeVisible();
    await page.getByLabel("Your name").fill("Lin");
    await expect(
      page.getByRole("heading", { name: "Hello, Lin" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Decrease" })).toBeDisabled();
    await page.getByRole("button", { name: "Increase", exact: true }).click();
    await expect(page.locator("output")).toHaveText("1");
    await page.getByRole("button", { name: "Use Grace" }).click();
    await expect(page.getByLabel("Your name")).toHaveValue("Grace");
    await page.getByRole("button", { name: "Reset", exact: true }).click();
    await expect(page.getByLabel("Your name")).toHaveValue("Ada");
    await expect(page.locator("output")).toHaveText("0");

    activeDist = join(scratch, "forms/dist");
    await page.goto(origin);
    const shown = (id) => page.locator(`output[for~="${id}"]`);
    await page.getByLabel("Hide after subscribing").click();
    await expect(page.getByLabel("Hide after subscribing")).toHaveCount(0);
    await expect(page.getByLabel("Newsletter")).toBeChecked();
    await expect(shown("newsletter")).toHaveText("true");
    await page.getByLabel("Name").fill("Lin");
    await expect(shown("name")).toHaveText("Lin");
    // Text that does not parse keeps the last number and is never rewritten.
    const seats = page.getByLabel("Seats");
    await seats.fill("");
    await expect(seats).toHaveValue("");
    await expect(shown("seats")).toHaveText("2");
    await seats.pressSequentially("12");
    await expect(shown("seats")).toHaveText("12");
    await page.getByRole("button", { name: "Book four seats" }).click();
    await expect(seats).toHaveValue("4");
    await page.getByLabel("Volume").fill("0.3");
    await expect(shown("volume")).toHaveText("0.3");
    await page.getByLabel("Olives").check();
    await page.getByLabel("Cheese").check();
    await expect(shown("cheese")).toHaveText("olives, cheese");
    await page.getByLabel("Olives").uncheck();
    await expect(shown("cheese")).toHaveText("cheese");
    await page.getByLabel("Large").check();
    await expect(shown("large")).toHaveText("large");
    await page.getByLabel("Delivery").selectOption("express");
    await expect(shown("delivery")).toHaveText("express");
    await expect(page.getByLabel("Days")).toHaveValues(["Tue"]);
    await page.getByLabel("Days").selectOption(["Mon", "Wed"]);
    await expect(shown("days")).toHaveText("Mon, Wed");
    await page.getByLabel("Notes").fill("Window seat");
    await expect(shown("notes")).toHaveText("11 characters");
    assert.deepEqual(errors, []);
    console.log(`PASS ${name}: every documented form control binds both ways`);

    activeDist = join(scratch, "routing/dist");
    await page.goto(origin);
    await expect(page.getByRole("heading", { name: "Library" })).toBeVisible();
    await page.evaluate(() => (window.navigationWitness = {}));
    await page.getByRole("link", { name: "Article 42" }).click();
    await expect(
      page.getByRole("heading", { name: "Article 42" }),
    ).toBeVisible();
    assert(
      await page.evaluate(() => !!window.navigationWitness),
      "link navigation must retain the document",
    );
    await page.goBack();
    await expect(page.getByRole("heading", { name: "Library" })).toBeVisible();
    await page.goto(origin + "/articles/7");
    await expect(
      page.getByRole("heading", { name: "Article 7" }),
    ).toBeVisible();
    await page.goto(origin + "/missing");
    await expect(
      page.getByRole("heading", { name: "Page not found" }),
    ).toBeVisible();

    activeDist = join(scratch, "context/dist");
    await page.goto(origin);
    await expect(page.locator("[data-theme]")).toHaveText("Theme: dark");
    await page.getByRole("button", { name: "Use light theme" }).click();
    await expect(page.locator("[data-theme]")).toHaveText("Theme: light");
    await expect(page.locator("[data-theme]")).toHaveAttribute(
      "data-theme",
      "light",
    );

    activeDist = join(scratch, "app", "dist");
    await page.goto(origin);
    await expect(page.locator("output")).toHaveText("0");
    await page.getByRole("button", {name:"Add one",exact:true}).click();
    await expect(page.locator("output")).toHaveText("1");
    await expect(page.locator("app")).toHaveCount(0);

    activeDist = join(scratch, "foreach", "dist");
    await page.goto(origin);
    await expect(page.locator("li span")).toHaveText(["1. Read the guide", "2. Build a page"]);
    await page.getByLabel("Note", {exact:true}).first().fill("my draft");
    await page.getByRole("button", {name:"Reverse",exact:true}).click();
    await expect(page.locator("li span")).toHaveText(["1. Build a page", "2. Read the guide"]);
    await expect(page.getByLabel("Note", {exact:true}).nth(1)).toHaveValue("my draft");
    await page.getByRole("button", {name:"Remove",exact:true}).first().click();
    await expect(page.locator("li span")).toHaveText(["1. Read the guide"]);

    activeDist = join(scratch, "control-flow", "dist");
    await page.goto(origin);
    await expect(page.getByText("Please sign in.", {exact:true})).toBeVisible();
    await page.getByRole("button", {name:"Toggle help",exact:true}).click();
    await expect(page.getByText("Sign in, then try the dashboard counter.", {exact:true})).toBeVisible();
    await page.getByRole("button", {name:"Sign in",exact:true}).click();
    await expect(page.locator("h1")).toHaveText("Welcome, Ada");
    await page.getByRole("button", {name:"Clicks: 0",exact:true}).click();
    await page.getByRole("button", {name:"Change name",exact:true}).click();
    await expect(page.locator("h1")).toHaveText("Welcome, Grace");
    await expect(page.locator("h2")).toHaveText("Grace’s dashboard");
    await expect(page.getByRole("button", {name:"Clicks: 1",exact:true})).toBeVisible();
    await page.getByRole("button", {name:"Sign out",exact:true}).click();
    await page.getByRole("button", {name:"Sign in",exact:true}).click();
    await expect(page.getByRole("button", {name:"Clicks: 0",exact:true})).toBeVisible();

    activeDist = join(tutorial, "dist");
    await page.goto(origin);
    await expect(page.locator(".details h2")).toHaveText("Issue 1");
    await page.getByLabel("Local note").fill("belongs to issue 1");
    await page
      .getByRole("button", { name: "Select issue 2", exact: true })
      .click();
    await expect(page.locator(".details h2")).toHaveText("Issue 2");
    await expect(page.getByLabel("Local note")).toHaveValue("");
    await page.getByRole("button", { name: "Toggle details" }).click();
    await expect(page.locator(".details")).toHaveCount(0);
    await page.getByRole("button", { name: "Toggle details" }).click();
    await expect(page.locator(".details")).toHaveCount(1);
    await page.getByLabel("Row note").nth(0).fill("first row draft");
    await page.evaluate(() => (window.firstRow = document.querySelector("li")));
    await page.getByRole("button", { name: "Reverse rows" }).click();
    assert(
      await page.evaluate(
        () => document.querySelectorAll("li")[1] === window.firstRow,
      ),
    );
    await expect(page.getByLabel("Row note").nth(1)).toHaveValue(
      "first row draft",
    );
    await page.getByRole("button", { name: "Rename issue 1" }).click();
    await expect(page.locator("li strong").nth(1)).toHaveText(
      "Renamed first issue",
    );
    await page.getByRole("button", { name: "Remove issue 1" }).click();
    await expect(page.locator("li")).toHaveCount(1);
    assert(await page.evaluate(() => !window.firstRow.isConnected));
    await expect(page.locator(".lifecycle")).toHaveText("Panel mounted");
    await page.getByRole("button", { name: "Toggle owned panel" }).click();
    await expect(page.locator(".lifecycle")).toHaveText(
      "Panel removed; cleanup ran",
    );
    await page.getByRole("button", { name: "Toggle owned panel" }).click();
    await expect(page.locator(".lifecycle")).toHaveText("Panel mounted");

    await expect(page.locator(".reader [role=status]")).toHaveText(
      "Loaded issue 2",
    );
    await page.getByRole("button", { name: "Try a missing issue" }).click();
    await expect(page.locator(".reader [role=status]")).toContainText(
      "HTTP 404",
    );
    await expect(page.locator(".issue-text")).toContainText("Issue 2:");
    await page.route("**/data/404.txt", (route) =>
      route.fulfill({ status: 200, body: "Recovered response" }),
    );
    await page.getByRole("button", { name: "Reload / retry" }).click();
    await expect(page.locator(".issue-text")).toContainText(
      "Issue 404: Recovered response",
    );

    let release;
    let requested;
    const pending = new Promise((resolve) => {
      release = resolve;
    });
    const started = new Promise((resolve) => {
      requested = resolve;
    });
    await page.route("**/data/2.txt", async (route) => {
      requested();
      await pending;
      await route
        .fulfill({ status: 200, body: "Obsolete response" })
        .catch(() => {});
    });
    await page
      .getByRole("button", { name: "Select issue 2", exact: true })
      .click();
    await started;
    await expect(page.locator(".reader [role=status]")).toHaveText(
      "Loading issue 2…",
    );
    await page.getByRole("button", { name: "Toggle reader" }).click();
    await expect(page.locator(".reader")).toHaveCount(0);
    await page
      .getByRole("button", { name: "Select issue 1", exact: true })
      .click();
    await page.getByRole("button", { name: "Toggle reader" }).click();
    await expect(page.locator(".reader [role=status]")).toHaveText(
      "Loaded issue 1",
    );
    release();
    await page.unroute("**/data/2.txt");
    await expect(page.locator(".issue-text")).toContainText("Issue 1:");

    let finishPrice, finishStock;
    const price = new Promise((resolve) => {
      finishPrice = resolve;
    });
    const stock = new Promise((resolve) => {
      finishStock = resolve;
    });
    await page.route("**/data/price/B.txt", async (route) => {
      await price;
      await route.fulfill({ body: "$24" });
    });
    await page.route("**/data/stock/B.txt", async (route) => {
      await stock;
      await route.fulfill({ body: "3 available" });
    });
    await expect(page.locator(".product .price")).toHaveText("Price: $12");
    await page.getByRole("button", { name: "Choose product B" }).click();
    await expect(page.locator(".boundary-status")).toHaveText("Pending");
    await expect(page.locator(".product h3")).toHaveText("Product A");
    const response = page.waitForResponse("**/data/price/B.txt");
    finishPrice();
    await response;
    await expect(page.locator(".product .price")).toHaveText("Price: $12");
    finishStock();
    await expect(page.locator(".product h3")).toHaveText("Product B");
    await expect(page.locator(".product .price")).toHaveText("Price: $24");
    await expect(page.locator(".product .stock")).toHaveText(
      "Stock: 3 available",
    );
    assert.deepEqual(errors, []);
    console.log(
      `PASS ${name}: documented bindings, route-to-template mapping, typed context, child replacement, keyed rows, cleanup, loading/error/retry, disposal and coherent publication`,
    );
    await context.close();
    await browser.close();
    browser = undefined;
  }
} finally {
  await browser?.close();
  server.closeAllConnections();
  if (server.listening) await new Promise((resolve) => server.close(resolve));
  await rm(scratch, { recursive: true, force: true });
}
