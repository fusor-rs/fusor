import { root, suiteEnv as env, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory } from "../../scripts/build.mjs";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { request } from "node:https";
import { join } from "node:path";
import { chromium } from "@playwright/test";

const scratch = await realpath(await temporaryDirectory("fusor-https-"));
const app = join(scratch, "app");
const tls = join(app, "tls");
const manifest = join(app, "Cargo.toml");
const executable = join(root, "target/debug", process.platform === "win32" ? "fusor.exe" : "fusor");
const offline = { ...env, CARGO_NET_OFFLINE: "true" };
const name = "dev.example.local";
let server, browser, pendingEvents, rootCertificate;

const backend = createServer((incoming, response) => {
  if (incoming.url === "/api/events") {
    pendingEvents = response;
    response.writeHead(200, { "content-type": "text/event-stream" });
    response.write("data: first\n\n");
    return;
  }
  response.writeHead(200, { "x-host": incoming.headers.host, "x-cookie": incoming.headers.cookie || "" });
  response.end();
});

const openssl = args => exec("openssl", args, { cwd: tls, env });
async function certificates() {
  await mkdir(tls);
  await openssl(["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes", "-days", "2",
    "-keyout", "root.key", "-out", "root.pem", "-subj", "/CN=fusor test root",
    "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign"]);
  await openssl(["req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:P-256", "-nodes",
    "-keyout", "key.pem", "-out", "leaf.csr", "-subj", `/CN=${name}`]);
  await writeFile(join(tls, "leaf.ext"), `subjectAltName=DNS:${name}\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n`);
  await openssl(["x509", "-req", "-in", "leaf.csr", "-CA", "root.pem", "-CAkey", "root.key", "-CAcreateserial",
    "-days", "2", "-out", "cert.pem", "-extfile", "leaf.ext"]);
  await openssl(["genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256", "-out", "other.pem"]);
}

// Verifies the certificate against the test root under the configured name.
function secureRequest(port, path, headers = {}) {
  return new Promise((resolve, reject) => {
    const outgoing = request({
      host: name, port, path, headers, ca: rootCertificate,
      lookup: (_host, options, callback) => options.all
        ? callback(null, [{ address: "127.0.0.1", family: 4 }])
        : callback(null, "127.0.0.1", 4),
    }, resolve);
    outgoing.on("error", reject);
    outgoing.end();
  });
}

async function body(response) {
  const chunks = [];
  for await (const chunk of response) chunks.push(chunk);
  return Buffer.concat(chunks).toString();
}

async function rejectsBeforeBuilding(args, file, message) {
  const failed = await exec(executable, ["dev", "--manifest-path", manifest, "--offline", ...args], { cwd: app, env: offline, timeout: 60_000 })
    .then(() => assert.fail(`fusor dev ${args.join(" ")} started`), error => error);
  assert.equal(failed.code, 3, failed.message);
  assert.match(failed.stderr, message);
  assert.ok(failed.stderr.includes(join(tls, file)), `the error names ${file}: ${failed.stderr}`);
  assert.equal(existsSync(join(app, ".fusor/dev")), false, "nothing was built");
}

async function proxyOverHttps(port) {
  const echoed = await secureRequest(port, "/api/echo", { cookie: "session=kept" });
  assert.equal(echoed.headers["x-host"], `127.0.0.1:${backend.address().port}`);
  assert.equal(echoed.headers["x-cookie"], "session=kept");
  await body(echoed);
  const events = await secureRequest(port, "/api/events");
  assert.equal(events.headers["content-type"], "text/event-stream");
  const first = await new Promise(resolve => events.once("data", resolve));
  assert.equal(first.toString(), "data: first\n\n", "an event arrives while the backend is still sending");
  pendingEvents.end("data: second\n\n");
  assert.equal(await body(events), "data: second\n\n");
}

try {
  await exec("cargo", ["build", "-p", "fusor-cli", "--offline", "--locked"], { cwd: root, env, timeout: 300_000 });
  await exec(executable, ["new", app, "--framework-path", root, "--skip-install", "--yes"], { cwd: scratch, env, timeout: 120_000 });
  await certificates();
  rootCertificate = await readFile(join(tls, "root.pem"));
  await new Promise(resolve => backend.listen(0, "127.0.0.1", resolve));
  const generated = await readFile(manifest, "utf8");
  await writeFile(manifest, generated.replace("[package.metadata.fusor]", `[package.metadata.fusor]
dev.name = "${name}"
dev.tls-cert = "tls/cert.pem"
dev.tls-key = "tls/key.pem"
dev.proxy."/api" = "http://127.0.0.1:${backend.address().port}"`));
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: app, env });

  await rejectsBeforeBuilding(["--tls-cert", join(tls, "missing.pem"), "--tls-key", join(tls, "key.pem")], "missing.pem", /cannot read a certificate/);
  await rejectsBeforeBuilding(["--tls-cert", join(tls, "cert.pem"), "--tls-key", join(tls, "other.pem")], "other.pem", /does not match the certificate/);
  await rejectsBeforeBuilding(["--tls-cert", join(tls, "cert.pem"), "--tls-key", join(tls, "cert.pem")], "cert.pem", /cannot read a private key/);

  const port = await reservePort();
  server = startProcess(executable, ["dev", "--manifest-path", manifest, "--offline", "--host", "127.0.0.1", "--port", String(port)], { cwd: app, env: offline });
  await waitUntil(() => server.output.includes("Ctrl+C to stop."), "development server startup", { timeout: 300_000, process: server });
  const url = `https://${name}:${port}/`;
  assert.ok(server.stdout.includes(url), `fusor dev prints ${url}: ${server.output}`);
  const page = await secureRequest(port, "/");
  assert.equal(page.statusCode, 200);
  assert.match(await body(page), /Rust, inside HTML\./);
  await proxyOverHttps(port);

  browser = await chromium.launch({
    channel: process.env.PLAYWRIGHT_CHANNEL || undefined,
    args: [`--host-resolver-rules=MAP ${name} 127.0.0.1`],
  });
  const tab = await (await browser.newContext({ ignoreHTTPSErrors: true })).newPage();
  const errors = [];
  tab.on("pageerror", error => errors.push(error.message));
  await tab.goto(url);
  await tab.waitForFunction(() => document.querySelector("output")?.textContent === "0");
  assert.equal(await tab.evaluate(() => window.isSecureContext), true);
  await tab.locator("button").first().click();
  await tab.waitForFunction(() => document.querySelector("output")?.textContent === "1");
  const index = join(app, "web/index.html");
  await writeFile(index, (await readFile(index, "utf8")).replace("Rust, inside HTML.", "Edited over HTTPS"));
  await tab.waitForFunction(() => document.querySelector("h1")?.textContent === "Edited over HTTPS", null, { timeout: 120_000 });
  assert.equal(await tab.locator("output").textContent(), "1", "the edit updates the page in place");
  assert.deepEqual(errors, []);
  console.log("Development over HTTPS: certificate checks before building, the configured name, verified TLS, proxy streaming and in-place refresh, passed");
} finally {
  await browser?.close();
  await stopProcess(server);
  backend.close();
  await rm(scratch, { recursive: true, force: true });
}
