// Compiled hydration regression coverage. Separate from the timing evaluator.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chromium, firefox, webkit } from 'playwright';
import { createSiteServer } from '../../benchmarks/harness/server.mjs';
const html = execFileSync('cargo', ['run', '-p', 'fusor-bench-workload', '--bin', 'ssr', '--release', '--locked', '--', '1000', 'html'], { encoding: 'utf8', maxBuffer: 4e6, stdio: ['ignore', 'pipe', 'inherit'] });
const server = createSiteServer();
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
let browser;
try {
  for (const engine of (process.env.PLAYWRIGHT_BROWSERS || 'chromium').split(',')) {
    browser = await { chromium, firefox, webkit }[engine].launch(
      engine === 'chromium' && process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
    const page = await browser.newPage();
    const pageErrors = [];
    page.on('pageerror', error => pageErrors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/workloads/fusor/`);
    await page.waitForFunction(() => globalThis.__benchReady !== undefined);
    const result = await page.evaluate(async html => {
      const api = globalThis.__bench, host = document.querySelector('#app');
      const cases = ['missing', 'duplicate', 'tag', 'namespace', 'noncanonical-element',
        'missing-text-marker', 'noncanonical-text', 'unknown-text', 'duplicate-text', 'text-tag', 'text-namespace',
        'wrong-text-host', 'extra-text-node', 'element-slot', 'comment-slot',
        'obsolete-text-anchors', 'mount-marker', 'unknown-element', 'old-schema'];
      for (const damage of cases) {
        api.unmount(); host.innerHTML = html;
        const row = host.querySelector('li'), button = row.querySelector('button'), span = row.querySelector('span');
        const id = span.getAttribute('data-fusor-text');
        if (id === null || span.childNodes.length !== 1 || span.firstChild.nodeType !== Node.TEXT_NODE)
          throw Error('fixture did not compile to a sole text child');
        if (damage === 'missing') button.removeAttribute('data-fusor-node');
        if (damage === 'duplicate') button.after(button.cloneNode(true));
        if (damage === 'tag' || damage === 'namespace') {
          const replacement = damage === 'tag' ? document.createElement('a') : document.createElementNS('http://www.w3.org/2000/svg', 'button');
          for (const attr of button.attributes) replacement.setAttribute(attr.name, attr.value);
          button.replaceWith(replacement);
        }
        if (damage === 'noncanonical-element') button.setAttribute('data-fusor-node', `0${button.getAttribute('data-fusor-node')}`);
        if (damage === 'missing-text-marker') span.removeAttribute('data-fusor-text');
        if (damage === 'noncanonical-text') span.setAttribute('data-fusor-text', `0${id}`);
        if (damage === 'unknown-text') span.setAttribute('data-fusor-text', '999999');
        if (damage === 'duplicate-text') span.after(span.cloneNode(true));
        if (damage === 'text-tag' || damage === 'text-namespace') {
          const replacement = damage === 'text-tag' ? document.createElement('output') : document.createElementNS('http://www.w3.org/2000/svg', 'span');
          for (const attr of span.attributes) replacement.setAttribute(attr.name, attr.value);
          replacement.textContent = span.textContent; span.replaceWith(replacement);
        }
        if (damage === 'wrong-text-host') {
          span.removeAttribute('data-fusor-text'); button.setAttribute('data-fusor-text', id);
        }
        if (damage === 'extra-text-node') span.append(document.createTextNode('extra'));
        if (damage === 'element-slot') span.firstChild.replaceWith(document.createElement('b'));
        if (damage === 'comment-slot') span.append(document.createComment('ordinary comment'));
        if (damage === 'obsolete-text-anchors') {
          span.before(document.createComment(`fusor:${id}`));
          span.after(document.createComment(`/fusor:${id}`));
        }
        if (damage === 'mount-marker') row.append(document.createComment('fusor:mount:999'));
        if (damage === 'unknown-element') button.setAttribute('data-fusor-node', '999999');
        if (damage === 'old-schema') host.firstElementChild.setAttribute('data-fusor-version', '1');
        let rejected = false;
        try { api.hydrate(1000); } catch (error) { rejected = String(error).includes('template mismatch'); }
        if (!rejected) throw Error(`accepted malformed hydration: ${damage}`);
      }
      // Keyed server rows must match the list's keys in order, none missing or extra.
      for (const [damage, expected] of [['missing-row', 'missing native row'], ['extra-row', 'unexpected native row'],
        ['key', 'native row key mismatch'], ['order', 'native row key mismatch']]) {
        api.unmount(); host.innerHTML = html;
        const rows = [...host.querySelectorAll('li')];
        if (damage === 'missing-row') rows.at(-1).remove();
        if (damage === 'extra-row') rows.at(-1).after(rows.at(-1).cloneNode(true));
        if (damage === 'key') rows[500].setAttribute('data-fusor-key', '"500"');
        if (damage === 'order') rows[1].before(rows[2]);
        let message = '';
        try { api.hydrate(1000); } catch (error) { message = String(error); }
        if (message !== expected) throw Error(`keyed hydration ${damage}: ${message || 'accepted'}`);
      }
      // Validation of a descriptor must finish before filling any empty text host.
      api.unmount(); host.innerHTML = html;
      const invalidRow = host.querySelector('li'), empty = invalidRow.querySelector('span');
      empty.replaceChildren();
      invalidRow.querySelector('button').removeAttribute('data-fusor-node');
      let rejected = false;
      try { api.hydrate(1000); } catch (error) { rejected = String(error).includes('template mismatch'); }
      if (!rejected || empty.childNodes.length !== 0) throw Error('failed hydration mutated an empty text host');

      // A late mismatch drops already-prepared siblings. They still borrow
      // server DOM: do not remove their roots, run deferred writes or leave an
      // active event listener behind. Empty-node insertion is atomic within the
      // invalid descriptor, not a transaction across every earlier descriptor.
      for (const late of [1, 500, 999]) {
        api.unmount(); host.innerHTML = html;
        const borrowedRows = [...host.querySelectorAll('li')];
        const firstSpan = borrowedRows[0].querySelector('span');
        const firstText = firstSpan.firstChild;
        firstText.data = 'server content awaiting commit';
        const lateSpan = borrowedRows[late].querySelector('span');
        lateSpan.replaceChildren();
        borrowedRows[late].querySelector('button').removeAttribute('data-fusor-node');
        let lateRejected = false;
        try { api.hydrate(1000); } catch (error) { lateRejected = String(error).includes('template mismatch'); }
        if (!lateRejected) throw Error(`accepted malformed late row ${late}`);
        const remaining = [...host.querySelectorAll('li')];
        if (remaining.length !== borrowedRows.length || remaining.some((row, i) => row !== borrowedRows[i]))
          throw Error(`failed late hydration removed/replaced borrowed rows at ${late}`);
        if (firstSpan.firstChild !== firstText || firstText.data !== 'server content awaiting commit')
          throw Error(`failed late hydration ran a deferred text write at ${late}`);
        if (lateSpan.childNodes.length !== 0) throw Error(`failed late descriptor inserted empty text at ${late}`);
        borrowedRows[0].querySelector('button').click(); api.flush();
        if (firstSpan.firstChild !== firstText || firstText.data !== 'server content awaiting commit')
          throw Error(`failed late hydration left an active first-row listener at ${late}`);
        // The workload's explicit unmount clears #app itself. Scope rollback
        // has already happened and is checked above, before that public reset.
        api.unmount();
      }

      api.unmount(); host.innerHTML = html;
      const rows = [...host.querySelectorAll('li')], text = rows[1].querySelector('span').firstChild;
      // Empty server text parses to no node; retain existing nonempty Text nodes.
      rows[0].querySelector('span').replaceChildren();
      api.hydrate(1000);
      if ([...host.querySelectorAll('li')].some((row, i) => row !== rows[i])) throw Error('replaced rows');
      if (rows[1].querySelector('span').firstChild !== text) throw Error('replaced existing text');
      const filled = rows[0].querySelector('span');
      if (filled.childNodes.length !== 1 || filled.firstChild.nodeType !== Node.TEXT_NODE) throw Error('invalid empty slot');
      rows[0].querySelector('button').click(); api.flush();
      if (filled.textContent !== '1') throw Error('missing/duplicate event');
      api.update(1, 77); api.flush();
      if (text.data !== '77') throw Error('reactivity lost');
      api.unmount();
      if (host.childNodes.length) throw Error('unmount retained DOM');
      return cases.length;
    }, html);
    assert.equal(result, 19);
    assert.deepEqual(pageErrors, [], `${engine}: unexpected browser exception`);
    console.log(`PASS ${engine}: ${result} malformed hydration cases, three late-row rollback cases, failure atomicity, empty slot, node identity, events, reactivity and cleanup`);
    await browser.close(); browser = null;
  }
} finally {
  await browser?.close(); server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
