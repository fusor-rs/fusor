// Lifecycle adapter for wasm-bindgen-rayon 1.3.0's public PoolBuilder ABI.
// The UI owns every physical worker, so a crashed or blocked coordinator cannot
// prevent whole-pool termination or partial-startup cleanup.
self.addEventListener('message', ({data}) => {
  if (data?.type !== 'fusor-rayon-init') return;
  initialize(data).catch(error => {
    postMessage({type: 'fusor-rayon-error', message: String(error)});
    self.close();
  });
});
async function initialize(data) {
  const app = await import(data.mainJS);
  await app.default({module_or_path: data.module, memory: data.memory});
  postMessage({type: 'fusor-rayon-ready'});
  app.wbg_rayon_start_worker(data.receiver);
}
export async function startWorkers(module, memory, builder) {
  const channel = new MessageChannel();
  try {
    await new Promise((resolve, reject) => {
      channel.port1.onmessage = ({data}) => data.ready ? resolve() : reject(Error(data.error));
      postMessage({spawnCompute: {
        url: import.meta.url, count: builder.numThreads(), module, memory,
        receiver: builder.receiver(), mainJS: builder.mainJS(), port: channel.port2,
      }}, [channel.port2]);
    });
    builder.build();
  } finally { channel.port1.close(); }
}
