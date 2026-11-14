use std::path::PathBuf;

use eframe::egui;

const HELP: &str = "\
rex-lite — a minimal Rex client

Usage: rex-lite [--server ENDPOINT]

  -S, --server  Unix socket, ws:// or wss:// Rex endpoint
      --probe   Verify attachment and Ghostty decoding without opening a window
  -h, --help    Show this help
      --version Show version

Defaults to REX_SERVER, then the local Rex socket.
Closing the client leaves your sessions running.";

fn default_endpoint() -> String {
    if let Ok(endpoint) = std::env::var("REX_SERVER")
        && !endpoint.is_empty()
    {
        return endpoint;
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // SAFETY: geteuid has no arguments and cannot fail.
            PathBuf::from(format!("/run/user/{}", unsafe { libc::geteuid() }))
        });
    format!("unix://{}", runtime.join("rex/server.sock").display())
}

fn main() -> eframe::Result {
    let mut endpoint = default_endpoint();
    let mut probe = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" | "-S" => {
                endpoint = args.next().unwrap_or_else(|| {
                    eprintln!("--server requires an endpoint");
                    std::process::exit(2);
                });
            }
            "--probe" => probe = true,
            "--help" | "-h" => {
                println!("{HELP}");
                return Ok(());
            }
            "--version" => {
                println!("rex-lite {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {
                eprintln!("Unknown argument: {arg}. Try --help.");
                std::process::exit(2);
            }
        }
    }
    if probe {
        match rex_lite::backend::probe(endpoint, std::time::Duration::from_secs(12)) {
            Ok(summary) => println!("{summary}"),
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rex Lite")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([640.0, 420.0])
            .with_app_id("dev.rex-lite.client"),
        renderer: eframe::Renderer::Glow,
        // Ctrl+C, X and V belong to terminal programs; the UI maps the clipboard itself.
        clipboard_shortcuts: false,
        ..Default::default()
    };
    eframe::run_native(
        "Rex Lite",
        options,
        Box::new(move |cc| Ok(Box::new(rex_lite::ui::App::new(cc, endpoint)))),
    )
}
