//! Small UI-facing value model. Terminal frames are immutable, shared snapshots.
use crate::{program::Program, terminal::Frame};
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
    /// Every session on the server, each with its tabs.
    pub sessions: Vec<Session>,
    /// The session on show, and the tab of it on show.
    pub selected_session: Option<String>,
    pub selected_window: Option<String>,
}

impl View {
    pub fn session(&self) -> Option<&Session> {
        let selected = self.selected_session.as_ref()?;
        self.sessions.iter().find(|s| &s.id == selected)
    }

    /// The tabs of the session on show.
    pub fn windows(&self) -> &[Window] {
        self.session().map_or(&[], |s| &s.windows)
    }

    pub fn current_window(&self) -> Option<&Window> {
        let selected = self.selected_window.as_ref()?;
        self.windows().iter().find(|w| &w.id == selected)
    }

    /// The panes of the session on show.
    pub fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.windows().iter().flat_map(|w| &w.panes)
    }

    pub fn panes_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        let selected = self.selected_session.as_ref();
        let sessions = self.sessions.iter_mut();
        sessions
            .filter(move |s| Some(&s.id) == selected)
            .flat_map(|s| &mut s.windows)
            .flat_map(|w| &mut w.panes)
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
    /// The tab the server shows for this session.
    pub active: Option<String>,
    pub windows: Vec<Window>,
}

/// A tab.
#[derive(Clone)]
pub struct Window {
    pub id: String,
    /// The name it was given, if it was given one.
    pub label: Option<String>,
    /// The pane that typing goes to.
    pub focus: Option<String>,
    pub panes: Vec<Pane>,
}

impl Window {
    /// The pane that speaks for the tab: the one typing goes to.
    fn front(&self) -> Option<&Pane> {
        let focused = |pane: &&Pane| Some(&pane.id) == self.focus.as_ref();
        self.panes.iter().find(focused).or(self.panes.first())
    }

    /// A tab nobody named goes by what is running in it.
    pub fn name(&self) -> &str {
        let title = || self.front().map_or("Tab", |pane| &pane.title);
        self.label.as_deref().unwrap_or_else(title)
    }

    pub fn program(&self) -> Program {
        self.front().map(|pane| pane.program).unwrap_or_default()
    }

    pub fn activity(&self) -> Activity {
        let panes = self.panes.iter().map(|p| p.activity);
        panes.max().unwrap_or_default()
    }
}

/// What the programs in a terminal say they are doing, most pressing last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Activity {
    #[default]
    Idle,
    Working,
    /// Waiting for the user.
    Blocked,
}

/// Geometry is a fraction of the tab area.
#[derive(Clone)]
pub struct Pane {
    pub id: String,
    /// The terminal's title, as its program set it.
    pub title: String,
    pub program: Program,
    pub activity: Activity,
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
    /// Show a tab, of any session.
    SelectWindow {
        session: String,
        window: String,
    },
    /// Stop watching the session on show. Sent by `Client::leave`.
    Leave,
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
    /// Without a name, the server chooses one.
    CreateSession(Option<String>),
    /// A new tab in a session.
    CreateWindow(String),
    Split {
        pane: String,
        direction: Direction,
    },
    /// Change a session or, with `window`, one of its tabs.
    Edit {
        session: String,
        window: Option<String>,
        edit: Edit,
    },
    Reconnect,
}

#[derive(Clone, Debug)]
pub enum Edit {
    /// An empty name restores the one it had before it was given any.
    Rename(String),
    /// Move it to just before another, or to the end.
    MoveBefore(Option<String>),
    /// End it and every program in it.
    Close,
}
