// A complete protocol scan for templates without managed regions or child mounts.
// This never caches a live DOM node, skips validation, or installs behavior.
const START = 'fusor:';
const END = '/fusor:';

export function flatPlan(elementIds, tags, textIds, textElements) {
  return {
    elements: new Map(elementIds.map((id, index) => [id, index])),
    tags,
    texts: new Map(textIds.map((id, index) => [id, index])),
    elementIds,
    textIds,
    textElements,
    textHosts: new Map(Array.from({ length: textElements.length / 3 }, (_, i) =>
      [textElements[i * 3], i * 3])),
  };
}

function mismatch(message) {
  throw `fusor: template mismatch: ${message}`;
}

export function resolveFlat(plan, root) {
  const textOffset = plan.elementIds.length;
  const hostOffset = textOffset + plan.textIds.length * 3;
  // One descriptor-sized output array also holds validation state. It is local
  // to this call; no application nodes enter the cached immutable plan.
  const elements = new Array(hostOffset + plan.textElements.length / 3 * 2);
  let node = root;
  do {
    if (node.nodeType === 1) {
      const id = node.getAttribute('data-fusor-node');
      const text = node.getAttribute('data-fusor-text');
      if (text !== null) {
        const offset = plan.textHosts.get(text);
        if (offset === undefined) mismatch('unexpected text element');
        const host = plan.textElements[offset + 1], tag = plan.textElements[offset + 2];
        if (host !== null && host !== id) mismatch('mismatched text host');
        if (node.localName !== tag || node.namespaceURI !== 'http://www.w3.org/1999/xhtml') mismatch(`text element ${text} must be <${tag}>`);
        const slot = hostOffset + offset / 3 * 2;
        if (elements[slot]) mismatch(`duplicate text element ${text}`);
        elements[slot] = node;
      }
      if (id !== null) {
        const index = plan.elements.get(id);
        if (index === undefined) mismatch('unexpected element identifiers');
        if (elements[index]) mismatch(`duplicate element ${id}`);
        if (node.localName !== plan.tags[index] || node.namespaceURI !== 'http://www.w3.org/1999/xhtml') {
          mismatch(`element ${id} must be <${plan.tags[index]}>, found <${node.localName}>`);
        }
        elements[index] = node;
      }
    } else if (node.nodeType === 8) {
      const value = node.nodeValue;
      let id, end;
      if (value.startsWith(START)) {
        id = value.slice(START.length); end = false;
      } else if (value.startsWith(END)) {
        id = value.slice(END.length); end = true;
      } else continue;
      const index = plan.texts.get(id);
      // Mount anchors are impossible in this descriptor too; unknown, malformed
      // and noncanonical IDs all fail before any node is inserted or bound.
      if (index === undefined) mismatch('unexpected text or component identifiers');
      const slot = textOffset + index * 3 + (end ? 1 : 0);
      if (elements[slot]) mismatch(`duplicate text ${end ? 'end' : 'start'} ${id}`);
      elements[slot] = node;
    }
  } while ((node = following(node, root)));
  return validateSlots(plan, elements);
}

// The next node of a SHOW_ALL TreeWalker's pre-order walk confined to `root`,
// without allocating a walker for every scan.
function following(node, root) {
  const child = node.firstChild;
  if (child) return child;
  while (node !== root) {
    const sibling = node.nextSibling;
    if (sibling) return sibling;
    node = node.parentNode;
  }
  return null;
}

function validateSlots(plan, elements) {
  const textOffset = plan.elementIds.length;
  const hostOffset = textOffset + plan.textIds.length * 3;
  for (let index = 0; index < plan.elementIds.length; index++) {
    if (!elements[index]) mismatch(`missing element ${plan.elementIds[index]}`);
  }
  for (let index = 0; index < plan.textIds.length; index++) {
    const slot = textOffset + index * 3;
    const id = plan.textIds[index], start = elements[slot], end = elements[slot + 1];
    if (!start) mismatch(`missing text start ${id}`);
    if (!end) mismatch(`missing text end ${id}`);
    const next = start.nextSibling;
    if (!next) mismatch(`unpaired text slot ${id}`);
    let text = null;
    if (next !== end) {
      if (next.nextSibling !== end) mismatch(`unexpected nodes in text slot ${id}`);
      if (next.nodeType !== 3) mismatch(`expected a text node in slot ${id}`);
      text = next;
    }
    elements[slot + 2] = text;
  }
  for (let index = 0; index < plan.textElements.length; index += 3) {
    const slot = hostOffset + index / 3 * 2;
    const id = plan.textElements[index], host = elements[slot];
    if (!host) mismatch(`missing or mismatched text element ${id}`);
    const text = host.firstChild;
    if (text && text.nextSibling) mismatch(`unexpected nodes in text element ${id}`);
    if (text && text.nodeType !== 3) mismatch(`expected a text node in text element ${id}`);
    elements[slot + 1] = text;
  }
  return elements;
}

// These checks replace the typed Rust dyn_into calls at the same validation
// boundary. Use the application's global constructors, including cross-realm
// rejection, rather than treating nodeType/tag checks as equivalent casts.
function checked(value, constructor) {
  let valid = false;
  try {
    switch (constructor) {
      case 'Element': valid = value instanceof Element; break;
      case 'HTMLInputElement': valid = value instanceof HTMLInputElement; break;
      case 'Node': valid = value instanceof Node; break;
      case 'Text': valid = value instanceof Text; break;
    }
  } catch (_) {}
  if (!valid) throw value;
}

function validateBundleTypes(plan, nodes) {
  const textOffset = plan.elementIds.length;
  const hostOffset = textOffset + plan.textIds.length * 3;
  for (let i = 0; i < textOffset; i++) {
    checked(nodes[i], plan.tags[i] === 'input' ? 'HTMLInputElement' : 'Element');
  }
  for (let i = 0; i < plan.textIds.length; i++) {
    const slot = textOffset + i * 3;
    checked(nodes[slot], 'Node');
    checked(nodes[slot + 1], 'Node');
    if (nodes[slot + 2] !== null) checked(nodes[slot + 2], 'Text');
  }
  for (let i = 0; i < plan.textElements.length / 3; i++) {
    const slot = hostOffset + i * 2;
    checked(nodes[slot], 'Element');
    if (nodes[slot + 1] !== null) checked(nodes[slot + 1], 'Text');
  }
}

function pathFrom(root, target) {
  const path = [];
  let node = target;
  while (node !== root) {
    let sibling = node.previousSibling, index = 0;
    while (sibling) { index++; sibling = sibling.previousSibling; }
    path.push(index);
    node = node.parentNode;
    if (!node) mismatch('detached cached handle');
  }
  return path.reverse();
}

function followPath(root, path) {
  let node = root;
  for (const index of path) {
    // Sibling steps address the same child as childNodes.item(index) without
    // materializing a live NodeList for every step.
    node = node.firstChild;
    for (let step = 0; node && step < index; step++) node = node.nextSibling;
    if (!node) mismatch('template changed during resolution');
  }
  return node;
}

function rememberBundle(plan, root, nodes) {
  const textOffset = plan.elementIds.length;
  const hostOffset = textOffset + plan.textIds.length * 3;
  // Match the typed cache's inert-document import. Do not run custom-element
  // construction again or retain an application node in cached metadata.
  const wrapper = root.ownerDocument.createElement('template');
  const inert = wrapper.content.ownerDocument;
  const pristine = inert.importNode(root, true);
  const paths = nodes.map((node, index) => {
    const existingText = index >= hostOffset
      ? (index - hostOffset) % 2 === 1
      : index >= textOffset && (index - textOffset) % 3 === 2;
    return existingText ? null : pathFrom(root, node);
  });
  plan.bindingCache = { pristine, paths };
}

export function resolveBindings(plan, root, cached) {
  let nodes;
  const certificate = cached && plan.bindingCache;
  const cacheHit = certificate && root.isEqualNode(certificate.pristine);
  if (cacheHit) {
    // Resolve every location before insertion can shift a later native path.
    nodes = certificate.paths.map(path => path === null ? null : followPath(root, path));
    validateSlots(plan, nodes);
    // An equal clone of the validated inert certificate has the same node
    // interfaces at every path: inert template documents never upgrade.
  } else {
    nodes = resolveFlat(plan, root);
    validateBundleTypes(plan, nodes);
  }
  if (cached && !cacheHit) {
    // Optional cache construction must not make a validated mount fail.
    try { rememberBundle(plan, root, nodes); } catch (_) {}
  }
  const textOffset = plan.elementIds.length;
  const hostOffset = textOffset + plan.textIds.length * 3;
  // The complete scan, shape checks and typed casts have finished. Keep the
  // exact original targets through user construction and synchronous callbacks.
  // Match finish_resolution's global document rather than root.ownerDocument.
  for (let i = 0; i < plan.textIds.length; i++) {
    const slot = textOffset + i * 3;
    const start = nodes[slot], end = nodes[slot + 1];
    let text = nodes[slot + 2];
    if (text === null) {
      text = document.createTextNode('');
      const parent = start.parentNode;
      if (!parent) mismatch('detached text anchor');
      parent.insertBefore(text, end);
    }
    nodes[textOffset + i] = text;
  }
  for (let i = 0; i < plan.textElements.length / 3; i++) {
    const slot = hostOffset + i * 2, host = nodes[slot];
    let text = nodes[slot + 1];
    if (text === null) { text = document.createTextNode(''); host.appendChild(text); }
    nodes[textOffset + plan.textIds.length + i] = text;
  }
  nodes.length = textOffset + plan.textIds.length + plan.textElements.length / 3;
  return nodes;
}

// The document-template mount of a flat bundled descriptor in one native call.
// Same checks, clone source, errors and order as the typed Rust sequence:
// unique root, schema, template kind, one root element, descriptor version,
// then complete validation before instance marking. The root follows the
// binding targets in the returned bundle.
export function mountTemplate(plan, selector, schema, identity, versionOk) {
  const roots = document.querySelectorAll(selector);
  if (roots.length !== 1) mismatch(`component ${identity} requires exactly one root, found ${roots.length}`);
  const template = roots[0];
  if (template.getAttribute('data-fusor-version') !== schema) mismatch('HTML schema version differs from Wasm; rebuild the application');
  if (!(template instanceof HTMLTemplateElement)) mismatch('expected an HTML template');
  const content = template.content;
  if (content.childElementCount !== 1) throw 'fusor: a row template needs exactly one root element';
  const root = content.firstElementChild.cloneNode(true);
  if (!versionOk) mismatch('unsupported descriptor version; rebuild the application');
  const nodes = resolveBindings(plan, root, true);
  root.setAttribute('data-fusor-instance', identity);
  nodes.push(root);
  return nodes;
}

// A server-rendered root of a flat bundled descriptor in one native call, in
// the typed Rust order: server identity, descriptor version, the complete
// protocol scan, then instance marking.
export function hydrateRoot(plan, root, schema, identity, versionOk) {
  if (root.getAttribute('data-fusor-version') !== schema || root.getAttribute('data-fusor-component') !== identity) {
    mismatch('server root identity differs from the browser template');
  }
  if (!versionOk) mismatch('unsupported descriptor version; rebuild the application');
  const nodes = resolveBindings(plan, root, false);
  root.setAttribute('data-fusor-instance', identity);
  return nodes;
}

export function bindingText(nodes, index, value) {
  const text = nodes[index];
  if (text.data !== value) text.data = value;
}
export function bindingIntegerText(nodes, index, number) {
  const text = nodes[index], value = "" + number;
  if (text.data !== value) text.data = value;
}
export function bindingSetAttribute(nodes, index, name, value) {
  nodes[index].setAttribute(name, value);
}
export function bindingSetIntegerAttribute(nodes, index, name, number) {
  nodes[index].setAttribute(name, "" + number);
}
export function bindingRemoveAttribute(nodes, index, name) {
  nodes[index].removeAttribute(name);
}
export function bindingElement(nodes, index) { return nodes[index]; }
