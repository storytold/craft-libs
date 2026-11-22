//! `Http::new` and `install::check_download_link` as an app gets them: an integration test builds
//! the library without `cfg(test)`, so this is the release configuration (or the `local-mirror`
//! one, with that feature on).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_update::App;
use craft_update::http::{Fetched, Http};
use craft_update::install::check_download_link;
use craft_update::release::{self, Asset};

/// A plain-HTTP server on loopback that answers every request with `{}`; counts the requests.
fn serve() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(AtomicUsize::new(0));
    let count = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            count.fetch_add(1, Ordering::SeqCst);
            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}");
        }
    });
    (base, seen)
}

#[test]
fn the_remote_agents_take_plain_http_only_with_a_local_mirror() {
    let (base, seen) = serve();
    let http = Http::new("craft-update-tests/0").unwrap();
    // `get_api` and `redirect_target` always use the agents for remote hosts.
    let api = http.get_api(&format!("{base}/repos/storytold/x/releases"), None);
    let redirect = http.redirect_target(&format!("{base}/releases/latest"));
    if cfg!(feature = "local-mirror") {
        assert!(matches!(api, Ok(Fetched::Body { .. })), "{api:?}");
        assert!(matches!(redirect, Err(ref e) if e.contains("HTTP 200")), "{redirect:?}");
        assert_eq!(seen.load(Ordering::SeqCst), 2);
    } else {
        assert!(api.as_ref().is_err_and(|e| e.contains("refused plain HTTP")), "{api:?}");
        assert!(redirect.as_ref().is_err_and(|e| e.contains("refused plain HTTP")), "{redirect:?}");
        // An upper-case scheme isn't a loopback link, so it goes to a remote agent too.
        assert!(http.get_text(&format!("HTTP{}", &base[4..])).is_err_and(|e| e.contains("refused plain HTTP")));
        assert_eq!(seen.load(Ordering::SeqCst), 0, "a refused request never connects");
    }
    // A loopback link goes to the direct agents, which serve plain HTTP in every build.
    assert_eq!(http.get_text(&format!("{base}/SHA256SUMS.txt")).unwrap().as_deref(), Some("{}"));
}

/// The crate's unit tests accept a loopback download link, as their downloads come from a server
/// on 127.0.0.1, so only a build without `cfg(test)` shows the rule an app ships with: a loopback
/// mirror's link, for the asset or for the `SHA256SUMS.txt` it is checked against, is accepted
/// only with `local-mirror`, and the release's own links in every build.
#[test]
fn a_loopback_download_link_passes_only_with_a_local_mirror() {
    let app = App::by_id("photocraft").unwrap();
    let name = "photocraft-1.0.0-linux-x86_64.AppImage";
    let release = release::release_from_sums(app.repo, "v1.0.0", &[("a".repeat(64), name.into())]);
    let asset = release.assets.iter().find(|a| a.name == name).unwrap().clone();
    assert!(release.sums_asset().is_some(), "the release lists the SHA256SUMS.txt the download is checked against");
    assert!(check_download_link(app, &release, &asset).is_ok());

    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let mirror = |file: &str| format!("http://{host}:8080/v1.0.0/{file}");
        let mirrored_asset = Asset { url: mirror(name), ..asset.clone() };
        let mut mirrored_sums = release.clone();
        for a in mirrored_sums.assets.iter_mut().filter(|a| a.name == "SHA256SUMS.txt") {
            a.url = mirror(&a.name);
        }
        let checks = [
            ("asset", check_download_link(app, &release, &mirrored_asset)),
            ("SHA256SUMS.txt", check_download_link(app, &mirrored_sums, &asset)),
            ("both", check_download_link(app, &mirrored_sums, &mirrored_asset)),
        ];
        for (mirrored, result) in checks {
            if cfg!(feature = "local-mirror") {
                assert!(result.is_ok(), "{host}, {mirrored} on the mirror: {result:?}");
            } else {
                let Err(err) = result else { panic!("{host}, {mirrored} on the mirror passed without local-mirror") };
                assert!(err.contains("isn't the one its release publishes"), "{host}, {mirrored}: {err}");
            }
        }
    }
}
