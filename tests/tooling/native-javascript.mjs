import {createServer} from 'node:http';
import {readFile,cp,mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import assert from 'node:assert/strict';
import {chromium, expect} from '@playwright/test';
import {command,env} from '../../scripts/build.mjs';

const fixture = resolve('tests/fixtures/native-javascript');
// Reuse installed, lock-checked tools. The test and Cargo build never download
// dependencies; contributors install examples/npm with the documented npm ci.
await mkdir(resolve(fixture,'node_modules'),{recursive:true});
for (const packageName of ['esbuild','@esbuild']) {
  await cp(resolve('examples/npm/node_modules',packageName),resolve(fixture,'node_modules',packageName),{recursive:true,force:true});
}
env.CARGO_TARGET_DIR = resolve('target');
await command('cargo',['run','-p','fusor-cli','--bin','fusor','--locked','--offline','--','build','--manifest-path',resolve(fixture,'Cargo.toml'),'--offline']);
const declaration = await readFile(resolve(fixture, '.fusor/types/web-index-html-Probe.d.ts'), 'utf8');
assert.match(declaration, /readonly value: ReadonlyInput<number>/);
const empty = await readFile(resolve(fixture, '.fusor/types/web-index-html-NoInputs.d.ts'), 'utf8');
assert.match(empty, /export type Inputs = Record<string, never>/);
const root = resolve(fixture,'dist');
const server = createServer(async (req,res) => {
  const url = new URL(req.url, 'http://localhost');
  if (url.pathname === '/favicon.ico') { res.writeHead(204).end(); return; }
  const relative = url.pathname.replace(/^\/review\//,'');
  const file = resolve(root, !relative || !extname(relative) ? 'index.html' : relative);
  if (!file.startsWith(root+sep)) {res.writeHead(404).end();return;}
  try {
    res.setHeader('content-type', ({'.js':'text/javascript','.wasm':'application/wasm','.html':'text/html','.css':'text/css'})[extname(file)] || 'text/plain');
    res.end(await readFile(file));
  } catch {res.writeHead(404).end();}
});
await new Promise(done => server.listen(0,'127.0.0.1',done));
let browser;
try {
  browser = await chromium.launch(process.env.PLAYWRIGHT_CHANNEL ? {channel:process.env.PLAYWRIGHT_CHANNEL} : {});
  const page = await browser.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => {if (m.type()==='error') errors.push(m.text());});
  await page.goto(`http://127.0.0.1:${server.address().port}/review/`);
  await expect(page.locator('#observed')).toHaveText('read:10');
  assert.deepEqual(await page.evaluate(()=>__bridgeReview.inputShape),{
    nullPrototype:true,frozen:true,fields:['__proto__','constructor','value'],proto:'ordinary field',constructor:true,
  },'prototype-like field names are ordinary own fields, and unmarked Rust state is private');
  assert.equal(await page.evaluate(()=>__emptyModuleLoads),1,'shared ES module top-level executes once');
  assert.deepEqual(await page.evaluate(()=>__emptyMounts),[{id:'no-inputs',fields:[]},{id:'empty-inputs',fields:[]}],'same module mounts per instance with optional or empty JsInputs derive');
  for (const [value,result] of [[0,'ok:0'],[42,'ok:42'],[4294967295,'ok:4294967295'],[-1,'error'],[1.5,'error'],[4294967296,'error'],['1','error'],[null,'error'],[false,'error'],[NaN,'error'],[Infinity,'error']]) {
    await page.locator('main').evaluate((root,value)=>root.dispatchEvent(new CustomEvent('typed-u32',{detail:value})),value);
    await expect(page.locator('#typed')).toHaveText(result);
  }
  await page.locator('main').evaluate(root=>root.dispatchEvent(new Event('typed-u32')));
  await expect(page.locator('#typed')).toHaveText('error');
  for (const [value,result] of [[null,'ok:None'],[[],'ok:Some([])'],[[-2147483648,2147483647],'ok:Some([-2147483648, 2147483647])'],[[1,null],'error'],[[1.5],'error'],[{},'error'],['array','error']]) {
    await page.locator('main').evaluate((root,value)=>root.dispatchEvent(new CustomEvent('typed-list',{detail:value})),value);
    await expect(page.locator('#typed')).toHaveText(result);
  }
  assert.deepEqual(await page.evaluate(()=>__bridgeReview.values),[1]);
  await page.locator('#change').click();
  await expect(page.locator('#observed')).toHaveText('read:10');
  assert.deepEqual(await page.evaluate(()=>__bridgeReview.values),[1,2,3], 'same-signal reentrant event should settle without recursion or lost update');
  const beforeUnrelated = await page.evaluate(()=>__bridgeReview.values.slice());
  await page.locator('#unrelated').click();
  await page.evaluate(()=>new Promise(requestAnimationFrame));
  assert.deepEqual(await page.evaluate(()=>__bridgeReview.values),beforeUnrelated,'Rust event reads must not become JavaScript input dependencies');
  await page.locator('#dispose').click();
  await expect(page.locator('#probe')).toHaveCount(0);
  assert.equal(await page.evaluate(()=>__bridgeReview.aborted),1);
  assert.deepEqual(await page.evaluate(()=>__bridgeReview.cleanup),['1:returned:true','1:second:true','1:first:true'],'cleanup must be LIFO and precede DOM removal');
  const beforeReset = await page.evaluate(()=>__bridgeReview.values.slice());
  await page.locator('#reset').click();
  assert.deepEqual(await page.evaluate(()=>__bridgeReview.values),beforeReset,'disposed observer must no longer run');
  await page.locator('#remount').click();
  await expect(page.locator('#observed')).toHaveText('read:20');
  assert.equal(await page.evaluate(()=>__bridgeReview.mounts),2);
  for (let i=0;i<10;i++) {
    await page.locator('#dispose').click();
    await expect(page.locator('#probe')).toHaveCount(0);
    await page.locator('#reset').click();
    await page.locator('#remount').click();
    await expect(page.locator('#probe')).toHaveCount(1);
  }
  assert.equal(await page.evaluate(()=>__bridgeReview.mounts),12);
  assert.equal(await page.evaluate(()=>__bridgeReview.aborted),11);
  assert.equal(await page.evaluate(()=>__bridgeReview.cleanup.length),33);
  assert.deepEqual(errors,[]);
  for (const mode of ['failure','async','bad-export','promise']) {
    const badPage = await browser.newPage();
    const caught = [];
    const uncaught = [];
    badPage.on('console',message=>{if(message.type()==='error')caught.push(message.text());});
    badPage.on('pageerror',error=>uncaught.push(error.message));
    await badPage.goto(`http://127.0.0.1:${server.address().port}/review/?mode=${mode}`);
    await expect.poll(()=>caught.length).toBeGreaterThan(0);
    assert.deepEqual(uncaught,[],`${mode}: setup errors should use runtime reporting, not escape as an uncaught browser error`);
    const snapshot = await badPage.evaluate(()=>__bridgeReview);
    if (mode==='failure') {
      assert(caught.some(value=>value.includes('EXPECTED setup failure')));
      assert(caught.some(value=>value.includes('EXPECTED cleanup failure')));
      assert.deepEqual(snapshot.cleanup,['1:second:true','1:throws:true','1:first:true']);
      assert.equal(snapshot.aborted,1);
      await badPage.locator('#change').click();
      assert.deepEqual(await badPage.evaluate(()=>__bridgeReview.values),[1],'failed setup must stop acquired input subscriptions');
    } else if (mode==='async') {
      assert.equal(snapshot.startedAsync,undefined,'async onMount body must not execute');
      assert(caught.some(value=>value.includes('synchronous')));
    } else if (mode==='bad-export') {
      assert(caught.some(value=>value.includes('not a function')));
    } else {
      assert.equal(snapshot.aborted,1);
      assert.deepEqual(snapshot.cleanup,['1:second:true','1:first:true']);
      assert(caught.some(value=>value.includes('promises are unsupported')));
    }
    await badPage.close();
  }
  const properties = await page.evaluate(async () => {
    const boot = document.querySelector('script[type="module"][src]').src;
    const app = await import(new URL('./pkg/app.js', boot).href);
    const check = (condition, message) => { if (!condition) throw Error(message); };
    let writes = 0, constructed = 0;
    customElements.define('fusor-properties', class extends HTMLElement {
      constructor() { super(); constructed++; }
      set someValue(value) {
        writes++;
        this.last = value;
        if (value === 2) this.dispatchEvent(new CustomEvent('ValueChanged', { detail: 3 }));
        if (value === 99) throw Error('EXPECTED setter failure');
        if (value === 6) app.property_drop();
      }
    });
    app.property_mount('fusor-properties', true);
    check(writes === 0, 'no setter in preparation');
    app.property_set(2);
    check(writes === 0, 'pending value before activation');
    app.property_commit();
    const element = document.querySelector('#property-fixture');
    check(element.last === 3 && writes === 2, 'case-sensitive reentrant custom event settles once');
    app.property_set(3); app.property_unrelated();
    check(writes === 2, 'equal and unrelated updates do no work');
    const object = { count: 1 };
    app.property_set(object);
    check(element.last === object, 'property retains object identity');
    app.property_set(object);
    check(writes === 3, 'same object does not rewrite');
    app.property_set(5); app.property_set(99); app.property_set(5);
    check(element.last === 5, 'setter exception remains recoverable');
    app.property_set(6);
    check(element.last === 6, 'setter can dispose its own view');
    element.remove();
    app.property_mount('fusor-delayed', false); app.property_set(7); app.property_set(8);
    const late = document.querySelector('#property-fixture');
    check(!Object.hasOwn(late, 'someValue'), 'no preupgrade property shadows an accessor');
    let lateWrites = 0;
    customElements.define('fusor-delayed', class extends HTMLElement { set someValue(value) { lateWrites++; this.last = value; } });
    await Promise.resolve(); await Promise.resolve();
    check(late.last === 8 && lateWrites === 1, 'only the latest value is assigned after upgrade');
    app.property_drop(); late.remove();
    app.property_mount('fusor-disposed', false);
    const disposed = document.querySelector('#property-fixture');
    app.property_drop(); disposed.remove();
    let disposedWrites = 0;
    customElements.define('fusor-disposed', class extends HTMLElement { set someValue(value) { disposedWrites++; } });
    customElements.upgrade(disposed);
    await Promise.resolve(); await Promise.resolve();
    check(disposedWrites === 0, 'no late writes after disposal');
    const whenDefined = customElements.whenDefined.bind(customElements);
    let waits = 0;
    customElements.whenDefined = function(name) { if (name === 'fusor-never') waits++; return whenDefined(name); };
    for (let i = 0; i < 50; i++) {
      app.property_mount('fusor-never', false);
      const pending = document.querySelector('#property-fixture');
      app.property_drop(); pending.remove();
    }
    check(waits === 1, 'one shared waiter per unresolved custom element, independent of mount count');
    customElements.whenDefined = whenDefined;
    return { writes, constructed, lateWrites, disposedWrites, waits };
  });
  assert.deepEqual(errors.filter(error => !error.includes('EXPECTED setter failure')), []);
  const beforeStop = await page.evaluate(()=>__bridgeReview.values.slice());
  await page.evaluate(async()=>{
    const app = await import(new URL('./pkg/app.js',document.querySelector('script[type="module"][src]').src));
    app.stop();
  });
  assert.deepEqual(await page.evaluate(()=>__bridgeReview.values),beforeStop,'parent cleanup signal writes cannot publish into an already disposed child owner');
  assert.equal(await page.evaluate(()=>__bridgeReview.aborted),12);
  assert.equal(await page.evaluate(()=>__bridgeReview.cleanup.length),36);
  assert.deepEqual(errors.filter(error => !error.includes('EXPECTED setter failure')), []);
  console.log('PASS custom-element properties: identity, equal suppression, case-sensitive events, throwing/reentrant setters, upgrade/disposal and shared pending waiter:', properties);
  console.log('PASS independent native-JS review: nested-base-path imports, reentrant same-signal writes, untracked event reads, cleanup before DOM removal, parent disposal writes, 12 mounts and 12 disposal cycles.');
  console.log('PASS independent setup failures: partial acquisition, throwing cleanup continuation, async function rejection, invalid export shape, returned Promise rejection.');
  console.log('PASS typed payload bounds, prototype-like input fields, private state, optional/empty derives and shared-module per-instance lifecycle.');
} finally {await browser?.close();await new Promise(done=>server.close(done));}
