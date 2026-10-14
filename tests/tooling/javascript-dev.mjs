import { root, suiteEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory } from "../../scripts/build.mjs";
import assert from 'node:assert/strict';
import { mkdir, readFile, realpath, readdir, writeFile, rm } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { chromium } from '@playwright/test';

const repository = root;
const scratch = await realpath(await temporaryDirectory('fusor-javascript-dev-'));
const app = join(scratch, 'native-module-watch');
const executable = join(repository, 'target/debug', process.platform === 'win32' ? 'fusor.exe' : 'fusor');
const env = { ...suiteEnv, RUSTUP_TOOLCHAIN: process.env.RUSTUP_TOOLCHAIN || "stable", CARGO_NET_OFFLINE: 'true', npm_config_cache: join(root, 'target/npm-cache') };
let server, browser;
const cli = (args, options = {}) => exec(executable, args, { cwd: scratch, env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024, ...options });
const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 120_000, interval: 100, process: server });
const output = async () => JSON.parse(await readFile(join(app, '.fusor/dev/.fusor-output.json'), 'utf8'));
try {
  await exec('cargo', ['build', '-p', 'fusor-cli', '--offline', '--locked'], { cwd: repository, env, timeout: 180_000 });
  await cli(['new', app, '--framework-path', repository, '--javascript', '--skip-install']);
  const cargoFile = join(app, 'Cargo.toml');
  await writeFile(cargoFile, (await readFile(cargoFile, 'utf8')).replace('base-path = "/"', 'base-path = "/watch/"'));
  await exec('cargo', ['generate-lockfile', '--offline'], { cwd: app, env });
  // A module-bearing application still checks and emits editor types with no
  // Node executable or installed npm packages.
  await cli(['check', '--manifest-path', cargoFile, '--offline'], { env: { ...env, FUSOR_NODE: join(scratch, 'no-node-installed') } });
  const declarations = await readdir(join(app, '.fusor/types'));
  assert(declarations.some(name => name.endsWith('.d.ts')), 'Node-free CLI check emits component editor declarations');
  const fixture = join(app, 'fixture-package');
  await mkdir(fixture);
  await writeFile(join(fixture, 'package.json'), JSON.stringify({ name: 'fusor-watch-fixture', version: '1.0.0', type: 'module', exports: './browser.js' }));
  await writeFile(join(fixture, 'browser.js'), 'export const platform = "browser1";');
  await exec(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['pack', '--pack-destination', app, '--ignore-scripts'], { cwd: fixture, env });
  const dependency = join(app, 'node_modules/fusor-watch-fixture');
  const pkg = { name: 'native-module-watch', private: true, type: 'module', devDependencies: { esbuild: '0.28.2' }, dependencies: { 'fusor-watch-fixture': 'file:./fusor-watch-fixture-1.0.0.tgz' } };
  await writeFile(join(app, 'package.json'), JSON.stringify(pkg));
  await exec(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['install', '--package-lock-only', '--prefer-offline', '--ignore-scripts', '--no-audit', '--no-fund'], { cwd: app, env });
  const htmlFile = join(app, 'web/index.html');
  await writeFile(htmlFile, (await readFile(htmlFile, 'utf8')).replace('<main>', '<main><div id="module-host"></div>'));
  const state = join(app, 'web/state.ts'), css = join(app, 'web/style.css'), svg = join(app, 'web/icon.svg');
  await writeFile(state, 'export const value: number = 1;');
  await writeFile(css, 'main { color: rgb(12, 34, 56); }');
  await writeFile(svg, '<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>');
  await writeFile(join(app, 'web/app.js'), `import { value } from './state.ts';
import { platform } from 'fusor-watch-fixture';
import './style.css';
import icon from './icon.svg';
export function onMount({ root }) {
  root.dataset.jsvalue = String(value);
  root.dataset.package = platform;
  const image = Object.assign(document.createElement('img'), { id: 'module-image', src: icon });
  root.querySelector('#module-host').append(image);
  return () => image.remove();
}`);
  const port = await reservePort();
  server = startProcess(executable, ['dev', '--manifest-path', cargoFile, '--offline', '--locked', '--port', String(port)], { cwd: app, env });

  await waitFor(() => server.output.includes('Ctrl+C to stop.'), 'development server startup');
  browser = await chromium.launch(process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
  const page = await browser.newPage(), errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${port}/watch/`);
  await page.waitForFunction(() => document.querySelector('main')?.dataset.jsvalue === '1');
  const graph = (await output()).javascript.inputs;
  for (const file of [state, css, svg, join(dependency, 'browser.js'), join(dependency, 'package.json')]) assert(graph.includes(file), `dev graph includes ${file}`);
  const changed = async (file, contents, predicate) => {
    const before = (await output()).generation;
    await writeFile(file, contents);
    await waitFor(async () => (await output()).generation !== before, `rebuild ${file}`);
    await page.waitForFunction(predicate);
    // Prior-generation files remain available to a page already loading them.
    assert((await readFile(join(app, '.fusor/dev/__fusor', before, 'pkg/app.js'))).length > 0);
  };
  await changed(state, 'export const value: number = 2;', () => document.querySelector('main')?.dataset.jsvalue === '2');
  await changed(join(dependency, 'browser.js'), 'export const platform = "browser2";', () => document.querySelector('main')?.dataset.package === 'browser2');
  await changed(css, 'main { color: rgb(65, 43, 21); }', () => { const main = document.querySelector('main'); return main && getComputedStyle(main).color === 'rgb(65, 43, 21)'; });
  await changed(svg, '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="20" height="20" fill="green"/></svg>', () => document.querySelector('#module-image')?.naturalWidth === 20);
  const lastGood = (await output()).generation, logPosition = server.output.length;
  await writeFile(state, 'export const value = ;');
  await waitFor(() => server.output.slice(logPosition).includes('Keeping the last successful build.'), 'syntax failure');
  assert.equal((await output()).generation, lastGood, 'failed JavaScript build retains the complete compatible generation');
  assert.equal(await page.locator('main').getAttribute('data-jsvalue'), '2');
  await changed(state, 'export const value: number = 3;', () => document.querySelector('main')?.dataset.jsvalue === '3');
  assert.deepEqual(errors, []);
  console.log('Native module development: optional scaffold, Node-free checks/declarations, transitive TS/npm/CSS/asset watching and atomic failed-build recovery passed');
} finally {
  await browser?.close();
  await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
