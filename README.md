# Rex Lite

A small native Linux client for [Rex](https://www.superlogical.com/rex). Rust 1.100, egui, and libghostty. A quiet sidebar, tabs, and the terminal.

Early working client. Closing the window leaves server sessions running. Connecting never starts or installs a server.

## Run

Start your existing Rex server, then:

```sh
./scripts/dev run --release
```

The endpoint defaults to `REX_SERVER`, then `$XDG_RUNTIME_DIR/rex/server.sock` (or `/run/user/<uid>/rex/server.sock`). An explicit endpoint takes precedence:

```sh
./scripts/dev run --release -- --server unix:///path/to/server.sock
./scripts/dev run --release -- --probe --server unix:///path/to/server.sock
```

Once built, run `target/release/rex-lite` directly. `--probe` checks connection and Ghostty snapshot decoding without opening a window or sending terminal input.

## Build

Rust **1.100.0** is pinned in `rust-toolchain.toml`. libghostty requires **Zig 0.17.x**; `mise.toml` pins 0.17.1. With [mise](https://mise.jdx.dev/):

```sh
mise install
mise exec -- cargo build --release --locked
```

Alternatively install Zig 0.17.x on `PATH` or set `ZIG` to its executable. `scripts/dev` also recognizes the verified local toolchain under the ignored `work/toolchains/` directory. The first build downloads the pinned Ghostty source and verifies its checksum. No Zig runtime is needed by the finished app.

## Controls

- Click a session or tab to switch. `+` creates a session or tab; **Split** opens split directions.
- Click a pane to focus it. Input and focus take ownership; only owned panes are resized.
- Drag to select; double-click a word. **Ctrl Shift C** copies, **Ctrl Shift V** pastes. **Shift** overrides a program's mouse capture for selection and scrollback.
- **Ctrl Shift N**: new session. **Ctrl Shift T**: new tab. **Ctrl Shift B**: toggle sidebar.
- **Ctrl + / - / 0**: terminal font size. **Esc** clears selection.
- The pane menu also offers copy/paste. **Ctrl C** still interrupts terminal programs.

## What is here

Session switching, server-defined tabs and splits, terminal snapshot restoration, live output, keyboard and mouse input, IME events, copy/paste, local scrollback, and automatic reconnect with fresh snapshots. Unix sockets have a disposable-server integration test. WebSocket/TLS transports are implemented but need remote-host testing.

This first version does not include terminal search, saved-host management, SSH setup, image rendering, drag-to-resize splits, full Rex keymap/action compatibility, or persistent appearance preferences. Keyboard layouts and IMEs need broader hands-on testing. It targets Linux; other platforms are not claimed.

## Small by construction

- One network runtime handles control and terminal transport. Each visible pane has a separate Ghostty worker; parsing never runs on the UI thread.
- Ordered terminal bytes use bounded queues with backpressure. Redraw notices coalesce; the UI reads the newest immutable frame.
- Ghostty dirty rows are shared between frames. The renderer caches shaped text by row identity; it does not build widgets for individual cells.
- Terminal workers sleep when idle. Hidden tabs release their streams and restore a fresh snapshot when selected again.
- The first frame waits for a complete streamed snapshot. Initial limits are 32 MiB scrollback and about 1 MiB queued input per visible pane.
- Failed input is reported and never replayed after reconnection. Local file/image escape access is disabled.

## Tests

All test code is under `tests/`. Small unit modules in `tests/unit/` retain access to private implementation details without exposing those details as public API.

```sh
./scripts/dev test --locked
./scripts/dev clippy --locked --all-targets -- -D warnings
./scripts/dev fmt --all -- --check
```

The complete live/native check creates its own disposable Rex server, Xvfb display, and D-Bus session, then cleans them up:

```sh
./scripts/dev build --release --locked
python3 tests/smoke.py
```

This additionally needs `rex`, `Xvfb`, `dbus-run-session`, ImageMagick's `import` (for optional screenshots), and the X11/XTest libraries. It checks real typing, Ctrl+C, selection/copy/paste, and typing immediately after clearing a selection. This is X11 validation; Wayland still needs a hands-on check.

For a backend-only run, `tests/live.rs` is opt-in and requires `REX_LITE_TEST_SERVER` pointing to a disposable Unix socket under this project's `work/` directory. It sends input and creates panes/tabs; never aim it at personal sessions.

```sh
REX_LITE_TEST_SERVER="unix://$PWD/work/smoke/server.sock" \
  ./scripts/dev test --test live -- --ignored --nocapture
```

See [THIRD_PARTY.md](THIRD_PARTY.md) for protocol references and adapted terminal code attribution.
