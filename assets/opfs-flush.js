// Warm-tier OPFS flush for craft-runtime (craft-libs#4).
// Maps StoreKey strings to paths under effectcraft-store/v1/…
const WARM_ROOT = 'effectcraft-store/v1';

function hasOpfs() {
  return (
    typeof navigator !== 'undefined' &&
    navigator.storage &&
    typeof navigator.storage.getDirectory === 'function'
  );
}

function keyToSegments(key) {
  const trimmed = String(key).replace(/^\/+/, '');
  return [WARM_ROOT, ...trimmed.split('/').filter(Boolean)];
}

async function ensureParentDir(root, segments) {
  let dir = root;
  for (let i = 0; i < segments.length - 1; i += 1) {
    dir = await dir.getDirectoryHandle(segments[i], { create: true });
  }
  return dir;
}

async function writeWarmKey(root, key, bytes) {
  const segments = keyToSegments(key);
  const fileName = segments[segments.length - 1];
  const dir = await ensureParentDir(root, segments);
  const handle = await dir.getFileHandle(fileName, { create: true });
  const writable = await handle.createWritable();
  const payload =
    bytes instanceof Uint8Array
      ? bytes
      : bytes instanceof ArrayBuffer
        ? new Uint8Array(bytes)
        : new Uint8Array(bytes);
  await writable.write(payload);
  await writable.close();
}

/**
 * Default SharedWorker hook: persist pending flush entries to OPFS.
 * @param {{ pending?: Array<{ key: string, bytes: number[]|Uint8Array }> }} data
 * @returns {Promise<{ pending_count: number }>}
 */
async function craftRuntimeFlushPending(data) {
  const pending = data && Array.isArray(data.pending) ? data.pending : [];
  if (pending.length === 0) {
    return { pending_count: 0 };
  }
  if (!hasOpfs()) {
    return { pending_count: pending.length };
  }
  const root = await navigator.storage.getDirectory();
  for (const entry of pending) {
    if (!entry || typeof entry.key !== 'string') {
      continue;
    }
    const raw = entry.bytes;
    const bytes = Array.isArray(raw) ? Uint8Array.from(raw) : raw;
    await writeWarmKey(root, entry.key, bytes);
  }
  return { pending_count: pending.length };
}

if (typeof globalThis !== 'undefined') {
  globalThis.craftRuntimeFlushPending =
    globalThis.craftRuntimeFlushPending || craftRuntimeFlushPending;
  globalThis.craftOpfsKeyToSegments = keyToSegments;
  globalThis.craftOpfsWarmRoot = WARM_ROOT;
}
