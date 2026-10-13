#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! `craft-updated`: the optional update service for a self-hosted web deployment of a Craft
//! app. See the craft-update README for setup.

use craft_update::password::{PasswordHash, SALT_LEN};
use craft_update::server::config::{Admin, Build, Security, Smtp, source_version};
use craft_update::server::{Config, Daemon, Parts, http, os_random};
use craft_update::{Error, mail};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
craft-updated: update notices and admin-triggered updates for a self-hosted Craft web app

Usage: craft-updated <command> --config <file>

Commands:
  setup          Ask for the settings and write the configuration file (run it again to change them)
  serve          Answer the app's requests on the loopback address and check GitHub on a timer
  check          Ask GitHub for the newest release now (emails the administrator about a new one)
  update         Update to the newest release now, from the command line
  test-email     Send a test email to the administrator
  password       Change the administrator password
  hash-password  Read a password from standard input and print its hash (for configuration tools)

Options:
  -c, --config <file>  The configuration file (kept private: chmod 600)
  -h, --help           This help
  -V, --version        The version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("craft-updated: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), Error> {
    let mut command = None;
    let mut config_path = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("craft-updated {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-c" | "--config" => config_path = Some(PathBuf::from(it.next().ok_or_else(|| usage("--config needs a file"))?)),
            other if other.starts_with('-') => return Err(usage(&format!("unknown option {other}"))),
            other if command.is_none() => command = Some(other.to_string()),
            other => return Err(usage(&format!("unexpected argument {other}"))),
        }
    }
    let command = command.ok_or_else(|| usage("no command given"))?;
    if command == "hash-password" {
        let password = read_line("")?;
        println!("{}", new_hash(&password)?);
        return Ok(());
    }
    let config_path = config_path.ok_or_else(|| usage("--config <file> is required"))?;
    match command.as_str() {
        "setup" => setup(&config_path),
        "password" => {
            let mut config = Config::load(&config_path)?;
            config.admin.password_hash = ask_new_password()?.to_string();
            config.save(&config_path)?;
            println!("The administrator password was changed. Restart craft-updated to use it.");
            Ok(())
        }
        "test-email" => {
            let config = Config::load(&config_path)?;
            let parts = Parts::real(&config)?;
            parts.mailer.send(&mail::test(&config.name))?;
            println!("Sent a test email to {}.", config.admin.email);
            Ok(())
        }
        "check" => {
            let daemon = daemon(&config_path)?;
            let status = daemon.check()?;
            match (status.update_available, status.latest) {
                (true, Some(latest)) => println!("{} {latest} is available; this site runs {}.", daemon.config().name, status.current),
                _ => println!("{} {} is the newest release.", daemon.config().name, status.current),
            }
            Ok(())
        }
        "update" => {
            let daemon = daemon(&config_path)?;
            let version = daemon.update_now()?;
            println!("{} {version} is live (log: {}).", daemon.config().name, daemon.log_path().display());
            Ok(())
        }
        "serve" => serve(&config_path),
        other => Err(usage(&format!("unknown command {other}"))),
    }
}

fn usage(message: &str) -> Error {
    Error::Config(format!("{message}\n\n{USAGE}"))
}

fn daemon(config_path: &Path) -> Result<std::sync::Arc<Daemon>, Error> {
    let config = Config::load(config_path)?;
    let parts = Parts::real(&config)?;
    Daemon::new(config, config_path, parts)
}

fn serve(config_path: &Path) -> Result<(), Error> {
    let daemon = daemon(config_path)?;
    let addr = daemon.config().listen_addr()?;
    let listener = std::net::TcpListener::bind(addr).map_err(|e| Error::Config(format!("couldn't listen on {addr}: {e}")))?;
    eprintln!("craft-updated {}: {} ({}) on http://{addr}/", env!("CARGO_PKG_VERSION"), daemon.config().name, daemon.config().repository);
    let checker = std::sync::Arc::clone(&daemon);
    std::thread::spawn(move || checker.run_checks());
    http::serve(&daemon, &listener);
    Err(Error::Config("the listener stopped".into()))
}

fn new_hash(password: &str) -> Result<PasswordHash, Error> {
    let mut salt = [0u8; SALT_LEN];
    os_random(&mut salt)?;
    PasswordHash::new(password, &salt)
}

fn ask_new_password() -> Result<PasswordHash, Error> {
    loop {
        let first = rpassword::prompt_password("Administrator password: ").map_err(|e| Error::Config(format!("reading the password: {e}")))?;
        if first.chars().count() < 10 {
            eprintln!("Use at least 10 characters: this password can start a rebuild of the site.");
            continue;
        }
        let again = rpassword::prompt_password("Again: ").map_err(|e| Error::Config(format!("reading the password: {e}")))?;
        if first == again {
            return new_hash(&first);
        }
        eprintln!("The passwords differ; try again.");
    }
}

fn read_line(prompt: &str) -> Result<String, Error> {
    if !prompt.is_empty() {
        print!("{prompt}");
        let _ = std::io::stdout().flush();
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| Error::Config(format!("reading standard input: {e}")))?;
    Ok(line.trim_end_matches(['\r', '\n']).to_string())
}

/// Asks a question with a default (shown in brackets, taken on an empty answer).
fn ask(question: &str, default: &str) -> Result<String, Error> {
    let answer = if default.is_empty() { read_line(&format!("{question}: "))? } else { read_line(&format!("{question} [{default}]: "))? };
    Ok(if answer.trim().is_empty() { default.to_string() } else { answer.trim().to_string() })
}

fn setup(config_path: &Path) -> Result<(), Error> {
    let old = if config_path.exists() { Some(Config::load(config_path)?) } else { None };
    println!("Setting up craft-updated in {}.", config_path.display());
    println!("This is for a self-hosted web deployment of a Craft app; press Enter to keep a [default].\n");

    let repository = ask("GitHub repository", old.as_ref().map_or("storytold/gridcraft", |c| &c.repository))?;
    let slug = repository.rsplit('/').next().unwrap_or("app").to_string();
    let default_name = match slug.as_str() {
        "gridcraft" => "GridCraft".to_string(),
        "wordcraft" => "WordCraft".to_string(),
        "deckcraft" => "DeckCraft".to_string(),
        other => other.to_string(),
    };
    let name = ask("App name, for the emails", old.as_ref().map_or(&default_name, |c| &c.name))?;
    let site_url = ask("The site's address, for the emails (optional)", old.as_ref().and_then(|c| c.site_url.as_deref()).unwrap_or(""))?;
    let listen = ask("Loopback address to listen on", old.as_ref().map_or("127.0.0.1:8790", |c| &c.listen))?;

    println!("\nAdministrator: the only account that can start an update from the site.");
    let username = ask("Administrator username", old.as_ref().map_or("admin", |c| &c.admin.username))?;
    let password_hash = match &old {
        Some(c) if ask("Keep the current administrator password? (yes/no)", "yes")?.starts_with('y') => c.admin.password_hash.clone(),
        _ => ask_new_password()?.to_string(),
    };
    let email = ask("Administrator email (notices go here)", old.as_ref().map_or("", |c| &c.admin.email))?;

    println!("\nSMTP server for the notices.");
    let host = ask("SMTP host", old.as_ref().map_or("", |c| &c.smtp.host))?;
    let security = match ask(
        "Security: starttls, tls or none (none only for a server on this machine)",
        old.as_ref().map_or("starttls", |c| security_name(c.smtp.security)),
    )?
    .as_str()
    {
        "tls" => Security::Tls,
        "none" => Security::None,
        _ => Security::Starttls,
    };
    let default_port = match security {
        Security::Tls => 465,
        Security::Starttls => 587,
        Security::None => 25,
    };
    let port_text = ask("SMTP port", &old.as_ref().map_or(default_port, |c| c.smtp.port).to_string())?;
    let port = port_text.parse().map_err(|_| Error::Config(format!("{port_text:?} isn't a port")))?;
    let smtp_user = ask("SMTP username (empty for none)", old.as_ref().and_then(|c| c.smtp.username.as_deref()).unwrap_or(""))?;
    let smtp_password = if smtp_user.is_empty() {
        None
    } else {
        match old.as_ref().and_then(|c| c.smtp.password.clone()) {
            Some(p) if ask("Keep the current SMTP password? (yes/no)", "yes")?.starts_with('y') => Some(p),
            _ => Some(rpassword::prompt_password("SMTP password: ").map_err(|e| Error::Config(format!("reading the password: {e}")))?),
        }
    };
    let from = ask("Sender address", &old.as_ref().map_or(format!("{name} <{email}>"), |c| c.smtp.from.clone()))?;

    println!("\nBuilding. The updater keeps its own git clone, builds a release tag there and swaps a symbolic link.");
    let base = format!("/srv/{slug}");
    let b = old.as_ref().map(|c| c.build.clone());
    let source_dir =
        PathBuf::from(ask("Git clone used by the updater", &b.as_ref().map_or(format!("{base}/src"), |b| b.source_dir.display().to_string()))?);
    let work_dir =
        PathBuf::from(ask("Build folder inside it", &b.as_ref().map_or(format!("apps/{slug}-web"), |b| b.work_dir.display().to_string()))?);
    let command_text =
        ask("Build command", &b.as_ref().map_or("trunk build --release --features self-hosted-update".to_string(), |b| b.command.join(" ")))?;
    let output_dir =
        PathBuf::from(ask("Build output inside the clone", &b.as_ref().map_or("dist/web".to_string(), |b| b.output_dir.display().to_string()))?);
    let releases_dir =
        PathBuf::from(ask("Folder for built releases", &b.as_ref().map_or(format!("{base}/releases"), |b| b.releases_dir.display().to_string()))?);
    let live_link =
        PathBuf::from(ask("Symbolic link the web server serves", &b.as_ref().map_or(format!("{base}/live"), |b| b.live_link.display().to_string()))?);

    let config = Config {
        name,
        repository,
        listen,
        site_url: (!site_url.is_empty()).then_some(site_url),
        check_interval_minutes: old.as_ref().map_or(360, |c| c.check_interval_minutes),
        state_file: old.as_ref().and_then(|c| c.state_file.clone()),
        admin: Admin { username, password_hash, email },
        smtp: Smtp { host, port, security, username: (!smtp_user.is_empty()).then_some(smtp_user), password: smtp_password, from },
        build: Build {
            source_dir,
            remote: b.as_ref().map_or_else(|| "origin".into(), |b| b.remote.clone()),
            work_dir,
            command: command_text.split_whitespace().map(str::to_string).collect(),
            output_dir,
            releases_dir,
            live_link,
            keep_releases: b.as_ref().map_or(3, |b| b.keep_releases),
            timeout_minutes: b.as_ref().map_or(60, |b| b.timeout_minutes),
        },
    };
    config.save(config_path)?;
    println!("\nWrote {} (mode 600).", config_path.display());
    match source_version(&config.build.source_dir) {
        Ok(v) => println!("The clone is at {v}; that is taken as the version the site runs until the first update."),
        Err(_) => println!(
            "Clone the repository into {} before starting: git clone https://github.com/{}.git {}",
            config.build.source_dir.display(),
            config.repository,
            config.build.source_dir.display()
        ),
    }
    println!("Next: craft-updated test-email --config {0}, then craft-updated serve --config {0}", config_path.display());
    Ok(())
}

fn security_name(security: Security) -> &'static str {
    match security {
        Security::Starttls => "starttls",
        Security::Tls => "tls",
        Security::None => "none",
    }
}
