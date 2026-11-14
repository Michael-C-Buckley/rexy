//! What the UI asks of a pane's worker about its selection and the text looked
//! for in it, and the selected text the worker hands back.
use anyhow::Result;
use libghostty_vt::Terminal;

use super::{
    Shared,
    find::Finding,
    select::{Selecting, text},
};

/// What the UI asks of a pane's worker, which does it in the order asked. A
/// place is given in cells from the grid's top left corner, and may lie outside
/// the grid.
pub enum Request {
    /// The button that selects goes down: once to let go of what is selected,
    /// twice for a word and three times for a line. To `extend` is to have
    /// what is selected reach to here instead.
    Press {
        at: [f32; 2],
        extend: bool,
    },
    /// The pointer moves with that button down. A `block` is selected by columns.
    /// Held past the top or the bottom, the view follows it.
    Drag {
        at: [f32; 2],
        block: bool,
    },
    /// The button is let go, and what is selected is the desktop's selection.
    Release,
    /// Select all there is, history too.
    SelectAll,
    Deselect,
    /// Hand over what is selected, for the clipboard.
    Copy,
    /// Look for this text and select the match nearest the end, or with no text
    /// stop looking.
    Find(String),
    /// Select the match before the one selected, or the one after it.
    FindNext,
    FindPrevious,
}

/// Selected text a worker hands back: for the clipboard, or as the desktop's
/// selection, which a middle click pastes.
#[derive(Debug, PartialEq)]
pub enum Copied {
    Clipboard(String),
    Selection(String),
}

/// Do what the handle asked, in the order asked. True when anything was.
pub fn heed(
    shared: &Shared,
    terminal: &mut Terminal<'static, '_>,
    selecting: &mut Selecting,
    finding: &mut Finding,
) -> Result<bool> {
    let asks = std::mem::take(&mut *shared.asks.lock().unwrap());
    let any = !asks.is_empty();
    for request in asks {
        // What is selected once it is done is handed back, where it is wanted.
        let wanted: Option<fn(String) -> Copied> = match request {
            Request::Release | Request::SelectAll => Some(Copied::Selection),
            Request::Copy => Some(Copied::Clipboard),
            _ => None,
        };
        match request {
            Request::Press { at, extend } => selecting.press(terminal, at, extend)?,
            Request::Drag { at, block } => selecting.drag(terminal, at, block)?,
            Request::Release => selecting.release(terminal)?,
            Request::SelectAll => {
                let all = terminal.select_all()?;
                terminal.set_selection(all.as_ref())?;
            }
            Request::Deselect => drop(terminal.set_selection(None)?),
            Request::Copy => {}
            Request::Find(text) => finding.look(terminal, &text)?,
            Request::FindNext => finding.step(terminal, true)?,
            Request::FindPrevious => finding.step(terminal, false)?,
        }
        if let Some(wanted) = wanted
            && let Some(text) = text(terminal)?
        {
            shared.copied.lock().unwrap().push(wanted(text));
            (shared.repaint)();
        }
    }
    Ok(any)
}
