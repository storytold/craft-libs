use sctk::data_device_manager::data_device::DataDeviceHandler;
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::data_source::{CopyPasteSource, DataSourceHandler};
use sctk::data_device_manager::{DataDeviceManagerState, WritePipe};
use sctk::reexports::calloop::{EventLoop, LoopHandle, PostAction, RegistrationToken, channel};
use sctk::reexports::calloop::{
    Interest, Mode,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::backend::ObjectId;
use sctk::reexports::client::globals::registry_queue_init;
use sctk::reexports::client::protocol::{wl_data_device::WlDataDevice, wl_data_device_manager::DndAction, wl_data_source::WlDataSource};
use sctk::reexports::client::{Connection, Proxy, QueueHandle};
use sctk::registry::{ProvidesRegistryState, RegistryState};
use sctk::{delegate_data_device, delegate_registry, registry_handlers};
use std::{
    collections::HashMap,
    os::fd::{AsFd, OwnedFd},
    sync::{
        Arc, Mutex, OnceLock, PoisonError, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use winit::platform::wayland::{WaylandClipboard, WindowExtWayland};
const MAX_BYTES: usize = 64 << 20;
const MAX_MIMES: usize = 32;
const MAX_TRANSFERS: usize = 2;
const TIMEOUT: Duration = Duration::from_secs(5);
static WORKERS: AtomicUsize = AtomicUsize::new(0);
static WORKER_CACHE: OnceLock<Mutex<HashMap<usize, Weak<Inner>>>> = OnceLock::new();
type Data = Vec<(String, Arc<[u8]>)>;
type Reply = mpsc::Sender<Result<Response, String>>;
enum Response {
    Formats(Vec<String>),
    Bytes(Vec<u8>),
    Stored,
}
enum Command {
    Clear(Reply),
    Formats(Reply),
    Read(String, Reply),
    Store(Data, Reply),
}
/// Cloneable app clipboard. Synchronous requests have a finite deadline; transport and
/// source servicing run on a separate event queue. Dropping the last handle wakes it to exit.
#[derive(Clone)]
pub struct Clipboard {
    inner: Arc<Inner>,
}
struct Inner {
    sender: channel::SyncSender<Command>,
}
impl Clipboard {
    pub fn clear(&self) -> Result<(), String> {
        match self.request(Command::Clear)? {
            Response::Stored => Ok(()),
            _ => Err("Unexpected clipboard response".into()),
        }
    }
    /// Reads the first supported format in the caller's preference order.
    pub fn read_preferred(&self, mimes: &[&str]) -> Result<Option<Vec<u8>>, String> {
        let formats = self.formats()?;
        match mimes.iter().find(|mime| formats.iter().any(|format| format == **mime)) {
            Some(mime) => self.read(mime).map(Some),
            None => Ok(None),
        }
    }

    pub fn text(&self) -> Option<String> {
        let bytes = self.read_preferred(&["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]).ok()??;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn write_text(&self, text: &str) -> Result<(), String> {
        self.write(vec![("text/plain;charset=utf-8".into(), text.as_bytes().to_vec()), ("text/plain".into(), text.as_bytes().to_vec())])
    }

    pub fn files(&self) -> Option<Vec<std::path::PathBuf>> {
        let formats = self.formats().ok()?;
        let portal = formats.iter().any(|mime| mime == crate::FILE_TRANSFER_MIME);
        let uri = formats.iter().any(|mime| mime == "text/uri-list" || mime == "x-special/gnome-copied-files");
        // Sandboxed receivers need the portal's exported paths, even when host URIs are offered.
        if portal && (!uri || std::env::var_os("FLATPAK_ID").is_some()) {
            let bytes = self.read(crate::FILE_TRANSFER_MIME).ok()?;
            let paths = crate::retrieve_files(crate::file_transfer_key(&bytes).ok()?).ok()?;
            return (!paths.is_empty()).then_some(paths);
        }
        let bytes = self.read_preferred(&["text/uri-list", "x-special/gnome-copied-files"]).ok()??;
        let paths = winit::platform::wayland::local_file_paths(&bytes);
        (!paths.is_empty()).then_some(paths)
    }

    pub fn from_window(window: &winit::window::Window) -> Result<Option<Self>, String> {
        let Some(connection) = window.wayland_connection() else { return Ok(None) };
        let clipboard = window.wayland_clipboard().ok_or("No Wayland clipboard state")?;
        let key = clipboard.identity();
        let mut cache = WORKER_CACHE.get_or_init(Default::default).lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|_, inner| inner.strong_count() > 0);
        if let Some(inner) = cache.get(&key).and_then(Weak::upgrade) {
            return Ok(Some(Self { inner }));
        }
        if !reserve_worker() {
            return Err("Too many clipboard workers".into());
        }
        let (sender, receiver) = channel::sync_channel(8);
        if let Err(error) = std::thread::Builder::new().name("craft-wayland-clipboard".into()).spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    WORKERS.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _guard = Guard;
            let _ = run(connection, receiver, clipboard);
        }) {
            WORKERS.fetch_sub(1, Ordering::AcqRel);
            return Err(error.to_string());
        }
        let inner = Arc::new(Inner { sender });
        cache.insert(key, Arc::downgrade(&inner));
        Ok(Some(Self { inner }))
    }
    fn request(&self, command: impl FnOnce(Reply) -> Command) -> Result<Response, String> {
        let (sender, receiver) = mpsc::channel();
        self.inner.sender.try_send(command(sender)).map_err(|_| "Clipboard is busy or unavailable".to_string())?;
        receiver.recv_timeout(TIMEOUT + Duration::from_millis(500)).map_err(|_| "Clipboard request timed out".to_string())?
    }
    pub fn formats(&self) -> Result<Vec<String>, String> {
        match self.request(Command::Formats)? {
            Response::Formats(mimes) => Ok(mimes),
            _ => Err("Unexpected clipboard response".into()),
        }
    }
    pub fn read(&self, mime: &str) -> Result<Vec<u8>, String> {
        if !valid_mime(mime) {
            return Err("Invalid clipboard MIME type".into());
        }
        match self.request(|reply| Command::Read(mime.into(), reply))? {
            Response::Bytes(bytes) => Ok(bytes),
            _ => Err("Unexpected clipboard response".into()),
        }
    }
    pub fn write(&self, data: Vec<(String, Vec<u8>)>) -> Result<(), String> {
        let mut total = 0usize;
        if data.is_empty() || data.len() > MAX_MIMES {
            return Err("Invalid clipboard format count".into());
        }
        for (mime, bytes) in &data {
            if !valid_mime(mime) {
                return Err("Invalid clipboard MIME type".into());
            }
            total = total.checked_add(bytes.len()).ok_or("Clipboard is too large")?;
            if total > MAX_BYTES {
                return Err("Clipboard exceeds 64 MiB".into());
            }
        }
        let data = data.into_iter().map(|(mime, bytes)| (mime, Arc::from(bytes))).collect();
        match self.request(|reply| Command::Store(data, reply))? {
            Response::Stored => Ok(()),
            _ => Err("Unexpected clipboard response".into()),
        }
    }
}
fn reserve_worker() -> bool {
    let mut count = WORKERS.load(Ordering::Acquire);
    loop {
        if count >= 16 {
            return false;
        }
        match WORKERS.compare_exchange_weak(count, count + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(current) => count = current,
        }
    }
}

fn valid_mime(mime: &str) -> bool {
    !mime.is_empty() && mime.len() <= 256 && mime.bytes().all(|b| b.is_ascii() && !b.is_ascii_control())
}
struct State {
    registry: RegistryState,
    manager: DataDeviceManagerState,
    clipboard: WaylandClipboard,
    qh: QueueHandle<Self>,
    handle: LoopHandle<'static, Self>,
    sources: HashMap<ObjectId, (CopyPasteSource, Data)>,
    reads: usize,
    writes: usize,
    exit: bool,
}
struct Read {
    bytes: Vec<u8>,
    reply: Option<Reply>,
    timer: Option<RegistrationToken>,
}
fn run(connection: Connection, receiver: channel::Channel<Command>, clipboard: WaylandClipboard) -> Result<(), String> {
    let (globals, queue) = registry_queue_init(&connection).map_err(|e| e.to_string())?;
    clipboard.wake_event_loop();
    let qh = queue.handle();
    let mut event_loop = EventLoop::<State>::try_new().map_err(|e| e.to_string())?;
    let handle = event_loop.handle();
    let mut state = State {
        registry: RegistryState::new(&globals),
        manager: DataDeviceManagerState::bind(&globals, &qh).map_err(|e| e.to_string())?,
        clipboard,
        qh,
        handle: handle.clone(),
        sources: HashMap::new(),
        reads: 0,
        writes: 0,
        exit: false,
    };
    handle
        .insert_source(receiver, |event, _, state| match event {
            channel::Event::Msg(command) => state.command(command),
            channel::Event::Closed => state.exit = true,
        })
        .map_err(|e| e.to_string())?;
    let source = crate::source::Source { inner: WaylandSource::new(connection, queue), clipboard: state.clipboard.clone() };
    handle.insert_source(source, |_, queue, state| queue.dispatch_pending(state)).map_err(|e| e.to_string())?;
    while !state.exit {
        event_loop.dispatch(None, &mut state).map_err(|e| e.to_string())?;
        state.clipboard.wake_event_loop();
    }
    Ok(())
}
impl State {
    fn command(&mut self, command: Command) {
        match command {
            Command::Clear(reply) => {
                let result = self.clipboard.focused_device().map(|(device, serial)| {
                    device.set_selection(None, serial);
                    Response::Stored
                });
                let _ = reply.send(result);
            }
            Command::Formats(reply) => {
                let result = self
                    .clipboard
                    .selection()
                    .and_then(|selection| selection.map(|(_, mimes)| Response::Formats(mimes)).ok_or("Clipboard is empty".into()));
                let _ = reply.send(result);
            }
            Command::Store(data, reply) => {
                let result = self.store(data);
                let _ = reply.send(result.map(|()| Response::Stored));
            }
            Command::Read(mime, reply) => {
                if let Err(error) = self.read(mime, reply.clone()) {
                    let _ = reply.send(Err(error));
                }
            }
        }
    }
    fn store(&mut self, data: Data) -> Result<(), String> {
        if self.sources.len() >= 4 {
            return Err("Clipboard sources are busy".into());
        }
        let (device, serial) = self.clipboard.focused_device()?;
        let source = self.manager.create_copy_paste_source(&self.qh, data.iter().map(|(mime, _)| mime));
        device.set_selection(Some(source.inner()), serial);
        self.sources.insert(source.inner().id(), (source, data));
        Ok(())
    }
    fn read(&mut self, mime: String, reply: Reply) -> Result<(), String> {
        if self.reads >= MAX_TRANSFERS {
            return Err("Clipboard transfers are busy".into());
        }
        let max_bytes = if mime == crate::FILE_TRANSFER_MIME { 4096 } else { MAX_BYTES };
        let (offer, mimes) = self.clipboard.selection()?.ok_or("Clipboard is empty")?;
        if !mimes.contains(&mime) {
            return Err("Clipboard format is unavailable".into());
        }
        let (fd, write) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK).map_err(|error| error.to_string())?;
        offer.receive(mime, write.as_fd());
        drop(write);
        nonblocking(&fd)?;
        let read = Arc::new(Mutex::new(Read { bytes: Vec::new(), reply: Some(reply), timer: None }));
        let read_ready = read.clone();
        let token = self
            .handle
            .insert_source(Generic::new(fd, Interest::READ, Mode::Level), move |_, file, state| {
                let mut read = read_ready.lock().unwrap_or_else(PoisonError::into_inner);
                let mut buffer = [0; 8192];
                let mut budget: usize = 256 << 10;
                loop {
                    let result = match rustix::io::read(&*file, &mut buffer) {
                        Ok(0) => Some(Ok(Response::Bytes(std::mem::take(&mut read.bytes)))),
                        Ok(n) if read.bytes.len().saturating_add(n) > max_bytes => Some(Err("Clipboard exceeds the format limit".into())),
                        Ok(n) => {
                            read.bytes.extend_from_slice(&buffer[..n]);
                            budget = budget.saturating_sub(n);
                            if budget == 0 {
                                return Ok(PostAction::Continue);
                            }
                            None
                        }
                        Err(rustix::io::Errno::AGAIN) => return Ok(PostAction::Continue),
                        Err(rustix::io::Errno::INTR) => None,
                        Err(error) => Some(Err(error.to_string())),
                    };
                    if let Some(result) = result {
                        if let Some(reply) = read.reply.take() {
                            let _ = reply.send(result);
                        }
                        if let Some(timer) = read.timer.take() {
                            state.handle.remove(timer);
                        }
                        state.reads = state.reads.saturating_sub(1);
                        return Ok(PostAction::Remove);
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let timed = read.clone();
        let timer = match self.handle.insert_source(Timer::from_duration(TIMEOUT), move |_, _, state| {
            state.handle.remove(token);
            let mut read = timed.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(reply) = read.reply.take() {
                let _ = reply.send(Err("Clipboard transfer timed out".into()));
            }
            state.reads = state.reads.saturating_sub(1);
            TimeoutAction::Drop
        }) {
            Ok(timer) => timer,
            Err(error) => {
                self.handle.remove(token);
                return Err(error.to_string());
            }
        };
        read.lock().unwrap_or_else(PoisonError::into_inner).timer = Some(timer);
        self.reads += 1;
        Ok(())
    }
    fn send(&mut self, source: &WlDataSource, mime: String, pipe: WritePipe) {
        if self.writes >= MAX_TRANSFERS {
            return;
        }
        let Some(bytes) =
            self.sources.get(&source.id()).and_then(|(_, data)| data.iter().find(|(format, _)| format == &mime)).map(|(_, bytes)| bytes.clone())
        else {
            return;
        };
        let fd: OwnedFd = pipe.into();
        if nonblocking(&fd).is_err() {
            return;
        }
        let mut written = 0usize;
        let timer = Arc::new(Mutex::new(None));
        let timer_ready = timer.clone();
        let Ok(token) = self.handle.insert_source(Generic::new(fd, Interest::WRITE, Mode::Level), move |_, file, state| {
            let end = bytes.len().min(written.saturating_add(256 << 10));
            let done = match rustix::io::write(&*file, bytes.get(written..end).unwrap_or_default()) {
                Ok(0) => true,
                Ok(n) => {
                    written = written.saturating_add(n);
                    written >= bytes.len()
                }
                Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => false,
                Err(_) => true,
            };
            if done {
                if let Some(timer) = timer_ready.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    state.handle.remove(timer);
                }
                state.writes = state.writes.saturating_sub(1);
                Ok(PostAction::Remove)
            } else {
                Ok(PostAction::Continue)
            }
        }) else {
            return;
        };
        let Ok(timeout) = self.handle.insert_source(Timer::from_duration(TIMEOUT), move |_, _, state| {
            state.handle.remove(token);
            state.writes = state.writes.saturating_sub(1);
            TimeoutAction::Drop
        }) else {
            self.handle.remove(token);
            return;
        };
        *timer.lock().unwrap_or_else(PoisonError::into_inner) = Some(timeout);
        self.writes += 1;
    }
}
fn nonblocking(fd: &OwnedFd) -> Result<(), String> {
    let flags = rustix::fs::fcntl_getfl(fd).map_err(|e| e.to_string())?;
    rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(|e| e.to_string())
}
impl DataDeviceHandler for State {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64, _: &WlSurface) {}
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}
impl DataSourceHandler for State {
    fn send_request(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource, mime: String, pipe: WritePipe) {
        self.send(source, mime, pipe)
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.sources.remove(&source.id());
    }
    fn accept_mime(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: Option<String>) {}
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
}
impl DataOfferHandler for State {
    fn source_actions(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
    fn selected_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
}
impl ProvidesRegistryState for State {
    registry_handlers![];
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
}
use sctk::reexports::client::protocol::wl_surface::WlSurface;
delegate_data_device!(State);
delegate_registry!(State);
