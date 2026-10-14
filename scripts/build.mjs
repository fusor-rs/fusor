import { spawn } from "node:child_process";
import { mkdtemp, cp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { createServer } from "node:net";
import { join, basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";
export const root = fileURLToPath(new URL("..", import.meta.url));
export const env = { ...process.env };
// Suites build into the workspace target directory so separate projects reuse
// its dependency builds. Cargo keeps another copy of every crate for each debug
// setting, and projects outside the checkout don't read .cargo/config.toml.
// Incremental caches only help crates being edited between builds.
export const suiteEnv = {
  ...env,
  CARGO_TARGET_DIR: join(root, "target"),
  CARGO_PROFILE_DEV_DEBUG: "line-tables-only",
  CARGO_INCREMENTAL: "0",
};
// Cargo names a crate's builds by its path relative to the workspace root, which
// a copy shares with its original, and a copy's files are newer. Copies build
// apart so an unchanged original never reuses the build of an edited copy.
export const copyEnv = { ...suiteEnv, CARGO_TARGET_DIR: join(root, "target/copies") };

const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

// Each caller owns its process and logs. POSIX process groups include Cargo's
// compiler/server children, so stopping a suite cannot leave them behind.
export function startProcess(program, args, { cwd = root, env: environment = env, inherit = false } = {}) {
  const child = spawn(program, args, {
    cwd, env: environment, detached: process.platform !== "win32",
    stdio: ["ignore", "pipe", "pipe"],
  });
  const owned = { child, output: "", stdout: "", stderr: "", error: null, exited: false };
  for (const [stream, destination] of [["stdout", process.stdout], ["stderr", process.stderr]]) {
    child[stream].on("data", data => {
      owned[stream] += data; owned.output += data;
      if (inherit) destination.write(data);
    });
  }
  owned.done = new Promise(resolve => {
    child.on("error", error => { owned.error = error; });
    child.on("close", (code, signal) => { owned.exited = true; resolve({ code, signal }); });
  });
  return owned;
}

export async function stopProcess(owned) {
  if (!owned || owned.exited) return;
  if (process.platform === "win32") {
    const killer = spawn("taskkill", ["/pid", String(owned.child.pid), "/t", "/f"], { stdio: "ignore" });
    await new Promise(resolve => { killer.on("error", resolve); killer.on("close", resolve); });
  } else {
    const kill = signal => {
      try { process.kill(-owned.child.pid, signal); }
      catch (error) { if (error.code !== "ESRCH") throw error; }
    };
    kill("SIGTERM");
    // Escalate only this owned process group when it ignores graceful shutdown.
    const timer = setTimeout(() => kill("SIGKILL"), 5000);
    try { await owned.done; } finally { clearTimeout(timer); }
  }
  await owned.done;
}

export async function exec(program, args, { timeout = 0, maxBuffer = 8 * 1024 * 1024, ...options } = {}) {
  const owned = startProcess(program, args, options);
  let timedOut = false, overflow = false;
  const timer = timeout && setTimeout(() => { timedOut = true; void stopProcess(owned); }, timeout);
  const limit = setInterval(() => {
    if (owned.stdout.length + owned.stderr.length > maxBuffer) { overflow = true; void stopProcess(owned); }
  }, 100);
  try {
    const { code, signal } = await owned.done;
    if (owned.error || code !== 0 || timedOut || overflow) {
      throw Object.assign(new Error(`${program} ${args.join(" ")}: ${owned.error?.message || (timedOut ? "timed out" : overflow ? "output limit exceeded" : `exited ${signal || code}`)}\n${owned.output}`),
        { stdout: owned.stdout, stderr: owned.stderr, code });
    }
    return { stdout: owned.stdout, stderr: owned.stderr };
  } finally { clearTimeout(timer); clearInterval(limit); }
}

export async function command(program, args, { capture = false, ...options } = {}) {
  const result = await exec(program, args, { inherit: !capture, ...options });
  return result.stdout;
}

export async function waitFor(predicate, description, { timeout, interval = 50, process: owned } = {}) {
  if (!(timeout > 0)) throw new Error("waitFor needs an explicit positive timeout");
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (owned && (owned.error || owned.exited || owned.child.exitCode !== null || owned.child.signalCode !== null))
      throw new Error(`process exited while waiting for ${description}\n${owned.output}\n${owned.error || ""}`);
    if (await predicate()) return;
    await delay(interval);
  }
  throw new Error(`timed out waiting for ${description}\n${owned?.output || ""}`);
}

export async function reservePort() {
  const server = createServer();
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const port = server.address().port;
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  return port;
}

export function temporaryDirectory(prefix) { return mkdtemp(join(tmpdir(), prefix)); }
export function copyProject(source, destination, excluded = ["target", "dist", "node_modules", ".fusor"]) {
  return cp(source, destination, { recursive: true, filter: path => !excluded.includes(basename(path)) });
}

// Give copied examples their own workspace while retaining every inherited
// dependency and package field. Keep versions in the repository manifest so
// adding an inherited dependency cannot silently break a consumer fixture.
export async function independentManifest(manifest) {
  const workspace = await readFile(join(root, "Cargo.toml"), "utf8");
  const tables = workspace.split(/(?=^\[)/m).filter(table =>
    /^\[workspace\.(package|dependencies)\]/.test(table));
  if (tables.length !== 2) throw new Error("Expected workspace package and dependency tables");
  const shared = tables.join("").replace(/\bpath = "([^"]+)"/g, (_, path) =>
    `path = ${JSON.stringify(resolve(root, path).replaceAll("\\", "/"))}`);
  return `${manifest}\n[workspace]\nresolver = "2"\n\n${shared}`;
}

export function buildPackage(name) {
  return command("cargo", [
    "run",
    "-p",
    "fusor-cli",
    "--bin",
    "fusor",
    "--locked",
    "--",
    "build",
    "-p",
    name,
    "--locked",
  ]);
}
