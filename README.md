# Rex Lite

A small native Linux client for [Rex](https://www.superlogical.com/rex). Rust 1.100, egui, and libghostty. Your servers, their sessions and their tabs in a quiet sidebar, or tabs along the top if you would rather, and the terminal, set in Inter and JetBrains Mono.

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

## Other servers

The sidebar lists the server you opened, then every host that server knows of, each with its sessions. Hosts are Rex's own list, shared with the `rex` CLI and other clients:

```sh
rex hosts add studio ssh://studio
```

An `ssh://[user@]host[:port]` address is reached the way the Rex CLI reaches it: `ssh` asks that machine where its server listens and forwards the socket. It needs `rex` on the other machine and an `ssh` that connects without asking anything. A host's addresses are tried in order. A server that will not connect is tried twice more, then left alone until you press the retry button beside its name, so that a refused login is not repeated until the machine blocks you. Sessions on other servers are only listed until you open one, and are left again when you open a session elsewhere.

## Build

Rust **1.100.0** is pinned in `rust-toolchain.toml`. libghostty requires **Zig 0.17.x**; `mise.toml` pins 0.17.1. With [mise](https://mise.jdx.dev/):

```sh
mise install
mise exec -- cargo build --release --locked
```

Alternatively install Zig 0.17.x on `PATH` or set `ZIG` to its executable. `scripts/dev` also recognizes the verified local toolchain under the ignored `work/toolchains/` directory. The first build downloads the pinned Ghostty source and verifies its checksum. No Zig runtime is needed by the finished app.

## Controls

- The sidebar lists every server, its sessions, and each session's tabs. Click a tab to show it, wherever it is. `+` beside a server creates a session there, which the server names. Pointing at a session shows `+` for a new tab in it and `×` to close it; pointing at a tab shows `×` to close it. Closing a tab happens at once; a session's `×` asks for a second press.
- **Ctrl Shift B** hides the sidebar and puts the tabs along the top instead. The button at their left lists the sessions on every server: type to find one, walk the list with the arrow keys, and press **Enter** to show the one picked out. A name that finds nothing creates a session with that name; `+` beside a server creates it there.
- Right-click a session or a tab to rename it, move it, or close it. Double-click renames too, and dragging moves it among its like. A tab nobody has named goes by its terminal's title; renaming one to nothing gives it back that name.
- The icon before a tab's name is that of the program in its foreground: Claude Code, Neovim, Python and some 150 more, from Rex's own icon set. A shell's prompt stands for shells and for programs without an icon. The set is Superlogical's and is not kept in this repository: run `./scripts/icons` once before building to fetch it, or every tab shows the prompt.
- A dot marks a tab whose program says it is working (blue) or waiting for you (amber). Programs say so with OSC 7501, as Rex's own tools do. **Mute Tab**, in a tab's menu, silences its dot. Muted tabs are remembered on this computer, in `rex-lite/muted` under `$XDG_STATE_HOME` or `~/.local/state`, and no server is told.
- The two buttons at the top split the focused pane right or down, as **Ctrl Shift D** and **Ctrl Shift E** do.
- Click a pane to focus it, or press **Ctrl Shift** and an arrow to move to the pane on that side. Input and focus take ownership; only owned panes are resized.
- **Ctrl Shift Enter** zooms the focused pane, so that it fills the tab, and pressed again restores the others. **Ctrl Shift W** closes it at once, with the program in it; a tab closes with its last pane. Pointing at a pane of a split tab shows both as buttons in its header, and the pane menu has them too.
- **Ctrl Tab** and **Ctrl Shift Tab** show the next and the previous tab of the session, round and round. So do **Ctrl PageDown** and **Ctrl PageUp**.
- Drag to select; double-click a word. **Ctrl Shift C** copies, **Ctrl Shift V** pastes. **Shift** overrides a program's mouse capture for selection and scrollback.
- Hold **Ctrl** over a link to underline it and be told where it leads, and click to open it with `xdg-open`. A link is an `http`, `https`, `ftp` or `mailto` address, written out in the text or marked by a program (OSC 8). A `file` link is never opened: the terminal may be showing another machine.
- A program can copy to the clipboard (OSC 52) from the pane you are at, while the window is the one you are at. None can read the clipboard.
- **Ctrl Shift N**: new session. **Ctrl Shift T**: new tab. **Ctrl Shift B**: sidebar, or tabs along the top.
- **Ctrl + / - / 0**: terminal font size. **Esc** clears selection.
- The pane menu also offers copy/paste. **Ctrl C** still interrupts terminal programs.
- Nothing is shown while all is well. A lost connection says so above the last screen, and a failed action says so briefly at the foot of the window.
- The terminal keeps the colours Rex gives it. The tabs along the top and the headers of split panes take a dark terminal's background, so they read as one surface with it.

## What is here

Several servers at once, over Unix sockets and SSH. Sessions and tabs can be created, renamed, reordered and closed, and panes split, zoomed and closed. Server-defined tabs and splits, live terminal titles, foreground programs and program status, links, copies made by programs, terminal snapshot restoration, live output, keyboard and mouse input, IME events, copy/paste, local scrollback, and reconnection with fresh snapshots. Unix sockets have a disposable-server integration test. WebSocket/TLS transports are implemented but need remote-host testing, and a host address without a scheme, such as a Tailscale name, is not understood yet.

This first version does not include terminal search, adding or editing hosts from the window, installing Rex on another machine, image rendering, drag-to-resize splits, full Rex keymap/action compatibility, or persistent appearance preferences. Keyboard layouts and IMEs need broader hands-on testing. It targets Linux; other platforms are not claimed.

## Small by construction

- Each server has one network runtime for control and terminal transport. Each visible pane has a separate Ghostty worker; parsing never runs on the UI thread.
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
./scripts/dev build --locked
python3 tests/smoke.py
```

It tries the debug build, which is the quick one to make. Before a release, try that build too, with `--binary target/release/rex-lite`.

This additionally needs `rex`, `Xvfb`, `dbus-run-session`, ImageMagick's `import` (for optional screenshots), and the X11/XTest libraries. It checks real typing, Ctrl+C, selection/copy/paste, and typing immediately after clearing a selection. This is X11 validation; Wayland is checked by hand.

For a backend-only run, `tests/live.rs` is opt-in and requires `REX_LITE_TEST_SERVER` pointing to a disposable Unix socket under this project's `work/` directory. It sends input and creates panes/tabs; never aim it at personal sessions.

```sh
REX_LITE_TEST_SERVER="unix://$PWD/work/smoke/server.sock" \
  ./scripts/dev test --test live -- --ignored --nocapture
```

See [THIRD_PARTY.md](THIRD_PARTY.md) for protocol references and adapted terminal code attribution.
