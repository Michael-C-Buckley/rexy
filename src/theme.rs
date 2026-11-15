//! The colours this window gives the terminals it owns. Rex has each client say
//! its own, and a pane shows its owner's. This one says Ghostty's, where
//! Ghostty's configuration is on this machine, and otherwise none: a pane then
//! keeps the colours it has.
use std::{collections::BTreeMap, env, fs, path::PathBuf, sync::LazyLock};

use serde_json::{Map, Value, json};

/// The theme Ghostty would use here, as Rex's `set_theme` takes it. It is read
/// once: a change to Ghostty's configuration shows in the next window.
pub fn ghostty() -> Option<&'static Value> {
    static THEME: LazyLock<Option<Value>> = LazyLock::new(|| {
        let var = |name: &str| env::var_os(name).filter(|value| !value.is_empty());
        let home = var("XDG_CONFIG_HOME").map(PathBuf::from);
        let home = home.or_else(|| Some(PathBuf::from(var("HOME")?).join(".config")))?;
        let config = fs::read_to_string(home.join("ghostty/config")).ok()?;
        // A theme is a file of that name: the user's own, or one Ghostty came with.
        let shared = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let shared: Vec<_> = env::split_paths(&shared).collect();
        let named = |name: &str| {
            let own = [PathBuf::from(name), home.join("ghostty/themes").join(name)];
            let came = shared
                .iter()
                .map(|dir| dir.join("ghostty/themes").join(name));
            let mut files = own.into_iter().chain(came);
            files.find_map(|file| fs::read_to_string(file).ok())
        };
        read(&config, named)
    });
    THEME.as_ref()
}

/// The settings of a file of Ghostty's: a key and its value to a line, with
/// `#` before what is said in passing.
fn settings(text: &str) -> impl Iterator<Item = (&str, &str)> {
    let lines = text
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'));
    lines.filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim().trim_matches('"')))
    })
}

/// A colour as Rex takes one, `#rrggbb`, if it is written so. One by name, or
/// that stands for another, is left to the terminal's own.
fn color(value: &str) -> Option<String> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let whole = hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    whole.then(|| format!("#{hex}"))
}

/// The theme a configuration comes to: the colours of the theme it names, as
/// `named` finds that, with those it sets itself in place of them. Nothing, if
/// it comes to no colour at all.
fn read(config: &str, named: impl Fn(&str) -> Option<String>) -> Option<Value> {
    // Of a theme for the light of day and one for the dark, the dark: the
    // window's own colours are dark.
    let theme = settings(config).filter(|(key, _)| *key == "theme").last();
    let theme = theme.map(|(_, name)| {
        let mut parts = name.split(',').map(str::trim);
        let dark = parts.find_map(|part| part.strip_prefix("dark:"));
        dark.map_or(name, str::trim)
    });
    let theme = theme.filter(|name| !name.is_empty()).and_then(named);
    let mut colors = Map::new();
    let mut palette = BTreeMap::new();
    for (key, value) in settings(theme.as_deref().unwrap_or_default()).chain(settings(config)) {
        let name = match key {
            "background" | "foreground" => key,
            "cursor-color" => "cursor",
            "palette" => {
                let entry = value.split_once('=');
                let entry = entry.and_then(|(index, value)| {
                    Some((index.trim().parse::<u8>().ok()?, color(value.trim())?))
                });
                palette.extend(entry);
                continue;
            }
            _ => continue,
        };
        // One set to nothing is the terminal's own again.
        match color(value) {
            Some(color) => colors.insert(name.to_owned(), json!(color)),
            None => colors.remove(name),
        };
    }
    if !palette.is_empty() {
        let entries = palette
            .iter()
            .map(|(index, color)| format!("{index}={color}"));
        colors.insert("palette".into(), entries.collect());
    }
    (!colors.is_empty()).then_some(Value::Object(colors))
}

#[cfg(test)]
#[path = "../tests/unit/theme.rs"]
mod tests;
