// SharedWorker bootstrap for craft-runtime (craft-libs#5).
// Routes HostMessage JSON on each MessagePort via craft-worker-host-routing.js.
const connections = new Set();

try {
  importScripts('craft-worker-host-routing.js');
} catch {
  // Node protocol smoke tests load routing directly without importScripts.
}

try {
  importScripts('opfs-flush.js');
} catch {
  // Optional warm-tier flush hook; tests may assign craftRuntimeFlushPending manually.
}

self.onconnect = (event) => {
  const port = event.ports[0];
  connections.add(port);
  const route =
    typeof handleHostMessage === 'function'
      ? handleHostMessage
      : globalThis.craftWorkerHostHandleMessage;
  port.onmessage = (ev) => route(port, ev.data);
  port.start();
};
