// Compile a separate application through public APIs. Negative cases are real
// rustc errors; their source maps must identify the correct HTML declaration.
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { cp, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { promisify } from "node:util";

const exec = promisify(execFile);
const root = process.cwd();
const scratch = await mkdtemp(join(tmpdir(), "fusor-consumer-"));
const fixture = join(root, "tests/fixtures/consumer");
const cargo = process.env.CARGO || "cargo";

async function compile() {
  const args = ["check", "--manifest-path", join(scratch, "Cargo.toml"), "--locked", "--offline", "--message-format=json"];
  let success = true;
  let result;
  try {
    result = await exec(cargo, args, {
      env: { ...process.env, CARGO_TARGET_DIR: join(root, "target/consumer-tests") },
      timeout: 120_000,
      maxBuffer: 8 * 1024 * 1024,
    });
  } catch (error) {
    if (error.killed || typeof error.code !== "number") throw error;
    result = error;
    success = false;
  }
  const messages = result.stdout.trim().split("\n").filter(Boolean).map((line) => JSON.parse(line));
  const variables = Object.fromEntries(messages.filter((message) => message.reason === "build-script-executed")
    .flatMap((message) => message.env));
  const errors = messages.filter((message) => message.reason === "compiler-message" && message.message.level === "error")
    .map((message) => message.message);
  return { success, variables, errors, stderr: result.stderr };
}

// Diagnostic spans can be nested in macro expansion backtraces.
function spans(value) {
  if (!value || typeof value !== "object") return [];
  const result = value.file_name && value.line_start ? [value] : [];
  return result.concat(Object.values(value).flatMap(spans));
}

try {
  await cp(fixture, scratch, {
    recursive: true,
    filter: (path) => basename(path) !== "target",
  });
  const manifestPath = join(scratch, "Cargo.toml");
  const manifest = await readFile(manifestPath, "utf8");
  await writeFile(manifestPath, manifest.replace('features = ["dom"]', 'features = ["dom", "javascript"]').replace(/"\.\.\/\.\.\/\.\.\/crates\/([^"]+)"/g,
    (_, crate) => JSON.stringify(join(root, "crates", crate))));
  const htmlPath = join(scratch, "web/index.html");
  const good = await readFile(htmlPath, "utf8");
  const sourcePath = join(scratch, "src/lib.rs");
  await writeFile(sourcePath, `#![deny(unused_variables)]\n${await readFile(sourcePath, "utf8")}`);
  const valid = await compile();
  assert.equal(valid.success, true, valid.stderr + JSON.stringify(valid.errors));
  const html = await readFile(valid.variables.FUSOR_HTML_OUTPUT, "utf8");
  assert.ok(!html.includes('type="text/rust"') && !html.includes("rust:component"));
  console.log("PASS: a separate Cargo application compiles every supported binding through public APIs");

  for (const [before, after] of [
    ['class:active="state.enabled.get()"', 'class:active="state.name.get() == \'dark\'"'],
    ['{{ selected.get().name }}', "{{ 'dark' }}"],
  ]) {
    assert.ok(good.includes(before));
    const broken = good.replace(before, after);
    const prefix = broken.slice(0, broken.indexOf("'dark'", broken.indexOf(after)));
    const line = prefix.split("\n").length;
    const column = [...prefix.split("\n").at(-1)].length + 1;
    await writeFile(htmlPath, broken);
    const result = await compile();
    assert.equal(result.success, false, after);
    assert.ok(result.stderr.includes(`index.html:${line}:${column}: Rust strings use double quotes`), result.stderr);
    assert.ok(result.stderr.includes("single-quoted character literals must contain one character"), result.stderr);
    console.log(`PASS: single-quoted strings point to HTML ${line}:${column} with Rust quote guidance`);
  }

  for (const [before, after, errorCode] of [
    ['key="{{ |item| item.id }}"', 'key="{{ |item| item.missing_id }}"', "E0609"],
    ['<Row item="{{ item.clone() }}">', '<Row nonexistent="{{ item.clone() }}">', "E0560"],
    ['{{ selected.get().name }}', '{{ selected.get().missing_name }}', "E0609"],
    ['{{ selected.get().name }}', '{{ { let unused_binding = 1; selected.get().name } }}', "unused_variables"],
    ['<input bind="state.name">', '<input bind="state.items">', "E0277"],
    ['disabled="{{ state.count.get() == 0 }}"', 'disabled="{{ state.count.get() }}"', "E0308"],
  ]) {
    assert.ok(good.includes(before));
    const broken = good.replace(before, after);
    const expectedLine = broken.slice(0, broken.indexOf(after)).split("\n").length;
    await writeFile(htmlPath, broken);
    const result = await compile();
    assert.equal(result.success, false, after);
    const diagnostic = result.errors.find((error) => error.code?.code === errorCode);
    assert.ok(diagnostic, result.stderr + JSON.stringify(result.errors));
    const lines = (await readFile(result.variables.FUSOR_BINDING_MAP, "utf8")).trimEnd().split("\n");
    assert.equal(lines.shift(), "fusor-source-map-v1");
    const entries = lines.map((line) => line.split("\t").map(Number));
    const generated = result.variables.FUSOR_RUST_SOURCE;
    const origins = spans(diagnostic).filter((span) => span.file_name === generated)
      .flatMap((span) => entries.filter(([start, end]) => start <= span.line_start && span.line_start < end));
    assert.ok(origins.some((entry) => entry[2] === expectedLine), JSON.stringify({ after, expectedLine, origins, diagnostic }));
    console.log(`PASS: ${after} is rejected by rustc and maps to HTML line ${expectedLine}`);
  }

  const nativeMismatch = good.replace('scope.on(&element, "click", |_| {})?;',
    'scope.checked(&element, || true)?;');
  assert.notEqual(nativeMismatch, good);
  await writeFile(htmlPath, nativeMismatch);
  const result = await compile();
  assert.equal(result.success, false);
  assert.ok(result.errors.some((error) => error.code?.code === "E0277" && error.message.includes("InputTarget")),
    JSON.stringify(result.errors));
  console.log("PASS: a generic Element cannot satisfy the native InputTarget contract");
  await writeFile(htmlPath, good);
  const rustSource = await readFile(sourcePath, 'utf8');
  const exposed = '\n#[derive(fusor::JsInputs)]\n#[js_inputs(crate = ::fusor)]\nstruct Exposed { #[js] value: fusor::Signal<f64> }\n';
  await writeFile(sourcePath, rustSource + exposed);
  const derived = await compile();
  assert.equal(derived.success, true, derived.stderr + JSON.stringify(derived.errors));
  for (const [before, after, message] of [
    ['#[js_inputs(crate = ::fusor)]', '#[js_inputs()]', 'expected #[js_inputs'],
    ['#[js_inputs(crate = ::fusor)]', '#[js_inputs(crate = ::fusor, crate = ::fusor)]', 'duplicate crate path'],
    ['#[js_inputs(crate = ::fusor)]', '#[js_inputs(crate = ::fusor)] #[js_inputs(crate = ::fusor)]', 'duplicate #[js_inputs]'],
    ['#[js] value:', '#[js_inputs(crate = ::fusor)] #[js] value:', 'belongs on the struct'],
  ]) {
    await writeFile(sourcePath, rustSource + exposed.replace(before, after));
    const rejected = await compile();
    assert.equal(rejected.success, false, after);
    assert.ok(rejected.errors.some(error => error.message.includes(message)), rejected.stderr + JSON.stringify(rejected.errors));
  }
  console.log('PASS: JsInputs overrides compile with one crate path and reject empty, duplicate, and misplaced attributes.');
} finally {
  await rm(scratch, { recursive: true, force: true });
}
