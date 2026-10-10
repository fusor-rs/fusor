// Independent generated SSR/hydration consumer; never imports benchmark workloads.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { extname, join, resolve, sep } from 'node:path';
import { promisify } from 'node:util';
import { chromium, firefox, webkit } from 'playwright';

const exec = promisify(execFile);
const root = process.cwd();
const scratch = await mkdtemp(join(tmpdir(), 'fusor-direct-text-'));
const suffix = process.platform === 'win32' ? '.exe' : '';
// One large descriptor, not a list of tiny component instances: its direct and
// anchored TextIds interleave, then reach handle installation in separate groups.
const largePairs = 256;
const largeMarkup = Array.from({ length: largePairs }, (_, index) =>
  `<output id="large-direct-${index}" title="{{ state.value.get() }}">{{ format!("${index}:{}", state.value.get()) }}</output><p id="large-anchored-${index}">[{{ format!("${index}:{}", state.empty.get()) }}]</p>`).join('');
const env = {
  ...process.env,
  CARGO_TARGET_DIR: join(root, 'target/direct-text-tests'),
};
const run = (program, args, cwd = scratch) => exec(program, args, {
  cwd, env, timeout: 240000, maxBuffer: 8e6,
});
let browser, server;
try {
  await mkdir(join(scratch, 'src/bin'), { recursive: true });
  await mkdir(join(scratch, 'web'));
  const dep = name => JSON.stringify(join(root, 'crates', name));
  await writeFile(join(scratch, 'Cargo.toml'), `[package]
name="direct-text-consumer"
version="0.1.0"
edition="2024"
[workspace]
[lib]
crate-type=["cdylib","rlib"]
[dependencies]
fusor-core={path=${dep('fusor-core')},features=["islands"]}
fusor-components={path=${dep('fusor-components')}}
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
  await writeFile(join(scratch, 'build.rs'),
    'fn main()->Result<(),fusor_build::BuildError>{fusor_build::compile_app()}\n');
  await writeFile(join(scratch, 'src/bin/render.rs'),
    'fn main(){print!("{}",direct_text_consumer::render().unwrap());}\n');
  await writeFile(join(scratch, 'src/lib.rs'), String.raw`
use fusor::{Signal, signal};
#[derive(Clone)]
struct Fixture {
    value: Signal<String>,
    empty: Signal<String>,
    newline: Signal<String>,
    rows: Signal<Vec<u32>>,
}
impl Fixture {
    fn new() -> Self {
        Self {
            value: signal(r#"日本語😀 <&"' </script>"#.into()),
            empty: signal(String::new()),
            newline: signal("\nline <& 日本語😀".into()),
            rows: signal(vec![1, 2]),
        }
    }
}
struct Large {
    value: Signal<String>,
    empty: Signal<String>,
}
struct LargeInputs {
    value: Signal<String>,
    empty: Signal<String>,
}
impl fusor::dom::FromInputs for Large {
    type Error = fusor::dom::JsValue;
    type Inputs = LargeInputs;
    fn from_inputs(inputs: Self::Inputs, _: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self { value: inputs.value, empty: inputs.empty })
    }
}
struct Panel;
struct PanelInputs {}
impl fusor::dom::FromInputs for Panel {
    type Error = fusor::dom::JsValue;
    type Inputs = PanelInputs;
    fn from_inputs(_: Self::Inputs, _: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self)
    }
}
fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));

#[cfg(not(target_arch = "wasm32"))]
pub fn render() -> fusor_server::Result<String> {
    use fusor_server::Render;
    Ok(Fixture::new().render(&mut fusor_server::Context::new())?.into_string())
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use fusor::dom::{Component, Scope, delivery, document};
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! { static APP: RefCell<Option<(Fixture, Scope)>> = const { RefCell::new(None) }; }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        document()?.document_element().unwrap().set_attribute("data-fixture-ready", "true")
    }
    #[wasm_bindgen]
    pub fn mount(hydrate: bool) -> Result<(), JsValue> {
        let state = Fixture::new();
        let captured = state.clone();
        let host = document()?.get_element_by_id("host").unwrap();
        delivery::enable();
        let mut scope = if hydrate {
            delivery::with_root(&host.first_element_child().unwrap(), || {
                Fixture::prepare_component(None, Box::new(move |_| Ok(captured)))
            })?
        } else {
            Fixture::prepare_component(None, Box::new(move |_| Ok(captured)))?
        };
        if !hydrate { scope.attach(&host)?; }
        scope.try_commit()?;
        APP.with(|app| app.replace(Some((state, scope))));
        Ok(())
    }
    // Exercise the default hand-written Component implementation with a
    // scope whose generated implementation already prepared its lifecycle.
    struct Wrapped(Scope);
    impl Component for Wrapped {
        fn mount(self) -> Result<Scope, JsValue> { Ok(self.0) }
    }
    #[wasm_bindgen]
    pub fn legacy_hydration(old_active: bool, new_active: bool, reprepare: bool) -> Result<(), JsValue> {
        let host = document()?.get_element_by_id("host").unwrap();
        delivery::enable();
        let mut original = delivery::with_root(&host.first_element_child().unwrap(), || {
            Fixture::prepare_component(None, Box::new(|_| Ok(Fixture::new())))
        })?;
        original.attach(&host)?;
        if old_active { original.try_commit()?; }
        let wrapped = Wrapped::prepare_component(None, Box::new(move |_| Ok(Wrapped(original))))?;
        let mut wrapped = Wrapped::prepare_component(None, Box::new(move |_| Ok(Wrapped(wrapped))))?;
        if reprepare { wrapped.prepare_owner(None); }
        if new_active { wrapped.try_commit()?; }
        drop(wrapped);
        Ok(())
    }
    #[wasm_bindgen]
    pub fn legacy_readiness() -> Result<(), JsValue> {
        use std::{cell::Cell, rc::Rc};
        let mut parent = Scope::new(document()?.create_element("div")?);
        parent.prepare_owner(None);
        let mut original = Scope::new(document()?.create_element("div")?);
        original.prepare_owner(None);
        original.try_commit()?; // This scope has an already-true readiness token.
        let mut wrapped = Wrapped::prepare(&parent.owner(), move |_| Ok(Wrapped(original)))?;
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        wrapped.before_commit(move || { observed.set(observed.get() + 1); Ok(()) })?;
        parent.finish_prepare_subtree()?;
        assert_eq!(calls.get(), 0, "replacement owner inherited stale readiness");
        wrapped.finish_prepare()?;
        parent.finish_prepare_subtree()?;
        assert_eq!(calls.get(), 1, "new owner did not become ready");
        parent.try_commit()?;
        wrapped.try_commit()?;
        assert_eq!(calls.get(), 1, "setup ran twice");
        Ok(())
    }
    #[wasm_bindgen]
    pub fn cancelled_readiness() -> Result<(), JsValue> {
        use std::{cell::Cell, rc::Rc};
        for ready in [false, true] {
            for reprepare in [false, true] {
                let mut parent = Scope::new(document()?.create_element("div")?);
                parent.prepare_owner(None);
                let mut child = Scope::new(document()?.create_element("div")?);
                child.prepare_owner(Some(&parent.owner()));
                let stale = Rc::new(Cell::new(0));
                let observed = stale.clone();
                let captured = Rc::new(());
                let released = Rc::downgrade(&captured);
                child.before_commit(move || {
                    let _ = &captured;
                    observed.set(observed.get() + 1);
                    Ok(())
                })?;
                if ready { child.finish_prepare()?; }
                if reprepare {
                    child.prepare_owner(Some(&parent.owner()));
                    let fresh = Rc::new(Cell::new(0));
                    let observed = fresh.clone();
                    child.before_commit(move || { observed.set(observed.get() + 1); Ok(()) })?;
                    parent.finish_prepare_subtree()?;
                    assert_eq!(fresh.get(), 0, "reprepared child inherited queued readiness");
                    assert_eq!(stale.get(), 0, "reprepared child ran stale setup");
                    assert!(released.upgrade().is_none(), "stale setup retained its captures");
                    child.finish_prepare()?;
                    parent.finish_prepare_subtree()?;
                    assert_eq!(fresh.get(), 1, "reprepared child did not run fresh setup");
                    parent.try_commit()?;
                    child.try_commit()?;
                    parent.finish_prepare_subtree()?;
                    assert_eq!(fresh.get(), 1, "fresh setup ran twice");
                } else {
                    drop(child);
                    parent.finish_prepare_subtree()?;
                    parent.try_commit()?;
                    parent.finish_prepare_subtree()?;
                }
                assert_eq!(stale.get(), 0, "cancelled child ran queued setup");
                assert!(released.upgrade().is_none(), "cancelled setup retained its captures");
            }
        }
        Ok(())
    }
    #[wasm_bindgen]
    pub fn sibling_readiness_disposal() -> Result<(), JsValue> {
        use std::rc::Rc;
        let mut parent = Scope::new(document()?.create_element("div")?);
        parent.prepare_owner(None);
        let mut first = Scope::new(document()?.create_element("div")?);
        first.prepare_owner(Some(&parent.owner()));
        let mut second = Scope::new(document()?.create_element("div")?);
        second.prepare_owner(Some(&parent.owner()));
        let second = Rc::new(RefCell::new(second));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let observed = calls.clone();
        let disposed = second.clone();
        first.before_commit(move || {
            observed.borrow_mut().push("first");
            disposed.borrow().dispose();
            Ok(())
        })?;
        let observed = calls.clone();
        second.borrow_mut().before_commit(move || {
            observed.borrow_mut().push("second");
            Ok(())
        })?;
        first.finish_prepare()?;
        second.borrow().finish_prepare()?;
        assert!(calls.borrow().is_empty(), "prepared parent did not defer setup");
        parent.finish_prepare_subtree()?;
        assert!(second.borrow().owner().is_disposed(), "first setup did not dispose sibling");
        assert_eq!(*calls.borrow(), ["first"], "disposed sibling ran already-ready setup");
        parent.finish_prepare_subtree()?;
        parent.try_commit()?;
        first.try_commit()?;
        assert_eq!(*calls.borrow(), ["first"], "setup reran after sibling disposal");
        Ok(())
    }
    #[wasm_bindgen]
    pub fn change(value: String) {
        let state = APP.with(|app| app.borrow().as_ref().unwrap().0.clone());
        fusor::batch(|| {
            state.empty.set(value.clone());
            state.newline.set(format!("\n{value}"));
            state.value.set(value);
        });
    }
    #[wasm_bindgen]
    pub fn reverse() {
        let rows = APP.with(|app| app.borrow().as_ref().unwrap().0.rows.clone());
        rows.update(|rows| rows.reverse());
    }
    #[wasm_bindgen]
    pub fn unmount() -> Result<(), JsValue> {
        APP.with(|app| app.borrow_mut().take());
        document()?.get_element_by_id("host").unwrap().set_text_content(None);
        Ok(())
    }
}
`);
  await writeFile(join(scratch, 'web/index.html'), `<!doctype html><html><head><meta charset="utf-8"></head><body><div id="host"></div>
<script type="text/rust" src="../src/lib.rs" rust:module="crate"></script>
<template rust:component="Fixture" rust:render="shared"><section id="fixture"><output id="value" title="{{ state.value.get() }}">{{ state.value.get() }}</output><output id="empty">{{ state.empty.get() }}</output><output id="normal">{{ "plain" }}</output><p id="mixed">before {{ state.value.get() }} after</p><pre id="pre">{{ state.newline.get() }}</pre><listing id="listing">{{ state.newline.get() }}</listing><Panel><strong id="projected">{{ state.value.get() }}</strong><ul id="rows"><ForEach items="{{ state.rows.get() }}" key="{{ |item| *item }}"><li data-id="{{ item.get() }}">{{ item.get() }}</li></ForEach></ul></Panel><Large value="{{ state.value.clone() }}" empty="{{ state.empty.clone() }}"></Large></section></template>
<template rust:component="Large" rust:render="shared"><section id="large">${largeMarkup}</section></template>
<template rust:component="Panel" rust:render="shared"><article id="panel"><Children></Children></article></template>
</body></html>`);

  await exec('cargo', ['build', '-p', 'fusor-cli', '--locked', '--offline'], {
    cwd: root, timeout: 240000, maxBuffer: 8e6,
  });
  await run('cargo', ['generate-lockfile', '--offline']);
  await run(join(root, 'target/debug', `fusor${suffix}`), ['build', '--locked', '--offline']);
  const { stdout: html } = await run('cargo', ['run', '--quiet', '--bin', 'render', '--locked', '--offline']);
  assert.match(html, /<output\b(?=[^>]*id="empty")[^>]*><\/output>/);
  assert.match(html, /&lt;\/script&gt;/);
  assert.ok(html.includes('日本語😀'));
  for (const tag of ['pre', 'listing']) {
    assert.match(html, new RegExp(`<${tag}\\b[^>]*><!--fusor:\\d+-->\\nline `));
  }

  const dist = join(scratch, 'dist');
  server = createServer(async (req, res) => {
    const pathname = new URL(req.url, 'http://localhost').pathname;
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
    const driver = { chromium, firefox, webkit }[engine];
    assert.ok(driver, `unknown browser: ${engine}`);
    browser = await driver.launch(engine === 'chromium' && process.env.PLAYWRIGHT_CHANNEL ? { channel: process.env.PLAYWRIGHT_CHANNEL } : {});
    const page = await browser.newPage(), errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction(() => document.documentElement.dataset.fixtureReady === 'true');
    const result = await page.evaluate(async ({ html, largePairs }) => {
      const boot = document.querySelector('script[type=module][src]');
      const app = await import(new URL('pkg/app.js', boot.src));
      const host = document.querySelector('#host');
      const initial = `日本語😀 <&"' </script>`, updated = 'changed 日本語😀 <&';
      const require = (condition, message) => { if (!condition) throw Error(message); };
      const byId = id => host.querySelector(`#${id}`);
      const kinds = id => [...byId(id).childNodes].map(node => node.nodeType).join();
      const values = (value, empty, newline) => {
        for (const id of ['value', 'projected']) require(byId(id).textContent === value, `${id}: wrong text`);
        require(byId('value').getAttribute('title') === value, 'escaped attribute value');
        require(byId('empty').textContent === empty, 'empty slot value');
        require(byId('normal').textContent === 'plain', 'normal direct text');
        require(byId('mixed').textContent === `before ${value} after`, 'mixed content');
        for (const id of ['pre', 'listing']) require(byId(id).textContent === newline, `${id}: lost initial newline`);
        for (let index = 0; index < largePairs; index++) {
          require(byId(`large-direct-${index}`).textContent === `${index}:${value}`, `large direct ${index}: wrong handle/value`);
          require(byId(`large-direct-${index}`).getAttribute('title') === value, `large direct ${index}: wrong element handle`);
          require(byId(`large-anchored-${index}`).textContent === `[${index}:${empty}]`, `large anchored ${index}: wrong handle/value`);
        }
      };
      app.legacy_readiness();
      app.cancelled_readiness();
      app.sibling_readiness_disposal();
      for (const oldActive of [false, true]) {
        for (const newActive of [false, true]) {
          for (const reprepare of [false, true]) {
            host.innerHTML = html;
            const borrowed = host.firstElementChild;
            app.legacy_hydration(oldActive, newActive, reprepare);
            const owned = reprepare ? newActive : oldActive;
            require(owned ? host.childNodes.length === 0 : host.firstElementChild === borrowed,
              `hand-written scope changed hydration ownership: old=${oldActive}, new=${newActive}, reprepare=${reprepare}`);
            host.replaceChildren();
          }
        }
      }
      let retained = 0;
      for (const hydrate of [true, false, false]) {
        app.unmount();
        if (hydrate) host.innerHTML = html;
        const originalRoot = host.firstElementChild;
        const originalText = [];
        if (hydrate) {
          values(initial, '', '\nline <& 日本語😀');
          require(kinds('empty') === '', 'empty SSR output must parse without a Text node');
          const walker = document.createTreeWalker(host, NodeFilter.SHOW_TEXT);
          while (walker.nextNode()) originalText.push([walker.currentNode, walker.currentNode.parentNode]);
        }
        app.mount(hydrate);
        values(initial, '', '\nline <& 日本語😀');
        if (hydrate) {
          require(host.firstElementChild === originalRoot, 'hydration replaced the native root');
          for (const [text, parent] of originalText) require(text.isConnected && text.parentNode === parent, 'hydration replaced native Text');
          retained += originalText.length;
        }
        for (const id of ['value', 'empty', 'normal', 'projected']) {
          require(kinds(id) === '3', `${id}: expected exactly one Text node`);
          require(byId(id).hasAttribute('data-fusor-text'), `${id}: missing direct marker`);
        }
        require(byId('value').hasAttribute('data-fusor-node'), 'bound direct host lost element marker');
        require(!byId('empty').hasAttribute('data-fusor-node'), 'text-only host allocated an element marker');
        require(kinds('mixed') === '3,8,3,8,3', 'mixed content lost anchors/static text');
        for (const id of ['pre', 'listing']) {
          require(kinds(id) === '8,3,8', `${id}: missing text anchors`);
          require(!byId(id).hasAttribute('data-fusor-text'), `${id}: incorrectly specialized`);
        }
        const textNodes = new Map(['value', 'empty', 'projected'].map(id => [id, byId(id).firstChild]));
        require(byId('large').children.length === largePairs * 2, 'large descriptor lost elements');
        const largeTextNodes = [];
        for (let index = 0; index < largePairs; index++) {
          const direct = byId(`large-direct-${index}`), anchored = byId(`large-anchored-${index}`);
          require(kinds(direct.id) === '3', `${direct.id}: wrong direct Text shape`);
          require(kinds(anchored.id) === '3,8,3,8,3', `${anchored.id}: wrong anchored Text shape`);
          require(direct.hasAttribute('data-fusor-node') && direct.hasAttribute('data-fusor-text'), `${direct.id}: lost host association`);
          largeTextNodes.push([direct, direct.firstChild, 0], [anchored, anchored.childNodes[2], 2]);
        }
        const largeIdentity = () => {
          for (const [element, text, index] of largeTextNodes) {
            require(element.childNodes[index] === text, `${element.id}: update replaced Text`);
          }
        };
        app.change(updated);
        values(updated, updated, `\n${updated}`);
        largeIdentity();
        for (const [id, text] of textNodes) require(byId(id).firstChild === text, `${id}: update replaced Text`);
        app.change('');
        values('', '', '\n');
        largeIdentity();
        for (const [id, text] of textNodes) require(byId(id).firstChild === text && text.data === '', `${id}: clearing replaced Text`);
        const rows = [...byId('rows').children], rowText = rows.map(row => row.firstChild);
        require(rows.length === 2 && rows.map(row => row.textContent).join() === '1,2', 'native ForEach rows');
        app.reverse();
        require(byId('rows').children[0] === rows[1] && byId('rows').children[1] === rows[0], 'keyed row identity/order');
        require(rows.every((row, index) => row.firstChild === rowText[index] && row.firstChild.nodeType === 3), 'keyed row Text identity');
        app.unmount();
        require(host.childNodes.length === 0, 'cleanup retained fixture DOM');
      }
      return { retainedNativeTextNodes: retained, largeMixedSlots: largePairs * 2, modes: ['hydration', 'CSR', 'cached CSR'] };
    }, { html, largePairs });
    assert.deepEqual(errors, []);
    assert.ok(result.retainedNativeTextNodes > 0);
    assert.equal(result.largeMixedSlots, largePairs * 2);
    console.log(`PASS ${engine}: generated native escaping/newlines, empty/direct/mixed text, Children/ForEach, identity, updates and cleanup; ${JSON.stringify(result)}`);
    await browser.close(); browser = null;
  }
} finally {
  await browser?.close();
  server?.closeAllConnections();
  if (server) await new Promise(resolve => server.close(resolve));
  await rm(scratch, { recursive: true, force: true });
}
