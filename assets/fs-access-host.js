// Cold-tier File System Access host for craft-tier-store (craft-libs).
// Writes key data to a caller-supplied FileSystemDirectoryHandle.
//
// Path layout matches DefaultTierCoordinator::cold_relative_path:
//   volumes/{handle_id}/{key}
//
// OPFS (warm tier) is handled by opfs-flush.js. This file handles cold-tier
// ExternalVolume writes via the File System Access API (showDirectoryPicker).
//
// Worker usage:
//   importScripts('fs-access-host.js');
//   // Mount a volume from a picked handle:
//   craftFsAccessMountVolume(volumeHandleId, dirHandle);
//   // Flush pending cold writes for that volume (the caller selects which keys are cold and
//   // calls this directly -- craft-worker-host-routing.js's storage_flush does not call this on
//   // its own; cold-tier volume selection and flush routing are not wired into that dispatch):
//   await craftFsAccessFlushPending(volumeHandleId, pending);

const FS_ACCESS_VOLUMES_PREFIX = 'volumes';

// Bytes a single craftFsAccessReadKey call will read before refusing. A user-selected file has
// no budget check anywhere upstream of this host, so this is the only place that can stop a
// large media file from being pulled whole into an ArrayBuffer before the caller ever sees its
// size.
const DEFAULT_MAX_READ_BYTES = 256 * 1024 * 1024;

// Registry: handle_id -> FileSystemDirectoryHandle.
const volumeRegistry = new Map();

function hasFsAccess() {
  return typeof FileSystemDirectoryHandle !== 'undefined';
}

/**
 * Compute path segments for a key under a volume.
 * Matches DefaultTierCoordinator::cold_relative_path output split by '/'.
 * Config keys (config/*) are not stored cold -- returns empty array.
 */
function fsAccessKeySegments(volumeHandleId, key) {
  const trimmed = String(key).replace(/^\/+/, '');
  if (trimmed.startsWith('config/')) {
    return [];
  }
  const keyParts = trimmed.split('/').filter(Boolean);
  return [FS_ACCESS_VOLUMES_PREFIX, String(volumeHandleId), ...keyParts];
}

async function ensureFsDir(root, segments) {
  let dir = root;
  // Navigate to parent directory, creating intermediate dirs as needed.
  for (let i = 0; i < segments.length - 1; i += 1) {
    dir = await dir.getDirectoryHandle(segments[i], { create: true });
  }
  return dir;
}

/**
 * Register a FileSystemDirectoryHandle for a volume handle ID.
 * Call this after the user picks a directory (showDirectoryPicker).
 */
function craftFsAccessMountVolume(volumeHandleId, dirHandle) {
  volumeRegistry.set(String(volumeHandleId), dirHandle);
}

/**
 * Unregister a volume (e.g. when the user revokes access).
 */
function craftFsAccessUnmountVolume(volumeHandleId) {
  volumeRegistry.delete(String(volumeHandleId));
}

/**
 * Write bytes for a single key to an ExternalVolume.
 * @param {FileSystemDirectoryHandle} rootHandle - Volume root (from showDirectoryPicker).
 * @param {string} volumeHandleId - ID matching ExternalVolume.handle_id.
 * @param {string} key - StoreKey string.
 * @param {Uint8Array|number[]} bytes
 */
async function craftFsAccessWriteKey(rootHandle, volumeHandleId, key, bytes) {
  const segments = fsAccessKeySegments(volumeHandleId, key);
  if (segments.length === 0) {
    // Config keys are not stored cold; skip silently.
    return;
  }
  const fileName = segments[segments.length - 1];
  const dir = await ensureFsDir(rootHandle, segments);
  const fileHandle = await dir.getFileHandle(fileName, { create: true });
  const writable = await fileHandle.createWritable();
  const payload =
    bytes instanceof Uint8Array
      ? bytes
      : bytes instanceof ArrayBuffer
        ? new Uint8Array(bytes)
        : Uint8Array.from(bytes);
  await writable.write(payload);
  await writable.close();
}

/**
 * Read bytes for a key from an ExternalVolume. Returns null if not found. Refuses (throws)
 * rather than buffering the whole file when `file.size` exceeds `maxBytes`: a user-selected
 * file is unbounded, and checking the size before `arrayBuffer()` is the only way to catch an
 * oversized read before it already happened.
 * @param {FileSystemDirectoryHandle} rootHandle
 * @param {string} volumeHandleId
 * @param {string} key
 * @param {number} [maxBytes]
 * @returns {Promise<Uint8Array|null>}
 */
async function craftFsAccessReadKey(rootHandle, volumeHandleId, key, maxBytes = DEFAULT_MAX_READ_BYTES) {
  const segments = fsAccessKeySegments(volumeHandleId, key);
  if (segments.length === 0) {
    return null;
  }
  const fileName = segments[segments.length - 1];
  let dir = rootHandle;
  for (let i = 0; i < segments.length - 1; i += 1) {
    try {
      dir = await dir.getDirectoryHandle(segments[i]);
    } catch {
      return null;
    }
  }
  let fileHandle;
  try {
    fileHandle = await dir.getFileHandle(fileName);
  } catch {
    return null;
  }
  const file = await fileHandle.getFile();
  if (file.size > maxBytes) {
    throw new Error(`craft-fs-access: ${key} is ${file.size} bytes, over the ${maxBytes}-byte read budget`);
  }
  return new Uint8Array(await file.arrayBuffer());
}

/**
 * Flush a pending write batch to a named volume.
 * Uses volumeRegistry if rootHandle is not supplied.
 *
 * @param {string} volumeHandleId
 * @param {Array<{ key: string, bytes: number[]|Uint8Array }>} pending
 * @param {FileSystemDirectoryHandle} [rootHandle] - override registry lookup
 * @returns {Promise<{ pending_count: number }>}
 */
async function craftFsAccessFlushPending(volumeHandleId, pending, rootHandle) {
  const handle =
    rootHandle !== undefined
      ? rootHandle
      : volumeRegistry.get(String(volumeHandleId));

  if (!hasFsAccess() || !handle) {
    // Nothing was written: reporting any of `pending` as confirmed here would let a caller
    // that removes confirmed entries drop the only queued copy of data that was never actually
    // persisted. Zero means zero, until the destination is actually available.
    return { pending_count: 0 };
  }
  for (const entry of pending) {
    if (!entry || typeof entry.key !== 'string') {
      continue;
    }
    const raw = entry.bytes;
    const bytes = Array.isArray(raw) ? Uint8Array.from(raw) : raw;
    await craftFsAccessWriteKey(handle, volumeHandleId, entry.key, bytes);
  }
  return { pending_count: pending.length };
}

if (typeof globalThis !== 'undefined') {
  globalThis.craftFsAccessKeySegments = fsAccessKeySegments;
  globalThis.craftFsAccessVolumesPrefix = FS_ACCESS_VOLUMES_PREFIX;
  globalThis.craftFsAccessWriteKey = craftFsAccessWriteKey;
  globalThis.craftFsAccessReadKey = craftFsAccessReadKey;
  globalThis.craftFsAccessFlushPending = craftFsAccessFlushPending;
  globalThis.craftFsAccessMountVolume = craftFsAccessMountVolume;
  globalThis.craftFsAccessUnmountVolume = craftFsAccessUnmountVolume;
}
