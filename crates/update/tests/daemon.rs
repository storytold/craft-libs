//! `craft-updated`'s request handling, end to end except for the outside world: GitHub, SMTP
//! and the build are stand-ins, so these tests prove the authentication and update logic, not
//! the network. The last test drives the real HTTP listener over a loopback socket.

use craft_update::mail::{Email, Mailer};
use craft_update::password::{MIN_ITERATIONS, PasswordHash};
use craft_update::protocol::{Job, LoginReply, Status};
use craft_update::release::Release;
use craft_update::server::config::{Admin, Build, Security, Smtp};
use craft_update::server::http::{Request, Response};
use craft_update::server::{Config, Daemon, LOCKOUT_SECONDS, Parts, ReleaseSource, Runner, SESSION_SECONDS, State};
use craft_update::{Error, Version};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PASSWORD: &str = "correct horse battery";

#[derive(Clone, Default)]
struct World {
    release: Arc<Mutex<Option<Release>>>,
    mail: Arc<Mutex<Vec<Email>>>,
    mail_fails: Arc<Mutex<bool>>,
    runs: Arc<Mutex<Vec<String>>>,
    run_fails: Arc<Mutex<bool>>,
    /// While set, the runner waits (to observe a running update).
    hold: Arc<Mutex<bool>>,
    clock: Arc<AtomicU64>,
    counter: Arc<AtomicUsize>,
}

impl World {
    fn new() -> World {
        let w = World::default();
        w.clock.store(1_760_000_000, Ordering::SeqCst);
        w.publish("v0.6.1");
        w
    }

    fn publish(&self, tag: &str) {
        let version = Version::parse(tag).unwrap();
        let url = Some(format!("https://github.com/storytold/gridcraft/releases/tag/{tag}"));
        *self.release.lock().unwrap() = Some(Release { tag: tag.into(), version, url, published_at: None });
    }

    fn advance(&self, seconds: u64) {
        self.clock.fetch_add(seconds, Ordering::SeqCst);
    }

    fn emails(&self) -> Vec<Email> {
        self.mail.lock().unwrap().clone()
    }

    fn parts(&self) -> Parts {
        let (w1, w2, w3, w4, w5) = (self.clone(), self.clone(), self.clone(), self.clone(), self.clone());
        Parts {
            source: Box::new(Source(w1)),
            mailer: Box::new(Recorder(w2)),
            runner: Box::new(FakeRunner(w3)),
            clock: Box::new(move || w4.clock.load(Ordering::SeqCst)),
            random: Box::new(move |buf: &mut [u8]| {
                let n = w5.counter.fetch_add(1, Ordering::SeqCst);
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = (n + i) as u8;
                }
                Ok(())
            }),
        }
    }
}

struct Source(World);
impl ReleaseSource for Source {
    fn latest(&self) -> Result<Release, Error> {
        self.0.release.lock().unwrap().clone().ok_or_else(|| Error::Release("offline".into()))
    }
}

struct Recorder(World);
impl Mailer for Recorder {
    fn send(&self, email: &Email) -> Result<(), Error> {
        if *self.0.mail_fails.lock().unwrap() {
            return Err(Error::Mail("smtp down".into()));
        }
        self.0.mail.lock().unwrap().push(email.clone());
        Ok(())
    }
}

struct FakeRunner(World);
impl Runner for FakeRunner {
    fn run(&self, release: &Release, log: &std::fs::File) -> Result<(), Error> {
        self.0.runs.lock().unwrap().push(release.tag.clone());
        let started = Instant::now();
        while *self.0.hold.lock().unwrap() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut log = log;
        writeln!(log, "$ trunk build --release").unwrap();
        if *self.0.run_fails.lock().unwrap() {
            writeln!(log, "error[E0425]: cannot find value `x` in this scope").unwrap();
            return Err(Error::Update("trunk failed".into()));
        }
        writeln!(log, "built {}", release.version).unwrap();
        Ok(())
    }
}

struct Site {
    _dir: tempfile::TempDir,
    config: Config,
    config_path: PathBuf,
}

fn site() -> Site {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("Cargo.toml"), "[workspace.package]\nversion = \"0.4.0\"\n").unwrap();
    let config = Config {
        name: "GridCraft".into(),
        repository: "storytold/gridcraft".into(),
        listen: "127.0.0.1:0".into(),
        site_url: Some("https://example.org/gridcraft/".into()),
        check_interval_minutes: 360,
        state_file: None,
        admin: Admin {
            username: "admin".into(),
            password_hash: PasswordHash::with_iterations(PASSWORD, b"0123456789abcdef", MIN_ITERATIONS).unwrap().to_string(),
            email: "admin@example.org".into(),
        },
        smtp: Smtp {
            host: "smtp.example.org".into(),
            port: 587,
            security: Security::Starttls,
            username: Some("updates".into()),
            password: Some("smtp secret".into()),
            from: "GridCraft <updates@example.org>".into(),
        },
        build: Build {
            source_dir: source,
            remote: "origin".into(),
            work_dir: "apps/gridcraft-web".into(),
            command: vec!["trunk".into(), "build".into(), "--release".into()],
            output_dir: "dist/web".into(),
            releases_dir: dir.path().join("releases"),
            live_link: dir.path().join("live"),
            keep_releases: 3,
            timeout_minutes: 60,
        },
    };
    let config_path = dir.path().join("gridcraft.toml");
    config.save(&config_path).unwrap();
    Site { _dir: dir, config, config_path }
}

fn daemon(site: &Site, world: &World) -> Arc<Daemon> {
    Daemon::new(site.config.clone(), &site.config_path, world.parts()).unwrap()
}

fn get(d: &Arc<Daemon>, path: &str) -> Response {
    d.handle(&Request { method: "GET".into(), path: path.into(), ..Request::default() })
}

fn post(d: &Arc<Daemon>, path: &str, authorization: Option<&str>, body: &str) -> Response {
    d.handle(&Request {
        method: "POST".into(),
        path: path.into(),
        authorization: authorization.map(str::to_string),
        content_type: Some("application/json".into()),
        body: body.as_bytes().to_vec(),
    })
}

fn login(d: &Arc<Daemon>, username: &str, password: &str) -> Response {
    post(d, "/api/login", None, &serde_json::json!({"username": username, "password": password}).to_string())
}

fn token(d: &Arc<Daemon>) -> String {
    let r = login(d, "admin", PASSWORD);
    assert_eq!(r.status, 200, "{}", r.body);
    serde_json::from_str::<LoginReply>(&r.body).unwrap().token
}

fn status(d: &Arc<Daemon>) -> Status {
    let r = get(d, "/api/status");
    assert_eq!(r.status, 200);
    serde_json::from_str(&r.body).unwrap()
}

fn wait_for_job(d: &Arc<Daemon>) -> Status {
    let started = Instant::now();
    loop {
        let s = status(d);
        if !matches!(s.job, Job::Running { .. }) || started.elapsed() > Duration::from_secs(10) {
            return s;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_status_is_public_and_reports_a_newer_release_after_a_check() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    let s = status(&d);
    assert_eq!((s.current.as_str(), s.latest.as_deref(), s.update_available), ("0.4.0", None, false));
    d.check().unwrap();
    let s = status(&d);
    assert_eq!((s.current.as_str(), s.latest.as_deref(), s.update_available), ("0.4.0", Some("0.6.1"), true));
    assert_eq!(s.release_url.as_deref(), Some("https://github.com/storytold/gridcraft/releases/tag/v0.6.1"));
    // A proxy that doesn't strip its prefix reaches the same routes.
    assert_eq!(get(&d, "/gridcraft/craft-update/api/status").status, 200);
    assert_eq!(get(&d, "/api/nope").status, 404);
    assert_eq!(post(&d, "/api/status", None, "").status, 405);
}

#[test]
fn a_new_release_is_emailed_once_even_across_restarts_and_a_newer_one_again() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    d.check().unwrap();
    d.check().unwrap();
    assert_eq!(world.emails().len(), 1);
    assert_eq!(world.emails()[0].subject, "GridCraft 0.6.1 is available (this site runs 0.4.0)");
    let restarted = daemon(&site, &world);
    restarted.check().unwrap();
    assert_eq!(world.emails().len(), 1, "a restart must not repeat the notice");
    world.publish("v0.7.0");
    restarted.check().unwrap();
    assert_eq!(world.emails().len(), 2);
    assert!(world.emails()[1].subject.starts_with("GridCraft 0.7.0 is available"));
}

#[test]
fn a_notice_that_could_not_be_sent_is_tried_again() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    *world.mail_fails.lock().unwrap() = true;
    d.check().unwrap();
    assert!(world.emails().is_empty());
    *world.mail_fails.lock().unwrap() = false;
    d.check().unwrap();
    assert_eq!(world.emails().len(), 1);
}

#[test]
fn updating_and_checking_need_an_administrator_session() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    d.check().unwrap();
    let basic = format!("Basic {}", "YWRtaW46Y29ycmVjdCBob3JzZSBiYXR0ZXJ5");
    for auth in [None, Some(""), Some("Bearer"), Some("Bearer "), Some("Bearer not-a-session"), Some(basic.as_str()), Some(PASSWORD)] {
        for route in ["/api/update", "/api/check", "/api/logout"] {
            let r = post(&d, route, auth, "");
            assert_eq!(r.status, 401, "{route} with {auth:?}: {}", r.body);
        }
    }
    assert!(world.runs.lock().unwrap().is_empty(), "nothing may run without a session");
}

#[test]
fn sign_in_refuses_wrong_credentials_and_malformed_requests() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    assert_eq!(login(&d, "admin", "wrong").status, 401);
    assert_eq!(login(&d, "Admin", PASSWORD).status, 401);
    assert_eq!(login(&d, "admin ", PASSWORD).status, 401);
    assert_eq!(login(&d, "", "").status, 401);
    assert_eq!(post(&d, "/api/login", None, "").status, 400);
    assert_eq!(post(&d, "/api/login", None, r#"{"username":"admin"}"#).status, 400);
    let form = d.handle(&Request {
        method: "POST".into(),
        path: "/api/login".into(),
        content_type: Some("application/x-www-form-urlencoded".into()),
        body: format!("username=admin&password={PASSWORD}").into_bytes(),
        ..Request::default()
    });
    assert_eq!(form.status, 415, "a cross-site form post can't sign in");
    let r = login(&d, "admin", PASSWORD);
    assert_eq!(r.status, 200);
    let reply: LoginReply = serde_json::from_str(&r.body).unwrap();
    assert_eq!(reply.token.len(), 64);
    assert_eq!(reply.expires_in, SESSION_SECONDS);
}

#[test]
fn the_administrator_can_update_to_the_newest_release_and_nothing_else() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    d.check().unwrap();
    let auth = format!("Bearer {}", token(&d));
    // Whatever the request asks for, the update goes to the release the daemon found.
    let r = post(&d, "/api/update", Some(&auth), r#"{"tag":"v0.1.0","command":["sh","-c","id"]}"#);
    assert_eq!(r.status, 202, "{}", r.body);
    let s = wait_for_job(&d);
    assert_eq!(*world.runs.lock().unwrap(), ["v0.6.1"]);
    assert!(matches!(&s.job, Job::Succeeded { version, .. } if version == "0.6.1"), "{s:?}");
    assert_eq!((s.current.as_str(), s.update_available), ("0.6.1", false));
    let last = world.emails().pop().unwrap();
    assert_eq!(last.subject, "GridCraft was updated to 0.6.1");
    assert!(last.body.contains("from 0.4.0 to 0.6.1"));
    assert_eq!(State::load(&site.config_path.with_extension("state.toml")).unwrap().deployed.as_deref(), Some("0.6.1"));
    let again = post(&d, "/api/update", Some(&auth), "");
    assert_eq!(again.status, 409, "{}", again.body);
    assert_eq!(world.runs.lock().unwrap().len(), 1);
}

#[test]
fn a_failed_update_keeps_the_old_version_and_emails_the_log_but_tells_visitors_no_details() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    d.check().unwrap();
    *world.run_fails.lock().unwrap() = true;
    let auth = format!("Bearer {}", token(&d));
    assert_eq!(post(&d, "/api/update", Some(&auth), "").status, 202);
    let s = wait_for_job(&d);
    assert!(matches!(&s.job, Job::Failed { version, .. } if version == "0.6.1"), "{s:?}");
    assert_eq!((s.current.as_str(), s.update_available), ("0.4.0", true));
    let last = world.emails().pop().unwrap();
    assert_eq!(last.subject, "GridCraft update to 0.6.1 failed");
    assert!(last.body.contains("cannot find value `x`"), "{}", last.body);
    let public = get(&d, "/api/status").body;
    assert!(!public.contains("cannot find") && !public.contains("trunk"), "{public}");
}

#[test]
fn a_second_update_while_one_runs_is_refused() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    d.check().unwrap();
    *world.hold.lock().unwrap() = true;
    let auth = format!("Bearer {}", token(&d));
    assert_eq!(post(&d, "/api/update", Some(&auth), "").status, 202);
    assert!(matches!(status(&d).job, Job::Running { .. }));
    let second = post(&d, "/api/update", Some(&auth), "");
    assert_eq!(second.status, 409);
    assert!(second.body.contains("already running"));
    *world.hold.lock().unwrap() = false;
    wait_for_job(&d);
    assert_eq!(world.runs.lock().unwrap().len(), 1);
}

#[test]
fn repeated_failed_sign_ins_lock_sign_in_for_a_while() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    for _ in 0..5 {
        assert_eq!(login(&d, "admin", "guess").status, 401);
    }
    let locked = login(&d, "admin", PASSWORD);
    assert_eq!(locked.status, 429, "even the right password waits out the lock");
    assert!(locked.body.contains("15 minutes"), "{}", locked.body);
    world.advance(LOCKOUT_SECONDS + 1);
    assert_eq!(login(&d, "admin", PASSWORD).status, 200);
}

#[test]
fn sessions_expire_and_end_at_sign_out() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    let auth = format!("Bearer {}", token(&d));
    assert!(d.authorized(Some(&auth)));
    world.advance(SESSION_SECONDS + 1);
    assert!(!d.authorized(Some(&auth)));
    assert_eq!(post(&d, "/api/check", Some(&auth), "").status, 401);
    let auth = format!("Bearer {}", token(&d));
    assert_eq!(post(&d, "/api/logout", Some(&auth), "").status, 204);
    assert!(!d.authorized(Some(&auth)));
    // Sessions live in the process: a restart ends them.
    let auth = format!("Bearer {}", token(&d));
    assert!(!daemon(&site, &world).authorized(Some(&auth)));
}

#[test]
fn the_administrator_can_ask_for_a_check_now() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    let auth = format!("Bearer {}", token(&d));
    let r = post(&d, "/api/check", Some(&auth), "");
    assert_eq!(r.status, 200);
    assert!(serde_json::from_str::<Status>(&r.body).unwrap().update_available);
    *world.release.lock().unwrap() = None;
    assert_eq!(post(&d, "/api/check", Some(&auth), "").status, 502);
}

#[test]
fn configuration_files_must_be_private_and_listen_on_loopback() {
    let site = site();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&site.config_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(Config::load(&site.config_path).unwrap(), site.config);
        std::fs::set_permissions(&site.config_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = Config::load(&site.config_path).unwrap_err().to_string();
        assert!(err.contains("chmod 600"), "{err}");
    }
    let mut c = site.config.clone();
    c.listen = "0.0.0.0:8790".into();
    assert!(c.validate().unwrap_err().to_string().contains("loopback"));
    let mut c = site.config.clone();
    c.smtp.security = Security::None;
    assert!(c.validate().is_err(), "an unencrypted SMTP password may only go to this machine");
    c.smtp.host = "localhost".into();
    assert!(c.validate().is_ok());
    let mut c = site.config.clone();
    c.build.output_dir = "../../etc".into();
    assert!(c.validate().is_err());
    let mut c = site.config.clone();
    c.build.live_link = c.build.source_dir.join("live");
    assert!(c.validate().is_err());
    let mut c = site.config.clone();
    c.admin.password_hash = "plaintext-password".into();
    assert!(c.validate().is_err(), "the password must be stored hashed");
    let mut c = site.config.clone();
    c.admin.email = "admin@example.org\r\nBcc: x@example.org".into();
    assert!(c.validate().is_err());
}

#[test]
fn the_http_listener_enforces_the_same_rules_over_a_real_socket() {
    let (site, world) = (site(), World::new());
    let d = daemon(&site, &world);
    d.check().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Arc::clone(&d);
    std::thread::spawn(move || craft_update::server::http::serve(&server, &listener));
    let call = |raw: String| {
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };
    let status = call("GET /api/status HTTP/1.1\r\nHost: x\r\n\r\n".into());
    assert!(status.starts_with("HTTP/1.1 200 OK\r\n") && status.contains("\"latest\":\"0.6.1\""), "{status}");
    let refused = call("POST /api/update HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer forged\r\nContent-Length: 0\r\n\r\n".into());
    assert!(refused.starts_with("HTTP/1.1 401 "), "{refused}");
    let body = serde_json::json!({"username": "admin", "password": PASSWORD}).to_string();
    let signed_in =
        call(format!("POST /api/login HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()));
    assert!(signed_in.starts_with("HTTP/1.1 200 OK\r\n"), "{signed_in}");
    let token = serde_json::from_str::<LoginReply>(signed_in.split("\r\n\r\n").nth(1).unwrap()).unwrap().token;
    let started = call(format!("POST /api/update HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"));
    assert!(started.starts_with("HTTP/1.1 202 "), "{started}");
    wait_for_job(&d);
    assert_eq!(*world.runs.lock().unwrap(), ["v0.6.1"]);
}
