import {createServer} from 'node:http';
import {readFile, cp, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import assert from 'node:assert/strict';
import {chromium, firefox, webkit} from '@playwright/test';
import {command} from '../../scripts/build.mjs';
import {checkWorkerDiagnostics} from '../../scripts/worker-compile.mjs';
if (!process.env.FUSOR_TEST_POOL) await checkWorkerDiagnostics();
const fixture = resolve('tests/fixtures/worker');
await mkdir(resolve(fixture, 'node_modules'), {recursive: true});
for (const name of ['esbuild', '@esbuild']) await cp(resolve('examples/npm/node_modules', name), resolve(fixture, 'node_modules', name), {recursive: true, force: true});
await command('cargo', ['run', '-p', 'fusor-cli', '--bin', 'fusor', '--locked', '--offline', '--', 'build', '--manifest-path', resolve(fixture, 'Cargo.toml'), '--offline', ...(process.env.FUSOR_TEST_POOL ? ['--features', 'pool'] : [])]);
const root = resolve(fixture, 'dist');
const loadFixture = page => page.evaluate(async () => {
  const boot = document.querySelector('script[type="module"][src]').src;
  globalThis.workerFixture = await import(new URL('./pkg/app.js', boot).href);
});
const server = createServer(async (request, response) => {
  const path = new URL(request.url, 'http://localhost').pathname;
  const file = resolve(root, ['/workers/', '/plain/'].includes(path) ? 'index.html' : path.replace(/^\/workers\//, ''));
  if (!file.startsWith(root + sep)) { response.writeHead(404).end(); return; }
  try {
    if (process.env.FUSOR_TEST_POOL && path !== '/plain/') { response.setHeader('Cross-Origin-Opener-Policy', 'same-origin'); response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp'); }
    response.setHeader('content-type', ({'.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm'})[extname(file)] || 'text/plain');
    response.end(await readFile(file));
  } catch { response.writeHead(404).end(); }
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
try {
  for (const engine of (process.env.PLAYWRIGHT_BROWSERS || 'chromium').split(',')) {
    const browser = await ({chromium, firefox, webkit})[engine].launch();
    let deadline;
    try {
      const page = await browser.newPage();
      await page.addInitScript(() => {
        const NativeWorker = Worker;
        globalThis.activeWorkers = new Set();
        globalThis.Worker = class extends NativeWorker {
          constructor(url, options) {
            super(url, options); activeWorkers.add(this);
          }
          terminate() { activeWorkers.delete(this); super.terminate(); }
        };
      });
      const errors = [];
      deadline = setTimeout(() => { console.error(engine + ': worker suite stalled', errors); browser.close(); }, 90000);
      page.on('pageerror', error => page.evaluate(() => globalThis.workerStage).then(stage => { console.error(engine + ': ' + stage + ': ' + error.message); return browser.close(); }).catch(() => {}));
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => {
        if (message.type() === 'error') {
          errors.push(message.text());
          console.error(engine + ': ' + message.text());
        }
      });
      await page.goto(`http://127.0.0.1:${server.address().port}/workers/`);
      await page.getByText('UI mounted', {exact: true}).waitFor();
      assert.equal(await page.evaluate(() => window.workerFixtureUiLoads), 1);
      await loadFixture(page);
      await page.evaluate(() => workerFixture.exercise());
      await page.evaluate(() => workerFixture.exercise_lifetimes());
      await page.evaluate(() => workerFixture.exercise_failures());
      const ticks = await page.evaluate(async () => {
        let ticks = 0;
        const timer = setInterval(() => { if (globalThis.workerStage === 'CPU work') ticks++; }, 10);
        try {
          await workerFixture.exercise_transport();
          return ticks;
        } finally { clearInterval(timer); }
      });
      assert.ok(ticks >= 5, 'UI timers keep running during CPU work');
      assert.equal(await page.evaluate(() => activeWorkers.size), 0, 'owner cleanup terminates physical workers');
      for (const command of ['Call', 'Cancel', 'Credit']) {
        const message = await page.evaluate(async command => {
          const NativeWorker = globalThis.Worker;
          globalThis.Worker = class extends NativeWorker {
            postMessage(message, ...arguments_) {
              if (message.type === command) throw Error(`injected ${command} transport failure`);
              super.postMessage(message, ...arguments_);
            }
          };
          try {
            const operation = {
              Call: 'startup_probe',
              Cancel: 'cancel_transport_failure',
              Credit: 'credit_transport_failure',
            }[command];
            await workerFixture[operation]();
          } catch (error) {
            return String(error);
          } finally {
            globalThis.Worker = NativeWorker;
          }
        }, command);
        assert.match(message, new RegExp(`worker loading failed:.*injected ${command} transport failure`));
        assert.equal(await page.evaluate(() => activeWorkers.size), 0, 'transport failure terminates physical workers');
      }
      const startupFailure = await page.evaluate(async () => {
        const NativeWorker = globalThis.Worker;
        globalThis.Worker = class extends NativeWorker {
          postMessage(message, ...arguments_) {
            if (!message.initialize) super.postMessage(message, ...arguments_);
          }
        };
        try {
          await workerFixture.startup_probe();
        } catch (error) {
          return String(error);
        } finally {
          globalThis.Worker = NativeWorker;
        }
      });
      assert.equal(startupFailure, 'worker loading failed: Worker startup timed out');
      assert.equal(await page.evaluate(() => activeWorkers.size), 0, 'startup deadline terminates stalled workers');
      if (process.env.FUSOR_TEST_POOL) {
        await page.evaluate(async () => {
          await workerFixture.exercise_pool();
          await workerFixture.exercise_compute();
          const Worker = globalThis.Worker;
          globalThis.Worker = class extends Worker { postMessage(message, ...args) { if (message?.type === 'fusor-rayon-init') throw Error('injected compute startup failure'); super.postMessage(message, ...args); } };
          try { if (!(await workerFixture.pool_probe()).startsWith('Load')) throw Error('partial startup must fail'); }
          finally { globalThis.Worker = Worker; }
        });
        assert.equal(await page.evaluate(() => activeWorkers.size), 0, 'pool failure kills every physical worker');
        await page.goto('http://127.0.0.1:' + server.address().port + '/plain/');
        await page.getByText('UI mounted', {exact: true}).waitFor();
        await loadFixture(page);
        assert.match(await page.evaluate(() => workerFixture.pool_probe()), /Unsupported.*SharedMemory/);
      }
      clearTimeout(deadline);
      assert.deepEqual(errors, []);
      console.log(engine + ': worker ' + (process.env.FUSOR_TEST_POOL ? 'and pool ' : '') + 'contracts passed');
    } finally { clearTimeout(deadline); await browser.close(); }
  }
} finally { await new Promise(done => server.close(done)); }
