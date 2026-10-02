// Hydrating component tags: a generated child adopts its server root in place;
// a hand-written TemplateComponent, which cannot adopt it, replaces it.
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
const scratch = await mkdtemp(join(tmpdir(), 'fusor-component-hydration-'));
const suffix = process.platform === 'win32' ? '.exe' : '';
const env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target/component-hydration-tests') };
const run = (program, args, cwd = scratch) => exec(program, args, { cwd, env, timeout: 240000, maxBuffer: 8e6 });
let browser, server;
try {
  await mkdir(join(scratch, 'src/bin'), { recursive: true });
  await mkdir(join(scratch, 'web'));
  const dep = name => JSON.stringify(join(root, 'crates', name));
  await writeFile(join(scratch, 'Cargo.toml'), `[package]
name="component-hydration-consumer"
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
`);
  await writeFile(join(scratch, 'build.rs'),
    'fn main()->Result<(),Box<dyn std::error::Error>>{fusor_build::compile_app()}\n');
  await writeFile(join(scratch, 'src/bin/render.rs'),
    'fn main(){print!("{}",component_hydration_consumer::render().unwrap());}\n');
  await writeFile(join(scratch, 'src/lib.rs'), String.raw`
use fusor::{Signal, signal};
use fusor::dom::{Component, FromInputs, JsValue, Scope, TemplateComponent, document};
#[derive(Clone)]
struct Fixture {
    show: Signal<bool>,
    key: Signal<u32>,
}
impl Fixture {
    fn new() -> Self {
        Self { show: signal(true), key: signal(0) }
    }
}
struct Generated;
struct GeneratedInputs {}
impl FromInputs for Generated {
    type Error = fusor::dom::JsValue;
    type Inputs = GeneratedInputs;
    fn from_inputs(_: Self::Inputs, _: fusor::OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self)
    }
}
// A hand-written component builds its own root and ignores server DOM.
struct Manual;
struct ManualInputs {}
impl FromInputs for Manual {
    type Error = fusor::dom::JsValue;
    type Inputs = ManualInputs;
    fn from_inputs(_: Self::Inputs, _: fusor::OwnerHandle) -> Result<Self, JsValue> {
        Ok(Self)
    }
}
impl TemplateComponent for Manual {}
impl Component for Manual {
    fn mount(self) -> Result<Scope, JsValue> {
        let root = document()?.create_element("p")?;
        root.set_attribute("data-origin", "browser")?;
        root.set_text_content(Some("browser"));
        Ok(Scope::new(root))
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl fusor_server::Render for Manual {
    const TEMPLATE_HASH: &'static str = "manual-v1";
    fn render(&self, _: &mut fusor_server::Context<'_>) -> fusor_server::Result<fusor_server::Html> {
        let mut writer = fusor_server::Writer::new();
        writer.open("p");
        writer.attr("data-origin", "server");
        writer.end_open();
        writer.text("server");
        writer.close("p");
        Ok(writer.finish())
    }
}
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
    use fusor::dom::delivery;
    use std::cell::RefCell;
    use wasm_bindgen::prelude::*;
    thread_local! { static APP: RefCell<Option<(Fixture, Scope)>> = const { RefCell::new(None) }; }
    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        document()?.document_element().unwrap().set_attribute("data-fixture-ready", "true")
    }
    #[wasm_bindgen]
    pub fn mount() -> Result<(), JsValue> {
        let state = Fixture::new();
        let captured = state.clone();
        let host = document()?.get_element_by_id("host").unwrap();
        delivery::enable();
        let scope = delivery::with_root(&host.first_element_child().unwrap(), || {
            Fixture::prepare_component(None, Box::new(move |_| Ok(captured)))
        })?;
        scope.try_commit()?;
        APP.with(|app| app.replace(Some((state, scope))));
        Ok(())
    }
    fn state() -> Fixture {
        APP.with(|app| app.borrow().as_ref().unwrap().0.clone())
    }
    #[wasm_bindgen]
    pub fn set_key(key: u32) {
        state().key.set(key);
    }
    #[wasm_bindgen]
    pub fn set_show(show: bool) {
        state().show.set(show);
    }
    #[wasm_bindgen]
    pub fn unmount() {
        APP.with(|app| app.borrow_mut().take());
    }
}
`);
  await writeFile(join(scratch, 'web/index.html'), `<!doctype html><html><head><meta charset="utf-8"></head><body><div id="host"></div>
<script type="text/rust" src="../src/lib.rs" rust:module="crate"></script>
<template rust:component="Fixture" rust:render="shared"><section id="fixture"><div id="manual"><Manual rust:if="state.show.get()" rust:key="state.key.get()"></Manual></div><div id="generated"><Generated rust:key="state.key.get()"></Generated></div></section></template>
<template rust:component="Generated" rust:render="shared"><b>generated</b></template>
</body></html>`);

  await exec('cargo', ['build', '-p', 'fusor-cli', '--locked', '--offline'], { cwd: root, timeout: 240000, maxBuffer: 8e6 });
  await run('cargo', ['generate-lockfile', '--offline']);
  await run(join(root, 'target/debug', `fusor${suffix}`), ['build', '--locked', '--offline']);
  const { stdout: html } = await run('cargo', ['run', '--quiet', '--bin', 'render', '--locked', '--offline']);
  assert.match(html, /<p data-origin="server">server<\/p>/);

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
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForFunction(() => document.documentElement.dataset.fixtureReady === 'true');
    await page.evaluate(async html => {
      const boot = document.querySelector('script[type=module][src]');
      const app = await import(new URL('pkg/app.js', boot.src));
      const host = document.querySelector('#host');
      const require = (condition, message) => { if (!condition) throw Error(message); };
      const shown = id => [...host.querySelector(`#${id}`).children];
      const manual = origin => {
        const children = shown('manual');
        require(children.length === 1, `manual slot holds ${children.length} elements`);
        require(children[0].dataset.origin === origin, `manual slot shows the ${children[0].dataset.origin} root`);
        return children[0];
      };

      host.innerHTML = html;
      const fixture = host.firstElementChild;
      const serverManual = manual('server');
      const [serverGenerated] = shown('generated');
      app.mount();
      require(host.firstElementChild === fixture, 'hydration replaced the parent root');
      const browserManual = manual('browser');
      require(!serverManual.isConnected, 'the unadopted server root stayed in the document');
      require(shown('generated').length === 1 && shown('generated')[0] === serverGenerated, 'the generated child did not adopt its server root');

      app.set_key(1);
      const replaced = manual('browser');
      require(replaced !== browserManual && !browserManual.isConnected, 'a new identity kept the previous manual child');
      require(shown('generated').length === 1 && shown('generated')[0] !== serverGenerated && !serverGenerated.isConnected, 'a new identity kept the adopted generated child');

      app.set_show(false);
      require(shown('manual').length === 0 && !replaced.isConnected, 'None kept the manual child');
      app.set_show(true);
      manual('browser');

      app.unmount();
      require(host.firstElementChild === fixture, 'unmount removed hydrated DOM it does not own');
    }, html);
    assert.deepEqual(errors, []);
    console.log(`PASS ${engine}: a generated child adopts its server root; a hand-written child replaces it; identity changes and None replace or remove either`);
    await browser.close(); browser = null;
  }
} finally {
  await browser?.close();
  server?.closeAllConnections();
  if (server) await new Promise(resolve => server.close(resolve));
  await rm(scratch, { recursive: true, force: true });
}
