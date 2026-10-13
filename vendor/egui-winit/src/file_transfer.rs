//! File Transfer portal reception (org.freedesktop.portal.FileTransfer v1).
//! Each viewport owns its pending drops. Retrieval runs off the UI thread; the offer
//! remains alive until retrieval completes, so the source cannot stop the transfer early.
use crate::dropped_file::NativeFile;
use core::time::Duration;
use std::{path::PathBuf, sync::mpsc, time::Instant};
use winit::{platform::wayland::WindowExtWayland as _, window::Window};
const MAX_PENDING: usize = 8;
const TIMEOUT: Duration = Duration::from_secs(10);
pub(super) struct PendingDrop {
    token: winit::platform::wayland::FileTransferToken,
    receiver: mpsc::Receiver<Result<Vec<PathBuf>, String>>,
    deadline: Instant,
}

pub(super) fn poll(window: &Window, ctx: &egui::Context, viewport: egui::ViewportId, pending: &mut Vec<PendingDrop>, input: &mut egui::RawInput) {
    for token in window.take_file_transfer_tokens() {
        if pending.len() >= MAX_PENDING {
            window.finish_file_transfer(token.id);
            log::warn!("too many pending file-transfer drops");
            continue;
        }
        let (sender, receiver) = mpsc::channel();
        let key = token.key.clone();
        let wake = ctx.clone();
        match std::thread::Builder::new().name("wayland-file-transfer".into()).spawn(move || {
            let result = retrieve(&key);
            let _ = sender.send(result);
            wake.request_repaint_of(viewport);
        }) {
            Ok(_) => pending.push(PendingDrop { token, receiver, deadline: Instant::now() + TIMEOUT }),
            Err(error) => {
                window.finish_file_transfer(token.id);
                log::warn!("could not start file-transfer retrieval: {error}");
            }
        }
    }
    pending.retain(|drop| {
        let result = match drop.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) if Instant::now() < drop.deadline => {
                ctx.request_repaint_after_for(Duration::from_millis(100), viewport);
                return true;
            }
            Err(mpsc::TryRecvError::Empty) => Err("file-transfer portal timed out".into()),
            Err(mpsc::TryRecvError::Disconnected) => Err("file-transfer retrieval ended unexpectedly".into()),
        };
        window.finish_file_transfer(drop.token.id);
        match result {
            Ok(paths) => {
                let position = egui::pos2(drop.token.position.x as f32, drop.token.position.y as f32) / ctx.zoom_factor();
                input.events.push(egui::Event::PointerMoved(position));
                input.dropped_files.extend(
                    paths.into_iter().map(|path| std::sync::Arc::new(NativeFile::from(path)) as std::sync::Arc<dyn egui::DroppedFile + Send + Sync>),
                );
            }
            Err(error) => log::warn!("Wayland file drop: {error}"),
        }
        false
    });
}
fn retrieve(key: &str) -> Result<Vec<PathBuf>, String> {
    craft_clipboard::retrieve_files(key)
}
