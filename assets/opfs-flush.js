// Warm-tier OPFS flush for craft-runtime (craft-libs#4).
// Maps StoreKey strings to paths under <app-id>/store/v1/…, namespaced per app so two Craft
// apps sharing an origin never collide in OPFS. The app id comes from the `init` HostMessage;
// craft-worker-host-routing.js passes it explicitly on each flush call (`data.app_id`), falling
// back to globalThis.craftAppId only for a caller that predates that parameter. This file never
// hard-codes one app's name.
const FALLBACK_APP_SEGMENT = 'craft-app';
const WARM_ROOT_SUFFIX = ['store', 'v1'];

function hasOpfs() {
  return (
    typeof navigator !== 'undefined' &&
    navigator.storage &&
    typeof navigator.storage.getDirectory === 'function'
  );
}

/**
 * A single OPFS path SEGMENT may not be empty, `.`, `..`, or contain `/` (Chrome's
 * getDirectoryHandle throws "Name is not allowed" otherwise). Every byte outside
 * `[A-Za-z0-9_]` is hex-escaped as `~xx`, which makes the encoding injective: two different app
 * ids can never collide on the same segment. The lossy scheme this replaced (collapsing every
 * run of disallowed characters to a single `-`) sent "collab.photo" and "collab-photo" to the
 * same segment. `~` escaping guarantees the output never equals `.` or `..` and never contains
 * `/`, so only a genuinely empty app id needs the fallback segment.
 *
 * Must byte-for-byte match `sanitize_app_segment` in `crates/tier-store/src/coordinator.rs`: the
 * two run in different processes and must still compute the identical warm-tier path for the
 * same app id.
 */
function sanitizeAppSegment(appId) {
  const str = appId == null ? '' : String(appId);
  if (str.length === 0) {
    return FALLBACK_APP_SEGMENT;
  }
  const bytes = new TextEncoder().encode(str);
  let out = '';
  for (const byte of bytes) {
    const isDigit = byte >= 0x30 && byte <= 0x39;
    const isUpper = byte >= 0x41 && byte <= 0x5a;
    const isLower = byte >= 0x61 && byte <= 0x7a;
    const isUnderscore = byte === 0x5f;
    if (isDigit || isUpper || isLower || isUnderscore) {
      out += String.fromCharCode(byte);
    } else {
      out += `~${byte.toString(16).padStart(2, '0')}`;
    }
  }
  return out;
}

/** Warm-tier root as separate path segments -- each applied via its own getDirectoryHandle call. */
function warmRootSegments(appId) {
  const resolved = appId == null ? globalThis.craftAppId : appId;
  return [sanitizeAppSegment(resolved), ...WARM_ROOT_SUFFIX];
}

function keyToSegments(key, appId) {
  const trimmed = String(key).replace(/^\/+/, '');
  return [...warmRootSegments(appId), ...trimmed.split('/').filter(Boolean)];
}

async function ensureParentDir(root, segments) {
  let dir = root;
  for (let i = 0; i < segments.length - 1; i += 1) {
    dir = await dir.getDirectoryHandle(segments[i], { create: true });
  }
  return dir;
}

async function writeWarmKey(root, key, bytes, appId) {
  const segments = keyToSegments(key, appId);
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

/** True for the specific "it is not there" failure (a real `NotFoundError` DOMException in the
 * browser; the string the test doubles throw for the same condition). Any other error -- a
 * revoked permission, a locked file, quota, IO -- must not be treated as "already deleted". */
function isNotFoundError(err) {
  if (!err) {
    return false;
  }
  if (err.name === 'NotFoundError') {
    return true;
  }
  return typeof err.message === 'string' && err.message.includes('NotFoundError');
}

/** Remove a previously-persisted key's file. Missing already (never written, or already
 * removed by an earlier retry) counts as success: the end state -- no file -- is what a delete
 * wants either way. Any OTHER failure must reject: swallowing it here would let the flush hook
 * report the delete as persisted and the caller would drop the tombstone, while the real file
 * (and whatever actually went wrong) stays on disk. */
async function removeWarmKey(root, key, appId) {
  const segments = keyToSegments(key, appId);
  const fileName = segments[segments.length - 1];
  let dir = root;
  for (let i = 0; i < segments.length - 1; i += 1) {
    try {
      dir = await dir.getDirectoryHandle(segments[i]);
    } catch (err) {
      if (isNotFoundError(err)) {
        return; // a missing parent directory means the file was never written: already absent.
      }
      throw err;
    }
  }
  try {
    await dir.removeEntry(fileName);
  } catch (err) {
    if (!isNotFoundError(err)) {
      throw err;
    }
    // Already absent: nothing to do.
  }
}

/**
 * Default SharedWorker hook: persist pending flush entries to OPFS.
 * @param {{ pending?: Array<{ key: string, bytes: number[]|Uint8Array, deleted?: boolean }>, app_id?: string }} data
 * @returns {Promise<{ pending_count: number }>}
 */
async function craftRuntimeFlushPending(data) {
  const pending = data && Array.isArray(data.pending) ? data.pending : [];
  const appId = data && typeof data.app_id === 'string' ? data.app_id : undefined;
  if (pending.length === 0) {
    return { pending_count: 0 };
  }
  if (!hasOpfs()) {
    throw new Error('OPFS is unavailable');
  }
  const root = await navigator.storage.getDirectory();
  for (const entry of pending) {
    if (!entry || typeof entry.key !== 'string') {
      continue;
    }
    if (entry.deleted) {
      await removeWarmKey(root, entry.key, appId);
      continue;
    }
    const raw = entry.bytes;
    const bytes = Array.isArray(raw) ? Uint8Array.from(raw) : raw;
    await writeWarmKey(root, entry.key, bytes, appId);
  }
  return { pending_count: pending.length };
}

if (typeof globalThis !== 'undefined') {
  globalThis.craftRuntimeFlushPending =
    globalThis.craftRuntimeFlushPending || craftRuntimeFlushPending;
  globalThis.craftOpfsKeyToSegments = keyToSegments;
  globalThis.craftOpfsWarmRootSegments = warmRootSegments;
}
