//! Finding text in a pane, through all of its history. Ghostty looks, a page of
//! history at a time, so the worker goes on with its terminal in between. The
//! match found is selected, which shows it and lets it be copied.
use anyhow::Result;
use libghostty_vt::{
    Terminal,
    point::Space,
    search::{Search, Status},
    terminal::ScrollViewport,
};
use std::time::{Duration, Instant};

/// How long is spent looking before the terminal's own work is seen to.
const SLICE: Duration = Duration::from_millis(4);

/// The search of one pane, while there is text to look for.
#[derive(Default)]
pub struct Finding(Option<Search<'static, 'static>>);

impl Finding {
    /// Look for other text, from the end of the terminal's output back. Letters
    /// match in either case, which is all Ghostty offers. With no text there is
    /// nothing to look for, and what was found is forgotten.
    pub fn look(&mut self, terminal: &mut Terminal<'static, '_>, text: &str) -> Result<()> {
        if text.is_empty() {
            self.0 = None;
            return Ok(());
        }
        let search = match &mut self.0 {
            Some(search) => search,
            None => self.0.insert(Search::new(terminal)?),
        };
        // A match of the text looked for before is one no longer.
        let old = search.needle()?;
        if !old.is_some_and(|old| old.eq_ignore_ascii_case(text.as_bytes())) {
            terminal.set_selection(None)?;
        }
        search.set_needle(terminal, text.as_bytes())?;
        Ok(())
    }

    /// Look a little further, and select the match nearest the end once there is
    /// one. `wrote` says the terminal has changed since the last look. True
    /// while there is further to look.
    pub fn work(&mut self, terminal: &mut Terminal<'static, '_>, wrote: bool) -> Result<bool> {
        let Some(search) = &mut self.0 else {
            return Ok(false);
        };
        let began = Instant::now();
        if wrote {
            search.feed(terminal)?;
        }
        let more = loop {
            match search.status()? {
                Status::FeedRequired | Status::Running if began.elapsed() >= SLICE => break true,
                Status::FeedRequired => search.feed(terminal)?,
                Status::Running => drop(search.tick()?),
                _ => break false,
            }
        };
        if search.selected_index()?.is_none() && search.total_matches()? > 0 {
            self.step(terminal, true)?;
        }
        Ok(more)
    }

    /// Select the match before the one selected, which is the `older`, or the
    /// one after it, and bring it into view. Past the last comes the first.
    pub fn step(&mut self, terminal: &mut Terminal<'static, '_>, older: bool) -> Result<()> {
        let Some(search) = &mut self.0 else {
            return Ok(());
        };
        let found = match older {
            true => search.select_next(terminal)?,
            false => search.select_prev(terminal)?,
        };
        if !found {
            return Ok(());
        }
        let matches = search.snapshot(terminal)?;
        let found = matches.selected_match()?;
        matches.terminal().set_selection(found.as_ref())?;
        let at = match &found {
            Some(found) => {
                matches.terminal().point_from_grid_ref(&found.start(), Space::Viewport)?
            }
            None => None,
        };
        // Ghostty brings it into view at the top, where the field that looks for
        // it lies over the end of the row. It is shown a third of the way down
        // instead, where there is history above it to show.
        let rows = terminal.rows()?;
        if at.is_some_and(|at| at.y < 2) && rows > 6 {
            terminal.scroll_viewport(ScrollViewport::Delta(-(rows as isize / 3)));
        }
        Ok(())
    }

    /// Which match is selected, the newest being the first, and how many there
    /// are so far. None while nothing is looked for.
    pub fn found(&self) -> Result<Option<(usize, usize)>> {
        let Some(search) = &self.0 else {
            return Ok(None);
        };
        let at = search.selected_index()?.map_or(0, |index| index + 1);
        Ok(Some((at, search.total_matches()?)))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/terminal_find.rs"]
mod tests;
