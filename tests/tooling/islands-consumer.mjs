// Compile the public island APIs from an independent Cargo workspace.
import assert from "node:assert/strict";
import { cp, mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
const exec = promisify(execFile),
  root = process.cwd(),
  scratch = await mkdtemp(join(tmpdir(), "fusor-islands-consumer-"));
const env = {
  ...process.env,
  CARGO_NET_OFFLINE: "true",
  CARGO_TARGET_DIR: resolve("target/islands-consumer"),
};
const run = (program, args) =>
  exec(program, args, {
    cwd: scratch,
    env,
    timeout: 240000,
    maxBuffer: 8 * 1024 * 1024,
  });
try {
  await cp(resolve("examples/islands"), scratch, {
    recursive: true,
    filter: (path) =>
      !["dist", "target", "node_modules"].includes(basename(path)),
  });
  const manifest = await readFile("Cargo.toml", "utf8");
  const shared = manifest.slice(
    manifest.indexOf("[workspace.package]"),
    manifest.indexOf("[workspace.lints.rust]"),
  );
  const independent =
    '[workspace]\nmembers = ["*"]\nresolver = "2"\n' +
    shared +
    manifest.slice(manifest.indexOf("[profile.release]"));
  await writeFile(
    join(scratch, "Cargo.toml"),
    independent.replaceAll(
      'path = "crates/',
      `path = "${root.replaceAll("\\", "/")}/crates/`,
    ),
  );
  await run("cargo", ["generate-lockfile", "--offline"]);
  const cli = resolve(
    "target/debug",
    process.platform === "win32" ? "fusor.exe" : "fusor",
  );
  await run(cli, ["build", "-p", "catalog-site", "--offline", "--locked"]);
  const html = await readFile(join(scratch, "site/dist/index.html"), "utf8");
  assert(
    html.replace(/<!--[\s\S]*?-->/g, "").includes("Product 9007199254740993"),
  );
  assert(html.includes('data-fusor-unit="designer"'));
  // Named inputs are checked by rustc at the native template boundary.
  const sourcePath = join(scratch, "site/web/index.html");
  const source = await readFile(sourcePath, "utf8");
  for (const [from, to, diagnostic] of [
    ['quantity="1"', '', /missing field `quantity`/],
    ['quantity="1"', 'unknown="1" quantity="1"', /has no field named `unknown`/],
    ['product_id="{{ 9_007_199_254_740_993 }}"', 'product_id="{{ false }}"', /mismatched types/],
  ]) {
    await writeFile(sourcePath, source.replace(from, to));
    await assert.rejects(run("cargo", ["check", "-p", "catalog-site", "--offline"]), error => {
      assert.match(error.stderr, diagnostic);
      return true;
    });
  }
  // ID omission uses native per-render instance allocation, even for repeated types.
  await writeFile(sourcePath, source.replace(' hydrate:id="cart-one"', '').replace(' hydrate:id="cart-two"', ''));
  await run(cli, ["build", "-p", "catalog-site", "--offline", "--locked"]);
  const automatic = await readFile(join(scratch, "site/dist/index.html"), "utf8");
  assert(automatic.includes('id="fusor-island-1"'));
  assert(automatic.includes('id="fusor-island-2"'));
  assert(automatic.includes('data-fusor-activate-target="designer"'));
  console.log(
    "PASS independent native/browser island consumer: public APIs, registration witness, props and production output",
  );
} finally {
  await rm(scratch, { recursive: true, force: true });
}
