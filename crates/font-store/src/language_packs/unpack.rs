//! Reading a Language pack archive with the checks a hostile archive needs.

use super::*;

/// The zstd frames of `archive` one after another, as one stream (`zstd` writes one frame, but
/// a concatenation of frames is a valid `.zst` too, and must not stop after the first).
struct Frames<'a> {
    decoder: Option<ruzstd::decoding::StreamingDecoder<&'a [u8], ruzstd::decoding::FrameDecoder>>,
    /// Frames started so far, empty ones included (at most [`MAX_PACK_FRAMES`]).
    frames: usize,
}

/// Most zstd frames in a pack archive (`zstd` writes one; empty frames count too).
pub(super) const MAX_PACK_FRAMES: usize = 4096;

impl<'a> Frames<'a> {
    fn new(archive: &'a [u8]) -> Result<Self, String> {
        Ok(Self { decoder: Some(ruzstd::decoding::StreamingDecoder::new(archive).map_err(|e| format!("not zstd: {e}"))?), frames: 1 })
    }
}

impl Read for Frames<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let Some(d) = self.decoder.as_mut() else { return Ok(0) };
            let n = d.read(buf)?;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            let rest: &[u8] = d.get_ref();
            if !rest.is_empty() {
                self.frames += 1;
                if self.frames > MAX_PACK_FRAMES {
                    return Err(std::io::Error::other(format!("more than {MAX_PACK_FRAMES} zstd frames")));
                }
            }
            self.decoder = if rest.is_empty() { None } else { Some(ruzstd::decoding::StreamingDecoder::new(rest).map_err(std::io::Error::other)?) };
        }
    }
}

/// Unpacks the `.tar.zst` of `pack` into `dest` (created). Only the contract's files are allowed:
/// `pack.toml`, `spelling.aff`, `spelling.dic`, `hyphenation.pat` at the root and
/// `LICENCES/<name>` ([`licence_name`]); regular files and that folder only, each file at most
/// [`MAX_PACK_FILE_BYTES`], all of them [`MAX_PACK_UNPACKED_BYTES`], at most [`MAX_PACK_ENTRIES`].
/// Every zstd frame is read, and nothing but tar padding may follow the archive's end. `pack.toml`
/// must name `pack`'s locale, and the files the index promises (`spelling`, `hyphenation`) must
/// be there.
pub(super) fn unpack(archive: &[u8], dest: &Path, pack: &PackEntry) -> Result<(), String> {
    // The decompressed stream is capped too: tar padding and headers beyond the files' bytes.
    let cap = MAX_PACK_UNPACKED_BYTES.saturating_add((MAX_PACK_ENTRIES as u64 + 2) * 1024 + 20 * 512);
    let mut tar = tar::Archive::new(Frames::new(archive)?.take(cap));
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let (mut total, mut count) = (0u64, 0usize);
    let mut written: Vec<String> = Vec::new();
    for entry in tar.entries().map_err(|e| format!("not a tar archive: {e}"))? {
        let mut entry = entry.map_err(|e| format!("damaged archive: {e}"))?;
        count += 1;
        if count > MAX_PACK_ENTRIES {
            return Err("too many files".into());
        }
        let raw = entry.path().map_err(|e| format!("bad file name: {e}"))?.into_owned();
        let rel = safe_entry_path(&raw).ok_or_else(|| format!("{} is outside the pack", raw.display()))?;
        let parts: Vec<&str> = rel.iter().filter_map(|c| c.to_str()).collect();
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            match parts.as_slice() {
                [] => {}
                [LICENCES_DIR] => std::fs::create_dir_all(dest.join(LICENCES_DIR)).map_err(|e| e.to_string())?,
                _ => return Err(format!("{}: not a folder a pack has", rel.display())),
            }
            continue;
        }
        if !kind.is_file() {
            return Err(format!("{}: only files and folders are allowed", rel.display()));
        }
        let allowed = match parts.as_slice() {
            [name] => ROOT_FILES.contains(name),
            [LICENCES_DIR, name] => licence_name(name),
            _ => false,
        };
        let key = parts.join("/");
        if !allowed || written.contains(&key) {
            return Err(format!("{}: not a file a pack has, or listed twice", rel.display()));
        }
        let size = entry.header().size().map_err(|e| e.to_string())?;
        if size > MAX_PACK_FILE_BYTES {
            return Err(format!("{}: larger than {MAX_PACK_FILE_BYTES} bytes", rel.display()));
        }
        total = total.saturating_add(size);
        if total > MAX_PACK_UNPACKED_BYTES {
            return Err(format!("larger than {MAX_PACK_UNPACKED_BYTES} bytes unpacked"));
        }
        let path = dest.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = std::fs::File::options().write(true).create_new(true).open(&path).map_err(|e| format!("{}: {e}", rel.display()))?;
        let copied = std::io::copy(&mut (&mut entry).take(MAX_PACK_FILE_BYTES), &mut out).map_err(|e| format!("{}: {e}", rel.display()))?;
        if copied != size {
            return Err(format!("{}: truncated", rel.display()));
        }
        written.push(key);
    }
    // Only zeros (the tar end blocks and padding) may follow: not a second archive, not a frame
    // that wasn't read.
    let mut rest = tar.into_inner();
    let mut buf = [0u8; 8192];
    loop {
        let n = rest.read(&mut buf).map_err(|e| format!("damaged archive: {e}"))?;
        if n == 0 {
            break;
        }
        if buf.get(..n).is_some_and(|b| b.iter().any(|&x| x != 0)) {
            return Err("data after the end of the archive".into());
        }
    }
    let has = |f: &str| written.iter().any(|w| w == f);
    if !has("pack.toml") {
        return Err("no pack.toml".into());
    }
    let toml = std::fs::read_to_string(dest.join("pack.toml")).map_err(|e| format!("pack.toml: {e}"))?;
    let named = toml.parse::<toml::Table>().ok().and_then(|t| t.get("locale").and_then(toml::Value::as_str).map(str::to_string));
    if named.as_deref() != Some(pack.locale.as_str()) {
        return Err(format!("pack.toml names locale {named:?}, not {:?}", pack.locale));
    }
    if pack.spelling && !(has("spelling.aff") && has("spelling.dic")) {
        return Err("the index promises a spelling dictionary the archive doesn't have".into());
    }
    if pack.hyphenation && !has("hyphenation.pat") {
        return Err("the index promises hyphenation patterns the archive doesn't have".into());
    }
    Ok(())
}

/// The files a pack holds at its root.
const ROOT_FILES: [&str; 4] = ["pack.toml", "spelling.aff", "spelling.dic", "hyphenation.pat"];
/// The folder of a pack's licence texts.
const LICENCES_DIR: &str = "LICENCES";

/// A licence file's name: `[A-Za-z0-9._-]`, at most 100 characters, not starting with a dot or
/// ending with one, and not a name Windows reserves.
fn licence_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 100
        && !n.starts_with('.')
        && !n.ends_with('.')
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !windows_reserved(n)
}

/// `p` as a plain relative path (`./` prefixes dropped), or `None` for a root, a drive, `..` or a
/// name that isn't UTF-8 or holds a backslash, colon or control character.
fn safe_entry_path(p: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(n) => {
                let n = n.to_str()?;
                if n.is_empty() || n.contains(['\\', ':']) || n.chars().any(char::is_control) {
                    return None;
                }
                out.push(n);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}
