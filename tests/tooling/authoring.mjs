// Public authoring contract in an independent Cargo package, including native
// editor services. Requires rust-analyzer (`rustup component add rust-analyzer`).
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";

const exec = promisify(execFile);
const root = process.cwd();
const scratch = await mkdtemp(join(tmpdir(), "fusor-authoring-"));
const app = join(scratch, "native-app");
const cli = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
const env = {
  ...process.env, RUSTUP_TOOLCHAIN: process.env.RUSTUP_TOOLCHAIN || "stable", CARGO_NET_OFFLINE: "true", CARGO_TARGET_DIR: join(root, "target/authoring-tests"),
};
const run = (args) => exec(cli, args, { cwd: app, env, timeout: 180_000, maxBuffer: 8 * 1024 * 1024 });
let analyzer;

async function editor() {
  analyzer = spawn(process.env.FUSOR_RUST_ANALYZER || "rust-analyzer", [], { cwd: app, env, stdio: ["pipe", "pipe", "pipe"] });
  const waiting = new Map();
  let buffer = Buffer.alloc(0), id = 0, logs = "";
  const settings = { cargo: { offline: true, buildScripts: { enable: true } }, procMacro: { enable: true }, checkOnSave: false };
  const send = (message) => {
    const body = JSON.stringify({ jsonrpc: "2.0", ...message });
    analyzer.stdin.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
  };
  analyzer.stderr.on("data", data => { logs += data; });
  analyzer.on("error", error => { for (const pending of waiting.values()) pending.reject(error); });
  analyzer.on("exit", code => { for (const pending of waiting.values()) pending.reject(new Error(`rust-analyzer exited ${code}: ${logs}`)); });
  analyzer.stdout.on("data", data => {
    buffer = Buffer.concat([buffer, data]);
    while (true) {
      const end = buffer.indexOf("\r\n\r\n");
      if (end < 0) return;
      const size = Number(buffer.subarray(0, end).toString().match(/Content-Length: (\d+)/i)[1]);
      if (buffer.length < end + 4 + size) return;
      const message = JSON.parse(buffer.subarray(end + 4, end + 4 + size));
      buffer = buffer.subarray(end + 4 + size);
      if (message.method && message.id !== undefined) {
        send({ id: message.id, result: message.method === "workspace/configuration" ? message.params.items.map(() => settings) : null });
      } else if (message.id !== undefined && waiting.has(message.id)) {
        const pending = waiting.get(message.id); waiting.delete(message.id);
        if (message.error) pending.reject(Object.assign(new Error(JSON.stringify(message.error)), { code: message.error.code }));
        else pending.resolve(message.result);
      }
    }
  });
  const requestOnce = (method, params) => new Promise((resolve, reject) => {
    const key = ++id;
    const timer = setTimeout(() => { waiting.delete(key); reject(new Error(`editor timeout: ${method}\n${logs}`)); }, 60_000);
    waiting.set(key, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
    send({ id: key, method, params });
  });
  const request = async (method, params) => {
    const deadline = Date.now() + 60_000;
    while (true) {
      try { return await requestOnce(method, params); }
      catch (error) {
        // Background Cargo/macro analysis can invalidate an in-flight query.
        if (![-32800, -32801, -32802].includes(error.code) || Date.now() >= deadline) throw error;
        await new Promise(resolve => setTimeout(resolve, 150));
      }
    }
  };
  const uri = pathToFileURL(join(app, "src/app.rs")).href;
  const source = await readFile(join(app, "src/app.rs"), "utf8");
  await request("initialize", { processId: process.pid, rootUri: pathToFileURL(app).href, capabilities: {}, initializationOptions: settings });
  send({ method: "initialized", params: {} });
  send({ method: "textDocument/didOpen", params: { textDocument: { uri, languageId: "rust", version: 1, text: source } } });
  const position = offset => ({ line: source.slice(0, offset).split("\n").length - 1, character: source.slice(0, offset).split("\n").at(-1).length });
  const at = position(source.indexOf("self.count") + 5);
  const deadline = Date.now() + 120_000;
  let hover;
  while (Date.now() < deadline) {
    hover = await request("textDocument/hover", { textDocument: { uri }, position: at });
    if (JSON.stringify(hover).includes("Signal")) break;
    await new Promise(resolve => setTimeout(resolve, 300));
  }
  assert.match(JSON.stringify(hover), /Signal/);
  let definition;
  while (Date.now() < deadline) {
    definition = await request("textDocument/definition", { textDocument: { uri }, position: at });
    if (JSON.stringify(definition)?.includes(uri)) break;
    await new Promise(resolve => setTimeout(resolve, 300));
  }
  assert.ok(JSON.stringify(definition).includes(uri), JSON.stringify(definition));
  let completion;
  while (Date.now() < deadline) {
    completion = await request("textDocument/completion", { textDocument: { uri }, position: at });
    if ((completion?.items || completion || []).some(item => item.label === "count")) break;
    await new Promise(resolve => setTimeout(resolve, 300));
  }
  assert.ok((completion?.items || completion || []).some(item => item.label === "count"), JSON.stringify(completion));
  // Resolve a method supplied by generated HTML impls through bindings!.
  const generatedAt = position(source.indexOf("Counter::try_mount_with") + 11);
  let generated;
  while (Date.now() < deadline) {
    generated = await request("textDocument/definition", { textDocument: { uri }, position: generatedAt });
    if (JSON.stringify(generated)?.includes("dom.rs")) break;
    await new Promise(resolve => setTimeout(resolve, 300));
  }
  assert.match(JSON.stringify(generated), /dom.rs/, `generated Component method did not resolve\n${logs}`);
  await request("shutdown", null);
  send({ method: "exit", params: null });
  console.log("PASS: rust-analyzer completion, authored definitions, private-field types and generated Component methods resolve");
}

try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--locked", "--offline"], { cwd: root, timeout: 180_000 });
  await exec(cli, ["new", app, "--framework-path", root], { cwd: scratch, env });
  const rustPath = join(app, "src/app.rs"), htmlPath = join(app, "web/index.html");
  const rust = `//! Native module documentation.\n#![allow(dead_code)]\nmod nested;\n#[path = "support.rs"]\nmod support;\nconst LABEL: &str = include_str!("label.txt");\n${await readFile(rustPath, "utf8")}\nimpl App {\n    fn read(&self) -> i32 { self.count.get() + nested::value() + support::value() }\n    fn manual() -> Result<Scope, fusor::dom::JsValue> { Counter::try_mount_with(|owner| Counter::from_inputs(crate::counter::CounterInputs { count: signal(0) }, owner).map_err(fusor::dom::IntoMountError::into_mount_error)) }\n}\n`;
  await mkdir(join(app, "src/app"));
  await writeFile(join(app, "src/app/nested.rs"), "pub fn value() -> i32 { 1 }\n");
  await writeFile(join(app, "src/support.rs"), "pub fn value() -> i32 { 2 }\n");
  await writeFile(join(app, "src/label.txt"), "Native relative include\n");
  await writeFile(rustPath, rust);
  await run(["check", "--offline"]);
  await exec("cargo", ["fmt"], { cwd: app, env });
  let formatted = await readFile(rustPath, "utf8");
  assert.notEqual(formatted, rust);
  await run(["check", "--offline", "--locked"]);
  let html = await readFile(htmlPath, "utf8");
  console.log("PASS: private fields, inner docs/attributes, nested mod, #[path], relative include_str and cargo fmt remain native");

  await writeFile(rustPath, formatted.replace("count: signal(0)", 'count: signal("wrong")'));
  await assert.rejects(run(["check", "--offline", "--locked"]), error => {
    assert.match(error.stderr, /src[\/]app\.rs:\d+/);
    assert.match(error.stderr, /mismatched types/);
    return true;
  });
  await writeFile(rustPath, formatted);
  for (const [before, after] of [["state.count.get()", "state.missing.get()"], ["App::new()", "App::missing()"]]) {
    await writeFile(htmlPath, html.replace(before, after));
    await assert.rejects(run(["check", "--offline", "--locked"]), /HTML binding at web[\/]index\.html:/);
  }
  await writeFile(htmlPath, html);
  await editor(); // Exercise template! through native rust-analyzer services.
  // Keep the original reverse-registration compatibility checks as well.
  formatted = formatted.replace('fusor::template!("web/index.html");', 'fusor::bindings!(app);');
  html = html.replace('</head>', '<script type="text/rust" src="../src/app.rs" rust:module="crate::app"></script>\n</head>');
  await writeFile(htmlPath, html);
  await writeFile(join(app, "src/lib.rs"), 'mod app;\nmod counter;\ninclude!(env!("FUSOR_MODULE"));\n');
  await writeFile(rustPath, formatted.replace("fusor::bindings!(app);", ""));
  await assert.rejects(run(["check", "--offline", "--locked"]), /HTML binding at web[\/]index\.html:.*|__FUSOR_BINDINGS_APP/s);
  await writeFile(rustPath, formatted);
  await writeFile(join(app, "src/wrong.rs"), formatted);
  await writeFile(htmlPath, html.replace("../src/app.rs", "../src/wrong.rs"));
  await assert.rejects(run(["check", "--offline", "--locked"]), /external Rust source mismatch/);
  await writeFile(htmlPath, html.replace('rust:module="crate::app"', 'rust:module="crate::counter"'));
  await assert.rejects(run(["check", "--offline", "--locked"]), /__FUSOR_BINDINGS_APP/);
  await writeFile(htmlPath, html);
  await run(["check", "--offline", "--locked"]);
  console.log("PASS: Rust body errors keep .rs locations; HTML bindings/startup and missing/wrong registrations fail through rustc");
  await writeFile(rustPath, `${formatted}\n#[wasm_bindgen::prelude::wasm_bindgen(start)]\npub fn another_start() {}\n`);
  await assert.rejects(run(["build", "--debug", "--offline", "--locked"]), error => {
    assert.match(error.stderr, /cannot specify two `start` functions/);
    return true;
  });
  await writeFile(rustPath, formatted);
  await run(["check", "--offline", "--locked"]);
  console.log("PASS: native Wasm tooling rejects conflicting manual and generated start functions");
} finally {
  analyzer?.kill();
  await rm(scratch, { recursive: true, force: true });
}
