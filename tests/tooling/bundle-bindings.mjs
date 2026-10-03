// Independent flat generated consumer: validates fast-path observable behavior.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { extname, join, resolve, sep } from 'node:path';
import { promisify } from 'node:util';
import { chromium, firefox, webkit } from 'playwright';
const exec = promisify(execFile), root = process.cwd();
const scratch = await mkdtemp(join(tmpdir(), 'fusor-bundle-bindings-'));
const suffix = process.platform === 'win32' ? '.exe' : '';
const env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target/bundle-binding-tests') };
const run = (program, args, cwd = scratch) => exec(program, args, { cwd, env, timeout: 240000, maxBuffer: 8e6 });
let browser, server;
try {
  await mkdir(join(scratch, 'src/bin'), { recursive: true });
  await mkdir(join(scratch, 'web'));
  const dep = name => JSON.stringify(join(root, 'crates', name));
  await writeFile(join(scratch, 'Cargo.toml'), `[package]
name="bundle-binding-consumer"
version="0.1.0"
edition="2024"
[workspace]
[lib]
crate-type=["cdylib","rlib"]
[dependencies]
fusor-core={path=${dep('fusor-core')},features=["islands"]}
fusor-islands={path=${dep('fusor-islands')}}
wasm-bindgen="=0.2.117"
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
fusor-server={path=${dep('fusor-server')}}
[build-dependencies]
fusor-build={path=${dep('fusor-build')}}
[package.metadata.fusor]
entry="web/index.html"
[profile.release]
opt-level="s"
lto=true
codegen-units=1
`);
  await writeFile(join(scratch, 'build.rs'), 'fn main()->Result<(),fusor_build::BuildError>{fusor_build::compile_app()}\n');
  await writeFile(join(scratch, 'src/bin/render.rs'), 'fn main(){print!("{}",bundle_binding_consumer::render().unwrap());}\n');
  await writeFile(join(scratch, 'src/lib.rs'), String.raw`
use fusor::{Signal, signal};
#[derive(Clone)]
struct Fixture { value: Signal<String>, empty: Signal<String>, tick: Signal<u32>, clicks: Signal<u32>, number: Signal<u32>, signed: Signal<i32> }
impl Fixture {
    fn new() -> Self { Self { value: signal("seed 日本語😀 <&".into()), empty: signal(String::new()), tick: signal(0), clicks: signal(0), number: signal(u32::MAX), signed: signal(i32::MIN) } }
    fn text(&self) -> String { self.tick.get(); self.value.get() }
    fn integer(&self) -> u32 { self.tick.get(); self.number.get() }
}
struct Custom(i32);
impl std::ops::Deref for Custom { type Target = i32; fn deref(&self) -> &i32 { &self.0 } }
impl std::fmt::Display for Custom { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "custom({})", self.0) } }
fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));
#[cfg(not(target_arch = "wasm32"))]
pub fn render() -> Result<String, String> {
    use fusor_server::Render;
    Ok(Fixture::new().render(&mut fusor_server::Context::new())?.into_string())
}
#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use fusor::dom::{Component, Scope, delivery, document};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = globalThis, js_name = __fixtureFactory)]
        fn factory_hook();
    }
    thread_local! { static APP: RefCell<Option<(Fixture, Scope)>> = const { RefCell::new(None) }; }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> { document()?.document_element().unwrap().set_attribute("data-fixture-ready", "true") }
    #[wasm_bindgen]
    pub fn mount(hydrate: bool) -> Result<(), JsValue> { mount_impl(hydrate, true) }
    #[wasm_bindgen]
    pub fn mount_template() -> Result<(), JsValue> { mount_impl(false, false) }
    fn mount_impl(hydrate: bool, delivered: bool) -> Result<(), JsValue> {
        let state = Fixture::new(); let captured = state.clone();
        let host = document()?.get_element_by_id("host").unwrap();
        if delivered { delivery::enable(); }
        let make = move |_| { factory_hook(); Ok(captured) };
        let mut scope = if hydrate {
            delivery::with_root(&host.first_element_child().unwrap(), || Fixture::prepare_component(None, Box::new(make)))?
        } else { Fixture::prepare_component(None, Box::new(make))? };
        if !hydrate { scope.attach(&host)?; }
        scope.try_commit()?;
        APP.with(|app| app.replace(Some((state, scope))));
        Ok(())
    }
    #[wasm_bindgen]
    pub fn change(value: String) { let state = APP.with(|app| app.borrow().as_ref().unwrap().0.clone()); state.value.set(value); }
    #[wasm_bindgen]
    pub fn touch() { let tick = APP.with(|app| app.borrow().as_ref().unwrap().0.tick.clone()); tick.update(|value| *value += 1); }
    #[wasm_bindgen]
    pub fn numbers(number: u32, signed: i32) { let state = APP.with(|app| app.borrow().as_ref().unwrap().0.clone()); fusor::batch(|| { state.number.set(number); state.signed.set(signed); }); }
    #[wasm_bindgen]
    pub fn drop_scope() { APP.with(|app| app.borrow_mut().take()); }
}
`);
  await writeFile(join(scratch, 'web/index.html'), `<!doctype html><html><head><meta charset="utf-8"></head><body><div id="host"></div>
<script type="text/rust" src="../src/lib.rs" rust:module="crate"></script>
<template id="fixture-template" rust:component="Fixture" rust:render="shared"><section id="fixture"${Array.from({ length: 80 }, (_, index) => ` data-cache-${index}="{{ state.value.get() }}"`).join('')}><x-bundle-probe></x-bundle-probe><output id="empty">{{ state.empty.get() }}</output><output id="first" title="{{ state.value.get() }}">{{ state.text() }}</output><p id="mixed">[{{ state.text() }}]</p><button id="action" on:click="state.clicks.update(|count| *count += 1)">+</button><output id="count">{{ state.clicks.get() }}</output><output id="later" title="{{ state.value.get() }}">{{ state.text() }}</output><output id="integer" data-number="{{ state.integer() }}">{{ state.integer() }}</output><output id="signed" data-signed="{{ state.signed.get() }}">{{ state.signed.get() }}</output><output id="small">{{ u8::MAX }}|{{ i8::MIN }}|{{ u16::MAX }}|{{ i16::MIN }}</output><output id="wide">{{ u64::MAX }}</output><output id="float">{{ -0.0f64 }}</output><output id="custom">{{ Custom(state.signed.get()) }}</output></section></template>
</body></html>`);
  await exec('cargo', ['build', '-p', 'fusor-cli', '--locked', '--offline'], { cwd: root, timeout: 240000, maxBuffer: 8e6 });
  await run('cargo', ['generate-lockfile', '--offline']);
  await run(join(root, 'target/debug', `fusor${suffix}`), ['build', '--locked', '--offline']);
  const { stdout: html } = await run('cargo', ['run', '--quiet', '--bin', 'render', '--locked', '--offline']);
  const dist = join(scratch, 'dist');
  server = createServer(async (req, res) => {
    const pathname = new URL(req.url, 'http://localhost').pathname;
    if (pathname === '/favicon.ico') { res.writeHead(204).end(); return; }
    const path = resolve(dist, `.${pathname === '/' ? '/index.html' : pathname}`);
    if (!path.startsWith(`${dist}${sep}`)) { res.writeHead(404).end(); return; }
    try {
      const data = await readFile(path);
      res.setHeader('content-type', ({ '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm' })[extname(path)] || 'application/octet-stream');
      res.end(data);
    } catch { res.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  for (const engine of (process.env.PLAYWRIGHT_BROWSERS || 'chromium').split(',')) {
    browser = await { chromium, firefox, webkit }[engine].launch(engine === 'chromium' && process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
    const page = await browser.newPage(), errors = [], logged = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') logged.push(message.text()); });
    await page.addInitScript(() => {
      globalThis.__fixtureConstructed = 0;
      customElements.define('x-bundle-probe', class extends HTMLElement {
        constructor() { super(); globalThis.__fixtureConstructed++; }
      });
      const original = console.error;
      globalThis.__fixtureLoggedErrors = [];
      console.error = function (...args) {
        globalThis.__fixtureLoggedErrors.push(args.map(String).join(' '));
        return Reflect.apply(original, console, args);
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction(() => document.documentElement.dataset.fixtureReady === 'true');
    await page.evaluate(async html => {
      const boot = document.querySelector('script[type=module][src]');
      const app = await import(new URL('pkg/app.js', boot.src)), host = document.querySelector('#host');
      const initial = 'seed 日本語😀 <&', changed = 'changed 日本語😀 <&';
      const check = (condition, message) => { if (!condition) throw Error(message); };
      // More names than the private cache capacity: installed bindings must
      // retain their own immutable name even after another name evicts it.
      const checkNames = fixture => {
        for (let index = 0; index < 80; index++) {
          check(fixture.getAttribute(`data-cache-${index}`) === changed, `static attribute name ${index} was lost or redirected`);
        }
      };
      const componentId = new DOMParser().parseFromString(html, 'text/html').body.firstElementChild.getAttribute('data-fusor-component');
      check(componentId !== null, 'server fixture lacks component identity');
      const reset = hydrate => { app.drop_scope(); host.replaceChildren(); globalThis.__fixtureFactory = () => {}; if (hydrate) host.innerHTML = html; };
      // Run before delivery::enable, using the ordinary mutable DOM template.
      // Instrument native APIs only in this correctness fixture, never timing.
      const template = document.querySelector('#fixture-template');
      check(template instanceof HTMLTemplateElement, 'compiled template missing');
      const originalTemplate = template.innerHTML;
      const nativeEqual = Node.prototype.isEqualNode, nativeImport = Document.prototype.importNode;
      let cacheHits = 0, certificateImports = 0, activeDocumentImports = 0;
      Node.prototype.isEqualNode = function(other) {
        const equal = nativeEqual.call(this, other); if (equal) cacheHits++; return equal;
      };
      Document.prototype.importNode = function(...args) {
        certificateImports++; if (this.defaultView !== null) activeDocumentImports++;
        return Reflect.apply(nativeImport, this, args);
      };
      let mounts = 0;
      const checkTemplateMount = () => {
        globalThis.__fixtureFactory = () => {};
        app.mount_template(); mounts++;
        check(globalThis.__fixtureConstructed === mounts, 'certificate constructed a custom element');
        const fixture = host.firstElementChild;
        check(fixture.getAttribute('data-fusor-instance') === componentId, 'template instance marker');
        app.change(changed); checkNames(fixture);
        check(fixture.querySelector('#later').textContent === changed, 'cached later target');
        fixture.querySelector('#action').click();
        check(fixture.querySelector('#count').textContent === '1', 'cached event target');
      };
      try {
        checkTemplateMount(); app.drop_scope();
        check(certificateImports === 1 && activeDocumentImports === 0, 'cold certificate must use one inert import');
        checkTemplateMount(); app.drop_scope();
        check(cacheHits >= 1 && certificateImports === 1, 'warm mount did not reuse certificate');
        // Static changes invalidate equality, then shifted native paths are saved.
        const extra = document.createElement('aside'); extra.textContent = 'changed static prefix';
        template.content.querySelector('#fixture').prepend(extra);
        const importsBeforeMutation = certificateImports;
        checkTemplateMount();
        check(host.firstElementChild.firstElementChild.localName === 'aside', 'live template edit ignored');
        check(certificateImports === importsBeforeMutation + 1, 'changed template did not replace certificate');
        app.drop_scope(); const hitsBeforeRemount = cacheHits;
        checkTemplateMount(); app.drop_scope();
        check(cacheHits > hitsBeforeRemount, 'changed template certificate was not reused');
        // A late malformed slot must invalidate the certificate and reject before
        // even the first missing Text is inserted. Observe the private clone only
        // through the native clone API, not a framework test hook.
        template.content.querySelector('#later').removeAttribute('data-fusor-text');
        const nativeClone = Node.prototype.cloneNode; let clonedRoot;
        Node.prototype.cloneNode = function(...args) {
          const result = Reflect.apply(nativeClone, this, args);
          if (this === template.content.firstElementChild) clonedRoot = result;
          return result;
        };
        let rejected = false;
        try { app.mount_template(); } catch (error) { rejected = String(error).includes('template mismatch'); }
        finally { Node.prototype.cloneNode = nativeClone; }
        check(rejected && clonedRoot?.querySelector('#empty').childNodes.length === 0, 'mutated template escaped full validation');
        check(!host.childNodes.length, 'failed template mount attached a root');
        check(activeDocumentImports === 0, 'certificate imported into an active document');
      } finally {
        app.drop_scope(); host.replaceChildren(); template.innerHTML = originalTemplate;
        Node.prototype.isEqualNode = nativeEqual; Document.prototype.importNode = nativeImport;
      }
      for (const hydrate of [false, false, true]) {
        reset(hydrate);
        const borrowed = host.firstElementChild;
        app.mount(hydrate);
        const first = host.querySelector('#first'), text = first.firstChild, button = host.querySelector('#action'), count = host.querySelector('#count');
        if (hydrate) check(host.firstElementChild === borrowed, 'hydration replaced root');
        check(host.firstElementChild.getAttribute('data-fusor-instance') === componentId, 'missing or wrong instance marker');
        check(text.data === initial && first.title === initial, 'initial bindings');
        const integer = host.querySelector('#integer').firstChild;
        check(integer.data === '4294967295' && host.querySelector('#signed').textContent === '-2147483648', 'integer initial range');
        check(host.querySelector('#integer').getAttribute('data-number') === '4294967295' && host.querySelector('#signed').getAttribute('data-signed') === '-2147483648', 'integer attribute initial range');
        check(host.querySelector('#small').textContent === '255|-128|65535|-32768', 'small signed and unsigned ranges');
        check(host.querySelector('#wide').textContent === '18446744073709551615' && host.querySelector('#float').textContent === '-0', 'wide integer and float fallback');
        for (const [number, signed] of [[0, 0], [1, -1], [9, 10], [99, -100], [2147483648, 2147483647], [4294967295, -2147483648]]) {
          const prototypeToString = Number.prototype.toString;
          try { Number.prototype.toString = () => { throw Error('mutable numeric formatting hook'); }; app.numbers(number, signed); }
          finally { Number.prototype.toString = prototypeToString; }
          check(integer.data === String(number) && host.querySelector('#signed').textContent === String(signed), 'integer update precision');
          check(host.querySelector('#integer').getAttribute('data-number') === String(number) && host.querySelector('#signed').getAttribute('data-signed') === String(signed), 'integer attribute update precision');
          check(host.querySelector('#custom').textContent === 'custom(' + signed + ')', 'custom Deref formatting bypassed');
        }
        integer.data = 'external integer mutation'; app.touch();
        check(host.querySelector('#integer').firstChild === integer && integer.data === '4294967295', 'integer live Text repair');
        text.data = 'external modification'; app.touch();
        check(first.firstChild === text && text.data === initial, 'same rendered value did not repair live Text');
        app.change(changed); checkNames(host.firstElementChild);
        check(first.firstChild === text && text.data === changed && first.title === changed, 'reactive text/attribute update');
        check(host.querySelector('#mixed').textContent === `[${changed}]`, 'mixed text binding');
        button.click(); check(count.textContent === '1', 'missing or duplicated native handler');
        app.drop_scope(); button.click(); check(count.textContent === '1', 'handler survived scope drop');
        check(hydrate ? host.firstElementChild === borrowed : !host.childNodes.length, 'scope root ownership changed');
      }
      // Targets are validated and captured before user construction. Neither a
      // factory nor a synchronous attribute callback may redirect later bindings.
      for (const mode of ['factory', 'attribute']) {
        reset(true);
        const first = host.querySelector('#first'), originalLater = host.querySelector('#later'), originalText = originalLater.firstChild;
        const originalButton = host.querySelector('#action'), count = host.querySelector('#count');
        const replacement = originalLater.cloneNode(true); replacement.textContent = 'replacement';
        const replacementButton = originalButton.cloneNode(true);
        let replaced = false;
        const replace = () => { if (!replaced) { replaced = true; originalLater.replaceWith(replacement); originalButton.replaceWith(replacementButton); } };
        globalThis.__fixtureFactory = mode === 'factory' ? replace : () => {
          const nativeSet = first.setAttribute;
          first.setAttribute = function(name, value) { nativeSet.call(this, name, value); if (name === 'title') replace(); };
        };
        app.mount(true); check(replaced, `${mode} hook did not run`);
        app.change(changed);
        check(originalText.data === changed && originalLater.title === changed, `${mode} redirected original targets`);
        check(replacement.textContent === 'replacement' && replacement.title === initial, `${mode} bound replacement nodes`);
        originalButton.click(); check(count.textContent === '1', `${mode} lost original event target`);
        replacementButton.click(); check(count.textContent === '1', `${mode} bound replacement event target`);
        app.drop_scope(); originalButton.click(); check(count.textContent === '1', `${mode} failed listener cleanup`);
      }
      reset(true);
      const empty = host.querySelector('#empty'), later = host.querySelector('#later');
      later.removeAttribute('data-fusor-text');
      let rejected = false;
      try { app.mount(true); } catch (error) { rejected = String(error).includes('template mismatch'); }
      check(rejected && empty.childNodes.length === 0, 'late descriptor failure filled early empty text host');
      // Initial errors propagate and clean listeners; later errors retain the
      // existing logging policy without turning into uncaught page exceptions.
      reset(true);
      const failedButton = host.querySelector('#action'), failedCount = host.querySelector('#count');
      globalThis.__fixtureFactory = () => { host.querySelector('#first').setAttribute = () => { throw Error('intentional initial attribute failure'); }; };
      rejected = false;
      try { app.mount(true); } catch (error) { rejected = String(error).includes('intentional initial attribute failure'); }
      check(rejected, 'initial attribute error was not returned');
      failedButton.click(); check(failedCount.textContent === '0', 'failed initialization retained a listener');
      reset(false); app.mount(false);
      host.querySelector('#first').setAttribute = () => { throw Error('intentional later attribute failure'); };
      app.change(changed); app.drop_scope(); host.replaceChildren();
    }, html);
    assert.deepEqual(errors, []);
    const recorded = await page.evaluate(() => globalThis.__fixtureLoggedErrors);
    assert.equal(recorded.filter(message => message.includes('intentional later attribute failure')).length, 1);
    assert.deepEqual(recorded.filter(message => !message.includes('intentional later attribute failure')), []);
    console.log(JSON.stringify({ engine, nativeConsoleMessages: logged, recordedErrors: recorded }));
    console.log(`PASS ${engine}: flat bindings, cold/cache/hydration, original targets, live Text repair, native events, late validation, attribute errors and cleanup`);
    await browser.close(); browser = null;
  }
} finally {
  await browser?.close(); server?.closeAllConnections();
  if (server) await new Promise(resolve => server.close(resolve));
  await rm(scratch, { recursive: true, force: true });
}
