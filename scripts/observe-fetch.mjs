export async function observeFetch(page, prefix) {
  await page.addInitScript(prefix => {
    const observation = { started: 0, consumed: new Set() };
    globalThis.fusorFetchesForTest = observation;
    const fetch = window.fetch;
    window.fetch = function (input, ...options) {
      const url = new URL(input instanceof Request ? input.url : input, location.href);
      if (url.pathname.startsWith(prefix)) observation.started++;
      return fetch.call(this, input, ...options);
    };
    const text = Response.prototype.text;
    Response.prototype.text = async function () {
      const body = await text.call(this);
      const url = new URL(this.url, location.href);
      if (url.pathname.startsWith(prefix)) observation.consumed.add(url.pathname);
      return body;
    };
  }, prefix);
  return {
    reset: () => page.evaluate(() => {
      fusorFetchesForTest.started = 0;
      fusorFetchesForTest.consumed.clear();
    }),
    started: () => page.evaluate(() => fusorFetchesForTest.started),
    consumed: path => page.waitForFunction(
      path => fusorFetchesForTest.consumed.has(path), path,
    ),
  };
}
