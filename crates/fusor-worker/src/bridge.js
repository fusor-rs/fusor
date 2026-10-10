// The Rust facade owns observation; this module owns physical browser workers.
const errorEvent = error => ({Result: {Err: {Worker: error}}});
const failure = (name, message) => ({[name]: {message: String(message)}});
const config = () => globalThis.__fusor_worker_artifact;
const runtimes = new Set();
const STARTUP_TIMEOUT_MS = 10000;
if (typeof document !== 'undefined') {
  const stopRuntimes = () => {
    for (const runtime of runtimes) shutdown(runtime, '"Terminated"');
  };
  globalThis.addEventListener('pagehide', stopRuntimes);
  document.addEventListener('fusor:reload', stopRuntimes);
}
export const hardware_parallelism = () => globalThis.navigator?.hardwareConcurrency || 1;
export const dedicated_workers = () => typeof Worker === 'function';
export const shared_memory = () => dedicated_workers() && globalThis.crossOriginIsolated === true && typeof SharedArrayBuffer === 'function';
export function require_pool() {}
export const pool_identity = runtime => runtime.pool;
export const generation = runtime => runtime.generation;
export const ready = runtime => runtime.ready;

export function open_runtime(pool, threads, active, capacity, callback) {
  const artifact = config();
  if (!artifact || artifact.version !== 1 || (pool && !artifact.threaded)) throw JSON.stringify('IncompatibleArtifact');
  const worker = new Worker(pool ? artifact.threaded : artifact.ordinary, {type: 'module'});
  const runtime = {worker, callback, pool: pool ? crypto.randomUUID() : '', generation: artifact.generation, progress: new Map(), computes: [], timers: new Set(), queued: [], loaded: false, error: null, closes: new Map(), capacity, active};
  runtimes.add(runtime);
  runtime.ready = new Promise((resolve, reject) => {
    runtime.resolve = resolve;
    runtime.reject = reject;
  });
  // Callers may observe job failures without awaiting the initialization promise.
  runtime.ready.catch(() => {});
  runtime.startup = setTimeout(() => {
    shutdown(runtime, JSON.stringify(failure('Load', 'Worker startup timed out')));
  }, STARTUP_TIMEOUT_MS);
  runtime.timers.add(runtime.startup);
  worker.onmessage = ({data}) => receiveRuntime(runtime, data);
  worker.onerror = event => {
    event.preventDefault();
    shutdown(runtime, JSON.stringify(failure(runtime.loaded ? 'Crashed' : 'Load', event.message)));
  };
  worker.onmessageerror = () => shutdown(runtime, JSON.stringify(failure('Decode', 'worker message could not be decoded')));
  try {
    worker.postMessage({initialize: {version: 1, pool: runtime.pool, generation: runtime.generation, base: artifact.base, threads, active, capacity}});
  } catch (error) {
    shutdown(runtime, JSON.stringify(failure('Load', error)));
    throw error;
  }
  return runtime;
}
function receiveRuntime(runtime, data) {
  if (data.spawnCompute) {
    spawnCompute(runtime, data.spawnCompute);
    return;
  }
  if (data.initializationError) {
    shutdown(runtime, JSON.stringify(data.initializationError));
    return;
  }
  if (data.ready === 1) {
    clearTimeout(runtime.startup);
    runtime.timers.delete(runtime.startup);
    runtime.loaded = true;
    try {
      for (const frame of runtime.queued.splice(0)) runtime.worker.postMessage(frame);
      runtime.resolve();
    } catch (error) {
      shutdown(runtime, JSON.stringify(failure('Load', error)));
    }
  } else if (data.closed !== undefined) {
    const closing = runtime.closes.get(data.closed);
    runtime.closes.delete(data.closed);
    if (closing) {
      clearTimeout(closing.timer);
      data.error ? closing.reject(JSON.stringify(data.error)) : closing.resolve();
    }
    if (!data.closed || !runtime.pool) shutdown(runtime, JSON.stringify(data.error || 'Closed'));
  } else if (data.id !== undefined) {
    if (data.event.Progress) {
      const old = runtime.progress.get(data.id);
      if (old?.event.Progress.Shared) send(runtime, {type: 'Lease', retain: false, shared: old.event.Progress.Shared});
      runtime.progress.set(data.id, data);
      if (!runtime.progressTimer) runtime.progressTimer = setTimeout(() => {
        runtime.progressTimer = null;
        for (const id of runtime.progress.keys()) deliverProgress(runtime, id);
      }, 16);
    } else {
      deliverProgress(runtime, data.id);
      if (!runtime.error) runtime.callback(JSON.stringify(data));
    }
  }
}
function deliverProgress(runtime, id) {
  const update = runtime.progress.get(id);
  runtime.progress.delete(id);
  if (update && !runtime.error) runtime.callback(JSON.stringify(update));
}
export function lease(pool, retain, shared) {
  for (const runtime of runtimes) if (runtime.pool === pool) {
    send(runtime, {type: 'Lease', retain, shared: JSON.parse(shared)});
    return;
  }
}
export const command = (runtime, encoded) => send(runtime, JSON.parse(encoded));
function send(runtime, frame) {
  if (runtime.error) throw Error(JSON.stringify(runtime.error));
  if (frame.type === 'Cancel' && frame.id === 0) {
    shutdown(runtime, '"Cancelled"');
    return;
  }
  if (frame.type === 'Cancel' && !runtime.loaded) {
    const index = runtime.queued.findIndex(queued => queued.type === 'Call' && queued.id === frame.id);
    if (index >= 0) {
      const [call] = runtime.queued.splice(index, 1);
      releaseQueued(runtime, call);
      return;
    }
  }
  if (frame.type === 'Call' && !runtime.loaded && runtime.queued.filter(frame => frame.type === 'Call').length >= runtime.capacity + runtime.active) {
    releaseQueued(runtime, frame);
    runtime.callback(JSON.stringify({id: frame.id, event: errorEvent({QueueFull: {capacity: runtime.capacity}})}));
    return;
  }
  if (runtime.loaded) runtime.worker.postMessage(frame);
  else runtime.queued.push(frame);
}
function releaseQueued(runtime, call) {
  for (const payload of call.arguments) if (payload.Shared) runtime.queued.push({type: 'Lease', retain: false, shared: payload.Shared});
}
export function shutdown(runtime, encoded) {
  if (runtime.error) return;
  const error = JSON.parse(encoded);
  runtime.error = error;
  runtimes.delete(runtime);
  runtime.callback(JSON.stringify({id: 0, event: errorEvent(error)}));
  clearTimeout(runtime.progressTimer);
  runtime.worker.onmessage = runtime.worker.onerror = runtime.worker.onmessageerror = null;
  runtime.worker.terminate();
  for (const worker of runtime.computes) worker.terminate();
  runtime.computes.length = 0;
  for (const timer of runtime.timers) clearTimeout(timer);
  runtime.timers.clear();
  runtime.reject(encoded);
  for (const closing of runtime.closes.values()) {
    clearTimeout(closing.timer);
    closing.reject(encoded);
  }
  runtime.closes.clear();
  runtime.queued.length = 0;
  runtime.progress.clear();
}
export function close_runtime(runtime, service, timeout) {
  service = Number(service);
  if (runtime.closes.has(service)) return runtime.closes.get(service).promise;
  if (runtime.error) return runtime.error === 'Closed' ? Promise.resolve() : Promise.reject(JSON.stringify(runtime.error));
  const closing = {};
  closing.promise = new Promise((resolve, reject) => {
    closing.resolve = resolve;
    closing.reject = reject;
  });
  // A close deadline may expire before its Rust future is polled again.
  closing.promise.catch(() => {});
  closing.timer = setTimeout(() => {
    runtime.closes.delete(service);
    closing.reject('"CloseTimedOut"');
    if (!service || !runtime.pool) shutdown(runtime, '"CloseTimedOut"');
    else send(runtime, {type: 'CloseTimeout', instance: service});
  }, timeout);
  runtime.closes.set(service, closing);
  send(runtime, {type: 'Close', instance: service});
  return closing.promise;
}

let host;
export function emit(id, event) {
  id = Number(id);
  const operation = host.running.get(id);
  if (!operation) return;
  const value = JSON.parse(event);
  if (operation.cancelled) {
    discardEvent(value);
    return;
  }
  if (value.Item) operation.reservation = null;
  postMessage({id, event: value});
}
export function reserve(id) {
  const operation = host.running.get(Number(id));
  if (!operation || operation.cancelled) return Promise.reject('Cancelled');
  // StreamSender is exclusive: at most one send can wait for this operation.
  const reservation = operation.reservation = {granted: false};
  const promise = new Promise((resolve, reject) => {
    reservation.resolve = resolve;
    reservation.reject = reject;
  });
  if (operation.credits > 0) {
    operation.credits--;
    credit(Number(id));
  }
  return promise;
}
export function return_credit(id) {
  id = Number(id);
  const operation = host.running.get(id);
  const reservation = operation?.reservation;
  if (!reservation) return;
  operation.reservation = null;
  if (reservation.granted) credit(id);
  else reservation.reject('Cancelled');
}
function credit(id) {
  const operation = host.running.get(id);
  if (!operation) return;
  const reservation = operation.reservation;
  if (reservation && !reservation.granted) {
    reservation.granted = true;
    reservation.resolve();
  }
  else operation.credits++;
}
function release(payload) {
  if (payload?.Instance) host.app.__fusor_worker_drop_service(BigInt(payload.Instance));
  if (payload?.Shared) host.app.__fusor_worker_lease(false, JSON.stringify(payload.Shared));
}
function discardResult(result) {
  if (result.Ok) release(result.Ok);
  else if (result.Err?.Application) release(result.Err.Application);
}
function discardEvent(event) {
  if (event.Item) release(event.Item);
  if (event.Progress) release(event.Progress);
  if (event.Result) discardResult(event.Result);
  if (event.End?.Err?.Application) release(event.End.Err.Application);
}
function cancel(operation) {
  operation.cancelled = true;
  host.app.__fusor_worker_cancel(BigInt(operation.id));
  operation.reservation?.reject('Cancelled');
  operation.reservation = null;
}
export function start_worker(app, encoded) {
  const configuration = JSON.parse(encoded);
  const [version, entries] = JSON.parse(app.__fusor_worker_manifest());
  if (version !== 1 || configuration.version !== version) throw JSON.stringify('IncompatibleArtifact');
  const registry = new Map();
  for (const entry of entries) {
    if (registry.has(entry.id)) throw JSON.stringify('IncompatibleArtifact');
    registry.set(entry.id, entry);
  }
  host = {app, configuration, registry, waiting: [], running: new Map(), busy: new Set(), services: new Set(), closing: new Set(), async: 0, cpu: 0, allClosed: false};
  globalThis.__fusor_worker_base_url = configuration.base;
  app.__fusor_worker_initialize(configuration.pool, configuration.generation, configuration.threads, configuration.capacity);
  onmessage = ({data}) => receive(data);
  postMessage({ready: 1});
}
function receive(frame) {
  switch (frame.type) {
    case 'Call':
      admit(frame);
      break;
    case 'Cancel': {
      const queued = host.waiting.findIndex(op => op.id === frame.id);
      if (queued >= 0) {
        const [operation] = host.waiting.splice(queued, 1);
        operation.arguments.forEach(release);
        postMessage({id: frame.id, event: errorEvent('Cancelled')});
      } else {
        const operation = host.running.get(frame.id);
        if (operation) cancel(operation);
      }
      drain();
      break;
    }
    case 'Credit':
      credit(frame.id);
      break;
    case 'Dispose':
      stopService(frame.instance, 'OwnerDisposed');
      break;
    case 'Lease':
      host.app.__fusor_worker_lease(frame.retain, JSON.stringify(frame.shared));
      break;
    case 'Close':
      host.services.delete(frame.instance);
      host.closing.add(frame.instance);
      if (!frame.instance) host.allClosed = true;
      finishCloses();
      break;
    case 'CloseTimeout':
      stopService(frame.instance, 'CloseTimedOut');
      break;
  }
}
function reject(operation, error) {
  operation.arguments.forEach(release);
  postMessage({id: operation.id, event: errorEvent(error)});
}
function admit(operation) {
  const registration = host.registry.get(operation.entry);
  if (!registration) {
    reject(operation, 'IncompatibleArtifact');
    return;
  }
  if (host.allClosed || (operation.instance && !host.services.has(operation.instance))) {
    reject(operation, 'Closed');
    return;
  }
  if (registration.pool && !host.configuration.pool) {
    reject(operation, 'PoolRequired');
    return;
  }
  operation.cpu = !!host.configuration.pool && registration.kind === 'sync';
  if (canRun(operation)) run(operation);
  else if (host.waiting.length >= host.configuration.capacity) reject(operation, {QueueFull: {capacity: host.configuration.capacity}});
  else host.waiting.push(operation);
}
function canRun(operation) {
  if (operation.instance && host.busy.has(operation.instance)) return false;
  if (!host.configuration.pool) return host.running.size === 0;
  return operation.cpu ? host.cpu < host.configuration.threads : host.async < host.configuration.active;
}
function drain() {
  for (let index = 0; index < host.waiting.length;) {
    const operation = host.waiting[index];
    if (!canRun(operation)) {
      index++;
      continue;
    }
    host.waiting.splice(index, 1);
    run(operation);
  }
  finishCloses();
}
async function run(operation) {
  operation.cancelled = false;
  operation.credits = operation.capacity;
  if (!host.tick) host.tick = setInterval(() => host.app.__fusor_worker_tick(), 16);
  host.running.set(operation.id, operation);
  if (operation.instance) host.busy.add(operation.instance);
  operation.cpu ? host.cpu++ : host.async++;
  try {
    let result = JSON.parse(await host.app.__fusor_worker_call(BigInt(operation.id), operation.entry, JSON.stringify(operation.arguments), BigInt(operation.instance), operation.batch_bytes));
    if (operation.cancelled) {
      discardResult(result);
      result = {Err: {Worker: 'Cancelled'}};
    }
    if (operation.stream) {
      if (result.Ok) release(result.Ok);
      postMessage({id: operation.id, event: {End: result.Err ? {Err: result.Err} : {Ok: null}}});
    } else {
      if (result.Ok?.Instance) host.services.add(result.Ok.Instance);
      postMessage({id: operation.id, event: {Result: result}});
    }
  } catch (error) {
    // A Wasm trap invalidates the complete runtime. Propagate to its Worker error handler.
    setTimeout(() => {
      throw error;
    });
  } finally {
    host.running.delete(operation.id);
    if (operation.instance) host.busy.delete(operation.instance);
    operation.cpu ? host.cpu-- : host.async--;
    drain();
    if (!host.running.size) {
      clearInterval(host.tick);
      host.tick = null;
    }
  }
}
function finishCloses() {
  for (const service of host.closing) {
    const pending = [...host.running.values(), ...host.waiting].some(op => !service || op.instance === service);
    if (pending) continue;
    host.closing.delete(service);
    if (service) host.app.__fusor_worker_drop_service(BigInt(service));
    postMessage({closed: service});
    if (!service) {
      clearInterval(host.tick);
      self.close();
    }
  }
}

function stopService(instance, error) {
  host.services.delete(instance);
  host.waiting = host.waiting.filter(op => {
    if (op.instance !== instance) return true;
    op.arguments.forEach(release);
    postMessage({id: op.id, event: errorEvent(error)});
    return false;
  });
  for (const operation of host.running.values()) {
    if (operation.instance === instance) {
      cancel(operation);
      postMessage({id: operation.id, event: errorEvent(error)});
    }
  }
  host.closing.add(instance);
  finishCloses();
}

async function spawnCompute(runtime, request) {
  try {
    await Promise.all(Array.from({length: request.count}, () => new Promise((resolve, reject) => {
      const worker = new Worker(request.url, {type: 'module', name: 'fusor-compute'});
      runtime.computes.push(worker);
      const timer = setTimeout(() => reject(Error('Compute worker startup timed out')), STARTUP_TIMEOUT_MS);
      runtime.timers.add(timer);
      worker.onerror = event => {
        event.preventDefault();
        clearTimeout(timer);
        reject(Error(event.message));
        shutdown(runtime, JSON.stringify(failure(runtime.loaded ? 'Crashed' : 'Load', event.message)));
      };
      worker.onmessageerror = () => {
        clearTimeout(timer);
        reject(Error('Compute initialization could not be decoded'));
      };
      worker.onmessage = ({data}) => {
        if (data?.type === 'fusor-rayon-ready') {
          clearTimeout(timer);
          resolve();
        }
        if (data?.type === 'fusor-rayon-error') {
          clearTimeout(timer);
          reject(Error(data.message));
          shutdown(runtime, JSON.stringify(failure(runtime.loaded ? 'Crashed' : 'Load', data.message)));
        }
      };
      worker.postMessage({type: 'fusor-rayon-init', module: request.module, memory: request.memory, receiver: request.receiver, mainJS: request.mainJS});
    })));
    request.port.postMessage({ready: true});
  } catch (error) {
    request.port.postMessage({error: String(error)});
    shutdown(runtime, JSON.stringify(failure('Load', error)));
  } finally {
    request.port.close();
  }
}
