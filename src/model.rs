//! Small UI-facing value model. Terminal frames are immutable, shared snapshots.
use crate::terminal::Frame;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct View {
    /// Connection trouble in words. Empty while connected.
    pub status: String,
    pub connected: bool,
    /// An action that failed, reported by one snapshot only.
    pub error: Option<String>,
    /// The other servers this one knows of.
    pub hosts: Vec<Host>,
    pub sessions: Vec<Session>,
    pub selected_session: Option<String>,
    pub selected_window: Option<String>,
    pub windows: Vec<Window>,
}

impl View {
    pub fn current_window(&self) -> Option<&Window> {
        let selected = self.selected_window.as_ref()?;
        self.windows.iter().find(|w| &w.id == selected)
    }

    pub fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.windows.iter().flat_map(|w| &w.panes)
    }

    pub fn panes_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        self.windows.iter_mut().flat_map(|w| &mut w.panes)
    }

    pub fn pane(&self, id: &str) -> Option<&Pane> {
        self.panes().find(|p| p.id == id)
    }

    pub fn pane_mut(&mut self, id: &str) -> Option<&mut Pane> {
        self.panes_mut().find(|p| p.id == id)
    }
}

/// A server, by the name it goes by and the addresses that reach it, in the order to try them.
#[derive(Clone, PartialEq)]
pub struct Host {
    pub label: String,
    pub endpoints: Vec<String>,
}

#[derive(Clone)]
pub struct Session {
    pub id: String,
    pub name: String,
}

#[derive(Clone)]
pub struct Window {
    pub id: String,
    pub name: String,
    pub panes: Vec<Pane>,
}

/// Geometry is a fraction of the tab area.
#[derive(Clone)]
pub struct Pane {
    pub id: String,
    pub title: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub frame: Option<Arc<Frame>>,
    pub status: String,
    pub owner: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Right,
    Down,
}

/// Bits of the `modifiers` fields below.
pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const SUPER: u8 = 8;

#[derive(Clone, Debug)]
pub enum Action {
    SelectSession(String),
    /// Detach from a session that is no longer shown. Sent by `Client::leave`.
    Leave(String),
    SelectWindow(String),
    FocusPane {
        pane: String,
    },
    Key {
        pane: String,
        key: String,
        code: String,
        modifiers: u8,
        pressed: bool,
    },
    Text {
        pane: String,
        text: String,
    },
    Paste {
        pane: String,
        text: String,
    },
    Resize {
        pane: String,
        size: Size,
    },
    /// `kind` is `press`, `release` or `move`; `button` is `left`, `middle` or `right`.
    Mouse {
        pane: String,
        kind: &'static str,
        x: u16,
        y: u16,
        button: Option<&'static str>,
        modifiers: u8,
    },
    /// A wheel turn forwarded to a program that tracks the mouse. Negative is up.
    Wheel {
        pane: String,
        x: u16,
        y: u16,
        lines: i32,
    },
    /// The server chooses the new session's name.
    CreateSession,
    CreateWindow,
    Split {
        pane: String,
        direction: Direction,
    },
    Reconnect,
}
