//! The administrator's notification emails, and the transport they go through.
//!
//! Two notices are sent, in plain text:
//! - **A new release is out**, once per release: the first time a check finds a version newer
//!   than the one deployed. `craft-updated` remembers which version it announced, so restarts
//!   and later checks don't repeat it; a still newer release is announced again.
//! - **An update finished**, when one started from the site or the command line ends, whether it
//!   worked or not. A failure includes the end of the build log.
//!
//! Nothing is sent when an update starts: the administrator just asked for it.

use crate::{Error, Version};

/// A message to the administrator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Email {
    pub subject: String,
    pub body: String,
}

/// Sends [`Email`]s to the administrator. `craft-updated` uses SMTP; tests use a recorder.
pub trait Mailer: Send + Sync {
    fn send(&self, email: &Email) -> Result<(), Error>;
}

/// Lines of build log a failure email carries.
pub const LOG_TAIL_LINES: usize = 40;

/// "GridCraft 0.6.1 is available".
pub fn new_release(app: &str, current: &Version, latest: &Version, release_url: Option<&str>, site_url: Option<&str>) -> Email {
    let mut body = format!("{app} {latest} has been released. This site runs {current}.\n\n");
    if let Some(url) = release_url {
        body.push_str(&format!("Release notes: {url}\n\n"));
    }
    body.push_str("To update, sign in as the administrator from the update notice on the site");
    if let Some(site) = site_url {
        body.push_str(&format!(" ({site})"));
    }
    body.push_str(" and choose Update now, or run `craft-updated update` on the server.\n");
    body.push_str(FOOTER);
    Email { subject: format!("{app} {latest} is available (this site runs {current})"), body }
}

/// "GridCraft was updated to 0.6.1", or that it failed with the end of the log.
pub fn update_finished(app: &str, from: &Version, to: &Version, outcome: Result<(), &str>) -> Email {
    match outcome {
        Ok(()) => Email {
            subject: format!("{app} was updated to {to}"),
            body: format!(
                "{app} on this site was updated from {from} to {to}. Visitors get the new version when they next load or reload the page.\n{FOOTER}"
            ),
        },
        Err(log) => Email {
            subject: format!("{app} update to {to} failed"),
            body: format!(
                "Updating {app} from {from} to {to} failed. The site still serves {from}; nothing was swapped.\n\n\
                 The end of the update log:\n\n{}\n{FOOTER}",
                tail(log, LOG_TAIL_LINES)
            ),
        },
    }
}

/// What `craft-updated test-email` sends.
pub fn test(app: &str) -> Email {
    Email {
        subject: format!("{app} update notices: test message"),
        body: format!("This test message shows that {app}'s update service can email you.\n{FOOTER}"),
    }
}

const FOOTER: &str = "\n-- \nSent by craft-updated, the optional self-hosted update service of the Craft apps.\n";

/// The last `lines` lines of `text`.
pub fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all.get(all.len().saturating_sub(lines)..).unwrap_or_default().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(t: &str) -> Version {
        Version::parse(t).unwrap()
    }

    #[test]
    fn a_new_release_notice_names_both_versions_and_how_to_update() {
        let e = new_release(
            "GridCraft",
            &v("0.4.0"),
            &v("0.6.1"),
            Some("https://github.com/storytold/gridcraft/releases/tag/v0.6.1"),
            Some("https://example.org/gridcraft/"),
        );
        assert_eq!(e.subject, "GridCraft 0.6.1 is available (this site runs 0.4.0)");
        assert!(e.body.contains("GridCraft 0.6.1 has been released. This site runs 0.4.0."));
        assert!(e.body.contains("Release notes: https://github.com/storytold/gridcraft/releases/tag/v0.6.1"));
        assert!(e.body.contains("(https://example.org/gridcraft/)"));
        assert!(e.body.contains("craft-updated update"));
        let bare = new_release("WordCraft", &v("0.4.0"), &v("0.6.1"), None, None);
        assert!(!bare.body.contains("Release notes") && !bare.body.contains("()"));
    }

    #[test]
    fn finished_notices_report_success_and_failure_with_the_log_tail() {
        let ok = update_finished("DeckCraft", &v("0.4.0"), &v("0.6.1"), Ok(()));
        assert_eq!(ok.subject, "DeckCraft was updated to 0.6.1");
        let log: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let failed = update_finished("DeckCraft", &v("0.4.0"), &v("0.6.1"), Err(&log));
        assert_eq!(failed.subject, "DeckCraft update to 0.6.1 failed");
        assert!(failed.body.contains("still serves 0.4.0"));
        assert!(failed.body.contains("line 61\n") && failed.body.contains("line 100"));
        assert!(!failed.body.contains("line 60\n"));
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        assert_eq!(tail("a\nb\nc\n", 2), "b\nc");
        assert_eq!(tail("a", 5), "a");
        assert_eq!(tail("", 5), "");
    }
}
