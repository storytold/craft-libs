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
//   // Flush pending cold writes (called by storage_flush for cold-tier keys):
//   await craftFsAccessFlushPending(volumeHandleId, pending);

const FS_ACCESS_VOLUMES_PREFIX = 'volumes';

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
 * Read bytes for a key from an ExternalVolume. Returns null if not found.
 * @param {FileSystemDirectoryHandle} rootHandle
 * @param {string} volumeHandleId
 * @param {string} key
 * @returns {Promise<Uint8Array|null>}
 */
async function craftFsAccessReadKey(rootHandle, volumeHandleId, key) {
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
    return { pending_count: pending.length };
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
