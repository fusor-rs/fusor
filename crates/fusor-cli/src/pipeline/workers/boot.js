onmessage = async ({data}) => {
  if (!data.initialize) throw Error('Missing worker initialization');
  onmessage = null;
  const configuration = data.initialize;
  try {
  const app = await import('./pkg/app.js');
  await app.default();
  if (configuration.pool) {
    if (!crossOriginIsolated || typeof SharedArrayBuffer !== 'function') throw Error('Shared-memory workers require Cross-Origin-Opener-Policy: same-origin and Cross-Origin-Embedder-Policy: require-corp');
    if (typeof app.initThreadPool !== "function") throw JSON.stringify("IncompatibleArtifact");
    await app.initThreadPool(configuration.threads);
  }
  app.__fusor_worker_boot(app, JSON.stringify(configuration));
  } catch (error) {
    let failure;
    try { failure = JSON.parse(error); } catch { failure = {Load: {message: String(error)}}; }
    postMessage({initializationError: failure}); self.close();
  }
};
