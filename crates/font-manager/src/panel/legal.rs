//! A family's licence and NOTICE texts, read through the store on request.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use craft_host::Host;

use crate::index::Family;
use crate::panel::app::{Detail, FontsPanel, family_key};
use crate::panel::jobs;
use crate::store::{LegalText, StoreError};

/// A licence or NOTICE text being read, or read.
pub(crate) enum Legal {
    Loading(Receiver<Result<Option<String>, StoreError>>),
    /// `None` when the family has no such file.
    Ready(Option<String>),
    Failed(String),
}

/// What a licence or NOTICE text is kept by: the family's key, which text, and the file's sha256 (else its path), so a
/// file an index refresh changed is read again.
pub(crate) type LegalKey = (String, LegalText, String);

pub(crate) type LegalTexts = HashMap<LegalKey, Legal>;

pub(crate) fn legal_key(family: &Family, which: LegalText) -> LegalKey {
    let file = match which {
        LegalText::License => family.license_file.as_ref(),
        LegalText::Notice => family.notice_file.as_ref(),
    };
    let identity = match (file, which) {
        (Some(file), _) => file.sha256.clone(),
        (None, LegalText::License) => family.license_path.clone().unwrap_or_default(),
        (None, LegalText::Notice) => String::new(),
    };
    (family_key(&family.source, &family.family), which, identity)
}

impl FontsPanel {
    /// Takes in the texts that were read.
    pub(crate) fn poll_legal(&mut self) {
        for state in self.legal.values_mut() {
            if let Legal::Loading(rx) = state {
                match rx.try_recv() {
                    Ok(Ok(text)) => *state = Legal::Ready(text),
                    Ok(Err(e)) => *state = Legal::Failed(e.0),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => *state = Legal::Ready(None),
                }
            }
        }
    }

    /// Reads the NOTICE of the family the details sheet shows once downloads are allowed: an Apache family's NOTICE is
    /// shown with its fonts.
    pub(crate) fn read_notice(&mut self, host: Option<&Arc<Host>>) {
        let Some((index, i)) = self.index().zip(self.detail_family()) else { return };
        let unread = index.families().get(i).is_some_and(|f| f.notice_file.is_some() && !self.legal.contains_key(&legal_key(f, LegalText::Notice)));
        if unread && self.allowed() {
            self.load_legal(host, i, LegalText::Notice);
        }
    }

    fn detail_family(&self) -> Option<usize> {
        match &self.detail {
            Some(Detail::Family(i)) => Some(*i),
            _ => None,
        }
    }

    /// Reads the licence or NOTICE text of the `i`th family through the store, as a job.
    pub(crate) fn load_legal(&mut self, host: Option<&Arc<Host>>, i: usize, which: LegalText) {
        let Some(index) = self.index() else { return };
        let Some(family) = index.families().get(i).cloned() else { return };
        let (tx, rx) = channel();
        self.legal.insert(legal_key(&family, which), Legal::Loading(rx));
        let installer = Arc::clone(&self.installer);
        let wake = self.jobs.waker();
        jobs::spawn(
            host,
            "font-licence",
            Box::new(move || {
                let _ = tx.send(installer.legal_text(&index, &family, which));
                wake.wake();
            }),
        );
    }
}
