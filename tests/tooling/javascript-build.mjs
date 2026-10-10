import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { cp, mkdir, mkdtemp, readFile, realpath, writeFile, rm, readdir } from 'node:fs/promises';
import { join, resolve, extname, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { createServer } from 'node:http';
import { chromium, firefox, webkit } from '@playwright/test';

const exec = promisify(execFile), repository = process.cwd();
const scratch = await realpath(await mkdtemp(join(tmpdir(), 'fusor-native-modules-')));
const tool = join(repository, 'crates/fusor-npm/src/tool.mjs');
let server, browser;
try {
  await cp(join(repository, 'examples/npm/package.json'), join(scratch, 'package.json'));
  await cp(join(repository, 'examples/npm/package-lock.json'), join(scratch, 'package-lock.json'));
  for (const name of ['esbuild', '@esbuild', 'is-lower-case', 'tslib']) {
    await cp(join(repository, 'examples/npm/node_modules', name), join(scratch, 'node_modules', name), { recursive: true });
  }
  await mkdir(join(scratch, 'web'), { recursive: true });
  await mkdir(join(scratch, 'generated'), { recursive: true });
  const pkg = JSON.parse(await readFile(join(scratch, 'package.json'), 'utf8'));
  const lock = JSON.parse(await readFile(join(scratch, 'package-lock.json'), 'utf8'));
  pkg.dependencies['fusor-browser-fixture'] = '1.0.0';
  lock.packages[''].dependencies = pkg.dependencies;
  lock.packages['node_modules/fusor-browser-fixture'] = { version: '1.0.0' };
  const conditional = join(scratch, 'node_modules/fusor-browser-fixture');
  await mkdir(conditional, { recursive: true });
  await writeFile(join(conditional, 'package.json'), JSON.stringify({ name: 'fusor-browser-fixture', version: '1.0.0', type: 'module', exports: { browser: './browser.js', default: './node.js' } }));
  await writeFile(join(conditional, 'browser.js'), 'export const platform = "browser";');
  await writeFile(join(conditional, 'node.js'), 'throw new Error("Node export must not run");');
  await writeFile(join(scratch, 'package.json'), JSON.stringify(pkg));
  await writeFile(join(scratch, 'package-lock.json'), JSON.stringify(lock));
  const inline = 'import { shared } from "./shared.ts";\nexport function onMount({ root }) { root.dataset.inline = String(shared); }';
  await writeFile(join(scratch, 'web/index.html'), `<!doctype html>\n<template>\n<script type="module">${inline}</script>\n</template>`);
  await writeFile(join(scratch, 'generated/inline.js'), inline);
  await writeFile(join(scratch, 'web/shared.ts'), 'export const shared: number = 42;');
  await writeFile(join(scratch, 'web/lazy.ts'), 'globalThis.lazyExecutions = (globalThis.lazyExecutions || 0) + 1; export const answer: string = "loaded once";');
  await writeFile(join(scratch, 'web/dot.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle cx="5" cy="5" r="4" fill="red"/></svg>');
  await writeFile(join(scratch, 'web/component.css'), 'main { color: rgb(12, 34, 56); background-image: url("./dot.svg"); }');
  await writeFile(join(scratch, 'web/component.ts'), `import { isLowerCase } from 'is-lower-case';
import { platform } from 'fusor-browser-fixture';
import './component.css';
import dot from './dot.svg';
const transpiled: string = 9;
export function onMount({ root }: { root: HTMLElement }) {
  root.dataset.utility = String(isLowerCase('rust'));
  root.dataset.platform = platform;
  root.dataset.transpiled = String(transpiled);
  root.querySelector('img')!.src = dot;
  root.querySelector('button')!.addEventListener('click', async () => {
    const lazy = await import('./lazy.ts');
    root.dataset.lazy = lazy.answer;
  });
}`);
  const moduleFile = join(scratch, 'modules.json');
  await writeFile(moduleFile, JSON.stringify([
    { id: 'App', path: join(scratch, 'generated/inline.js'), source: join(scratch, 'web/index.html'), line: 3, column: 23, inline: true },
    { id: 'Panel', path: join(scratch, 'web/component.ts'), source: join(scratch, 'web/index.html'), line: 3, column: 1, inline: false },
  ]));
  const dist = join(scratch, 'dist'), output = join(dist, 'docs/__fusor/g-test/pkg'), entry = join(output, 'app.js');
  await mkdir(output, { recursive: true });
  const bindgen = 'export default async function init() { globalThis.wasmUrl = new URL("app_bg.wasm", import.meta.url).pathname; }';
  const bundle = async () => {
    await writeFile(entry, bindgen);
    return exec(process.execPath, [tool, scratch, entry, moduleFile, '/docs/__fusor/g-test/pkg', pkg.devDependencies.esbuild], { maxBuffer: 8 * 1024 * 1024 });
  };
  await bundle();
  const graph = JSON.parse(await readFile(join(output, 'javascript-bundle.json'), 'utf8'));
  for (const name of ['web/component.ts', 'web/shared.ts', 'web/lazy.ts', 'web/component.css', 'web/dot.svg', 'web/index.html', 'node_modules/fusor-browser-fixture/browser.js', 'node_modules/fusor-browser-fixture/package.json', 'package-lock.json']) {
    assert(graph.inputs.includes(join(scratch, name)), `${name} is watched`);
  }
  assert(!graph.inputs.some(file => file.includes('/generated/') || file.includes('/dist/')), 'generated outputs cannot trigger watcher rebuild loops');
  assert(graph.styles.length > 0, 'CSS is published');
  const maps = [];
  for (const file of await readdir(output, { recursive: true })) {
    if (file.endsWith('.map')) maps.push(JSON.parse(await readFile(join(output, file), 'utf8')));
  }
  for (const name of ['component.ts', 'shared.ts', 'index.html']) assert(maps.some(map => map.sources.some(source => source.endsWith(name))), `${name} appears in user source maps`);
  await writeFile(join(dist, 'docs/index.html'), `<!doctype html><html><head>${graph.styles.map(file => `<link rel="stylesheet" href="/__unused__">`.replace('/__unused__', `/docs/__fusor/g-test/pkg/${file}`)).join('')}</head><body><main><img alt="fixture"><button>Load</button></main><script type="module">
import init from './__fusor/g-test/pkg/app.js';
await init();
for (const module of globalThis[Symbol.for('fusor.javascript.modules.v1')].values()) module.onMount({root: document.querySelector('main')});
</script></body></html>`);
  server = createServer(async (request, response) => {
    const pathname = new URL(request.url, 'http://localhost').pathname;
    const file = resolve(dist, '.' + (pathname.endsWith('/') ? pathname + 'index.html' : pathname));
    if (!file.startsWith(dist + sep)) return response.writeHead(404).end();
    try {
      response.setHeader('content-type', ({ '.js': 'text/javascript', '.html': 'text/html', '.css': 'text/css', '.svg': 'image/svg+xml' })[extname(file)] || 'application/octet-stream');
      response.end(await readFile(file));
    } catch { response.writeHead(404).end(); }
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  for (const engine of (process.env.PLAYWRIGHT_BROWSERS || 'chromium').split(',')) {
    browser = await { chromium, firefox, webkit }[engine].launch(
      engine === 'chromium' && process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
    const page = await browser.newPage(), errors = [], requests = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => requests.push(request.url()));
    await page.goto(`http://127.0.0.1:${server.address().port}/docs/`);
    await page.waitForFunction(() => document.querySelector('main').dataset.inline === '42').catch(error => { throw Error(`${error.message}\n${errors.join('\n')}`); });
    assert.deepEqual(await page.locator('main').evaluate(root => ({ ...root.dataset })), { inline: '42', utility: 'true', platform: 'browser', transpiled: '9' }, 'native imports and TS transpilation preserve module behavior');
    assert.equal(await page.locator('main').evaluate(root => getComputedStyle(root).color), 'rgb(12, 34, 56)');
    await page.waitForFunction(() => document.querySelector('img').naturalWidth === 10);
    assert.equal(await page.evaluate(() => globalThis.wasmUrl), '/docs/__fusor/g-test/pkg/app_bg.wasm', 'bundled bindgen keeps its generation-relative Wasm URL');
    assert(!requests.some(url => /\/lazy-.*\.js$/.test(url)), 'dynamic module is not loaded at startup');
    await page.locator('button').click();
    await page.waitForFunction(() => document.querySelector('main').dataset.lazy === 'loaded once');
    await page.locator('button').click();
    assert.equal(await page.evaluate(() => globalThis.lazyExecutions), 1, 'dynamic imports use the ES module cache');
    assert.equal(errors.length, 0, errors.join('\n'));
    await browser.close();
    browser = undefined;
    console.log(`Native module browser checks passed: ${engine}`);
  }

  // Lock and installed-version failures happen before publishing any outputs.
  const installed = join(conditional, 'package.json'), original = await readFile(installed, 'utf8');
  await writeFile(installed, original.replace('1.0.0', '9.0.0'));
  await assert.rejects(bundle(), error => /does not match package-lock.json/.test(error.stderr));
  assert.equal(await readFile(entry, 'utf8'), bindgen);
  await writeFile(installed, original);
  pkg.dependencies['fusor-browser-fixture'] = '9.0.0';
  await writeFile(join(scratch, 'package.json'), JSON.stringify(pkg));
  await assert.rejects(bundle(), error => /differs from package-lock.json/.test(error.stderr));
  pkg.dependencies['fusor-browser-fixture'] = '1.0.0';
  await writeFile(join(scratch, 'package.json'), JSON.stringify(pkg));
  await rm(join(scratch, 'package-lock.json'));
  await assert.rejects(bundle(), error => /committed package-lock.json/.test(error.stderr));
  console.log('Native module build: inline/external TS, browser packages, deferred imports, CSS/assets, source maps, dependency graph, nested URLs and locked installation checks passed');
} finally {
  await browser?.close();
  if (server) await new Promise(done => server.close(done));
  await rm(scratch, { recursive: true, force: true });
}
