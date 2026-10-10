// Delivery protocol v1. This is the only loader/scheduler used by HTML and Rust.
// Unit state owns code; instance state owns behavior. Fetching is never mounting.
const VERSION = 1;
const schedulerDefaults = Object.freeze({ rootMargin: '0px', idleTimeout: 2000 });
const activationPolicies = new Set(['load', 'visible', 'idle', 'interaction', 'manual']);
const prefetchPolicies = new Set(['none', 'load', 'visible', 'idle']);
const metadataNames = ['id', 'data-fusor-island', 'data-fusor-unit', 'data-fusor-generation', 'data-fusor-schema', 'data-fusor-hash', 'data-fusor-activate', 'data-fusor-prefetch'];
function failure(code, message, cause) {
  return Object.assign(new Error(message, cause ? { cause } : undefined), { code });
}
function cancelled() {
  return failure('cancelled', 'The island request no longer has a live caller.');
}
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function deliveryUnits(manifest) {
  if (!manifest || manifest.version !== VERSION || typeof manifest.generation !== 'string' || !manifest.generation || !manifest.units || typeof manifest.units !== 'object' || Array.isArray(manifest.units)) throw failure('protocol-mismatch', 'Unsupported island manifest.');
  const units = new Map();
  const descriptors = new Set();
  // Validate and retain the same definitions before acquiring any global resources.
  for (const [name, value] of Object.entries(manifest.units)) {
    if (!name || !value || !Array.isArray(value.dependencies === undefined ? [] : value.dependencies) || !Array.isArray(value.entries) || !value.entries.length) throw failure('protocol-mismatch', 'Invalid delivery unit entries.');
    const definition = { javascript: value.javascript, wasm: value.wasm, dependencies: [...value.dependencies ?? []], entries: value.entries.map((entry) => ({ ...entry })) };
    for (const path of [definition.javascript, definition.wasm, ...definition.dependencies]) {
      if (typeof path !== 'string' || !path.startsWith('/') || path.startsWith('//') || path.includes('..') || /[?#\\]/.test(path)) throw failure('protocol-mismatch', 'Delivery URLs must be immutable same-origin paths.');
      const url = new URL(path, location.href);
      if (url.origin !== location.origin || url.search || url.hash) throw failure('protocol-mismatch', 'Delivery URLs must be immutable same-origin paths.');
    }
    for (const entry of definition.entries) {
      if (entry.unit !== name || !['descriptor', 'props_schema', 'template_hash'].every((field) => typeof entry[field] === 'string' && entry[field]) || !['attach', 'preview'].includes(entry.mode) || descriptors.has(entry.descriptor)) throw failure('protocol-mismatch', 'Invalid delivery unit entries.');
      descriptors.add(entry.descriptor);
    }
    units.set(name, { definition, state: 'absent', generation: 0, claims: new Set(), available: null, initialization: null, module: null, controller: null, error: null, recoverable: true });
  }
  return units;
}
export function install(manifest, { root = document } = {}) {
  if (globalThis.__fusor_islands) throw failure('protocol-mismatch', 'An island registry is already installed.');
  const registry = new Registry(manifest);
  return registry.install(root);
}
class Registry {
  constructor(manifest) {
    this.units = deliveryUnits(manifest);
    this.generation = manifest.generation;
    this.instances = new Map();
    this.ids = new Map();
    this.elements = new WeakMap();
    this.sequence = 0n;
    this.previousComposing = globalThis.__fusor_composing;
    this.composing = this.previousComposing ?? new WeakSet();
    this.onCompositionStart = (event) => this.composing.add(event.target);
    this.onCompositionEnd = (event) => this.composing.delete(event.target);
    this.onClick = this.onClick.bind(this);
    this.observer = new MutationObserver((records) => queueMicrotask(() => this.onMutations(records)));
    this.api = this.createApi();
  }
  state(instance, next, error = null) {
    if (instance.state === 'disposed') return;
    instance.state = next;
    instance.error = error;
    instance.host.setAttribute('data-fusor-status', next);
    if (error) instance.host.setAttribute('data-fusor-error', error.code || 'load-failed');
    else instance.host.removeAttribute('data-fusor-error');
    instance.host.dispatchEvent(new CustomEvent('fusor:island-status', { detail: { id: instance.id, status: next, error }, bubbles: true }));
  }
  valid(instance) {
    return this.instances.get(instance.token) === instance && instance.host.isConnected && metadataNames.every((name) => instance.host.getAttribute(name) === instance.metadata[name]) && instance.propsNode.parentElement === instance.host && instance.propsNode.matches('script[type="application/json"][data-fusor-props]') && instance.propsNode.textContent === instance.props;
  }
  target(token) {
    const instance = this.instances.get(token);
    if (!instance || !this.valid(instance)) throw failure('stale-instance', 'This island registration was removed or changed.');
    return instance;
  }
  wantsActivation(instance) {
    for (const claim of instance.claims) {
      if (claim.kind === 'activate' && !claim.done) return true;
    }
    return false;
  }
  preloadModule(url, signal) {
    const link = document.createElement('link');
    if (!link.relList.supports('modulepreload')) {
      // Older engines still get a safe HTTP-cache prefetch. They may refetch
      // glue on import; Wasm bytes and all activation ownership remain shared.
      return fetch(url, { signal, credentials: 'same-origin' }).then((response) => {
        if (!response.ok) throw failure('load-failed', `Cannot prefetch module ${url}: HTTP ${response.status}.`);
        return response.arrayBuffer();
      });
    }
    return new Promise((resolve, reject) => {
      const cleanup = () => {
        link.remove();
        signal.removeEventListener('abort', abort);
        link.onload = link.onerror = null;
      };
      const abort = () => {
        cleanup();
        reject(cancelled());
      };
      link.rel = 'modulepreload';
      link.href = url;
      link.crossOrigin = 'anonymous';
      link.onload = () => {
        cleanup();
        resolve();
      };
      link.onerror = () => {
        cleanup();
        reject(Object.assign(failure('load-failed', `Cannot preload module ${url}; reload after correcting this generation.`), { needsReload: true }));
      };
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) {
        abort();
        return;
      }
      document.head.append(link);
    });
  }
  async available(unit) {
    if (unit.state === 'failed') throw unit.error;
    if (unit.available) return unit.available;
    const generation = ++unit.generation;
    const controller = unit.controller = new AbortController();
    unit.state = 'fetching';
    unit.available = (async () => {
      const [, bytes] = await Promise.all([
        Promise.all([unit.definition.javascript, ...unit.definition.dependencies].map((url) => this.preloadModule(url, controller.signal))),
        fetch(unit.definition.wasm, { signal: controller.signal, credentials: 'same-origin' }).then((response) => {
          if (!response.ok) throw failure('load-failed', `Cannot load immutable Wasm ${unit.definition.wasm}: HTTP ${response.status}.`);
          return response.arrayBuffer();
        })
      ]);
      if (unit.generation !== generation) throw cancelled();
      unit.state = 'available';
      unit.controller = null;
      return bytes;
    })().catch((error) => {
      if (unit.generation === generation) {
        unit.controller = null;
        unit.available = null;
        if (controller.signal.aborted) {
          unit.state = 'absent';
        } else {
          unit.state = 'failed';
          unit.recoverable = !error.needsReload;
          unit.error = failure('load-failed', error.message, error);
        }
      }
      throw error;
    });
    return unit.available;
  }
  async initialize(unit) {
    if (unit.state === 'failed') throw unit.error;
    if (unit.initialization) return unit.initialization;
    const bytes = await this.available(unit);
    if (unit.initialization) return unit.initialization;
    unit.state = 'initializing';
    unit.initialization = (async () => {
      const module = await import(unit.definition.javascript);
      if (typeof module.default !== 'function' || typeof module.__fusor_manifest !== 'function' || typeof module.__fusor_activate !== 'function' || typeof module.__fusor_dispose !== 'function') throw failure('load-failed', 'A delivery unit is missing its typed entry exports.');
      await module.default({ module_or_path: bytes });
      const witness = JSON.parse(module.__fusor_manifest());
      const fields = ['unit', 'descriptor', 'props_schema', 'template_hash', 'mode'];
      if (witness.version !== VERSION || !Array.isArray(witness.entries) || witness.entries.length !== unit.definition.entries.length || unit.definition.entries.some((expected) => !witness.entries.some((actual) => fields.every((field) => actual[field] === expected[field])))) throw failure('descriptor-mismatch', 'Loaded Wasm registrations differ from this page’s immutable manifest.');
      unit.module = module;
      unit.state = 'ready';
      return module;
    })().catch((error) => {
      unit.state = 'failed';
      unit.recoverable = false;
      unit.error = failure('load-failed', 'Unit initialization failed. Publish a corrected generation and reload; failed module evaluation cannot be retried safely at the same URL.', error);
      throw unit.error;
    });
    return unit.initialization;
  }
  stopUnusedFetch(unit) {
    if (!unit.claims.size && unit.state === 'fetching') {
      ++unit.generation;
      unit.controller?.abort();
      unit.controller = null;
      unit.available = null;
      unit.state = 'absent';
    }
  }
  waitForComposition(instance, generation) {
    const active = document.activeElement;
    if (!active || !instance.host.contains(active) || !this.composing.has(active)) return Promise.resolve();
    const waiting = deferred();
    const cleanup = () => {
      document.removeEventListener('compositionend', end, true);
      instance.compositionWaiters.delete(cancel);
    };
    const cancel = () => {
      cleanup();
      waiting.reject(cancelled());
    };
    const end = (event) => {
      if (event.target !== active) return;
      cleanup();
      // Run after the composition event's native input handlers have completed.
      queueMicrotask(() => this.valid(instance) && instance.operation === generation ? waiting.resolve() : waiting.reject(cancelled()));
    };
    instance.compositionWaiters.add(cancel);
    document.addEventListener('compositionend', end, true);
    return waiting.promise;
  }
  activate(instance) {
    if (instance.state === 'active') return Promise.resolve();
    if (instance.work) return instance.work;
    const generation = ++instance.operation;
    this.state(instance, 'requested');
    const work = (async () => {
      const module = await this.initialize(instance.unit);
      if (!this.valid(instance) || instance.operation !== generation || !this.wantsActivation(instance)) throw cancelled();
      await this.waitForComposition(instance, generation);
      if (!this.valid(instance) || instance.operation !== generation || !this.wantsActivation(instance)) throw cancelled();
      this.state(instance, 'binding');
      const bindingToken = `${instance.token}/attempt-${generation}`;
      instance.bindingToken = bindingToken;
      try {
        // No await between inspecting the native DOM and synchronous Rust bind.
        // Generated Rust adopts live control values before its binding effects.
        const binding = module.__fusor_activate(instance.entry.descriptor, instance.host, instance.props, bindingToken);
        // Preview mode waits for its first coherent publication. Attach mode
        // has already adopted native state synchronously before this await.
        await binding;
      } catch (error) {
        module.__fusor_dispose(bindingToken);
        throw failure('binding-failed', 'The island could not attach to its initial HTML.', error);
      }
      if (!this.valid(instance) || instance.operation !== generation || !this.wantsActivation(instance)) {
        module.__fusor_dispose(bindingToken);
        throw cancelled();
      }
      this.state(instance, 'active');
    })().catch((error) => {
      if (this.instances.get(instance.token) === instance && instance.operation === generation) {
        this.state(instance, error.code === 'cancelled' ? 'dormant' : 'failed', error.code === 'cancelled' ? null : error);
      }
      throw error;
    }).finally(() => {
      if (instance.operation === generation) instance.work = null;
    });
    instance.work = work;
    return work;
  }
  request(token, kind) {
    const instance = this.target(token);
    if (!['prefetch', 'activate', 'retry'].includes(kind)) throw failure('invalid-operation', 'Unknown island operation.');
    if (kind === 'retry') {
      if (instance.unit.state === 'failed') {
        if (!instance.unit.recoverable) throw instance.unit.error;
        instance.unit.state = 'absent';
        instance.unit.available = null;
        instance.unit.error = null;
      }
      if (instance.state === 'failed') {
        ++instance.operation;
        instance.work = null;
        this.state(instance, 'dormant');
      }
      kind = 'activate';
    } else if (instance.state === 'failed') {
      throw instance.error;
    }
    const waiting = deferred();
    const claim = { kind, done: false, cancel: null };
    instance.claims.add(claim);
    instance.unit.claims.add(claim);
    const finish = (error) => {
      if (claim.done) return;
      claim.done = true;
      instance.claims.delete(claim);
      instance.unit.claims.delete(claim);
      if (kind === 'activate' && !this.wantsActivation(instance) && ['requested', 'binding'].includes(instance.state)) {
        ++instance.operation;
        instance.work = null;
        for (const cancel of [...instance.compositionWaiters]) cancel();
        if (instance.bindingToken) instance.unit.module?.__fusor_dispose(instance.bindingToken);
        instance.bindingToken = null;
        this.state(instance, 'dormant');
      }
      this.stopUnusedFetch(instance.unit);
      if (error) waiting.reject(error);
      else waiting.resolve();
    };
    claim.cancel = () => finish(cancelled());
    const work = kind === 'prefetch' ? this.available(instance.unit) : this.activate(instance);
    work.then(() => finish(null), finish);
    // A Rust future can be dropped before JsFuture starts polling the Promise.
    // Keep cancellation rejections handled without swallowing the caller result.
    waiting.promise.catch(() => {
    });
    return { promise: waiting.promise, cancel: claim.cancel };
  }
  schedule(instance, policy, kind) {
    const start = () => {
      if (!this.valid(instance)) return;
      try {
        const operation = this.request(instance.token, kind);
        operation.promise.catch((error) => {
          if (error.code !== 'cancelled') console.error(error);
        });
      } catch (error) {
        this.state(instance, 'failed', error);
      }
    };
    if (policy === 'load') {
      start();
    } else if (policy === 'visible') {
      if (!globalThis.IntersectionObserver) {
        start();
        return;
      }
      const observer = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          observer.disconnect();
          instance.triggers.delete(cancel);
          start();
        }
      }, { rootMargin: schedulerDefaults.rootMargin });
      const cancel = () => observer.disconnect();
      instance.triggers.add(cancel);
      observer.observe(instance.host);
    } else if (policy === 'idle') {
      let cancel;
      const run = () => {
        instance.triggers.delete(cancel);
        start();
      };
      if (globalThis.requestIdleCallback) {
        const id = requestIdleCallback(run, { timeout: schedulerDefaults.idleTimeout });
        cancel = () => cancelIdleCallback(id);
      } else {
        const id = setTimeout(run, 0);
        cancel = () => clearTimeout(id);
      }
      instance.triggers.add(cancel);
    }
  }
  register(host) {
    if (this.elements.has(host)) return;
    if (host.parentElement?.closest('[data-fusor-island]')) throw failure('nested-island', 'Nested independent islands are unsupported.');
    const metadata = Object.fromEntries(metadataNames.map((name) => [name, host.getAttribute(name)]));
    const id = metadata.id;
    const unit = this.units.get(metadata['data-fusor-unit']);
    const entry = unit?.definition.entries.find((candidate) => candidate.descriptor === metadata['data-fusor-island']);
    if (!id || this.ids.has(id) || document.getElementById(id) !== host || document.querySelectorAll(`[id="${CSS.escape(id)}"]`).length !== 1) throw failure('duplicate-instance', 'Island IDs must be unique in the document.');
    if (!entry || metadata['data-fusor-generation'] !== this.generation || entry.props_schema !== metadata['data-fusor-schema'] || entry.template_hash !== metadata['data-fusor-hash']) throw failure('descriptor-mismatch', `Island ${id} does not match this page\u2019s generation.`);
    const policy = metadata['data-fusor-activate'] || 'load';
    const prefetch = metadata['data-fusor-prefetch'] || 'none';
    if (!activationPolicies.has(policy) || !prefetchPolicies.has(prefetch)) throw failure('protocol-mismatch', 'Unsupported island scheduling policy.');
    const propsNodes = [...host.children].filter((node) => node.matches('script[type="application/json"][data-fusor-props]'));
    if (propsNodes.length !== 1) throw failure('descriptor-mismatch', 'An island needs one inert props payload.');
    const token = `island-${++this.sequence}`;
    const instance = { id, token, host, entry, unit, metadata, propsNode: propsNodes[0], props: propsNodes[0].textContent, state: 'dormant', bindingToken: null, operation: 0, error: null, work: null, claims: new Set(), triggers: new Set(), compositionWaiters: new Set() };
    this.instances.set(token, instance);
    this.ids.set(id, instance);
    this.elements.set(host, instance);
    this.state(instance, 'dormant');
    this.schedule(instance, prefetch, 'prefetch');
    this.schedule(instance, policy, 'activate');
  }
  discover(node) {
    const hosts = [...node.matches?.('[data-fusor-island]') ? [node] : [], ...node.querySelectorAll?.('[data-fusor-island]') || []];
    for (const host of hosts) {
      try {
        this.register(host);
      } catch (error) {
        host.setAttribute('data-fusor-error', error.code || 'protocol-mismatch');
        console.error(error);
      }
    }
  }
  dispose(instance) {
    if (this.instances.get(instance.token) !== instance) return;
    // Reentrant cleanup must see an expired token before any Rust destructor runs.
    this.instances.delete(instance.token);
    this.ids.delete(instance.id);
    this.elements.delete(instance.host);
    ++instance.operation;
    for (const cancel of instance.triggers) cancel();
    instance.triggers.clear();
    for (const cancel of [...instance.compositionWaiters]) cancel();
    for (const claim of [...instance.claims]) claim.cancel();
    if (instance.bindingToken) instance.unit.module?.__fusor_dispose(instance.bindingToken);
    instance.bindingToken = null;
    if (!this.elements.has(instance.host)) this.state(instance, 'disposed');
    instance.work = null;
    instance.props = '';
    instance.error = null;
  }
  byId(id) {
    const instance = this.ids.get(id);
    if (!instance) throw failure('unknown-instance', `Unknown island ${id}.`);
    return this.target(instance.token);
  }
  onClick(event) {
    const button = event.target.closest?.('button[data-fusor-activate-target]');
    if (!button || button.disabled) return;
    const instance = this.ids.get(button.getAttribute('data-fusor-activate-target'));
    if (!instance || instance.metadata['data-fusor-activate'] !== 'interaction') return;
    event.preventDefault();
    // Another deliberate click is an explicit retry; automatic policies never loop.
    try {
      this.request(instance.token, instance.state === 'failed' ? 'retry' : 'activate').promise.catch((error) => {
        if (error.code !== 'cancelled') console.error(error);
      });
    } catch (error) {
      this.state(instance, 'failed', error);
    }
  }
  onMutations(records) {
    // Check connectivity after the complete mutation batch: keyed DOM moves
    // retain behavior, while external removal disposes it.
    for (const instance of [...this.instances.values()]) {
      if (!this.valid(instance)) this.dispose(instance);
    }
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node.isConnected) this.discover(node);
      }
    }
  }
  createApi() {
    return {
      version: VERSION,
      lookup: (id, descriptor, schema) => {
        const instance = this.byId(id);
        if (instance.entry.descriptor !== descriptor || instance.entry.props_schema !== schema) throw failure('descriptor-mismatch', 'Island handle descriptor/schema mismatch.');
        return instance.token;
      },
      status: (token) => this.target(token).state,
      request: (token, kind) => this.request(token, kind),
      prefetch: (id) => this.request(this.byId(id).token, 'prefetch'),
      activate: (id) => this.request(this.byId(id).token, 'activate'),
      retry: (id) => this.request(this.byId(id).token, 'retry'),
      dispose: (id) => {
        const instance = this.ids.get(id);
        if (instance) this.dispose(instance);
      },
      disposeTree: (root) => {
        for (const instance of [...this.instances.values()]) {
          if (root === instance.host || root.contains(instance.host)) this.dispose(instance);
        }
      },
      register: (node) => this.discover(node),
      inspect: () => ({
        units: Object.fromEntries([...this.units].map(([name, unit]) => [name, unit.state])),
        instances: [...this.instances.values()].map((instance) => ({ id: instance.id, state: instance.state, waiters: instance.claims.size }))
      }),
      destroy: () => this.destroy()
    };
  }
  destroy() {
    this.observer.disconnect();
    document.removeEventListener('click', this.onClick);
    document.removeEventListener('compositionstart', this.onCompositionStart, true);
    document.removeEventListener('compositionend', this.onCompositionEnd, true);
    for (const instance of [...this.instances.values()]) this.dispose(instance);
    if (globalThis.__fusor_islands === this.api) delete globalThis.__fusor_islands;
    if (globalThis.__fusor_composing === this.composing) {
      if (this.previousComposing === undefined) delete globalThis.__fusor_composing;
      else globalThis.__fusor_composing = this.previousComposing;
    }
  }
  install(root) {
    try {
      globalThis.__fusor_composing = this.composing;
      document.addEventListener('compositionstart', this.onCompositionStart, true);
      document.addEventListener('compositionend', this.onCompositionEnd, true);
      document.addEventListener('click', this.onClick);
      this.observer.observe(root, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: [...metadataNames, 'type', 'data-fusor-props'] });
      globalThis.__fusor_islands = this.api;
      this.discover(root);
      return this.api;
    } catch (error) {
      this.destroy();
      throw error;
    }
  }
}
