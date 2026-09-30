import assert from 'node:assert/strict';
import {readFile, writeFile, rm} from 'node:fs/promises';
import {join} from 'node:path';
import {chromium} from '@playwright/test';
import {root, env, exec, temporaryDirectory, copyProject, independentManifest, reservePort, startProcess, stopProcess, waitFor} from '../../scripts/build.mjs';

const directory = await temporaryDirectory('fusor-worker-dev-');
const executable = join(root, 'target/debug', process.platform === 'win32' ? 'fusor.exe' : 'fusor');
let server, browser;
try {
  await exec('cargo', ['build', '-p', 'fusor-cli', '--offline', '--locked'], {env: {...env, CARGO_TARGET_DIR: join(root, 'target')}, timeout: 180000});
  await copyProject(join(root, 'examples/workers'), directory);
  const manifest = join(directory, 'Cargo.toml');
  await writeFile(manifest, await independentManifest((await readFile(manifest, 'utf8')).replace('[lints]\nworkspace = true', '[lints.rust]\nunsafe_code = "forbid"')));
  await exec('cargo', ['generate-lockfile', '--offline', '--manifest-path', manifest], {env});
  const port = await reservePort();
  server = startProcess(executable, ['dev', '--manifest-path', manifest, '--offline', '--port', String(port)], {env: {...env, CARGO_TARGET_DIR: join(root, 'target')}});
  await waitFor(() => server.output.includes('watching for changes'), 'worker dev startup', {process: server, timeout: 180000});
  browser = await chromium.launch();
  const page = await browser.newPage();
  const url = `http://127.0.0.1:${port}`;
  const response = await page.goto(url);
  assert.equal(response.headers()['cross-origin-opener-policy'], undefined, 'ordinary dev needs no isolation');
  await page.getByRole('status').filter({hasText: '9592 primes below 100000'}).waitFor();
  const before = await (await fetch(url + '/__fusor/version')).text();
  const source = join(directory, 'src/lib.rs');
  await writeFile(source, (await readFile(source, 'utf8')).replace('Ok(count)', 'Ok(count + 1)'));
  await waitFor(async () => (await (await fetch(url + '/__fusor/version')).text()) !== before, 'worker source rebuild', {process: server, timeout: 120000});
  await page.getByRole('status').filter({hasText: '9593 primes below 100000'}).waitFor();
  console.log('Worker dev: automatic packaging, startup, rebuild and generation reload passed');
} finally {
  await browser?.close();
  await stopProcess(server);
  await rm(directory, {recursive: true, force: true});
}
