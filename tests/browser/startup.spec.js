import { test, expect } from "@playwright/test";
import { collectBrowserErrors } from "./errors.js";

let browserErrors = [];
test.beforeEach(async ({ page }) => {
  browserErrors = collectBrowserErrors(page);
});

test.afterEach(() => expect(browserErrors).toEqual([]));

test("a module load failure is reported in the page", async ({ page }) => {
  let intercepted = 0;
  // Intercept before the first navigation: WebKit can reuse a preloaded module
  // on reload without consulting a route installed after the initial load.
  await page.route("**/pkg/app.js", (route) => {
    intercepted++;
    return route.fulfill({
      status: 200,
      contentType: "text/javascript",
      body: 'throw new Error("expected module load failure");',
    });
  });
  const failure = page.waitForEvent("console", {
    predicate: (message) => message.type() === "error"
      && message.text().startsWith("fusor failed to initialize:"),
  });
  await page.goto("/");
  await expect(page.locator("#load-error")).toBeVisible();
  expect(intercepted).toBeGreaterThan(0);
  await expect(page.locator("#runtime-state")).toHaveText("Unable to start WebAssembly");
  await expect(page.locator("#playground")).toHaveAttribute("data-ready", "false");
  expect(browserErrors).toHaveLength(1);
  // Console text formatting differs across engines; verify the Error itself.
  const message = await failure;
  expect(await message.args()[1].evaluate((error) => ({
    name: error.name,
    message: error.message,
  }))).toEqual({ name: "Error", message: "expected module load failure" });
  browserErrors.length = 0; // The deliberately injected load failure was handled.
});
