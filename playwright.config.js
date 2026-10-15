import { defineConfig } from "@playwright/test";

const engines = (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",");
for (const engine of engines) {
  if (!["chromium", "firefox", "webkit"].includes(engine)) {
    throw new Error(`Unknown PLAYWRIGHT_BROWSERS entry: ${engine}`);
  }
}
const projects = engines.map((browserName) => ({
  name: `${browserName}-desktop`,
  use: {
    browserName,
    viewport: { width: 1440, height: 1100 },
    ...(browserName === "chromium" ? { channel: process.env.PLAYWRIGHT_CHANNEL || undefined } : {}),
  },
}));
if (engines.includes("chromium")) {
  projects.push({
    name: "chromium-mobile",
    use: {
      browserName: "chromium",
      channel: process.env.PLAYWRIGHT_CHANNEL || undefined,
      viewport: { width: 393, height: 852 },
      isMobile: true,
      hasTouch: true,
    },
  });
}

export default defineConfig({
  testDir: "./tests/browser",
  fullyParallel: false,
  workers: 1,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: "list",
  use: {
    baseURL: "http://127.0.0.1:4173",
    trace: "retain-on-failure",
    // The playground scrolls smoothly, which can move a control between
    // Playwright's stability check and its click; its reduced-motion styles
    // scroll instantly.
    reducedMotion: "reduce",
  },
  projects,
  webServer: {
    command: "cargo fusor preview examples/playground/dist --port 4173",
    url: "http://127.0.0.1:4173",
    // Never accidentally validate a stale or unrelated process on this port.
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
