//! Bringing the window to the front on Wayland, which allows it only to one that
//! shows leave: a token, such as the desktop gives when a notification is picked.
//! The window's own toolkit can ask for such leave, but cannot use what another got.
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    backend::{Backend, ObjectId},
    delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_surface::WlSurface},
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

/// The way to bring one window to the front.
pub struct Raise {
    connection: Connection,
    activation: XdgActivationV1,
    surface: WlSurface,
    /// Kept for what was made on it.
    _queue: EventQueue<Globals>,
}

/// Nothing is wanted of what Wayland has to say here.
struct Globals;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Globals {
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

delegate_noop!(Globals: XdgActivationV1);

impl Raise {
    /// For a window on Wayland. None for one elsewhere, which needs no leave.
    pub fn of(window: &(impl HasWindowHandle + HasDisplayHandle)) -> Option<&'static Self> {
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
            return None;
        };
        let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: the display and the surface are the window's own, which are there
        // for as long as the window is. What is made of them is leaked, so that
        // none of it is undone once they are gone.
        let connection = unsafe {
            let display = display.display.as_ptr().cast();
            Connection::from_backend(Backend::from_foreign_display(display))
        };
        let surface = unsafe {
            let surface = surface.surface.as_ptr().cast();
            ObjectId::from_ptr(WlSurface::interface(), surface).ok()?
        };
        let surface = WlSurface::from_id(&connection, surface).ok()?;
        let (globals, queue) = registry_queue_init::<Globals>(&connection).ok()?;
        let activation = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        Some(Box::leak(Box::new(Self {
            connection,
            activation,
            surface,
            _queue: queue,
        })))
    }

    /// Come to the front, by the leave of a token.
    pub fn with(&self, token: &str) {
        self.activation.activate(token.to_owned(), &self.surface);
        let _ = self.connection.flush();
    }
}
