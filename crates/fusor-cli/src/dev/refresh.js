// Development only. Patches native DOM handles; never evaluates Rust.
//
// The `__fusor/version` and `__fusor/update` endpoints below are served by
// dev/http.rs. The directory name is layout::GENERATED; this file cannot import
// it, so the two must be changed together.
const ELEMENT = 1, TEXT = 3, COMMENT = 8;
const mismatch = reason => { throw new Error(reason); };
const children = node => {
  const result = [];
  const nodes = Array.from(node.childNodes);
  for (let i = 0; i < nodes.length; i++) {
    const child = nodes[i];
    result.push(child);
    if (child.nodeType === COMMENT && /^fusor:(?:mount:)?\d+$/.test(child.data)) {
      const end = `/${child.data}`;
      while (++i < nodes.length && !(nodes[i].nodeType === COMMENT && nodes[i].data === end)) {}
      if (i === nodes.length) mismatch("missing managed content anchor");
      result.push(nodes[i]);
    }
  }
  return result;
};

function plan(before, after, live, edits) {
  if (before.isEqualNode(after)) return;
  if (!live || before.nodeType !== after.nodeType || before.nodeType !== live.nodeType) mismatch("node structure changed");
  if (before.nodeType === TEXT) {
    if (live.data !== before.data && live.data !== after.data) mismatch("text is controlled by application code");
    edits.push(() => { live.data = after.data; });
    return;
  }
  if (before.nodeType !== ELEMENT) mismatch("document/comment structure changed");
  if (before.localName !== after.localName || before.namespaceURI !== after.namespaceURI
      || before.localName !== live.localName || before.namespaceURI !== live.namespaceURI) mismatch("element type changed");
  if (before.localName === "noscript") {
    // A scripting-enabled page stores body fallback markup as text; DOMParser
    // parses it as elements. Normalize the initial live snapshot in an inert
    // clone; later baselines already come from DOMParser. Leave live DOM alone.
    const normalized = before.cloneNode(true);
    if (live.firstChild?.nodeType === TEXT && before.isEqualNode(live)) {
      const parsed = new DOMParser().parseFromString(`<body><noscript>${live.textContent}</noscript>`, "text/html");
      normalized.replaceChildren(...parsed.querySelector("noscript").childNodes);
    }
    if (!normalized.isEqualNode(after)) mismatch("noscript markup changed");
    return;
  }
  if (before.hasAttribute("data-fusor-external") || live.hasAttribute("data-fusor-external")) mismatch("external widget markup changed");
  if (before.localName === "script") {
    // The loader carries the document's revision; updating it is not executable
    // code replacement. Every other script change requires a fresh document.
    const a = before.cloneNode(true), b = after.cloneNode(true);
    a.removeAttribute("data-fusor-revision"); b.removeAttribute("data-fusor-revision");
    if (!a.isEqualNode(b)) mismatch("script changed");
    return;
  }
  for (const name of new Set([...before.getAttributeNames(), ...after.getAttributeNames()])) {
    const old = before.getAttribute(name), next = after.getAttribute(name);
    if (old === next) continue;
    if ((name.startsWith("data-fusor-") && name !== "data-fusor-link") || name.startsWith("on") || ["value", "checked", "selected", "srcdoc"].includes(name)) mismatch(`attribute ${name} requires reload`);
    if (live.getAttribute(name) !== old && live.getAttribute(name) !== next) mismatch(`attribute ${name} is controlled by application code`);
    edits.push(() => next === null ? live.removeAttribute(name) : live.setAttribute(name, next));
  }
  if (before.hasAttribute("data-fusor-text")) {
    // Only the compiler marks exact sole-child bindings. Declarations stay
    // empty while the runtime owns one Text node; static host edits are safe.
    if (before.getAttribute("data-fusor-text") !== live.getAttribute("data-fusor-text")
        || before.childNodes.length || after.childNodes.length
        || live.childNodes.length > 1
        || (live.firstChild && live.firstChild.nodeType !== TEXT)) mismatch("dynamic text structure changed");
    return;
  }
  if (before.hasAttribute("data-fusor-managed")) {
    if (before.innerHTML !== after.innerHTML) mismatch("managed child markup changed");
    return;
  }
  const a = children(before.content || before), b = children(after.content || after), c = children(live.content || live);
  if (a.length !== b.length || a.length !== c.length) mismatch("child structure changed");
  for (let i = 0; i < a.length; i++) plan(a[i], b[i], c[i], edits);
}

/** Validate the full patch before mutating anything. Exported for browser tests. */
export function patchDocument(before, after, live = document) {
  const edits = [];
  plan(before.documentElement, after.documentElement, live.documentElement, edits);
  const templates = before.querySelectorAll("template[data-fusor-component]");
  for (const template of templates) {
    const id = template.getAttribute("data-fusor-component");
    const next = after.querySelector(`template[data-fusor-component="${id}"]`);
    const declaration = live.querySelector(`template[data-fusor-component="${id}"]`);
    if (!next || !declaration) mismatch("component declaration changed");
    if (template.content.isEqualNode(next.content)) continue;
    if (!template.content.firstElementChild || !next.content.firstElementChild
        || !declaration.content.firstElementChild) mismatch("component root structure changed");
    // Template content is a separate DocumentFragment: isEqualNode on the
    // surrounding document does not compare it. Patch future mounts as well
    // as already mounted instances.
    plan(template.content.firstElementChild, next.content.firstElementChild, declaration.content.firstElementChild, edits);
    for (const instance of live.querySelectorAll(`[data-fusor-instance="${id}"]`)) {
      plan(template.content.firstElementChild, next.content.firstElementChild, instance, edits);
    }
  }
  for (const edit of edits) edit();
  return edits.length;
}

export function watch(generation, base) {
  // Capture authored browser structure before Wasm inserts children/text/widgets.
  let baseline = document.cloneNode(true);
  let revision = Number(document.querySelector("script[data-fusor-revision]")?.getAttribute("data-fusor-revision") || 0);
  let busy = false;
  const timer = setInterval(async () => {
    if (busy) return;
    busy = true;
    try {
      const check = await fetch(`${base}__fusor/version`, { cache: "no-store" });
      if (!check.ok || (await check.text()) === `${generation}:${revision}`) return;
      const response = await fetch(`${base}__fusor/update`, { cache: "no-store" });
      if (!response.ok) return;
      const update = await response.json();
      if (update.generation !== generation || update.reload_after > revision) {
        document.dispatchEvent(new Event("fusor:reload")); location.reload(); return;
      }
      const next = new DOMParser().parseFromString(update.html, "text/html");
      try {
        const changed = patchDocument(baseline, next);
        baseline = next;
        revision = update.revision;
        for (const link of document.querySelectorAll('link[rel="stylesheet"][href]')) {
          const url = new URL(link.href);
          if (url.origin === location.origin && url.pathname.startsWith(base)) {
            url.searchParams.set("__fusor", String(revision)); link.href = url.href;
          }
        }
        document.dispatchEvent(new CustomEvent("fusor:refresh", { detail: { revision, changed } }));
      } catch (error) {
        console.info("fusor reload:", error.message); document.dispatchEvent(new Event("fusor:reload")); location.reload();
      }
    } catch { /* The server may be restarting. Retry without losing the live app. */ }
    finally { busy = false; }
  }, 200);
  return () => clearInterval(timer);
}
