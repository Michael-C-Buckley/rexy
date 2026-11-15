//! Small UI-facing value model. Terminal frames are immutable, shared snapshots.
use crate::{program::Program, terminal::Frame};
use serde_json::Value;
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
    /// What the server says this client's keys do: its answer, once it has one.
    pub keymap: Option<Arc<Value>>,
    /// What this client has been asked to do, reported by one snapshot only.
    pub asked: Vec<Asked>,
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

    /// What to ask of a server that is on show and shows no session: for its first.
    /// No session is shown unasked, since a window shows one server at a time.
    pub fn unshown(&self) -> Option<Action> {
        let first = self.sessions.first().filter(|_| self.connected)?;
        let none = self.selected_session.is_none();
        none.then(|| Action::SelectSession(first.id.clone()))
    }

    /// Keep what is on show to what there is. A session that is gone is on show no
    /// longer. Of the session on show, the tab is the one this window last showed,
    /// for as long as it is there: what another client has the server show does
    /// not move it. With none, it is the server's tab, or failing that the first.
    pub fn settle(&mut self) {
        let Some(session) = self.session() else {
            (self.selected_session, self.selected_window) = (None, None);
            return;
        };
        let there = |id: &&String| session.windows.iter().any(|tab| &tab.id == *id);
        let kept = self.selected_window.as_ref().filter(there);
        let shown = kept.or(session.active.as_ref().filter(there));
        let first = || session.windows.first().map(|tab| &tab.id);
        self.selected_window = shown.or_else(first).cloned();
    }
}

impl View {
    /// Show the tab the server shows of a session, if the window still shows
    /// the `tab` it showed when it asked the server to act for the user. The
    /// server showed that tab too, then: if it shows another now, what was
    /// asked of it moved it there.
    pub fn follow(&mut self, session: &str, tab: &str) {
        let shown = (self.selected_session.as_deref(), self.selected_window.as_deref());
        if shown == (Some(session), Some(tab)) {
            self.selected_window = None;
            self.settle();
        }
    }
}

/// A server, by the name it goes by and the addresses that reach it, in the order to try them.
#[derive(Clone, PartialEq)]
pub struct Host {
    pub label: String,
    pub endpoints: Vec<String>,
}

/// Something this client is asked to do, of its own actions.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    /// What another client's asking is answered by. None if an action of the
    /// server's that this client began asks, which waits for no answer.
    pub call: Option<String>,
    pub name: String,
    pub args: Value,
    /// The session and the pane it is to be done to, where the asking says.
    pub session: Option<String>,
    pub pane: Option<String>,
}

#[derive(Clone, Default)]
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
#[derive(Clone, Default)]
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
    /// The borders between the panes on show that can be dragged.
    pub borders: Vec<Border>,
}

/// The line between the two sides of a split, which drags to give one of them room
/// from the other.
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    /// A pane of each side, which name the split to the server: `before` is left
    /// of the line or above it, `after` right of it or below.
    pub before: String,
    pub after: String,
    /// The sides lie left and right of it, not above and below.
    pub upright: bool,
    /// The room the two sides share, as a pane's place is given: x, y, width and
    /// height, each a fraction of the tab.
    pub room: [f32; 4],
    /// Where in the tab it lies, along x if it is upright and along y if not.
    pub at: f32,
}

/// Where a border is asked to go: the one between these two panes, so that the
/// side before it has `ratio` of the room the two sides share.
#[derive(Clone, Debug, PartialEq)]
pub struct Division {
    pub before: String,
    pub after: String,
    pub upright: bool,
    pub ratio: f32,
}

impl Border {
    /// The least and the most of the room two sides share that its first side
    /// can be given, and how much of it one press of a key moves the border by.
    pub const LEAST: f32 = 0.05;
    pub const MOST: f32 = 1.0 - Self::LEAST;
    const STEP: f32 = 0.05;

    /// What asks it to go where its first side has `ratio` of its room.
    pub fn moved(&self, ratio: f32) -> Division {
        Division {
            before: self.before.clone(),
            after: self.after.clone(),
            upright: self.upright,
            ratio,
        }
    }

    /// The same, for a share it may be asked for by key: no nearer either end
    /// than a border goes, and nothing where it is there already.
    fn sent(&self, share: f32) -> Option<Division> {
        let ratio = share.clamp(Self::LEAST, Self::MOST);
        ((ratio - self.share()).abs() > 0.005).then(|| self.moved(ratio))
    }

    /// Which of x and y it moves along.
    fn axis(&self) -> usize {
        usize::from(!self.upright)
    }

    /// The share of its room that its first side has.
    fn share(&self) -> f32 {
        (self.at - self.room[self.axis()]) / self.room[2 + self.axis()]
    }

    /// Whether a pane is on either side of it: one whose middle is in its room.
    fn parts(&self, pane: &Pane) -> bool {
        let [x, y, width, height] = self.room;
        let (across, down) = (pane.x + pane.width / 2.0, pane.y + pane.height / 2.0);
        across > x && across < x + width && down > y && down < y + height
    }
}

impl Window {
    /// Every pane, on show or not.
    pub fn every(&self) -> impl Iterator<Item = &Pane> {
        self.panes.iter().chain(&self.hidden)
    }

    pub fn every_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        self.panes.iter_mut().chain(&mut self.hidden)
    }

    /// The pane with the most pressing thing to say. Of several as pressing, the first.
    pub fn top(&self) -> Option<&Pane> {
        self.every().min_by_key(|pane| Reverse(pane.activity))
    }

    /// The most pressing thing its panes have to say, and the line said of it.
    pub fn says(&self) -> (Activity, &str) {
        let top = self.top();
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

    /// What moves a pane's edge a step `toward` one side, along x or y: it asks
    /// that of the border of the innermost split that lies that way across and
    /// has the pane in it. None when there is none, or it is as far that way as
    /// a border goes.
    pub fn nudged(&self, pane: &str, toward: [f32; 2]) -> Option<Division> {
        let pane = self.panes.iter().find(|held| held.id == pane)?;
        let upright = toward[1] == 0.0;
        let across = |border: &&Border| border.upright == upright && border.parts(pane);
        let room = |border: &&Border| border.room[2] * border.room[3];
        let borders = self.borders.iter().filter(across);
        let border = borders.min_by(|a, b| room(a).total_cmp(&room(b)))?;
        border.sent(border.share() + Border::STEP * (toward[0] + toward[1]).signum())
    }

    /// What gives every pane as much room as those beside it, and as those
    /// above and below it: what it asks of each border that is not there already.
    pub fn balanced(&self) -> Vec<Division> {
        let even = |border: &Border| {
            let (along, across) = (border.axis(), 1 - border.axis());
            // How many panes lie one after another on a side of it, the way it
            // moves: those stacked the other way share their place in the row.
            let row = |first: bool| {
                let side = |pane: &&Pane| {
                    let before = [pane.x, pane.y][along] + 0.001 < border.at;
                    border.parts(pane) && before == first
                };
                let side = || self.panes.iter().filter(side);
                // Where a pane starts the other way, and its size: one is level
                // with another whose middle is within that.
                let span =
                    |pane: &Pane| ([pane.x, pane.y][across], [pane.width, pane.height][across]);
                let level = |by: &Pane, pane: &&Pane| {
                    let ((at, size), (start, room)) = (span(by), span(pane));
                    (at + size / 2.0 - start - room / 2.0).abs() < room / 2.0
                };
                let most = side().map(|by| side().filter(|pane| level(by, pane)).count());
                most.max().unwrap_or(1)
            };
            row(true) as f32 / (row(true) + row(false)) as f32
        };
        let sent = |border: &Border| border.sent(even(border));
        self.borders.iter().filter_map(sent).collect()
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
            let gap = if toward[along] > 0.0 { start - from_end } else { from_start - end };
            // It lies that way, and alongside rather than merely past a corner.
            let ((start, end), (from_start, from_end)) =
                (span(pane, 1 - along), span(from, 1 - along));
            let alongside = start < from_end - SLACK && end > from_start + SLACK;
            (gap > -SLACK && alongside).then_some((gap, start))
        };
        let near = self.panes.iter().filter_map(|pane| Some((reach(pane)?, pane)));
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

impl Pane {
    /// What it has to say in words: what it is waiting for or ended with, or how
    /// far along its work is.
    pub fn word(&self) -> Option<String> {
        let along = || Some(format!("{}%", self.along()?));
        self.activity.word().map(str::to_owned).or_else(along)
    }

    /// How far along its work is, in hundredths, while it is at work and says.
    pub fn along(&self) -> Option<u8> {
        self.progress.filter(|_| self.activity == Activity::Working)
    }

    /// All that its program says of what it is doing, on one line.
    pub fn saying(&self) -> String {
        line(&self.task, &self.message)
    }
}

/// A directory as a name short enough for a title: under a home it starts at
/// `~`, and of one many levels down only the last two are given.
pub fn folder(pwd: &str) -> String {
    // A shell may give it as an address, with its machine's name.
    let path = pwd
        .strip_prefix("file://")
        .map_or(pwd, |rest| rest.find('/').map_or("/", |slash| &rest[slash..]));
    let levels: Vec<_> = path.split('/').filter(|level| !level.is_empty()).collect();
    let (head, rest) = match levels.as_slice() {
        ["home" | "Users", _, rest @ ..] | ["root", rest @ ..] => ("~", rest),
        rest => ("", rest),
    };
    match rest {
        [] if !head.is_empty() => head.to_owned(),
        [.., parent, last] if rest.len() > 2 => format!("…/{parent}/{last}"),
        _ => format!("{head}/{}", rest.join("/")),
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
#[derive(Clone, Default)]
pub struct Pane {
    pub id: String,
    /// The terminal's title, as its program set it. At a shell's prompt, the
    /// shell and the directory it is in, where the server says.
    pub title: String,
    /// The server's name for it, which it goes by when it has no title.
    pub label: String,
    pub program: Program,
    pub activity: Activity,
    /// The line its program has to say of that, if it has one.
    pub message: String,
    /// The program's own short name for what it is at, if it gave one.
    pub task: String,
    /// How far along that is, in hundredths, where the program says.
    pub progress: Option<u8>,
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
        /// With a function on the server bound to it, the keys before it in the
        /// sequence it ends, as Rex spells them: none where it is bound alone.
        served: Option<Vec<String>>,
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
    /// The window has become the one in use, or has stopped being it, for a
    /// program in the pane that asked to be told (DEC mode 1004).
    Focus {
        pane: String,
        gained: bool,
    },
    /// Without a name, the server chooses one.
    CreateSession(Option<String>),
    /// A new tab in a session.
    CreateWindow(String),
    Split {
        pane: String,
        direction: Direction,
    },
    /// Move borders. A border asked to move again before the server has moved
    /// it goes only where it was last asked to.
    Divide(Vec<Division>),
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
    /// Do one of the server's own actions, from its configuration or of a
    /// terminal: to a pane, if one has the keyboard. `origin` is `key` or `palette`.
    Perform {
        name: String,
        args: Value,
        origin: &'static str,
        pane: Option<String>,
    },
    /// Answer another client's asking: that it was done, or by the code of an
    /// error that it was not.
    Reply {
        call: String,
        error: Option<&'static str>,
    },
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
