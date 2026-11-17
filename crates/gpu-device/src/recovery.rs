//! What to open after a device loss.
//!
//! A device is lost when the driver resets the GPU, the OS ends a long submission, the PC sleeps,
//! or the GPU is unplugged. The app drops everything it made with the device and opens a new one.
//! [`Recovery`] decides with what preference: the user's own, until the device has been lost
//! [`RecoveryPolicy::max_losses`] times within [`RecoveryPolicy::window`], and software after that,
//! so a GPU that keeps failing does not stop the app from drawing.

use std::collections::VecDeque;
use std::time::Duration;

use crate::preference::{Notice, Preference};

/// When repeated losses move drawing to software.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryPolicy {
    /// Losses within [`window`](Self::window) after which the next device is a software one.
    /// 0 never moves to software.
    pub max_losses: u32,
    pub window: Duration,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        RecoveryPolicy { max_losses: 3, window: Duration::from_secs(600) }
    }
}

/// The preference to open the next device with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryStep {
    pub preference: Preference,
    /// [`Notice::SoftwareAfterLosses`] when this step moved drawing to software.
    pub notice: Option<Notice>,
}

/// Device losses seen so far, against the user's preference.
#[derive(Clone, Debug)]
pub struct Recovery {
    preference: Preference,
    policy: RecoveryPolicy,
    /// When each loss inside the window happened, oldest first.
    losses: VecDeque<Duration>,
    software: bool,
}

impl Recovery {
    pub fn new(preference: Preference, policy: RecoveryPolicy) -> Self {
        Recovery { preference, policy, losses: VecDeque::new(), software: false }
    }

    /// The preference to open a device with now.
    pub fn preference(&self) -> &Preference {
        if self.software { &Preference::Software } else { &self.preference }
    }

    /// The user changed the preference: losses counted so far no longer apply.
    pub fn set_preference(&mut self, preference: Preference) {
        self.preference = preference;
        self.losses.clear();
        self.software = false;
    }

    /// Record a loss at `at`, a monotonic time the caller keeps (since the app started), and
    /// return what to open next. A time earlier than the last loss counts as the same time.
    pub fn lost(&mut self, at: Duration) -> RecoveryStep {
        let at = self.losses.back().map_or(at, |last| at.max(*last));
        self.losses.push_back(at);
        // Only the last `max_losses` losses decide anything, so no more are kept.
        let keep = usize::try_from(self.policy.max_losses.max(1)).unwrap_or(usize::MAX);
        while self.losses.len() > keep || self.losses.front().is_some_and(|first| at.saturating_sub(*first) > self.policy.window) {
            self.losses.pop_front();
        }
        let count = u32::try_from(self.losses.len()).unwrap_or(u32::MAX);
        let mut notice = None;
        if !self.software && self.policy.max_losses > 0 && count >= self.policy.max_losses && self.preference != Preference::Software {
            self.software = true;
            notice = Some(Notice::SoftwareAfterLosses(count));
        }
        RecoveryStep { preference: self.preference().clone(), notice }
    }

    /// Losses inside the window, as of the last one, counting at most
    /// [`RecoveryPolicy::max_losses`] (at least 1).
    pub fn recent_losses(&self) -> usize {
        self.losses.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(secs: u64) -> Duration {
        Duration::from_secs(secs)
    }

    #[test]
    fn the_third_loss_in_ten_minutes_moves_to_software() {
        let mut r = Recovery::new(Preference::Auto, RecoveryPolicy::default());
        assert_eq!(r.lost(s(10)), RecoveryStep { preference: Preference::Auto, notice: None });
        assert_eq!(r.lost(s(100)), RecoveryStep { preference: Preference::Auto, notice: None });
        assert_eq!(r.lost(s(300)), RecoveryStep { preference: Preference::Software, notice: Some(Notice::SoftwareAfterLosses(3)) });
        // it stays on software, and says so once
        assert_eq!(r.lost(s(400)), RecoveryStep { preference: Preference::Software, notice: None });
        assert_eq!(r.preference(), &Preference::Software);
    }

    #[test]
    fn losses_outside_the_window_are_forgotten() {
        let mut r = Recovery::new(Preference::LowPower, RecoveryPolicy::default());
        r.lost(s(0));
        r.lost(s(500));
        // the first loss is more than 600 s before this one
        assert_eq!(r.lost(s(601)).preference, Preference::LowPower);
        assert_eq!(r.recent_losses(), 2);
        // a clock that goes back counts as the last loss's time
        assert_eq!(r.lost(s(1)).preference, Preference::Software);
    }

    #[test]
    fn changing_the_preference_starts_over() {
        let mut r = Recovery::new(Preference::Auto, RecoveryPolicy { max_losses: 2, window: s(60) });
        r.lost(s(1));
        assert_eq!(r.lost(s(2)).preference, Preference::Software);
        r.set_preference(Preference::LowPower);
        assert_eq!(r.preference(), &Preference::LowPower);
        assert_eq!(r.recent_losses(), 0);
        assert_eq!(r.lost(s(3)).preference, Preference::LowPower);
    }

    #[test]
    fn software_and_a_zero_limit_never_add_a_notice() {
        let mut r = Recovery::new(Preference::Software, RecoveryPolicy::default());
        for t in 0..5 {
            assert_eq!(r.lost(s(t)), RecoveryStep { preference: Preference::Software, notice: None });
        }
        let mut never = Recovery::new(Preference::Auto, RecoveryPolicy { max_losses: 0, window: s(60) });
        for t in 0..10 {
            assert_eq!(never.lost(s(t)).preference, Preference::Auto);
        }
        assert_eq!(never.recent_losses(), 1);
    }

    #[test]
    fn the_loss_history_stays_bounded() {
        let mut r = Recovery::new(Preference::Software, RecoveryPolicy { max_losses: 3, window: Duration::MAX });
        for _ in 0..10_000 {
            r.lost(s(5));
        }
        assert_eq!(r.recent_losses(), 3);
    }
}
