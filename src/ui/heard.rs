//! What the panes' programs have to say, and what of it is still news.
use eframe::egui::{self, Color32};

use super::{
    App,
    style::{ACCENT, AMBER, GREEN, Icon, RED},
};
use crate::model::{Activity, View};

/// What a pane's programs have had to say, and what the user has made of it.
#[derive(Default)]
pub struct Heard {
    said: Activity,
    /// With that said, the pane has been on show in this window, with the window in use.
    seen: bool,
    /// Its bell has rung since then.
    rang: bool,
    /// It was on show so when last the window drew.
    watched: bool,
}

impl App {
    /// Take in what every pane has to say, and that the panes on show have been
    /// seen to say it, unless the window is not the one in use. Each pane is left
    /// saying only what is still news of it: nothing in a muted tab, a bell that
    /// rang unseen, and what its program ended with until that has been looked at.
    pub(super) fn hear(&mut self, ctx: &egui::Context, views: &mut [View]) {
        let watching = ctx.input(|i| i.focused);
        // Something was said for the desktop, which is told what need not be passed on.
        let news = self.herald.news();
        let mut open = 0;
        for (server, view) in views.iter_mut().enumerate() {
            let shown = view.current_window().map(|tab| tab.id.clone());
            let shown = shown.filter(|_| watching && server == self.current);
            let tabs = view.sessions.iter_mut().flat_map(|s| &mut s.windows);
            for tab in tabs {
                let shown = shown.as_ref() == Some(&tab.id);
                let muted = self.muted.has(&tab.id);
                // Those out of sight are not seen, whatever tab is on show.
                let panes = tab.panes.iter_mut().map(|pane| (pane, shown));
                let hidden = tab.hidden.iter_mut().map(|pane| (pane, false));
                for (pane, seen) in panes.chain(hidden) {
                    open += 1;
                    let heard = match self.heard.get_mut(&pane.id) {
                        Some(heard) => heard,
                        None => self.heard.entry(pane.id.clone()).or_default(),
                    };
                    if heard.said != pane.activity {
                        heard.said = pane.activity;
                        heard.seen = false;
                    }
                    heard.rang |= view.rang.contains(&pane.id);
                    if seen {
                        heard.seen = true;
                        heard.rang = false;
                        if news || !heard.watched {
                            self.desktop.watched(&pane.id);
                        }
                    }
                    heard.watched = seen;
                    let said = match pane.activity {
                        _ if muted => Activity::Idle,
                        Activity::Done | Activity::Error if heard.seen => Activity::Idle,
                        said => said,
                    };
                    let said = match heard.rang && !muted {
                        true => said.max(Activity::Rang),
                        false => said,
                    };
                    if said != pane.activity {
                        pane.activity = said;
                        pane.message.clear();
                    }
                }
            }
        }
        // A pane has closed.
        if self.heard.len() != open {
            let open = |pane: &str| views.iter().any(|view| view.find(pane).is_some());
            self.heard.retain(|pane, _| open(pane));
        }
    }
}

/// The mark of a tab whose programs have something to say, in the colour of it.
pub fn dot(activity: Activity) -> Option<(Icon, Color32)> {
    let color = match activity {
        Activity::Idle => return None,
        Activity::Working => ACCENT,
        Activity::Done => GREEN,
        Activity::Error => RED,
        Activity::Rang | Activity::Blocked(_) => AMBER,
    };
    Some((Icon::Dot, color))
}
