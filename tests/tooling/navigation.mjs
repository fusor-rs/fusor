import { root, env as buildEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort } from "../../scripts/build.mjs";
// Real browser History API + Fetch tests against the standalone preview server.
// The proxy holds selected responses and observes actual transport disconnects.
import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import { join } from "node:path";
import { chromium, firefox, webkit, expect } from "@playwright/test";
import { observeFetch } from "../../scripts/observe-fetch.mjs";
import { observeConsoleErrors } from "../../scripts/observe-console.mjs";

const executable = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
const env = buildEnv;
let server, proxy, browser;
const requests = [];
const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 20_000, interval: 25, process: server });
async function connect(page, mode) {
  await page.waitForFunction(() => document.querySelector("#outlet h1"));
  await page.evaluate(async () => {
    const boot = document.querySelector('script[type="module"]').src;
    window.client = await import(new URL("./pkg/app.js", boot).href);
  });
  if (mode === "typed") await page.evaluate(() => window.client.use_typed_router());
}

async function exactRoutes(page) {
  await page.evaluate(() => {
    window.client.unmount();
    history.replaceState({ application: "exact routes" }, "", "/reader/articles/1?key=one#start");
    window.client.start_exact_router();
    window.exact = document.querySelector("#outlet section");
  });
  const current = page.locator("#outlet section");
  await exactHistory(page, current);
  const beforeFailure = page.url();
  await current.evaluate(node => window.exact = node);
  assert.match(await page.evaluate(() => {
    try { window.client.exact_navigate("/reader/articles/1?mode=fail", false); }
    catch (error) { return String(error); }
  }), /exact preparation failed/);
  assert.equal(page.url(), beforeFailure);
  assert(await current.evaluate(node => node === window.exact));
  await fragmentFailures(page);
  await presentationFailures(page);
  await page.evaluate(() => window.client.exact_navigate("/reader/articles/1?mode=reenter", false));
  await expect(current).toHaveAttribute("data-url", "/articles/1?mode=reenter");
  await page.evaluate(() => window.client.exact_navigate("/reader/articles/1?mode=dispose", false));
  await expect(page.locator("#outlet")).toBeEmpty();
  await page.evaluate(() => {
    window.client.exact_dispose(); // Repeated disposal remains harmless.
    window.client.unmount();
    history.replaceState({}, "", "/reader/");
    window.client.start_exact_router();
    window.client.unmount(); // The exported exact-router handle survives its parent.
  });
  await expect(page.locator("#outlet")).toBeEmpty();
  assert.match(await page.evaluate(() => {
    try { window.client.exact_navigate("/reader/articles/2", false); }
    catch (error) { return String(error); }
  }), /router is not active/);
  await page.evaluate(() => window.client.start());
  await expect(page.locator("h1")).toHaveText("A small library");
}

async function exactHistory(page, current) {
  await expect(current).toHaveAttribute("data-url", "/articles/1?key=one#start");
  const initialLength = await page.evaluate(() => history.length);
  await page.evaluate(() => window.client.exact_navigate(location.href, false));
  assert.equal(await page.evaluate(() => history.length), initialLength);
  assert(await current.evaluate(node => node === window.exact), "identical URL keeps the view");
  await page.locator("#exact-note").fill("old route");
  await page.evaluate(() => window.client.exact_navigate("/reader/articles/1?key=two#start", false));
  await expect(current).toHaveAttribute("data-url", "/articles/1?key=two#start");
  assert(await page.evaluate(() => !window.exact.isConnected), "query identity replaces the view");
  await expect(page.locator("#exact-note")).toHaveValue("");
  await current.evaluate(node => window.exact = node);
  await page.evaluate(() => location.hash = "changed");
  await expect(current).toHaveAttribute("data-url", "/articles/1?key=two#changed");
  assert(await page.evaluate(() => !window.exact.isConnected), "native fragment identity replaces the view");
  await page.goBack();
  await expect(current).toHaveAttribute("data-url", "/articles/1?key=two#start");
  await page.goForward();
  await expect(current).toHaveAttribute("data-url", "/articles/1?key=two#changed");
  await page.evaluate(() => window.client.exact_navigate("/reader/articles/1?key=two#end", false));
  await expect(page.locator("h1")).toBeFocused();
  await page.waitForFunction(() => window.scrollY > 1000);
  await page.evaluate(() => {
    document.querySelector("#home-link").focus();
    window.scrollTo(0, 350);
    window.client.exact_navigate("/reader/articles/1?key=three#end", true);
  });
  await expect(page.locator("#home-link")).toBeFocused();
  assert.equal(await page.evaluate(() => window.scrollY), 350);
  await page.evaluate(() => window.client.exact_navigate("/reader/articles/1?key=four", false));
  assert.equal(await page.evaluate(() => window.scrollY), 0);
}

async function fragmentFailures(page) {
  await page.evaluate(() => window.client.exact_navigate("/reader/articles/1?fragment=probe", false));
  assert.equal(await page.evaluate(() => window.client.exact_last_error()), undefined);
  const original = await page.evaluate(() => {
    window.exact = document.querySelector("#outlet section");
    const original = {href: location.href, state: history.state};
    location.hash = "fail";
    return original;
  });
  await page.waitForFunction(() => window.client.exact_last_error()?.includes("exact preparation failed"));
  await page.waitForFunction(href => location.href === href, original.href);
  assert.deepEqual(await page.evaluate(() => history.state), original.state);
  assert(await page.locator("#outlet section").evaluate(node => node === window.exact));
  const reloaded = page.waitForEvent("load");
  const failedWrite = page.waitForEvent("console", {
    predicate: message => message.text().includes("injected fragment history write failure"),
  });
  await page.evaluate(() => {
    history.replaceState = () => { throw Error("injected fragment history write failure"); };
    location.hash = "write-failure";
  });
  await Promise.all([failedWrite, reloaded]);
  assert.equal(new URL(page.url()).hash, "#write-failure");
  await connect(page, "typed");
  await page.evaluate(() => window.client.start_exact_router());
  const failedRead = await page.evaluate(() => {
    Object.defineProperty(history, "state", {configurable: true, get() { throw Error("injected history read failure"); }});
    try {
      dispatchEvent(new PopStateEvent("popstate"));
      return window.client.exact_last_error();
    } finally {
      delete history.state;
    }
  });
  assert.match(failedRead, /injected history read failure/);
}

async function presentationFailures(page) {
  for (const [method, prototype] of [["focus", "HTMLElement"], ["setAttribute", "Element"], ["querySelector", "Element"]]) {
    const result = await page.evaluate(({method, prototype}) => {
      const target = globalThis[prototype].prototype;
      const original = target[method];
      target[method] = function(...arguments_) {
        if ((method === "focus" && this.tagName === "H1")
          || (method === "setAttribute" && arguments_[0] === "tabindex")
          || (method === "querySelector" && arguments_[0] === "[autofocus]")) {
          throw Error(`injected presentation ${method} failure`);
        }
        return original.apply(this, arguments_);
      };
      try {
        window.client.exact_navigate(`/reader/articles/1?presentation=${method}`, false);
        return {error: window.client.exact_last_error(), url: document.querySelector("#outlet section").dataset.url};
      } finally {
        target[method] = original;
      }
    }, {method, prototype});
    assert.match(result.error, new RegExp(`injected presentation ${method} failure`));
    assert.equal(result.url, `/articles/1?presentation=${method}`);
  }
}

async function traversalFailures(page, mode) {
  for (const dispatch of ["throw", "stall"]) {
    await page.locator("#one-link").click();
    await expect(page.locator("#status")).toHaveText("Ready article 1 ()");
    await page.locator("#two-link").click();
    await expect(page.locator("#status")).toHaveText("Ready article 2 ()");
    await page.locator("#home-link").click();
    const reloaded = page.waitForEvent("load");
    const diagnostic = dispatch === "throw"
      ? "injected history dispatch failure"
      : "browser history restoration timed out";
    const reported = page.waitForEvent("console", {
      predicate: message => message.text().includes(diagnostic),
    });
    await page.evaluate(dispatch => {
      [...document.querySelectorAll("template")].find(template => template.content.querySelector(".article")).remove();
      history.go = () => {
        if (dispatch === "throw") throw Error("injected history dispatch failure");
      };
      history.back();
    }, dispatch);
    await Promise.all([reported, reloaded]);
    await connect(page, mode);
    await expect(page.locator("#status")).toHaveText("Ready article 2 ()");
    await page.locator("#home-link").click();
    await expect(page.locator("h1")).toHaveText("A small library");
  }
}
try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--locked", "--offline"], { env, timeout: 180_000 });
  await exec(executable, ["build", "-p", "fusor-navigation", "--features", "browser-tests", "--debug", "--locked", "--offline"], { env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
  const port = await reservePort();
  server = startProcess(executable, ["preview", "examples/navigation/dist", "--port", String(port), "--offline", "--locked"], { env });

  await waitFor(() => server.output.includes("Ctrl+C to stop."), "preview startup");
  const upstream = `http://127.0.0.1:${port}`;
  proxy = createServer(async (req, res) => {
    const url = new URL(req.url, upstream);
    const match = url.pathname.match(/^\/reader\/data\/(\d+)\.txt$/);
    if (match) {
      const request = { id: Number(match[1]), revision: url.searchParams.get("revision"), aborted: false };
      requests.push(request);
      request.finish = () => {
        res.writeHead(request.id === 999 ? 500 : 200, { "content-type": "text/plain", "cache-control": "no-store" });
        res.end(`Response ${request.id}, revision ${request.revision}`);
      };
      const held = request.revision === "slow" || [3, 4, 5].includes(request.id);
      const timer = held ? undefined : setTimeout(request.finish, 30);
      res.on("close", () => { if (!res.writableEnded) { request.aborted = true; clearTimeout(timer); } });
      return;
    }
    try {
      const response = await fetch(url, { method: req.method, headers: { accept: req.headers.accept || "*/*" } });
      res.writeHead(response.status, Object.fromEntries(response.headers));
      res.end(Buffer.from(await response.arrayBuffer()));
    } catch (error) { if (!res.destroyed) { res.writeHead(502); res.end(String(error)); } }
  });
  proxy.listen(0, "127.0.0.1"); await once(proxy, "listening");
  const origin = `http://127.0.0.1:${proxy.address().port}`;
  assert.equal((await fetch(`${origin}/reader/articles/1`, { headers: { accept: "text/html" } })).status, 200);
  assert.equal((await fetch(`${origin}/reader/articles/1`)).status, 404);
  for (const path of ["articles/missing.js", "articles/missing.wasm", "api/items", "__fusor/missing", "outside", "articles-old", "src/lib.rs"]) {
    assert.equal((await fetch(`${origin}/reader/${path}`, { headers: { accept: "text/html" } })).status, 404, path);
  }
  console.log("PASS: deep document fallback is explicit; assets, API paths and non-HTML requests stay 404");

  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    browser = await ({ chromium, firefox, webkit }[name]).launch(name === "chromium" ? { channel: process.env.PLAYWRIGHT_CHANNEL || undefined } : {});
    for (const mode of ["declarative", "typed"]) {
      const page = await browser.newPage();
      const errors = [];
      page.on("pageerror", error => errors.push(String(error)));
      const consoleErrors = await observeConsoleErrors(page);
      const fetches = await observeFetch(page, "/reader/data/");
      await page.goto(`${origin}/reader/articles/1?revision=initial`); await connect(page, mode);
      await page.evaluate(() => window.client.probe_commit_queue());
      await expect(page.locator("#status")).toHaveText("Ready article 1 (initial)");
      const beforeDuplicate = requests.length;
      assert.match(await page.evaluate(() => {
        try { window.client.start(); return "unexpected success"; }
        catch (error) { return String(error); }
      }), /already mounted/);
      assert.equal(requests.length, beforeDuplicate);
      const metadata = requests.findLast(request => request.id === 5);
      assert(metadata, "nested read started independently from article loading");
      await expect(page.locator("#metadata")).toContainText("Loading metadata");
      await page.locator("#toggle-metadata").click();
      await expect(page.locator("#metadata")).toBeEmpty();
      await waitFor(() => metadata.aborted, "conditional child disposal cancels its Fetch");
      await page.locator("#note").fill("Keep my note");
      await page.evaluate(() => { window.note = document.querySelector("#note"); window.note.focus(); window.note.setSelectionRange(2, 5); });
      const first = requests.length;
      await page.evaluate(() => window.client.navigate("/reader/articles/1?revision=slow", false));
      await waitFor(() => requests.length > first, "slow Fetch started");
      const slow = requests.at(-1);
      await expect(page.locator("#status")).toHaveText("Loading article 1 (slow)");
      await expect(page.locator("#article-text")).toContainText("Article 1 (initial)");
      await page.evaluate(() => window.client.navigate("/reader/articles/1?revision=fast", false));
      await expect(page.locator("#status")).toHaveText("Ready article 1 (fast)");
      await waitFor(() => slow.aborted, "superseded Fetch was aborted at the server");
      assert.deepEqual(await page.evaluate(() => [window.note === document.querySelector("#note"), document.activeElement === window.note, window.note.selectionStart, window.note.selectionEnd]), [true, true, 2, 5]);
      await expect(page.locator("#note")).toHaveValue("Keep my note");
      const count = requests.length;
      await page.locator("#refresh").click();
      await waitFor(() => requests.length === count + 1, "explicit refresh");
      await expect(page.locator("#status")).toHaveText("Ready article 1 (fast)");
      await page.locator("#two-link").click();
      await expect(page.locator("#status")).toHaveText("Ready article 2 ()");
      await expect(page.locator("#note")).toHaveValue("");
      assert.equal(await page.evaluate(() => window.note.isConnected), false);
      assert.equal(await page.evaluate(() => document.activeElement.tagName), "H1");
      await page.goBack(); await expect(page.locator("#status")).toHaveText("Ready article 1 (fast)");
      await page.goForward(); await expect(page.locator("#status")).toHaveText("Ready article 2 ()");
      await page.reload(); await connect(page, mode); await expect(page.locator("#status")).toHaveText("Ready article 2 ()");
      const beforeReplace = await page.evaluate(() => history.length);
      await page.evaluate(() => window.client.navigate("/reader/articles/2?revision=replaced", true));
      await expect(page.locator("#status")).toHaveText("Ready article 2 (replaced)");
      assert.equal(await page.evaluate(() => history.length), beforeReplace);
      await page.locator("#fragment-link").click();
      await expect(page).toHaveURL(/#end$/); await expect(page.locator("#status")).toHaveText("Ready article 2 (replaced)");
      await page.goBack(); await expect(page).not.toHaveURL(/#end$/);
      await page.evaluate(() => window.client.navigate("/reader/articles/999", false));
      await expect(page.locator("#status")).toContainText("HTTP 500");
      await page.evaluate(() => window.client.navigate("/reader/missing/page", false));
      await expect(page.locator("h1")).toHaveText("Page not found");
      await page.reload(); await connect(page, mode); await expect(page.locator("h1")).toHaveText("Page not found");
      console.log(`PASS (${name}/${mode}): real Fetch cancellation, keyed state, retry, query retention, links, history, errors and deep reload`);

      // Destination construction starts a resource, then a nested mount fails.
      // No fetch may start, and neither the old page nor history may change.
      await page.locator("#home-link").click();
      for (const method of ["pushState", "replaceState"]) {
        const beforeWrite = await fetches.started();
        const rollback = await page.evaluate(method => {
          const original = history[method], before = location.href, state = JSON.stringify(history.state);
          const view = document.querySelector("#outlet h1");
          history[method] = () => { throw new Error("expected history write failure"); };
          let message;
          try { window.client.navigate("/reader/articles/3", method === "replaceState"); }
          catch (error) { message = String(error); }
          finally { history[method] = original; }
          return { message, sameUrl: location.href === before, sameState: JSON.stringify(history.state) === state,
            sameView: document.querySelector("#outlet h1") === view, pages: document.querySelectorAll("#outlet h1").length };
        }, method);
        assert.match(rollback.message, /expected history write failure/);
        assert.deepEqual([rollback.sameUrl, rollback.sameState, rollback.sameView, rollback.pages], [true, true, true, 1]);
        assert.equal(await fetches.started(), beforeWrite, "abandoned prepared navigation starts no Fetch");
      }
      await page.evaluate(() => {
        window.metadataTemplate = [...document.querySelectorAll("template")].find(t => t.content.querySelector("small"));
        window.metadataTemplate.remove();
      });
      const beforeFailure = await fetches.started();
      await page.locator("#one-link").click();
      await expect(page.locator("h1")).toHaveText("A small library");
      await expect(page).toHaveURL(`${origin}/reader/`);
      assert.equal(await fetches.started(), beforeFailure);
      await page.evaluate(() => document.body.append(window.metadataTemplate));
      await page.locator("#one-link").click(); await expect(page.locator("#status")).toHaveText("Ready article 1 ()");
      await page.locator("#two-link").click(); await expect(page.locator("#status")).toHaveText("Ready article 2 ()");
      await page.locator("#home-link").click();
      await page.evaluate(() => {
        window.articleTemplate = [...document.querySelectorAll("template")].find(t => t.content.querySelector(".article"));
        window.articleTemplate.remove();
      });
      const priorErrors = consoleErrors.length;
      await page.evaluate(() => history.back());
      await waitFor(() => consoleErrors.length > priorErrors, "failed pop reported");
      await expect(page).toHaveURL(`${origin}/reader/`);
      await expect(page.locator("h1")).toHaveText("A small library");
      await page.evaluate(() => document.body.append(window.articleTemplate));
      // Restoration has completed; navigating again remains usable.
      await page.locator("#one-link").click(); await expect(page.locator("#status")).toHaveText("Ready article 1 ()");
      console.log(`PASS (${name}/${mode}): failed staged mount starts no reads; failed pop restores the previous URL and view`);

      await traversalFailures(page, mode);

      await page.evaluate(() => {
        [...document.querySelectorAll("template")].find(t => t.content.querySelector(".article")).content.querySelector("#note").setAttribute("autofocus", "");
      });
      await page.locator("#two-link").click();
      await expect(page.locator("#status")).toHaveText("Ready article 2 ()");
      assert.deepEqual(await page.evaluate(() => [document.activeElement.id, document.querySelector("#note").tabIndex, document.querySelector("#note").hasAttribute("tabindex")]), ["note", 0, false]);
      await page.evaluate(() => {
        [...document.querySelectorAll("template")].find(t => t.content.querySelector(".article")).content.querySelector("#note").removeAttribute("autofocus");
      });
      const exceptions = await page.evaluate(() => {
        const cases = [
          { ctrlKey: true }, { metaKey: true }, { altKey: true }, { shiftKey: true }, { button: 1 },
          { target: "_blank" }, { download: "file" }, { rel: "external" },
          { href: "https://example.com/" }, { href: "/outside" }, { href: "#end" },
          { href: "/reader/missing/route" }, { marked: false },
        ];
        return cases.map(options => {
          const anchor = document.createElement("a"); anchor.href = options.href || "/reader/articles/2";
          if (options.marked !== false) anchor.setAttribute("data-fusor-link", "");
          for (const key of ["target", "download", "rel"]) if (key in options) anchor.setAttribute(key, options[key]);
          document.body.append(anchor);
          let intercepted;
          document.addEventListener("click", event => { intercepted = event.defaultPrevented; event.preventDefault(); }, { once: true });
          anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, composed: true, ...options }));
          anchor.remove(); return intercepted;
        });
      });
      assert.deepEqual(exceptions, [false, false, false, false, false, false, false, false, false, false, false, mode === "declarative", false]); // Typed links require a recognized route; declarative matching owns fallback paths.
      for (const href of ["https://example.com/", "/outside"]) {
        assert.equal(await page.evaluate(href => { try { window.client.navigate(href, false); return false; } catch { return true; } }, href), true);
      }
      const pendingCount = requests.length;
      await page.evaluate(() => window.client.navigate("/reader/articles/1?revision=slow", false));
      await waitFor(() => requests.length > pendingCount, "pending read before unmount");
      const pendingRead = requests.at(-1);
      await page.evaluate(() => window.client.unmount());
      await expect(page.locator("#outlet")).toBeEmpty();
      await waitFor(() => pendingRead.aborted, "unmount cancels Fetch");
      assert.equal(await page.evaluate(() => {
        let intercepted;
        document.addEventListener("click", event => { intercepted = event.defaultPrevented; event.preventDefault(); }, { once: true });
        document.querySelector("#two-link").click(); return intercepted;
      }), false);
      const beforeRemount = requests.length;
      const beforeRemountFetches = await fetches.started();
      const failedStart = await page.evaluate(() => {
        const template = [...document.querySelectorAll("template")].find(t => t.content.querySelector(".article"));
        const before = JSON.stringify(history.state);
        template.remove();
        let failed = false;
        try { window.client.start(); } catch { failed = true; }
        finally { document.body.append(template); }
        return { failed, unchanged: JSON.stringify(history.state) === before, empty: !document.querySelector("#outlet").children.length };
      });
      assert.deepEqual(failedStart, { failed: true, unchanged: true, empty: true });
      assert.equal(await fetches.started(), beforeRemountFetches, "failed initial driver construction starts no Fetch");
      // Preparation and later commit failures must release the router lease,
      // remove listeners, restore history metadata, and start no owned reads.
      for (const mode of [0, 1]) {
        const failure = await page.evaluate(mode => {
          history.replaceState({ application: "preserve this" }, "");
          const before = JSON.stringify(history.state);
          let message;
          try { window.client.probe_startup(mode); } catch (error) { message = String(error); }
          return { message, before, after: JSON.stringify(history.state), empty: !document.querySelector("#outlet").children.length };
        }, mode);
        assert.match(failure.message, /expected (preparation|commit) failure/);
        assert.equal(failure.after, failure.before);
        assert.equal(failure.empty, true);
        assert.equal(await fetches.started(), beforeRemountFetches);
      }
      await page.evaluate(() => window.client.start());
      await waitFor(() => requests.length === beforeRemount + 2, "remounted owned reads started");
      requests.findLast(request => request.id === 1 && request.revision === "slow").finish();
      await expect(page.locator("#status")).toHaveText("Ready article 1 (slow)");
      assert.equal(requests.length, beforeRemount + 2);
      await page.locator("#home-link").click();
      const beforeRows = requests.length;
      await page.locator("#previews").click();
      await expect(page.locator(".preview")).toHaveCount(2);
      await waitFor(() => requests.length === beforeRows + 2, "independent keyed row reads started");
      const rows = requests.slice(beforeRows);
      await page.locator("#clear-previews").click();
      await expect(page.locator(".preview")).toHaveCount(0);
      await waitFor(() => rows.every(request => request.aborted), "keyed row disposal cancels its reads");
      await page.evaluate(() => {
        window.client.unmount();
        window.client.probe_startup(2);
        window.client.unmount();
        window.client.unmount();
        window.client.start();
      });
      await expect(page.locator("h1")).toHaveText("A small library");
      assert.deepEqual(errors, []);
      console.log(`PASS (${name}/${mode}): native link exceptions, origin/base checks, conditional/keyed ownership, cancellation on disposal and clean remount`);
      if (mode === "typed") {
        await exactRoutes(page);
        assert.deepEqual(errors, []);
        console.log(`PASS (${name}): typed query/fragment identity, direct mount, focus/scroll, reentry and disposal with surviving handles`);
      }
      await page.close();
    }
    await browser.close(); browser = undefined;
  }
} finally {
  if (browser) await browser.close();
  if (proxy) { proxy.closeAllConnections(); await new Promise(resolve => proxy.close(resolve)); }
  await stopProcess(server);
}
