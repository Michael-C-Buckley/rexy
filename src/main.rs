use std::path::PathBuf;

use eframe::egui;

const HELP: &str = "\
rexy — a minimal Rex client

Usage: rexy [--server ENDPOINT]

  -S, --server  Unix socket, host name, ssh://, ws:// or wss:// Rex endpoint
      --probe   Verify attachment and Ghostty decoding without opening a window
  -h, --help    Show this help
      --version Show version

Defaults to your Rex home, as REX_HOME or `rex home set` chose it, then the
local Rex socket. Closing the client leaves your sessions running.";

/// Rex's home, where one has been chosen: the server this machine follows for its
/// hosts. `named` is what REX_HOME holds, which comes first, and `file` what `rex
/// home set` wrote to home.json. REX_SERVER is not asked: in a Rex terminal it
/// names that terminal's server, and Rex does not take it for home either.
fn home(named: Option<String>, file: Option<String>) -> Option<String> {
    let chosen = || {
        let home: serde_json::Value = serde_json::from_str(&file?).ok()?;
        Some(home["endpoint"].as_str()?.to_owned())
    };
    let set = |home: &String| !home.is_empty();
    named.filter(set).or_else(chosen).filter(set)
}

fn default_endpoint() -> String {
    let file = rexy::ui::state_home().map(|state| state.join("rex/home.json"));
    let file = file.and_then(|file| std::fs::read_to_string(file).ok());
    let chosen = file.is_some();
    if let Some(home) = home(std::env::var("REX_HOME").ok(), file) {
        return home;
    }
    if chosen {
        eprintln!("Rex's home.json names no home rexy can read; opening the local server.");
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: geteuid has no arguments and cannot fail.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }))
    });
    format!("unix:{}", runtime.join("rex/server.sock").display())
}

fn main() -> eframe::Result {
    let mut endpoint = None;
    let mut probe = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" | "-S" => {
                endpoint = Some(args.next().unwrap_or_else(|| {
                    eprintln!("--server requires an endpoint");
                    std::process::exit(2);
                }));
            }
            "--probe" => probe = true,
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            "--version" => {
                println!("rexy {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {arg}. Try --help.");
                std::process::exit(2);
            }
        }
    }
    let endpoint = endpoint.unwrap_or_else(default_endpoint);
    if probe {
        match rexy::backend::probe(endpoint, std::time::Duration::from_secs(12)) {
            Ok(summary) => println!("{summary}"),
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let remembered = rexy::ui::Remembered::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rexy")
            // The window draws its own frame, in the row its tabs and buttons have.
            .with_decorations(false)
            // Its corners are round, and what lies behind the window shows past them.
            .with_transparent(true)
            .with_inner_size(remembered.kept.window)
            .with_min_inner_size(rexy::ui::LEAST_WINDOW)
            .with_app_id(rexy::notify::APP_ID)
            .with_icon(rexy::ui::icon()),
        renderer: eframe::Renderer::Glow,
        // Ctrl+C, X and V belong to terminal programs; the UI maps the clipboard itself.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    eframe::run_native(
        "Rexy",
        options,
        Box::new(move |cc| Ok(Box::new(rexy::ui::App::new(cc, endpoint, remembered)))),
    )
}

#[cfg(test)]
#[path = "../tests/unit/home.rs"]
mod tests;
