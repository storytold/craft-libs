#![cfg(unix)]

use craft_atomic::{NewFilePermissions, WriteOptions, atomic_write, write_new};
use std::fs::{self, File, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> io::Result<Self> {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("craft-permissions-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn mode(path: &Path) -> io::Result<u32> {
    Ok(fs::metadata(path)?.permissions().mode() & 0o777)
}

/// Set umask only in a separate shell process; changing it in a parallel test process races.
fn child(dir: &Path, mask: &str, expected: &str, check_acl: bool) -> io::Result<()> {
    let output = Command::new("sh")
        .args(["-c", "umask \"$1\"; exec \"$2\" --exact permission_child --nocapture", "permissions", mask])
        .arg(std::env::current_exe()?)
        .env("CRAFT_PERMISSION_TEST_DIR", dir)
        .env("CRAFT_PERMISSION_TEST_EXPECTED", expected)
        .env("CRAFT_PERMISSION_TEST_ACL", if check_acl { "1" } else { "0" })
        .output()?;
    assert!(output.status.success(), "umask {mask}:\n{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "child must execute its test");
    Ok(())
}

#[test]
fn normal_creation_matches_os_permissions() -> io::Result<()> {
    for (mask, expected) in [("000", "666"), ("002", "664"), ("022", "644"), ("027", "640"), ("077", "600")] {
        let dir = TempDir::new()?;
        child(&dir.0, mask, expected, false)?;
    }
    Ok(())
}

#[test]
fn permission_child() -> io::Result<()> {
    let Some(dir) = std::env::var_os("CRAFT_PERMISSION_TEST_DIR") else { return Ok(()) };
    let dir = Path::new(&dir);
    let expected = u32::from_str_radix(&std::env::var("CRAFT_PERMISSION_TEST_EXPECTED").map_err(io::Error::other)?, 8).map_err(io::Error::other)?;
    let ordinary = dir.join("ordinary");
    fs::write(&ordinary, b"ordinary")?;
    assert_eq!(mode(&ordinary)?, expected);
    let options = WriteOptions::default();
    let payload = b"document bytes";
    for (tag, streaming, exclusive) in
        [("replace", false, false), ("replace-stream", true, false), ("exclusive", false, true), ("exclusive-stream", true, true)]
    {
        let path = dir.join(tag);
        let fill = |f: &mut File| {
            assert_eq!(f.metadata()?.permissions().mode() & 0o777, expected, "normal creation before writing");
            f.write_all(payload)?;
            assert_eq!(f.metadata()?.permissions().mode() & 0o777, expected, "normal creation during writing");
            Ok(())
        };
        match (streaming, exclusive) {
            (false, false) => options.atomic_write(&path, payload)?,
            (false, true) => options.write_new(&path, payload)?,
            (true, false) => options.atomic_write_with(&path, fill)?,
            (true, true) => options.write_new_with(&path, fill)?,
        }
        assert_eq!(mode(&path)?, expected, "{tag}");
        assert_eq!(fs::read(&path)?, payload);
        if std::env::var("CRAFT_PERMISSION_TEST_ACL").as_deref() == Ok("1") {
            let acl = |p: &Path| -> io::Result<Vec<u8>> {
                let output = Command::new("getfacl").args(["-c", "-n"]).arg(p).output()?;
                assert!(output.status.success());
                Ok(output.stdout)
            };
            assert_eq!(acl(&path)?, acl(&ordinary)?, "same inherited ACL for {tag}");
        }
        if exclusive {
            assert_eq!(options.write_new(&path, b"must not replace").map_err(|e| e.kind()), Err(io::ErrorKind::AlreadyExists));
            assert_eq!(fs::read(&path)?, payload);
        } else {
            fs::set_permissions(&path, Permissions::from_mode(0o660))?;
            options.atomic_write(&path, b"replacement")?;
            assert_eq!(mode(&path)?, 0o660, "preserve existing mode regardless of current umask");
        }
    }
    for (tag, write) in [("default", atomic_write as fn(&Path, &[u8]) -> io::Result<()>), ("default-exclusive", write_new)] {
        let p = dir.join(tag);
        write(&p, b"document")?;
        assert_eq!(mode(&p)?, expected);
    }
    let private = WriteOptions::default().with_new_file_permissions(NewFilePermissions::Private);
    for exclusive in [false, true] {
        let p = dir.join(format!("private-{exclusive}"));
        let fill = |f: &mut File| {
            assert_eq!(f.metadata()?.permissions().mode() & 0o077, 0, "private from creation");
            f.write_all(b"private")
        };
        if exclusive {
            private.write_new_with(&p, fill)?;
        } else {
            private.atomic_write_with(&p, fill)?;
        }
        assert_eq!(mode(&p)?, 0o600);
    }
    for exclusive in [false, true] {
        let path = dir.join(format!("callback-error-{exclusive}"));
        let fill = |f: &mut File| {
            f.write_all(b"partial")?;
            Err(io::Error::from(io::ErrorKind::StorageFull))
        };
        let result = if exclusive { options.write_new_with(&path, fill) } else { options.atomic_write_with(&path, fill) };
        assert_eq!(result.map_err(|e| e.kind()), Err(io::ErrorKind::StorageFull));
        assert!(!path.exists());
    }
    assert!(fs::read_dir(dir)?.all(|e| e.is_ok_and(|e| !e.file_name().to_string_lossy().ends_with(".craft_tmp"))));
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires setfacl/getfacl and a filesystem with POSIX ACL support"]
fn directory_default_acl_takes_precedence_over_restrictive_umask() -> io::Result<()> {
    let dir = TempDir::new()?;
    let output = Command::new("setfacl").args(["-m", "d:u::rwx,d:u:12345:r--,d:g::r-x,d:m::r-x,d:o::---"]).arg(&dir.0).output()?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    // With this default ACL, ordinary creation yields 0640 even under umask 077.
    child(&dir.0, "077", "640", true)
}
