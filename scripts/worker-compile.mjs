import {mkdtemp, mkdir, writeFile, rm} from 'node:fs/promises';
import {resolve, join} from 'node:path';
import assert from 'node:assert/strict';
import {command, exec, env} from './build.mjs';

export async function checkWorkerDiagnostics() {
  await mkdir(resolve('target'), {recursive: true});
  const root = await mkdtemp(resolve('target/worker-diagnostics-'));
  try {
    await mkdir(join(root, 'src'));
    await writeFile(join(root, 'Cargo.toml'), `[package]\nname="worker-diagnostics"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\nfusor-worker={path=${JSON.stringify(resolve('crates/fusor-worker'))}}\n`);
    const cases = [
      ['#[task] fn bad<T>(x:T)->TaskResult<T>{Ok(x)}', 'cannot be generic'],
      ['#[task] fn bad(x:&str)->TaskResult<String>{Ok(x.into())}', 'owned, concrete'],
      ['#[task] fn bad(ctx:TaskContext)->TaskResult<()>{Ok(())}', 'context must be final'],
      ['#[task(stream)] fn bad(x:(),out:StreamSender<u32>)->TaskResult<()>{Ok(())}', 'must be async'],
      ['struct State; #[worker] impl State { fn new(_:())->TaskResult<Self>{Ok(Self)} pub fn close(&mut self,_:())->TaskResult<()>{Ok(())} }', 'reserved worker control name'],
      ['struct State; #[worker] impl State { fn new(_:())->TaskResult<Self>{Ok(Self)} pub fn bad<T>(&mut self,x:T)->TaskResult<T>{Ok(x)} }', 'cannot be generic'],
      ['struct State; #[worker] impl State { fn new(_:())->TaskResult<Self>{Ok(Self)} #[cfg_attr(all(), deprecated(note="use replacement"))] pub fn old(&mut self,_:())->TaskResult<()>{Ok(())} } #[deny(deprecated)] fn caller(client: <State as Worker>::Client) { let _ = client.old(()); }', 'use of deprecated method'],
      ['#[task] #[deprecated(note="use replacement")] fn old()->TaskResult<()>{Ok(())} #[deny(deprecated)] fn caller() { let _ = old::run; }', 'use of deprecated function `old::run`'],
      ['#[task] fn bad(x:Vec<Shared<Vec<u8>>>)->TaskResult<()>{Ok(())}', 'Serialize'],
    ];
    for (const [source, expected] of cases) {
      await writeFile(join(root, 'src/lib.rs'), `use fusor_worker::*;\n${source}\n`);
      const result = await exec('cargo', ['check', '--offline', '--manifest-path', join(root, 'Cargo.toml')], {env: {...env, CARGO_TARGET_DIR: resolve('target/worker-diagnostics')}, timeout: 120000}).then(() => null, error => error);
      assert(result, `invalid declaration compiled: ${source}`);
      assert(result.stderr.includes(expected), result.stderr);
      assert(result.stderr.includes('src/lib.rs'), 'diagnostic must identify authored source');
    }
    await command('cargo', ['test', '--offline', '--manifest-path', resolve('tests/fixtures/worker/Cargo.toml'), '--lib'], {capture: true});
  } finally { await rm(root, {recursive: true, force: true}); }
}
