import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { chromium, expect } from '@playwright/test';
import { command, suiteEnv } from '../../scripts/build.mjs';

await command('cargo', ['run','-p','fusor-cli','--bin','fusor','--locked','--offline','--','build','--manifest-path','examples/npm/Cargo.toml','--offline'], { env: suiteEnv });
await command('npm', ['run', 'typecheck', '--prefix', 'examples/npm']);
const root = resolve('examples/npm/dist');
const server = createServer(async (req,res) => {
  if (req.url === '/favicon.ico') { res.writeHead(204).end(); return; }
  const file = resolve(root,'.'+(req.url==='/'?'/index.html':new URL(req.url,'http://localhost').pathname));
  if (!file.startsWith(root+sep)) { res.writeHead(404).end(); return; }
  try { res.setHeader('content-type',({'.js':'text/javascript','.wasm':'application/wasm','.html':'text/html','.css':'text/css','.svg':'image/svg+xml'})[extname(file)]||'text/plain');res.end(await readFile(file)); }
  catch { res.writeHead(404).end(); }
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
let browser;
try {
  browser=await chromium.launch(process.env.PLAYWRIGHT_CHANNEL?{channel:process.env.PLAYWRIGHT_CHANNEL}:{});
  const page=await browser.newPage(); const errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  await page.addInitScript(() => {
    const add = HTMLCanvasElement.prototype.addEventListener, remove = HTMLCanvasElement.prototype.removeEventListener;
    window.chartListeners = new Map(); window.chartAdds = 0;
    HTMLCanvasElement.prototype.addEventListener = function(type, fn, options) { window.chartAdds++; window.chartListeners.set(fn,type); return add.call(this,type,fn,options); };
    HTMLCanvasElement.prototype.removeEventListener = function(type, fn, options) { window.chartListeners.delete(fn); return remove.call(this,type,fn,options); };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);
  await expect(page.locator('#result')).toContainText('isLowerCase("rust"): true');
  assert.equal(await page.evaluate(()=>customElements.get('sl-animation')===undefined),true,'lazy registration has not executed');
  assert.equal(await page.locator('#animation').evaluate(el=>Object.hasOwn(el,'keyframes')),false,'no own property shadows later accessors');
  assert.equal(await page.locator('#animation').getAttribute('keyframes'),null,'object property is never serialized to an attribute');
  const chartBefore = await page.locator('#chart canvas').evaluate(canvas => { window.originalCanvas = canvas; return {image:canvas.toDataURL(), adds:window.chartAdds, listeners:window.chartListeners.size}; });
  assert(chartBefore.listeners > 0, 'real Chart.js registered canvas input listeners');
  await page.locator('#increment').click();
  await expect(page.locator('#increment')).toContainText('Count 2');
  const chartAfter = await page.locator('#chart canvas').evaluate(canvas => ({same:canvas===window.originalCanvas, image:canvas.toDataURL(), adds:window.chartAdds}));
  assert.equal(chartAfter.same,true); assert.equal(chartAfter.adds,chartBefore.adds,'update does not recreate Chart.js listeners');
  assert.notEqual(chartAfter.image,chartBefore.image,'Chart.js redraws the updated Rust data');
  await page.locator('#toggle-chart').click();
  await expect(page.locator('#chart')).toHaveCount(0);
  assert.equal(await page.evaluate(()=>window.chartListeners.size),0,'component removal destroys Chart.js and its listeners');
  for (let cycle = 0; cycle < 5; cycle++) {
    await page.locator('#toggle-chart').click();
    await expect(page.locator('#chart canvas')).toHaveCount(1);
    assert.equal(await page.evaluate(()=>window.chartListeners.size),chartBefore.listeners,'remount keeps exactly one chart listener set');
    await page.locator('#toggle-chart').click();
    await expect(page.locator('#chart canvas')).toHaveCount(0);
    assert.equal(await page.evaluate(()=>window.chartListeners.size),0,'repeated removal releases all chart listeners');
  }
  await page.locator('#toggle-chart').click();
  await page.locator('#promise').click();
  await expect(page.locator('#result')).toHaveText('Promise resolved; mutex locked: false');
  await page.locator('#register').click();
  await expect(page.locator('#finished')).toHaveText('1');
  assert.equal(await page.locator('#animation').evaluate(el=>Array.isArray(el.keyframes)&&el.keyframes[1].transform==='translateX(160px)'),true);
  assert.equal(await page.locator('#animation').evaluate(el=>el.getAttribute('keyframes')),null);
  // Stop the Rust-owned application; component cleanup destroys the library.
  await page.evaluate(async()=>{const script=document.querySelector('script[type="module"][src]').src;const url=new URL('./pkg/app.js',script);const app=await import(url.href);app.stop();});
  assert.equal(await page.evaluate(()=>window.chartListeners.size),0,'Chart.js destroy removes listeners on application disposal');
  assert.equal(errors.length,0,errors.join('\n'));
  console.log('Native JavaScript: direct utility/promise imports, Chart.js identity and disposal, Shoelace object props/event/dynamic registration passed');
} finally { await browser?.close(); await new Promise(done=>server.close(done)); }
