// Independent compiled consumer: cache correctness, not benchmark timing.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:http";
import { chromium, firefox, webkit } from "playwright";
const executableSuffix = process.platform === "win32" ? ".exe" : "";
const exec = promisify(execFile),
  root = process.cwd(),
  scratch = await mkdtemp(join(tmpdir(), "fusor-template-cache-"));
let browser, server;
try {
  await mkdir(join(scratch, "src"));
  await mkdir(join(scratch, "web"));
  const dep = (name) => JSON.stringify(join(root, "crates", name));
  await writeFile(
    join(scratch, "Cargo.toml"),
    `[package]
name="template-cache-consumer"
version="0.1.0"
edition="2024"
[workspace]
[lib]
crate-type=["cdylib","rlib"]
[dependencies]
fusor-core={path=${dep("fusor-core")},features=["dom"]}
wasm-bindgen="=0.2.117"
[build-dependencies]
fusor-build={path=${dep("fusor-build")}}
[package.metadata.fusor]
entry="web/index.html"
[profile.release]
opt-level="s"
lto=true
codegen-units=1
`,
  );
  await writeFile(
    join(scratch, "src/lib.rs"),
    'include!(env!("FUSOR_MODULE"));\n',
  );
  await writeFile(
    join(scratch, "build.rs"),
    "fn main()->Result<(),Box<dyn std::error::Error>>{fusor_build::compile_app()}\n",
  );
  await writeFile(
    join(scratch, "web/index.html"),
    `<!doctype html><html><head><meta charset="utf-8"><script>
globalThis.constructed=0;customElements.define('cache-probe',class extends HTMLElement {constructor(){super();globalThis.constructed++;}});
</script></head><body><div id="host"></div>
<script type="text/rust">
use fusor::prelude::*;
use fusor::template::*;
use wasm_bindgen::prelude::*;
use std::cell::RefCell;
thread_local!{static APP:RefCell<Option<Scope>>=const{RefCell::new(None)};}
thread_local!{static KEYED:RefCell<Option<(Scope,Signal<Vec<u32>>)>>=const{RefCell::new(None)};static DROPS:RefCell<Vec<u32>>=const{RefCell::new(Vec::new())};}
#[derive(Clone,Eq,PartialEq)]struct ReverseKey(u32);
impl Ord for ReverseKey{fn cmp(&self,other:&Self)->std::cmp::Ordering{other.0.cmp(&self.0)}}
impl PartialOrd for ReverseKey{fn partial_cmp(&self,other:&Self)->Option<std::cmp::Ordering>{Some(self.cmp(other))}}
#[wasm_bindgen]pub fn keyed_mount()->Result<(),JsValue>{
 let root=fusor::dom::document()?.create_element("section")?;root.set_id("keyed-fixture");
 let mut scope=Scope::new(root.clone());let items=signal(vec![1_u32,2,3,4]);let observed=items.clone();
 scope.keyed(&root,move||observed.get(),|id|ReverseKey(*id),|item|{
  let id=item.get_untracked();if id==99{return Err(JsValue::from_str("intentional staging failure"))}
  let root=fusor::dom::document()?.create_element("input")?;root.set_attribute("data-id",&id.to_string())?;
  let mut row=Scope::new(root);row.retain(row.owner().on_cleanup(move||DROPS.with(|drops|drops.borrow_mut().push(id))));Ok(row)
 })?;
 scope.attach(&fusor::dom::document()?.get_element_by_id("host").unwrap())?;KEYED.with(|app|app.replace(Some((scope,items))));Ok(())
}
#[wasm_bindgen]pub fn keyed_change(mode:u32){let items=KEYED.with(|app|app.borrow().as_ref().unwrap().1.clone());items.set(match mode{0=>vec![4,2,1,3],1=>vec![4,2,2,3],2=>vec![4,2,5,99],_=>vec![4,2]});}
#[wasm_bindgen]pub fn keyed_drops()->Vec<u32>{DROPS.with(|drops|drops.borrow().clone())}
#[wasm_bindgen]pub fn keyed_dense(){let items=KEYED.with(|app|app.borrow().as_ref().unwrap().1.clone());items.set([4,2].into_iter().chain(100..2100).collect());}
#[wasm_bindgen]pub fn keyed_set(values:Vec<u32>){let items=KEYED.with(|app|app.borrow().as_ref().unwrap().1.clone());items.set(values);}
#[wasm_bindgen]pub fn keyed_unmount(){KEYED.with(|app|app.borrow_mut().take());}
thread_local!{static KEY_DROP_APP:RefCell<Option<(Scope,Signal<Vec<u32>>)>>=const{RefCell::new(None)};static KEY_DROP_LOG:RefCell<Vec<String>>=const{RefCell::new(Vec::new())};}
#[derive(Clone,Eq,PartialEq)]struct DropKey(u32);
impl Ord for DropKey{fn cmp(&self,other:&Self)->std::cmp::Ordering{other.0.cmp(&self.0)}}
impl PartialOrd for DropKey{fn partial_cmp(&self,other:&Self)->Option<std::cmp::Ordering>{Some(self.cmp(other))}}
fn log_key_lifecycle(kind:&str,id:u32){
 let connected=fusor::dom::document().unwrap().get_element_by_id(&format!("drop-row-{id}")).is_some();
 KEY_DROP_LOG.with(|log|log.borrow_mut().push(format!("{kind}:{id}:{connected}")));
}
impl Drop for DropKey{fn drop(&mut self){log_key_lifecycle("key",self.0);}}
#[wasm_bindgen]pub fn key_drop_mount()->Result<(),JsValue>{
 let root=fusor::dom::document()?.create_element("section")?;root.set_id("key-drop-fixture");
 let mut scope=Scope::new(root.clone());let items=signal(vec![1_u32,2,3,4]);let observed=items.clone();
 scope.keyed(&root,move||observed.get(),|id|DropKey(*id),|item|{
  let id=item.get_untracked();let root=fusor::dom::document()?.create_element("input")?;root.set_id(&format!("drop-row-{id}"));
  let mut row=Scope::new(root);row.retain(row.owner().on_cleanup(move||log_key_lifecycle("row",id)));Ok(row)
 })?;
 scope.attach(&fusor::dom::document()?.get_element_by_id("host").unwrap())?;KEY_DROP_APP.with(|app|app.replace(Some((scope,items))));KEY_DROP_LOG.with(|log|log.borrow_mut().clear());Ok(())
}
#[wasm_bindgen]pub fn key_drop_remove(){let items=KEY_DROP_APP.with(|app|app.borrow().as_ref().unwrap().1.clone());items.set(vec![4,2]);}
#[wasm_bindgen]pub fn key_drop_log()->String{KEY_DROP_LOG.with(|log|log.borrow().join(","))}
#[wasm_bindgen]pub fn key_drop_unmount(){KEY_DROP_APP.with(|app|app.borrow_mut().take());}
struct Example {value:Signal<u32>,text:Signal<String>}
#[wasm_bindgen(start)]pub fn start()->Result<(),JsValue>{mount()}
#[wasm_bindgen]pub fn mount()->Result<(),JsValue>{let mut scope=Example{value:signal(0),text:signal("hello".into())}.mount()?;scope.attach(&fusor::dom::document()?.get_element_by_id("host").unwrap())?;APP.with(|app|app.replace(Some(scope)));Ok(())}
#[wasm_bindgen]pub fn unmount(){APP.with(|app|app.borrow_mut().take());}
#[wasm_bindgen]pub fn descriptor(id:usize,bad:bool)->Result<(),JsValue>{
 static GOOD:&[ElementDescriptor]=&[ElementDescriptor{id:ElementId::new(0),tag:"div",children:ChildPolicy::Static}];
 static BAD:&[ElementDescriptor]=&[ElementDescriptor{id:ElementId::new(0),tag:"span",children:ChildPolicy::Static}];
 let descriptor=TemplateDescriptor{version:VERSION,component:ComponentId::new(id),kind:RootKind::Template,elements:if bad{BAD}else{GOOD},texts:&[],text_elements:&[]};
 let _=descriptor.mount()?;Ok(())}
</script>
<template id="fixture" rust:component="Example"><section id="card" data-value="{{ state.value.get() }}">prefix {{ state.value.get() }} middle {{ state.value.get()+1 }} <button id="increment" on:click="state.value.update(|v|*v+=1)">next</button><input id="edit" bind="state.text"><output id="echo">{{ state.text.get() }}</output><div><article><strong id="nested-a">{{ state.value.get()+10 }}</strong><span id="nested-b">{{ state.value.get()+20 }}</span><em><output id="nested-c">{{ state.value.get()+30 }}</output></em></article></div><cache-probe></cache-probe></section></template>
</body></html>`,
  );
  await exec(
    "cargo",
    ["build", "-p", "fusor-cli", "--locked", "--offline"],
    { cwd: root, maxBuffer: 8e6 },
  );
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: scratch });
  await exec(
    join(root, "target/debug/fusor" + executableSuffix),
    ["build", "--offline"],
    {
      cwd: scratch,
      env: {
        ...process.env,
        CARGO_TARGET_DIR: join(root, "target/template-cache-tests"),
      },
      timeout: 240000,
      maxBuffer: 8e6,
    },
  );
  server = createServer(async (req, res) => {
    try {
      const path = req.url === "/" ? "/index.html" : req.url;
      const data = await readFile(join(scratch, "dist", path));
      res.setHeader(
        "content-type",
        path.endsWith(".wasm")
          ? "application/wasm"
          : path.endsWith(".js")
            ? "text/javascript"
            : "text/html",
      );
      res.end(data);
    } catch {
      res.writeHead(404);
      res.end();
    }
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const refreshSource = await readFile(join(root, "crates/fusor-cli/src/dev/refresh.js"), "utf8");
  for (const engine of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(
    ",",
  )) {
    browser = await { chromium, firefox, webkit }[engine].launch(
      engine === "chromium" && process.env.PLAYWRIGHT_CHANNEL
        ? { channel: process.env.PLAYWRIGHT_CHANNEL }
        : {},
    );
    const page = await browser.newPage(),
      errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.waitForSelector("#card");
    const result = await page.evaluate(async () => {
      const boot = document.querySelector("script[type=module][src]");
      const app = await import(new URL("pkg/app.js", boot.src));
      let hits = 0,
        imports = 0;
      const equal = Node.prototype.isEqualNode,
        importNode = Document.prototype.importNode;
      Node.prototype.isEqualNode = function (other) {
        const result = equal.call(this, other);
        if (result) hits++;
        return result;
      };
      Document.prototype.importNode = function (...args) {
        if (this.defaultView !== null)
          throw Error("certificate import must be inert");
        imports++;
        return importNode.apply(this, args);
      };
      const check = () => {
        const card = document.querySelector("#card");
        if (!card.textContent.startsWith("prefix 0 middle 1"))
          throw Error("multiple text paths");
        if (card.getAttribute("data-value") !== "0")
          throw Error("typed integer attribute");
        card.querySelector("#increment").click();
        if (!card.textContent.startsWith("prefix 1 middle 2"))
          throw Error("later bound button path");
        if (card.getAttribute("data-value") !== "1")
          throw Error("typed integer attribute update");
        if (
          ["a", "b", "c"].some(
            (suffix, i) =>
              card.querySelector(`#nested-${suffix}`).textContent !==
              String((i + 1) * 10 + 1),
          )
        )
          throw Error("shared deep path prefixes resolved incorrectly");
        for (const id of ["echo", "nested-a", "nested-b", "nested-c"]) {
          const host = card.querySelector(`#${id}`);
          if (host.childNodes.length !== 1 || host.firstChild.nodeType !== 3) throw Error(`not a direct text binding: ${id}`);
        }
        const input = card.querySelector("#edit");
        input.value = "日本語😀<&";
        input.dispatchEvent(new Event("input"));
        if (card.querySelector("#echo").textContent !== "日本語😀<&")
          throw Error("later input/third text path");
      };
      try {
        check();
        let mounts = 1;
        for (let i = 0; i < 4; i++) {
          app.unmount();
          app.mount();
          mounts++;
          check();
        }
        if (constructed !== mounts)
          throw Error("snapshot ran a custom-element constructor");
        if (hits < 4) throw Error("fixture did not exercise actual cache hits");
        app.unmount();
        let template = document.querySelector("#fixture");
        const original = template.innerHTML;
        const version = template.getAttribute("data-fusor-version");
        template.content.querySelector("#card").firstChild.data = "updated ";
        app.mount();
        mounts++;
        if (
          !document
            .querySelector("#card")
            .textContent.startsWith("updated 0 middle 1")
        )
          throw Error("live static change ignored");
        app.unmount();
        template.innerHTML = original;
        for (const damage of [
          "namespace",
          "marker",
          "missing",
          "duplicate",
          "schema",
          "duplicate-root",
          "direct-marker",
          "direct-children",
          "direct-comment",
        ]) {
          let extra;
          const button = template.content.querySelector("button");
          if (damage === "direct-marker") template.content.querySelector("#nested-a").setAttribute("data-fusor-text", "01");
          if (damage === "direct-children") template.content.querySelector("#nested-a").append(document.createTextNode("a"), document.createTextNode("b"));
          if (damage === "direct-comment") template.content.querySelector("#nested-a").append(document.createComment("unowned"));
          if (damage === "namespace") {
            const svg = document.createElementNS(
              "http://www.w3.org/2000/svg",
              "button",
            );
            for (const a of button.attributes)
              svg.setAttribute(a.name, a.value);
            button.replaceWith(svg);
          }
          if (damage === "marker") {
            const w = document.createTreeWalker(
              template.content,
              NodeFilter.SHOW_COMMENT,
            );
            w.nextNode().data = "fusor:01";
          }
          if (damage === "missing") button.removeAttribute("data-fusor-node");
          if (damage === "duplicate") button.after(button.cloneNode(true));
          if (damage === "schema")
            template.setAttribute("data-fusor-version", "999");
          if (damage === "duplicate-root") {
            extra = template.cloneNode(true);
            template.after(extra);
          }
          let rejected = false;
          try {
            app.mount();
          } catch (e) {
            rejected = String(e).includes("template mismatch");
          }
          if (!rejected)
            throw Error("cached malformed template accepted: " + damage);
          if (document.querySelector("#card"))
            throw Error("failed mount published a root");
          extra?.remove();
          template.setAttribute("data-fusor-version", version);
          template.innerHTML = original;
        }
        const replacement = template.cloneNode(true);
        template.replaceWith(replacement);
        template = replacement;
        app.mount();
        mounts++;
        check();
        app.unmount();
        if (constructed !== mounts)
          throw Error("cache retained or constructed application state");
        const manual = document.createElement("template");
        manual.setAttribute("data-fusor-component", "9001");
        manual.setAttribute("data-fusor-version", version);
        manual.innerHTML = '<div data-fusor-node="0"></div>';
        document.body.append(manual);
        app.descriptor(9001, false);
        app.descriptor(9001, false);
        let bad = false;
        try {
          app.descriptor(9001, true);
        } catch (e) {
          bad = String(e).includes("must be <span>");
        }
        if (!bad) throw Error("descriptor slice semantics ignored");
        // More than32 distinct descriptors must evict old certificates.
        for (let id = 9002; id < 9035; id++) {
          manual.setAttribute("data-fusor-component", String(id));
          app.descriptor(id, false);
        }
        manual.setAttribute("data-fusor-component", "9001");
        const before = imports;
        app.descriptor(9001, false);
        if (imports !== before + 1)
          throw Error("cache did not evict old certificate");
        app.keyed_mount();
        const keyed = document.querySelector("#keyed-fixture");
        const nodes = new Map(
          [...keyed.children].map((node) => [Number(node.dataset.id), node]),
        );
        const focused = nodes.get(2);
        focused.value = "abcdef";
        focused.focus();
        focused.setSelectionRange(2, 4, "backward");
        const assertRows = (expected) => {
          if (
            [...keyed.children]
              .map((node) => Number(node.dataset.id))
              .join() !== expected.join()
          )
            throw Error("custom Ord changed render order or failure atomicity");
          for (const id of expected)
            if (keyed.querySelector(`[data-id="${id}"]`) !== nodes.get(id))
              throw Error("surviving keyed identity changed");
          if (
            document.activeElement !== focused ||
            focused.selectionStart !== 2 ||
            focused.selectionEnd !== 4 ||
            focused.selectionDirection !== "backward"
          )
            throw Error("keyed focus or selection changed");
        };
        app.keyed_change(0);
        assertRows([4, 2, 1, 3]);
        app.keyed_change(1);
        assertRows([4, 2, 1, 3]);
        if (app.keyed_drops().length)
          throw Error("duplicate keys disposed rows");
        app.keyed_change(2);
        assertRows([4, 2, 1, 3]);
        if ([...app.keyed_drops()].join() !== "5")
          throw Error("failed staging did not dispose only staged row");
        app.keyed_change(3);
        assertRows([4, 2]);
        if ([...app.keyed_drops()].join() !== "5,3,1")
          throw Error("removed rows lost Ord cleanup order");
        app.keyed_dense();
        if (
          keyed.children.length !== 2002 ||
          keyed.children[0] !== nodes.get(4) ||
          keyed.children[1] !== focused ||
          document.activeElement !== focused ||
          focused.selectionStart !== 2 ||
          focused.selectionEnd !== 4
        )
          throw Error("dense addition lost existing identities or selection");
        if (
          [...keyed.children]
            .slice(2)
            .some((node, index) => Number(node.dataset.id) !== 100 + index)
        )
          throw Error("dense custom-Ord addition changed render order");
        app.keyed_unmount();
        const drops = [...app.keyed_drops()];
        if (
          drops.length !== 2005 ||
          drops.slice(0, 3).join() !== "5,3,1" ||
          drops.slice(-2).join() !== "4,2" ||
          drops.slice(3, -2).some((id, index) => id !== 2099 - index)
        )
          throw Error("remaining keyed rows not disposed in Ord order");
        app.keyed_mount();
        const boundary = document.querySelector("#keyed-fixture");
        let previous = new Map();
        for (const keys of [
          [],
          [10],
          [10, 11],
          [10, 11, 12],
          [10, 11, 12, 13, 14],
          [14, 13, 12, 11, 10],
          [],
        ]) {
          app.keyed_set(new Uint32Array(keys));
          const current = new Map(
            [...boundary.children].map((node) => [
              Number(node.dataset.id),
              node,
            ]),
          );
          if ([...current.keys()].join() !== keys.join())
            throw Error("empty/single/density boundary order");
          for (const [key, node] of current)
            if (previous.has(key) && previous.get(key) !== node)
              throw Error("boundary lost row identity");
          previous = current;
        }
        app.keyed_unmount();
        // Map-owned keys must outlive native removal, then drop before their
        // row cleanup, in custom Ord order. Other key copies may drop later;
        // this deliberately does not pin the number of clones or comparisons.
        app.key_drop_mount();
        app.key_drop_remove();
        const keyDrops = app.key_drop_log().split(",").filter((entry) => {
          const id = entry.split(":")[1];
          return id === "1" || id === "3";
        });
        if (keyDrops.some((entry) => entry.endsWith(":true")))
          throw Error("removed key dropped before its row detached: " + keyDrops.join(","));
        if (keyDrops.slice(0, 4).join() !== "key:3:false,row:3:false,key:1:false,row:1:false")
          throw Error("removed key and row cleanup lost Ord lifecycle order: " + keyDrops.join(","));
        app.key_drop_unmount();
        return { hits, mounts, constructors: constructed, imports };
      } finally {
        Node.prototype.isEqualNode = equal;
        Document.prototype.importNode = importNode;
        app.unmount();
        app.keyed_unmount();
        app.key_drop_unmount();
      }
    });
    await page.evaluate(async source => {
      const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
      try {
        const { patchDocument } = await import(url);
        const parse = html => new DOMParser().parseFromString(html, 'text/html');
        const before = parse('<output data-fusor-node="0" data-fusor-text="1" class="before"></output>');
        const after = parse('<output data-fusor-node="0" data-fusor-text="1" class="after"></output>');
        const live = before.cloneNode(true), host = live.querySelector('output');
        const text = live.createTextNode('reactive value'); host.append(text);
        patchDocument(before, after, live);
        if (host.className !== 'after' || host.firstChild !== text || text.data !== 'reactive value') throw Error('refresh lost direct text identity/value');
        const changed = parse('<output data-fusor-node="0" data-fusor-text="1" class="third">static</output>');
        let rejected = false;
        try { patchDocument(after, changed, live); } catch { rejected = true; }
        if (!rejected || host.className !== 'after' || host.firstChild !== text) throw Error('refresh accepted a structural change or partially applied it');
      } finally { URL.revokeObjectURL(url); }
    }, refreshSource);
    assert.deepEqual(errors, []);
    assert.equal(result.mounts, result.constructors);
    console.log(
      `PASS ${engine}: ${JSON.stringify(result)}; shifted text paths, live mutation/replacement, malformed metadata/namespace, descriptor changes, inert snapshots and bounded eviction`,
    );
    await browser.close();
    browser = null;
  }
} finally {
  await browser?.close();
  server?.closeAllConnections();
  if (server) await new Promise((r) => server.close(r));
  await rm(scratch, { recursive: true, force: true });
}
