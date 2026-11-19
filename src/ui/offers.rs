//! What the desktop offers the window on Wayland: files carried over it and
//! dropped, and what was copied. The window's toolkit hears of no drop there,
//! and of nothing copied but text, so the desktop is listened to here in its
//! own words, apart from the window. Files dropped are read where they land,
//! and what was copied is read when it is pasted.
//!
//! GNOME tells one listener of each program what is carried over its window,
//! the one that asked last. It tells what is copied to those that were there
//! when the window last came into use, and whoever asks while the window is
//! out of use puts out the one before it. So this listener cannot stand
//! beside the toolkit's: it takes its place in the hearing, and reads the
//! text that is pasted too. The toolkit still copies, and reads what a middle
//! click pastes.
use std::{
    io::Read,
    os::{fd::AsFd, unix::net::UnixStream},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use eframe::egui;
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::Backend,
    delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback::WlCallback,
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::{DndAction, WlDataDeviceManager},
        wl_data_offer::{self, WlDataOffer},
        wl_registry,
        wl_seat::WlSeat,
    },
};

use url::Url;

use crate::upload::File;

/// How files are offered: a line for each, as a URL.
const FILES: &str = "text/uri-list";
/// How text is offered, the best first.
const TEXT: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
/// How a picture is offered, the likeliest first, and the name each goes by
/// as a file.
const PICTURES: [(&str, &str); 4] = [
    ("image/png", "clipboard.png"),
    ("image/jpeg", "clipboard.jpg"),
    ("image/webp", "clipboard.webp"),
    ("image/gif", "clipboard.gif"),
];
/// How long whoever offers something may go without giving any more of it.
const PATIENCE: Duration = Duration::from_secs(5);

/// Where on the window something was dropped, in the desktop's units, and the
/// files it was.
pub type Dropped = ((f64, f64), Vec<PathBuf>);

/// What was pasted, once it is read.
pub enum Pasted {
    Text(String),
    /// A picture as a file, as it was copied.
    Picture(File),
}

/// The ways something is offered, which are told one by one.
type Ways = Mutex<Vec<String>>;
/// What was last copied, as it was offered when the window came into use or
/// since.
type Copied = Arc<Mutex<Option<WlDataOffer>>>;

/// The listening, which goes on until it is stopped.
pub struct Offers {
    pub dropped: flume::Receiver<Dropped>,
    pub pasted: flume::Receiver<Pasted>,
    read: flume::Sender<Pasted>,
    copied: Copied,
    /// The window, which is woken when there is something for it.
    window: egui::Context,
    over: Arc<AtomicBool>,
    connection: Connection,
    queue: QueueHandle<Hearing>,
    manager: WlDataDeviceManager,
    seat: WlSeat,
    /// The listener before the one that asked last, and that one.
    devices: [WlDataDevice; 2],
    thread: JoinHandle<()>,
}

struct Hearing {
    /// The files being carried over the window, and where they are.
    carried: Option<WlDataOffer>,
    at: (f64, f64),
    copied: Copied,
    dropped: flume::Sender<Dropped>,
    window: egui::Context,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Hearing {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Hearing: ignore WlSeat);
delegate_noop!(Hearing: ignore WlCallback);
delegate_noop!(Hearing: WlDataDeviceManager);

impl Dispatch<WlDataOffer, Ways> for Hearing {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        ways: &Ways,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            ways.lock().unwrap().push(mime_type);
        }
    }
}

/// The ways something is offered.
fn ways(offer: &WlDataOffer) -> Vec<String> {
    offer.data::<Ways>().map(|ways| ways.lock().unwrap().clone()).unwrap_or_default()
}

impl Dispatch<WlDataDevice, ()> for Hearing {
    fn event(
        hearing: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_data_device::Event;
        match event {
            Event::Enter { serial, x, y, id: Some(offer), .. }
                if ways(&offer).iter().any(|way| way == FILES) =>
            {
                offer.accept(serial, Some(FILES.into()));
                offer.set_actions(DndAction::Copy, DndAction::Copy);
                hearing.at = (x, y);
                // Where both listeners are told, the later telling stands.
                if let Some(before) = hearing.carried.replace(offer) {
                    before.destroy();
                }
            }
            // What is no file is none of the window's.
            Event::Enter { id: Some(offer), .. } => offer.destroy(),
            Event::Motion { x, y, .. } => hearing.at = (x, y),
            Event::Leave => {
                if let Some(offer) = hearing.carried.take() {
                    offer.destroy();
                }
            }
            Event::Drop => {
                let Some(offer) = hearing.carried.take() else {
                    return;
                };
                let files = files(&offer, connection).unwrap_or_default();
                offer.finish();
                offer.destroy();
                if !files.is_empty() && hearing.dropped.send((hearing.at, files)).is_ok() {
                    hearing.window.request_repaint();
                }
            }
            // What is copied now stands in place of what was.
            Event::Selection { id } => {
                let before = std::mem::replace(&mut *hearing.copied.lock().unwrap(), id);
                if let Some(before) = before {
                    before.destroy();
                }
            }
            _ => {}
        }
    }

    event_created_child!(Hearing, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Ways::default()),
    ]);
}

/// Ask for what is offered, in one of the ways it is. It comes to be read
/// from what is given back, and ends when whoever offers it has given it all.
fn ask(offer: &WlDataOffer, connection: &Connection, way: &str) -> std::io::Result<UnixStream> {
    let (ours, theirs) = UnixStream::pair()?;
    ours.set_read_timeout(Some(PATIENCE))?;
    offer.receive(way.into(), theirs.as_fd());
    connection.flush().map_err(std::io::Error::other)?;
    Ok(ours)
}

/// The files of an offer that was dropped, as whoever they were dragged from
/// lists them.
fn files(offer: &WlDataOffer, connection: &Connection) -> std::io::Result<Vec<PathBuf>> {
    let mut list = String::new();
    ask(offer, connection, FILES)?.read_to_string(&mut list)?;
    Ok(list.lines().filter_map(path).collect())
}

/// The file a line of such a list names, if it is one on this machine.
fn path(line: &str) -> Option<PathBuf> {
    // A machine may be named ahead of the path, and none is this one.
    let url = Url::parse(line.trim()).ok().filter(|url| url.scheme() == "file")?;
    url.to_file_path().ok()
}

/// The way to ask for what was copied, of those it is offered in: as text
/// where it is that, and else as a picture, with the name it then goes by.
fn way(ways: &[String]) -> Option<(&'static str, Option<&'static str>)> {
    let offered = |way: &str| ways.iter().any(|offered| offered == way);
    let text = TEXT.into_iter().find(|way| offered(way));
    let picture = || PICTURES.into_iter().find(|picture| offered(picture.0));
    text.map(|way| (way, None)).or_else(|| picture().map(|(way, name)| (way, Some(name))))
}

impl Offers {
    /// Listen to what a window on Wayland is offered, and wake it when there
    /// is something for it. None for a window elsewhere, whose toolkit tells
    /// of it all.
    pub fn listen(shown: &impl HasDisplayHandle, window: egui::Context) -> Option<Self> {
        let RawDisplayHandle::Wayland(display) = shown.display_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: the display is the window's own, which is there for as long as
        // the window is, and the listening is stopped before the window goes.
        let connection = unsafe {
            let display = display.display.as_ptr().cast();
            Connection::from_backend(Backend::from_foreign_display(display))
        };
        let (globals, mut events) = registry_queue_init::<Hearing>(&connection).ok()?;
        let queue = events.handle();
        let seat: WlSeat = globals.bind(&queue, 1..=1, ()).ok()?;
        // The third version is the first in which a drop is agreed on.
        let manager: WlDataDeviceManager = globals.bind(&queue, 3..=3, ()).ok()?;
        let devices = [(); 2].map(|()| manager.get_data_device(&seat, &queue, ()));
        let (sender, dropped) = flume::unbounded();
        let (read, pasted) = flume::unbounded();
        let copied = Copied::default();
        let mut hearing = Hearing {
            carried: None,
            at: (0.0, 0.0),
            copied: copied.clone(),
            dropped: sender,
            window: window.clone(),
        };
        let over = Arc::new(AtomicBool::new(false));
        let done = over.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) && events.blocking_dispatch(&mut hearing).is_ok() {}
        });
        Some(Self {
            dropped,
            pasted,
            read,
            copied,
            window,
            over,
            connection,
            queue,
            manager,
            seat,
            devices,
            thread,
        })
    }

    /// Ask anew to be told, which is to be done whenever the window comes into
    /// use or goes out of it. The listener that asks now is the one GNOME tells
    /// what is carried over the window, and the one before it goes on being
    /// told what is copied, as it was there when the window came into use.
    pub fn renew(&mut self) {
        let device = self.manager.get_data_device(&self.seat, &self.queue, ());
        self.devices.rotate_left(1);
        std::mem::replace(&mut self.devices[1], device).release();
        let _ = self.connection.flush();
    }

    /// Paste what was copied: its text, or failing that a picture, where
    /// `pictures` are wanted. It is read apart from the window, and is `pasted`
    /// once it has all come. False if nothing copied is known of here, and
    /// then the toolkit may know of it.
    pub fn paste(&self, pictures: bool) -> bool {
        let copied = self.copied.lock().unwrap();
        let Some(offer) = copied.as_ref() else {
            return false;
        };
        let wanted = way(&ways(offer)).filter(|(_, picture)| pictures || picture.is_none());
        let Some((way, picture)) = wanted else {
            return false;
        };
        let Ok(mut coming) = ask(offer, &self.connection, way) else {
            return false;
        };
        let (read, window) = (self.read.clone(), self.window.clone());
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if coming.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
                return;
            }
            let pasted = match picture {
                Some(name) => Pasted::Picture(File::Pasted(name, bytes.into())),
                // Lines of text may end as they do on the wire.
                None => Pasted::Text(String::from_utf8_lossy(&bytes).replace("\r\n", "\n")),
            };
            if read.send(pasted).is_ok() {
                window.request_repaint();
            }
        });
        true
    }

    /// Stop listening, which must be done while the window is still there. The
    /// listener is waiting to hear something, and so is told something.
    pub fn stop(self) {
        self.over.store(true, Ordering::Relaxed);
        self.connection.display().sync(&self.queue, ());
        let _ = self.connection.flush();
        let _ = self.thread.join();
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_offers.rs"]
mod tests;
