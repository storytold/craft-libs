// DedicatedWorker fallback for craft-runtime when SharedWorker cannot host WebGPU.
// Same HostMessage / WorkerMessage protocol and hooks as craft-worker-host.js.

try {
  importScripts('craft-worker-host-routing.js');
} catch {
  // Node protocol smoke tests load routing directly without importScripts.
}

try {
  importScripts('opfs-flush.js');
} catch {
  // Optional warm-tier flush hook.
}

try {
  importScripts('fs-access-host.js');
} catch {
  // Optional cold-tier FS Access host; browser-only.
}

const route =
  typeof handleHostMessage === 'function'
    ? handleHostMessage
    : globalThis.craftWorkerHostHandleMessage;

self.onmessage = (event) => {
  if (typeof route === 'function') {
    route(self, event.data);
  }
};
