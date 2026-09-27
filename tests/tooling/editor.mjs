import { root, env as buildEnv, exec, startProcess, stopProcess, waitFor as waitUntil, reservePort, temporaryDirectory, copyProject, independentManifest } from "../../scripts/build.mjs";
// Independent Cargo consumer + controlled HTTP backend + real browser controls.
import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile, writeFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { join } from "node:path";
import { chromium, firefox, webkit, expect } from "@playwright/test";
import { projectBackend } from "../../examples/editor/backend.mjs";

const scratch = await temporaryDirectory("fusor-editor-");
const executable = join(root, "target/debug", `fusor${process.platform === "win32" ? ".exe" : ""}`);
const env = { ...buildEnv, CARGO_NET_OFFLINE:"true", CARGO_TARGET_DIR:join(root,"target/editor-tests"), };
const cli = args => exec(executable, args, { cwd:scratch, env, timeout:180_000, maxBuffer:16 * 1024 * 1024 });
const backend = projectBackend({ controlled:true });
let server, proxy, browser;
const waitFor = (predicate, description) => waitUntil(predicate, description, { timeout: 20_000, interval: 20, process: server });
try {
  await exec("cargo", ["build","-p","fusor-cli","--locked","--offline"], { timeout:180_000 });
  await copyProject(join(root,"examples/editor"), scratch);
  const manifest = await independentManifest(await readFile(join(scratch,"Cargo.toml"),"utf8"));
  await writeFile(join(scratch,"Cargo.toml"), manifest);
  await exec("cargo", ["generate-lockfile", "--offline"], { cwd: scratch });
  await cli(["check","--offline","--features","browser-tests"]);
  const viewsPath = join(scratch,"web/views.html"), views = await readFile(viewsPath,"utf8");
  for (const [before, after, expected] of [
    ['bind="state.session.title"','bind="state.name"',/cannot be edited as text/],
    ['type="text" bind=','type="file" bind=',/bind cannot set a file input/],
    ['bind="state.session.title"','bind="state.session.title" value="{{ 42 }}"',/bind owns this control's value/],
    ['bind="state.session.body"></textarea>','bind="state.session.body">initial</textarea>',/leave its contents empty/],
  ]) {
    assert(views.includes(before)); const broken = views.replace(before,after);
    const line = broken.slice(0,broken.indexOf(after)).split("\n").length;
    await writeFile(viewsPath, broken);
    await assert.rejects(cli(["check","--offline","--locked"]), error => {
      assert.match(error.stderr,expected); assert(error.stderr.replaceAll("\\","/").includes(`web/views.html:${line}:`),error.stderr); return true;
    });
    await writeFile(viewsPath,views);
  }
  // Inline Rust uses exactly the same typed lowering.
  const originalLib = await readFile(join(scratch,"src/lib.rs"),"utf8");
  const fixtureManifest = manifest.replace('entry = "web/index.html"','entry = "web/fixture.html"').replace('views = "web/views.html"',"");
  await writeFile(join(scratch,"Cargo.toml"), fixtureManifest);
  await writeFile(join(scratch,"src/lib.rs"),'include!(env!("FUSOR_MODULE"));\n');
  await writeFile(join(scratch,"web/fixture.html"),`<script type="text/rust">\nuse fusor::prelude::*;\nstruct Demo { title: fusor_std::forms::TextField<String> }\n</script>\n<App state="{{ Demo { title: fusor_std::forms::TextField::new(String::new()) } }}"><main>\n<input bind="state.title"><textarea bind="state.title"></textarea>\n</main></App>`);
  await cli(["check","--offline","--locked"]);
  await writeFile(join(scratch,"Cargo.toml"),manifest); await writeFile(join(scratch,"src/lib.rs"),originalLib);
  console.log("PASS: external/inline consumers; wrong value/control, conflicting values and HTML source locations");
  await cli(["build","--offline","--features","browser-tests"]);
  const port = await reservePort();
  server = startProcess(executable,["preview","--port",String(port),"--offline","--locked"],{cwd:scratch,env});

  await waitFor(() => server.output.includes("Ctrl+C to stop."),"preview startup");
  proxy = createServer(async (req,res) => {
    if (await backend.handle(req,res)) return;
    try {
      const response = await fetch(new URL(req.url,`http://127.0.0.1:${port}`),{headers:{accept:req.headers.accept || "*/*"}});
      res.writeHead(response.status,Object.fromEntries(response.headers)); res.end(Buffer.from(await response.arrayBuffer()));
    } catch { if (!res.destroyed) { res.writeHead(502); res.end("Preview unavailable"); } }
  });
  proxy.listen(0,"127.0.0.1"); await once(proxy,"listening");
  const origin = `http://127.0.0.1:${proxy.address().port}`;
  for (const name of (process.env.PLAYWRIGHT_BROWSERS || "chromium").split(",")) {
    backend.reset();
    browser = await ({chromium,firefox,webkit}[name]).launch(name === "chromium" ? {channel:process.env.PLAYWRIGHT_CHANNEL || undefined} : {});
    const page = await browser.newPage(); const errors = [], consoleErrors = [];
    page.on("pageerror",error => errors.push(String(error)));
    page.on("console",message => { if (message.type() === "error") consoleErrors.push(message.text()); });
    await page.goto(`${origin}/editor/project`);
    const primary = page.locator("#primary"), secondary = page.locator("#secondary");
    const title = primary.getByLabel("Project title"), body = primary.getByLabel("Notes"), seats = primary.getByLabel("Seats");
    const status = primary.locator(".status"), save = primary.getByRole("button",{name:"Save",exact:true});
    await expect(title).toHaveValue("Thursday"); await expect(secondary.getByLabel("Project title")).toHaveValue("Thursday");
    assert.equal(await page.evaluate(() => { const ids = [...document.querySelectorAll("[id]")].map(node => node.id); return new Set(ids).size === ids.length; }),true);
    await page.evaluate(async () => { const boot = document.querySelector('script[type="module"]').src; window.client = await import(new URL("./pkg/app.js",boot).href); });
    await seats.fill("-"); await save.click(); await expect(primary.locator(".seats-error")).not.toBeEmpty();
    await expect(seats).toHaveValue("-"); assert.equal(backend.writes.length,0);
    await seats.fill("2"); await title.fill(""); await save.click(); await expect(primary.locator(".title-error")).toHaveText("Enter a project title");
    await title.fill("Friday"); await title.press("Enter");
    await waitFor(() => backend.writes.length === 1,"first save"); await expect(save).toBeDisabled(); await expect(title).toBeEnabled();
    assert.equal(backend.writes[0].command.expected_version,10); assert.equal(backend.writes[0].command.title,"Friday"); assert.equal(backend.writes[0].command.seats,2);
    await title.fill("Monday"); await body.fill("First line\nSecond line");
    await page.evaluate(() => { window.original = document.querySelector("#primary .title"); window.original.focus(); window.original.setSelectionRange(2,4); window.client.submit_again(); });
    await expect(status).toContainText("earlier edit"); assert.equal(backend.writes.length,1);
    backend.release(backend.writes[0]); await expect(primary.locator(".version")).toHaveText("11");
    await expect(title).toHaveValue("Monday"); await expect(secondary.locator(".body")).toHaveValue("First line\nSecond line");
    await expect(primary.locator(".baseline")).toHaveText("Friday"); await expect(status).toHaveText("Unsaved changes");
    assert.deepEqual(await page.evaluate(() => [window.original === document.querySelector("#primary .title"),document.activeElement === window.original,window.original.selectionStart,window.original.selectionEnd]),[true,true,2,4]);
    await title.press("Enter"); await waitFor(() => backend.writes.length === 2,"explicit second save");
    assert.equal(backend.writes[1].command.expected_version,11); assert.equal(backend.writes[1].command.title,"Monday");
    assert.equal(backend.writes[1].command.body,"First line\nSecond line"); backend.release(backend.writes[1]);
    await expect(status).toHaveText("Saved"); await expect(primary.locator(".version")).toHaveText("12");
    console.log(`PASS (${name}): Friday → Monday, explicit next version, keyboard submission, validation, textarea, shared fields, accessible IDs, cursor and node identity`);

    // A stale read, then an actual remote update, cannot rebase even a clean editor.
    await expect(page.locator("#remote")).toContainText("version 12");
    backend.holdReads = true; const readsBefore = backend.reads.length;
    await page.locator("#refresh").click(); await waitFor(() => backend.reads.length > readsBefore,"held read");
    const oldRead = backend.reads.at(-1); backend.externalEdit("Remote change"); backend.releaseRead(oldRead);
    backend.holdReads = false; await page.locator("#refresh").click(); await expect(page.locator("#remote")).toContainText("Remote change · version 13");
    await expect(title).toHaveValue("Monday"); await expect(primary.locator(".version")).toHaveText("12"); await expect(status).toHaveText("Saved");
    await save.click(); await waitFor(() => backend.writes.length === 3,"conflicting command"); backend.release(backend.writes[2]);
    await expect(status).toContainText("Version conflict"); await expect(save).toBeDisabled();
    await page.evaluate(() => window.client.submit_again()); assert.equal(backend.writes.length,3);
    await primary.locator(".reload").click(); await expect(title).toHaveValue("Remote change"); await expect(primary.locator(".version")).toHaveText("13");
    await title.fill("Response lost"); await save.click(); await waitFor(() => backend.writes.length === 4,"unknown command"); backend.release(backend.writes[3],"unknown");
    await expect(status).toContainText("outcome unknown"); await expect(save).toBeDisabled();
    await primary.locator(".reload").click(); await expect(primary.locator(".notice")).toContainText("does not establish"); await expect(status).toContainText("outcome unknown");
    await page.locator("#refresh").click(); await expect(page.locator("#remote")).toContainText("Response lost · version 14");
    await primary.locator(".reload").click(); await expect(status).toHaveText("Saved"); await expect(primary.locator(".version")).toHaveText("14");
    assert.equal(backend.writes.length,4,"reconciliation never retries");
    await title.fill("Reserved"); await save.click(); await waitFor(() => backend.writes.length === 5,"rejection"); backend.release(backend.writes[4]);
    await expect(primary.locator(".title-error")).toHaveText("This project title is reserved"); await expect(status).toContainText("rejected");
    await seats.fill("3"); await expect(primary.locator(".title-error")).toHaveText("");
    await save.click(); await waitFor(() => backend.writes.length === 6,"older rejected snapshot"); await title.fill("Newer title"); backend.release(backend.writes[5]);
    await expect(status).toContainText("rejected"); await expect(primary.locator(".title-error")).toHaveText(""); await expect(title).toHaveValue("Newer title");
    console.log(`PASS (${name}): refetch separation, backend version conflict, known rejection, stale server errors and explicit unknown-outcome reconciliation`);

    await title.fill(" Friday "); await save.click(); await waitFor(() => backend.writes.length === 7,"normalization during composition");
    await title.evaluate(input => { input.focus(); input.dispatchEvent(new CompositionEvent("compositionstart",{bubbles:true})); input.value = "Composing"; input.dispatchEvent(new InputEvent("input",{bubbles:true,isComposing:true})); input.setSelectionRange(4,4); });
    backend.release(backend.writes[6]); await expect(primary.locator(".baseline")).toHaveText("Friday"); await expect(title).toHaveValue("Composing");
    assert.deepEqual(await title.evaluate(input => [document.activeElement === input,input.selectionStart,input.selectionEnd]),[true,4,4]);
    await page.evaluate(() => window.client.reset_title("Reviewed reset")); await expect(title).toHaveValue("Composing"); await expect(primary.locator(".baseline")).toHaveText("Reviewed reset");
    await title.evaluate(input => { input.value = "Composed draft"; input.dispatchEvent(new CompositionEvent("compositionend",{bubbles:true,data:"Composed draft"})); input.dispatchEvent(new InputEvent("input",{bubbles:true})); });
    await expect(title).toHaveValue("Composed draft"); await expect(secondary.locator(".title")).toHaveValue("Composed draft"); await expect(status).toHaveText("Unsaved changes");
    // Textareas have the same composition contract, independently of text inputs.
    await body.evaluate(input => { input.dispatchEvent(new CompositionEvent("compositionstart",{bubbles:true})); input.value = "Notes in progress"; input.dispatchEvent(new InputEvent("input",{bubbles:true,isComposing:true})); input.dispatchEvent(new CompositionEvent("compositionend",{bubbles:true})); });
    await expect(secondary.locator(".body")).toHaveValue("Notes in progress");
    console.log(`PASS (${name}): synthetic composition protects newer text from acceptance/reset; real OS IME check remains manual`);

    await title.fill("Save while away"); await save.click(); await waitFor(() => backend.writes.length === 8,"retained save"); await title.fill("Draft while away");
    await page.evaluate(() => { window.oldTitle = document.querySelector("#primary .title"); window.oldSave = document.querySelector("#primary .save"); });
    await page.locator("#away-link").click(); await expect(page.locator("h2")).toHaveText("Another route");
    assert.equal(backend.writes[7].aborted,false); backend.release(backend.writes[7]);
    await expect(page.locator("#remote")).toContainText("Save while away"); await page.locator("#project-link").click();
    await expect(title).toHaveValue("Draft while away"); await expect(primary.locator(".baseline")).toHaveText("Save while away");
    assert.equal(await page.evaluate(() => window.oldTitle.isConnected),false);
    await page.evaluate(() => { window.oldTitle.value = "Detached edit"; window.oldTitle.dispatchEvent(new InputEvent("input",{bubbles:true})); window.oldSave.click(); });
    await expect(title).toHaveValue("Draft while away"); assert.equal(backend.writes.length,8);
    await title.fill("Fresh view"); assert.equal(await page.evaluate(() => window.oldTitle.value),"Detached edit","disposed subscriptions do not rewrite detached controls");
    await save.click(); await waitFor(() => backend.writes.length === 9,"save before close");
    await page.locator("#close-session").click(); await expect(page.locator(".title")).toHaveCount(0);
    await waitFor(() => backend.writes[8].aborted,"session disposal aborts the local transport");
    backend.release(backend.writes[8]); // A server can still commit after cancellation.
    await page.locator("#refresh").click(); await expect(page.locator("#remote")).toContainText("Fresh view");
    await expect(page.locator(".title")).toHaveCount(0);
    await page.locator("#open-session").click(); await expect(title).toHaveValue("Fresh view"); await expect(status).toHaveText("Saved");
    await title.fill("Disposed app"); await save.click(); await waitFor(() => backend.writes.length === 10,"save before app disposal");
    await page.evaluate(() => window.client.unmount()); await waitFor(() => backend.writes[9].aborted,"app disposal cancels work"); backend.release(backend.writes[9]);
    assert.deepEqual(errors,[]);
    // Browsers may report the intentionally rejected/conflicting/lost HTTP response.
    assert.deepEqual(consoleErrors.filter(error => !/Failed to load resource|NetworkError|Load failed|net::ERR_|status of (409|422)|cancelled/i.test(error)),[]);
    console.log(`PASS (${name}): retained session across route disposal, listener/effect cleanup, explicit close and app teardown; cancellation never claims rollback`);
    // Actual DOM types can differ from authored HTML. Fail before attaching
    // control subscriptions and recover by mounting fresh validated views.
    await page.close(); backend.reset(); backend.holdReads = true;
    const invalid = await browser.newPage(); const mountErrors = [];
    invalid.on("console",message => { if (message.type() === "error") mountErrors.push(message.text()); });
    await invalid.goto(`${origin}/editor/project`);
    await waitFor(() => backend.reads.length === 1,"initial read before mount");
    await invalid.evaluate(() => {
      for (const template of document.querySelectorAll("template")) {
        const input = template.content.querySelector(".title"); if (input) input.setAttribute("type","checkbox");
      }
    });
    backend.releaseRead(backend.reads[0]); backend.holdReads = false;
    await expect.poll(() => mountErrors.length).toBe(2);
    assert(mountErrors.every(message => message.includes("bind cannot edit an input of type checkbox")));
    await expect(invalid.locator(".title")).toHaveCount(0); assert.equal(backend.writes.length,0);
    await invalid.evaluate(() => {
      for (const template of document.querySelectorAll("template")) {
        const input = template.content.querySelector(".title"); if (input) input.setAttribute("type","text");
      }
    });
    await invalid.locator("#close-session").click(); await invalid.locator("#open-session").click();
    await expect(invalid.locator("#primary .title")).toHaveValue("Thursday");
    console.log(`PASS (${name}): actual target validation and failed-mount recovery`);
    await browser.close(); browser = undefined;
  }
} finally {
  if (browser) await browser.close(); backend.close();
  if (proxy) { proxy.closeAllConnections(); await new Promise(resolve => proxy.close(resolve)); }
  await stopProcess(server);
  await rm(scratch,{recursive:true,force:true});
}
