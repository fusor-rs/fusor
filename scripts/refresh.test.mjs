import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

test("refresh aborts stalled polls, retries, and cancels a pending request on disposal", async () => {
  const source = await readFile(new URL("../crates/fusor-cli/src/dev/refresh.js", import.meta.url), "utf8");
  let poll;
  let deadline;
  let requests = 0;
  let stopped = false;
  const stop = runInNewContext(`${source.replaceAll("export function", "function")}\nwatch('g-1', '/')`, {
    AbortController,
    document: { cloneNode: () => ({}), querySelector: () => null },
    setInterval(callback) {
      poll = callback;
      return "poll";
    },
    clearInterval(timer) {
      assert.equal(timer, "poll");
      stopped = true;
    },
    setTimeout(callback, timeout) {
      assert.equal(timeout, 5000);
      deadline = callback;
      return "deadline";
    },
    clearTimeout(timer) {
      assert.equal(timer, "deadline");
    },
    fetch(url, options) {
      assert.equal(url, "/__fusor/version");
      requests++;
      return new Promise((resolve, reject) => {
        options.signal?.addEventListener("abort", () => reject(options.signal.reason), { once: true });
      });
    },
  });
  const stalled = poll();
  await poll();
  assert.equal(requests, 1);
  deadline();
  await stalled;
  const retry = poll();
  assert.equal(requests, 2);
  stop();
  await retry;
  assert.equal(stopped, true);
});
