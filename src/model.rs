//! Small UI-facing value model. Terminal frames are immutable, shared snapshots.
use crate::{program::Program, terminal::Frame};
use std::{cmp::Reverse, collections::HashSet, sync::Arc};

#[derive(Clone, Default)]
pub struct View {
    /// Connection trouble in words. Empty while connected.
    pub status: String,
    pub connected: bool,
    /// An action that failed, reported by one snapshot only.
    pub error: Option<String>,
    /// A pane this client has just made by a split, or brought out from out of
    /// sight, reported likewise.
    pub made: Option<String>,
    /// A pane on show, and text a program in it has just copied, reported likewise.
    pub copied: Option<(String, String)>,
    /// The panes whose bells have rung since the last snapshot.
    pub rang: HashSet<String>,
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

    /// Every tab of every session, with its session.
    pub fn tabs(&self) -> impl Iterator<Item = (&Session, &Window)> {
        let sessions = self.sessions.iter();
        sessions.flat_map(|session| session.windows.iter().map(move |tab| (session, tab)))
    }

    /// A pane of any tab, on show or not, with the session and the tab that hold it.
    pub fn find(&self, pane: &str) -> Option<(&Session, &Window, &Pane)> {
        self.tabs().find_map(|(session, tab)| {
            let pane = tab.every().find(|held| held.id == pane)?;
            Some((session, tab, pane))
        })
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

impl Session {
    /// The most pressing thing its tabs have to say.
    pub fn activity(&self) -> Activity {
        let said = self.windows.iter().map(|tab| tab.says().0);
        said.max().unwrap_or_default()
    }
}

/// A tab.
#[derive(Clone)]
pub struct Window {
    pub id: String,
    /// The name it was given, if it was given one.
    pub label: Option<String>,
    /// The pane that typing goes to.
    pub focus: Option<String>,
    /// One pane fills the tab, and is the only one of them on show.
    pub zoomed: bool,
    /// The panes on show.
    pub panes: Vec<Pane>,
    /// Those out of sight, which run on unseen: behind the one that fills the tab,
    /// or in a floating layer that has been put away.
    pub hidden: Vec<Pane>,
}

impl Window {
    /// Every pane, on show or not.
    pub fn every(&self) -> impl Iterator<Item = &Pane> {
        self.panes.iter().chain(&self.hidden)
    }

    pub fn every_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        self.panes.iter_mut().chain(&mut self.hidden)
    }

    /// The most pressing thing its panes have to say, and the line said of it. Of
    /// several as pressing, the first.
    pub fn says(&self) -> (Activity, &str) {
        let top = self.every().min_by_key(|pane| Reverse(pane.activity));
        top.map_or((Activity::Idle, ""), |pane| (pane.activity, &pane.message))
    }

    /// Nothing runs in it but shells at their prompts, so closing it loses nothing.
    pub fn idle(&self) -> bool {
        self.every().all(|pane| pane.idle)
    }

    /// The pane that speaks for the tab: the one typing goes to.
    pub fn front(&self) -> Option<&Pane> {
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

    /// Whether it holds more panes than one, on show or not.
    pub fn split(&self) -> bool {
        self.zoomed || self.panes.len() > 1
    }

    /// The pane nearest a pane on one side of it: `toward` is a step along x or y.
    /// Of several as near, the first from the top or the left.
    pub fn beside(&self, pane: &str, toward: [f32; 2]) -> Option<&Pane> {
        const SLACK: f32 = 0.001;
        let from = self.panes.iter().find(|p| p.id == pane)?;
        let along = usize::from(toward[0] == 0.0);
        let span = |pane: &Pane, axis| match axis {
            0 => (pane.x, pane.x + pane.width),
            _ => (pane.y, pane.y + pane.height),
        };
        let reach = |pane: &Pane| {
            let ((start, end), (from_start, from_end)) = (span(pane, along), span(from, along));
            let gap = if toward[along] > 0.0 {
                start - from_end
            } else {
                from_start - end
            };
            // It lies that way, and alongside rather than merely past a corner.
            let ((start, end), (from_start, from_end)) =
                (span(pane, 1 - along), span(from, 1 - along));
            let alongside = start < from_end - SLACK && end > from_start + SLACK;
            (gap > -SLACK && alongside).then_some((gap, start))
        };
        let near = self
            .panes
            .iter()
            .filter_map(|pane| Some((reach(pane)?, pane)));
        let nearest = near.min_by(|(a, _), (b, _)| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        nearest.map(|(_, pane)| pane)
    }
}

/// What the programs in a terminal say they are doing, most pressing last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Activity {
    #[default]
    Idle,
    Working,
    /// The terminal's bell has rung, and the pane has not been looked at since.
    /// No server reports this: a window makes it of the bells it hears.
    Rang,
    /// Finished, with a result the user has yet to look at.
    Done,
    /// Failed and stopped.
    Error,
    /// Waiting for the user.
    Blocked(Wait),
}

/// What a program is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wait {
    /// It did not say.
    Input,
    Question,
    /// A password or the like.
    SignIn,
    Approval,
}

impl Activity {
    /// The word for it, of those that are told in words.
    pub fn word(self) -> Option<&'static str> {
        Some(match self {
            Self::Idle | Self::Working => return None,
            Self::Rang => "Bell",
            Self::Done => "Done",
            Self::Error => "Error",
            Self::Blocked(Wait::Input) => "Waiting",
            Self::Blocked(Wait::Question) => "Question",
            Self::Blocked(Wait::SignIn) => "Needs sign-in",
            Self::Blocked(Wait::Approval) => "Needs approval",
        })
    }
}

/// Two things said as one line. Either may be left unsaid.
pub fn line(first: &str, second: &str) -> String {
    match (first, second) {
        ("", only) | (only, "") => only.to_owned(),
        _ => format!("{first}: {second}"),
    }
}

/// Geometry is a fraction of the tab area.
#[derive(Clone)]
pub struct Pane {
    pub id: String,
    /// The terminal's title, as its program set it.
    pub title: String,
    pub program: Program,
    pub activity: Activity,
    /// The line its program has to say of that, if it has one.
    pub message: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub frame: Option<Arc<Frame>>,
    pub status: String,
    pub owner: bool,
    /// Nothing is in it but the shell it started with, at its prompt, so closing it
    /// loses nothing. False where that is not known.
    pub idle: bool,
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

/// Whether a key's code names a modifier, which held alone asks for nothing.
pub fn is_modifier(code: &str) -> bool {
    matches!(
        code,
        "shift_left"
            | "shift_right"
            | "control_left"
            | "control_right"
            | "alt_left"
            | "alt_right"
            | "meta_left"
            | "meta_right"
    )
}

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
    /// Bring a pane of the session on show out from out of sight, and take the
    /// keyboard to it.
    Reveal {
        pane: String,
    },
    FocusPane {
        pane: String,
    },
    Key {
        pane: String,
        key: String,
        code: String,
        modifiers: u8,
        pressed: bool,
        /// Pressed by being held down.
        repeat: bool,
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
    /// `kind` is `press`, `release` or `motion`; `button` is `left`, `middle` or `right`.
    Mouse {
        pane: String,
        kind: &'static str,
        x: u16,
        y: u16,
        button: Option<&'static str>,
        modifiers: u8,
    },
    /// One step of the wheel, for a program that tracks the mouse. Negative is up.
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
    /// Let a pane fill its tab, or give the tab back to all its panes.
    Zoom {
        pane: String,
    },
    /// End a pane and the program in it. A tab ends with its last pane.
    ClosePane {
        pane: String,
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

#[cfg(test)]
#[path = "../tests/unit/model.rs"]
mod tests;
