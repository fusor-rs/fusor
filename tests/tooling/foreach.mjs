import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve,extname} from 'node:path';
import {chromium, expect} from '@playwright/test';
import {buildPackage} from '../../scripts/build.mjs';
await buildPackage('fusor-foreach');
const root=resolve('examples/foreach/dist');
const server=createServer(async(req,res)=>{
  const pathname=new URL(req.url,'http://localhost').pathname;
  const path=resolve(root,'.'+(pathname==='/'?'/index.html':pathname));
  if(!path.startsWith(root+'/')){res.writeHead(404).end();return;}
  try{const bytes=await readFile(path);res.setHeader('content-type',({'.html':'text/html','.js':'text/javascript','.wasm':'application/wasm'})[extname(path)]||'text/plain');res.end(bytes);}catch{res.writeHead(404).end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
let browser;
try {
  browser=await chromium.launch(process.env.PLAYWRIGHT_CHANNEL?{channel:process.env.PLAYWRIGHT_CHANNEL}:{});
  const page=await browser.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  const rows=page.locator('#items > li');
  await expect(rows).toHaveCount(2);
  await expect(rows.locator('.position')).toHaveText(['0','1']);
  await expect(page.locator('#live')).toHaveText('2');
  await page.locator('#tracked > li').nth(0).getByRole('button').click();
  await rows.nth(0).getByRole('textbox').fill('keep this note');
  await rows.nth(0).evaluate(node=>window.adaRow=node);
  // Position-free forwarding rows: moves keep identity and local state; value
  // changes still reach retained rows.
  const forwarded=page.locator('#forwarded > li');
  await expect(forwarded.locator('.forwarded-title')).toHaveText(['Ada','Grace']);
  await forwarded.nth(0).getByRole('button').click();
  await forwarded.nth(0).evaluate(node=>window.adaForwarded=node);
  await page.getByRole('button',{name:'Reverse',exact:true}).click();
  await expect(rows.locator('.title')).toHaveText(['Grace','Ada']);
  await expect(rows.locator('.position')).toHaveText(['0','1']);
  await expect(page.locator('#tracked > li output')).toHaveText(['0','1']);
  await expect(page.locator('#tracked .tracked-index')).toHaveText(['0','1']);
  assert(await rows.nth(1).evaluate(node=>node===window.adaRow));
  await expect(rows.nth(1).getByRole('textbox')).toHaveValue('keep this note');
  await expect(forwarded.locator('.forwarded-title')).toHaveText(['Grace','Ada']);
  await expect(forwarded.locator('output')).toHaveText(['0','1']);
  assert(await forwarded.nth(1).evaluate(node=>node===window.adaForwarded));
  await page.getByRole('button',{name:'Rename',exact:true}).click();
  await expect(rows.locator('.title')).toHaveText(['Grace','Ada Lovelace']);
  await expect(forwarded.locator('.forwarded-title')).toHaveText(['Grace','Ada Lovelace']);
  assert(await forwarded.nth(1).evaluate(node=>node===window.adaForwarded));
  await page.getByRole('button',{name:'Insert',exact:true}).click();
  await expect(rows.locator('.position')).toHaveText(['0','1','2']);
  assert(await rows.nth(2).evaluate(node=>node===window.adaRow));
  await expect(forwarded.locator('.forwarded-title')).toHaveText(['Lin','Grace','Ada Lovelace']);
  await expect(forwarded.locator('output')).toHaveText(['0','0','1']);
  assert(await forwarded.nth(2).evaluate(node=>node===window.adaForwarded));
  await rows.nth(0).getByRole('button',{name:'Remove'}).click();
  await expect(rows.locator('.position')).toHaveText(['0','1']);
  await expect(forwarded.locator('.forwarded-title')).toHaveText(['Grace','Ada Lovelace']);
  assert(await forwarded.nth(1).evaluate(node=>node===window.adaForwarded));
  await expect(page.locator('#live')).toHaveText('2');
  await expect(page.locator('#groups .nested')).toHaveText(['0.0 Nested Ada','1.0 Nested Grace']);
  await page.getByRole('button',{name:'Reverse groups',exact:true}).click();
  await expect(page.locator('#groups .nested')).toHaveText(['0.0 Nested Grace','1.0 Nested Ada']);
  await expect(page.locator('foreach')).toHaveCount(0);
  await expect(page.locator('app')).toHaveCount(0);
  await page.evaluate(async () => { window.appApi = await import(new URL('./pkg/app.js', document.querySelector('script[type=module]').src).href); });
  assert.equal(await page.evaluate(() => appApi.constructions()), 1);
  const duplicate = await page.evaluate(() => { try { appApi.restart(); return ''; } catch(e) { return String(e); } });
  assert.match(duplicate, /already mounted/);
  assert.equal(await page.evaluate(() => appApi.constructions()), 1);
  const cleanupCount = await page.evaluate(() => appApi.cleanups());
  const stoppedTitles = await rows.locator('.title').allTextContents();
  await page.evaluate(() => appApi.stop());
  assert.equal(await page.evaluate(() => appApi.cleanups()), cleanupCount + 2);
  // Existing root markup remains, but disposed bindings no longer respond.
  await page.getByRole('button',{name:'Reverse',exact:true}).click();
  await expect(rows.locator('.title')).toHaveText(stoppedTitles);
  const failed = await page.evaluate(() => { appApi.fail_next(); try { appApi.restart(); return ''; } catch(e) { return String(e); } });
  assert.match(failed, /requested startup failure/);
  await expect(rows.locator('.title')).toHaveText(stoppedTitles);
  await page.evaluate(() => appApi.restart());
  await expect(rows.locator('.title')).toHaveText(['Ada','Grace']);
  await expect(page.locator('#live')).toHaveText('2');
  assert.equal(await page.evaluate(() => appApi.constructions()), 3);
  assert.deepEqual(errors,[]);
  console.log('PASS App: inferred state, duplicate-start guard, disposal, failure and retry; ForEach: inline HTML, insertion, deletion, value updates, reactive positions, nested scopes, component cleanup and keyed DOM/input preservation');
} finally {await browser?.close();server.closeAllConnections();await new Promise(r=>server.close(r));}
