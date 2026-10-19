// Static delivery: the native renderer writes every page, the CLI links the
// stylesheet into each one, and nothing runs in the browser.
import { root, suiteEnv as env, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory, copyProject, independentManifest } from "../../scripts/build.mjs";
import assert from "node:assert/strict";
import { readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";

const scratch = await realpath(await temporaryDirectory("fusor-static-pages-"));
const app = join(scratch, "static-pages");
const manifest = join(app, "Cargo.toml");
const executable = join(root, "target/debug", process.platform === "win32" ? "fusor.exe" : "fusor");
const offline = { ...env, CARGO_NET_OFFLINE: "true" };
const cli = (args, options = {}) => exec(executable, args, { cwd: app, env: offline, timeout: 300_000, ...options });
const documents = ["index.html", "notes/html-first/index.html", "notes/nothing-to-load/index.html"];
let server;

// Every published file under a directory, relative to it.
async function files(directory, prefix = "") {
  const found = [];
  for (const entry of await readdir(join(directory, prefix), { withFileTypes: true })) {
    const path = prefix ? `${prefix}/${entry.name}` : entry.name;
    found.push(...(entry.isDirectory() ? await files(directory, path) : [path]));
  }
  return found.sort();
}

async function serve(args, ready) {
  const port = await reservePort();
  server = startProcess(executable, [...args, "--port", String(port)], { cwd: app, env: offline });
  await waitUntil(() => server.output.includes(ready), `${args[0]} startup`, { timeout: 300_000, process: server });
  return (path, options) => fetch(`http://127.0.0.1:${port}${path}`, { redirect: "manual", ...options });
}

try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--offline", "--locked"], { cwd: root, env, timeout: 300_000 });
  await copyProject(join(root, "examples/static-pages"), app);
  await writeFile(manifest, await independentManifest(await readFile(manifest, "utf8")));
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: app, env });
  await exec(executable, ["install", "--manifest-path", manifest], { cwd: app, env, timeout: 600_000 });

  // A release build publishes the renderer's files, one stylesheet and no runtime.
  await cli(["build", "--manifest-path", manifest]);
  const dist = join(app, "dist");
  const [generation] = await readdir(join(dist, "__fusor"));
  const stylesheet = (await readdir(join(dist, "__fusor", generation))).filter(name => name.endsWith(".css"));
  assert.equal(stylesheet.length, 1, "one compiled stylesheet");
  assert.deepEqual(
    (await files(dist)).filter(path => !path.startsWith("__fusor/")),
    [".fusor-output.json", ...documents, "robots.txt"].sort(),
    "the renderer's files and nothing of a browser application",
  );
  assert.deepEqual(await readdir(join(dist, "__fusor", generation)), stylesheet, "no scripts or Wasm");
  assert.equal(JSON.parse(await readFile(join(dist, ".fusor-output.json"), "utf8")).delivery, "static");
  const titles = new Set();
  for (const document of documents) {
    const html = await readFile(join(dist, document), "utf8");
    assert.match(html, new RegExp(`<head><link rel="stylesheet" href="/__fusor/${generation}/${stylesheet[0]}">`), document);
    assert.doesNotMatch(html, /<script/, `${document} runs nothing`);
    titles.add(html.match(/<title[^>]*>([^<]*)<\/title>/)[1]);
  }
  assert.deepEqual([...titles], ["Field notes", "HTML first · Field notes", "Nothing to load · Field notes"]);
  assert.equal(await readFile(join(dist, "robots.txt"), "utf8"), "User-agent: *\nAllow: /\n");
  const css = await readFile(join(dist, "__fusor", generation, stylesheet[0]), "utf8");
  assert.match(css, /\.text-orange-700/, "classes from the templates are compiled");

  // Preview serves each page at its directory's URL, as static hosts do.
  let request = await serve(["preview", join(app, "dist")], "Ready");
  const note = await request("/notes/html-first/");
  assert.equal(note.status, 200);
  assert.match(await note.text(), /<title data-fusor-text="0">HTML first · Field notes<\/title>/);
  const bare = await request("/notes/html-first");
  assert.equal(bare.status, 308);
  assert.equal(bare.headers.get("location"), "/notes/html-first/");
  assert.equal((await request("/notes/missing/", { headers: { accept: "text/html" } })).status, 404);
  const linked = await request(`/__fusor/${generation}/${stylesheet[0]}`);
  assert.equal(linked.status, 200);
  assert.match(linked.headers.get("content-type"), /^text\/css/);
  await stopProcess(server);

  // Development links the live-reload module into every page and rebuilds on edits.
  request = await serve(["dev", "--manifest-path", manifest, "--offline"], "Ctrl+C to stop.");
  const developed = await (await request("/notes/nothing-to-load/")).text();
  const boot = developed.match(/<script type="module" src="(\/__fusor\/[^"]+\/boot\.js)"><\/script>/);
  assert.ok(boot, "each development page starts the live-reload client");
  assert.match(await (await request(boot[1])).text(), /^import \{ watch \} from '\.\/refresh\.js';/);
  const template = join(app, "web/index.html");
  await writeFile(template, (await readFile(template, "utf8")).replace("<main>", '<main data-edited="yes">'));
  await waitUntil(
    async () => (await (await request("/notes/nothing-to-load/")).text()).includes('data-edited="yes"'),
    "rebuilt pages after an edit",
    { timeout: 300_000, process: server },
  );
  console.log("static pages: rendered documents, linked stylesheet, no runtime, directory URLs and development rebuilds passed");
} finally {
  if (server) await stopProcess(server);
  await rm(scratch, { recursive: true, force: true });
}
