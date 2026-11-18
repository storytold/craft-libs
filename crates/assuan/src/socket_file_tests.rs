use proptest::prelude::*;

use crate::socket_file::{MAX_FILE, SocketFile, SocketFileError, parse};

const NONCE: [u8; 16] = *b"\x00\x01\n\r%nonce-bytes";

fn file(port: &str) -> Vec<u8> {
    let mut f = format!("{port}\n").into_bytes();
    f.extend_from_slice(&NONCE);
    f
}

#[test]
fn the_loopback_port_and_nonce() {
    assert_eq!(parse(&file("51234")).unwrap(), SocketFile::Loopback { port: 51234, nonce: NONCE });
    assert_eq!(parse(&file("1")).unwrap(), SocketFile::Loopback { port: 1, nonce: NONCE });
}

#[test]
fn a_bad_port_or_nonce_is_refused() {
    for port in ["", "0", "65536", "123456", "-1", " 80", "8o"] {
        assert_eq!(parse(&file(port)), Err(SocketFileError::NoPort), "{port:?}");
    }
    assert_eq!(parse(b"51234"), Err(SocketFileError::NoPort));
    assert_eq!(parse(b"51234\nshort"), Err(SocketFileError::BadNonce));
    let mut long = file("51234");
    long.push(0);
    assert_eq!(parse(&long), Err(SocketFileError::BadNonce));
    assert_eq!(parse(&vec![b'1'; MAX_FILE + 1]), Err(SocketFileError::TooBig));
}

#[test]
fn the_nonce_is_never_shown() {
    let shown = format!("{:?}", parse(&file("51234")).unwrap());
    assert_eq!(shown, "Loopback { port: 51234, .. }");
}

#[test]
fn a_redirect() {
    assert_eq!(
        parse(b"%Assuan%\nsocket=/run/user/1000/gnupg/S.gpg-agent\n").unwrap(),
        SocketFile::Redirect("/run/user/1000/gnupg/S.gpg-agent".into())
    );
    let windows = parse(b"%Assuan%\r\nsocket=C:\\gnupg\\S.gpg-agent\r\n");
    let forward = parse(b"%Assuan%\nsocket=C:/gnupg/S.gpg-agent\n");
    if cfg!(windows) {
        assert_eq!(windows.unwrap(), SocketFile::Redirect("C:\\gnupg\\S.gpg-agent".into()));
        assert_eq!(forward.unwrap(), SocketFile::Redirect("C:/gnupg/S.gpg-agent".into()));
    } else {
        // A drive letter means nothing elsewhere: `C:/…` would be a relative name.
        assert_eq!(windows, Err(SocketFileError::BadRedirect));
        assert_eq!(forward, Err(SocketFileError::BadRedirect));
    }
    for bad in [
        &b"%Assuan%\n"[..],
        b"%Assuan%\nsocket=relative/path\n",
        b"%Assuan%\nsocket=${HOME}/S.gpg-agent\n",
        b"%Assuan%\nsocket=/a\x07b\n",
        b"%Assuan%\nsocket=\xff\n",
    ] {
        assert_eq!(parse(bad), Err(SocketFileError::BadRedirect), "{:?}", String::from_utf8_lossy(bad));
    }
}

proptest! {
    #[test]
    fn any_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let _ = parse(&bytes);
        let mut redirect = b"%Assuan%\n".to_vec();
        redirect.extend_from_slice(&bytes);
        let _ = parse(&redirect);
    }
}

#[test]
fn a_redirect_to_another_computer_is_refused() {
    for remote in [
        "//host/share/S.gpg-agent",
        "\\\\host\\share\\S.gpg-agent",
        "\\\\?\\C:\\S.gpg-agent",
        "/\\host\\share",
        "\\/host/share",
        "1:/x",
        "/home/x\\..\\y",
    ] {
        let file = format!("%Assuan%\nsocket={remote}\n");
        assert_eq!(parse(file.as_bytes()), Err(SocketFileError::BadRedirect), "{remote}");
    }
}
