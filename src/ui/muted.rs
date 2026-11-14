//! The tabs the user wants to hear nothing from. They are remembered on this
//! machine alone: no server is told.
use std::{collections::HashSet, io, path::PathBuf};

pub struct Muted {
    tabs: HashSet<String>,
    /// Where they are remembered. With nowhere to, none can be saved.
    file: PathBuf,
}

impl Muted {
    /// Those remembered from before, in the user's state directory.
    pub fn load() -> Self {
        let home = || Some(std::env::home_dir()?.join(".local/state"));
        let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
        let file = state
            .or_else(home)
            .map(|state| state.join("rex-lite/muted"));
        Self::open(file.unwrap_or_default())
    }

    fn open(file: PathBuf) -> Self {
        let saved = std::fs::read_to_string(&file).unwrap_or_default();
        Self {
            tabs: saved.lines().map(str::to_owned).collect(),
            file,
        }
    }

    pub fn has(&self, tab: &str) -> bool {
        self.tabs.contains(tab)
    }

    /// Mute a tab, or let a muted one speak again.
    pub fn toggle(&mut self, tab: &str) -> io::Result<()> {
        if !self.tabs.remove(tab) {
            self.tabs.insert(tab.to_owned());
        }
        self.save()
    }

    /// Forget every tab that is no longer open.
    pub fn keep(&mut self, open: impl Fn(&str) -> bool) -> io::Result<()> {
        let before = self.tabs.len();
        self.tabs.retain(|tab| open(tab));
        if self.tabs.len() == before {
            return Ok(());
        }
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        if let Some(directory) = self.file.parent() {
            std::fs::create_dir_all(directory)?;
        }
        let mut tabs: Vec<_> = self.tabs.iter().map(String::as_str).collect();
        tabs.sort_unstable();
        std::fs::write(&self.file, tabs.join("\n"))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ui_muted.rs"]
mod tests;
