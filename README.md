# Rex Lite

A small native Linux client for [Rex](https://www.superlogical.com/rex). Rust 1.100, egui, and libghostty. Your servers, their sessions and their tabs in a quiet sidebar, or tabs along the top if you would rather, and the terminal, set in Inter and JetBrains Mono.

Early working client. Closing the window leaves server sessions running. Connecting never starts or installs a server.

## Run

Start your existing Rex server, then:

```sh
./scripts/dev run --release
```

With no `--server`, the window opens on your Rex home: the server `REX_HOME` names, or the one chosen with `rex home set`. With neither it is this machine's own server, at `$XDG_RUNTIME_DIR/rex/server.sock` (or `/run/user/<uid>/rex/server.sock`). `REX_SERVER` is not read: in a Rex terminal it names that terminal's server, and Rex does not take it for your home either. An explicit endpoint takes precedence:

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

Each window gives a server this machine's hostname as its label, so `rex -C` can tell one machine's window from another's.

An `ssh://[user@]host[:port]` address is reached the way the Rex CLI reaches it: `ssh` asks that machine where its server listens and forwards the socket. It needs `rex` on the other machine and an `ssh` that connects without asking anything. A host's addresses are tried in order. A server on this machine that will not connect is tried again for as long as it takes, every fifteen seconds after the first tries, so the window can be opened before the server is started. Another machine is tried twice more, then left alone until you press the retry button beside its name, so that a refused login is not repeated until the machine blocks you. A server older than API revision 13, or too new for this client, is refused in words that say what it speaks and what this client needs, and is not tried again. Sessions on other servers are only listed until you open one, and are left again when you open a session elsewhere.

## Build

Rust **1.100.0** is pinned in `rust-toolchain.toml`. libghostty requires **Zig 0.17.x**; `mise.toml` pins 0.17.1. With [mise](https://mise.jdx.dev/):

```sh
mise install
mise exec -- cargo build --release --locked
```

Alternatively install Zig 0.17.x on `PATH` or set `ZIG` to its executable. `scripts/dev` also recognizes the verified local toolchain under the ignored `work/toolchains/` directory. The first build fetches Ghostty's source with `git`, at the commit its Rust bindings name, and builds it for speed whatever the Cargo profile (see `.cargo/config.toml`). No Zig runtime is needed by the finished app.

## Desktop entry

```sh
install -Dm755 target/release/rex-lite ~/.local/bin/rex-lite
install -Dm644 assets/dev.rex-lite.client.desktop -t ~/.local/share/applications
install -Dm644 assets/dev.rex-lite.client.svg -t ~/.local/share/icons/hicolor/scalable/apps
```

The entry is named for the window's app ID, which is how a Wayland desktop finds the window's icon.

## Controls

- The sidebar lists every server, its sessions, and each session's tabs. Click a tab to show it, wherever it is. `+` beside a server creates a session there, which the server names. Pointing at a session shows `+` for a new tab in it and `×` to close it; pointing at a tab shows `×` to close it. Closing a pane, a tab or a session happens at once when nothing runs in it but shells at their prompts. Otherwise it asks first, where you clicked: the `×` becomes **Close?**, or the menu's entry turns red, and a second click within a few seconds closes it. Moving the pointer away takes the question back.
- The window remembers its size, the font size, whether the tabs are in the sidebar and how wide that is, and the session and tab it last showed on each server, in `rex-lite/state.json` under `$XDG_STATE_HOME` or `~/.local/state`. It opens on home where it was, if that session is still there. Its title names the session on show.
- Drag the sidebar's edge to make it wider or narrower. Both lists of tabs turn to the tab on show when it changes, and a name too long for its place is given in full when pointed at.
- **Ctrl Shift B** hides the sidebar and puts the tabs along the top instead, where the wheel turns them. The button at their left lists the sessions on every server: type to find one, walk the list with the arrow keys, and press **Enter** to show the one picked out. A name that finds nothing creates a session with that name; `+` beside a server creates it there.
- Right-click a session or a tab to rename it, move it, or close it. Double-click renames too, in a field that opens on the name it goes by, and dragging moves it among its like. A tab nobody has named goes by its terminal's title; renaming one to nothing gives it back that name.
- The icon before a tab's name is that of the program in its foreground: Claude Code, Neovim, Python and some 150 more, from Rex's own icon set. A shell's prompt stands for shells and for programs without an icon. The set is Superlogical's and is not kept in this repository: run `./scripts/icons` once before building to fetch it, or every tab shows the prompt.
- A tab says what its program has to say of itself. A blue dot: working. In amber, what it is waiting on you for: **Needs approval**, **Question**, **Needs sign-in**, or just **Waiting**. **Done** in green and **Error** in red are what it ended with, and **Bell** that it rang; these three stay until you have looked at the tab. A split tab's panes say the same in their headers. Along the top, where room is short, a tab keeps to a dot of the same colour. Point at a tab to be told in full, with the line the program adds. Programs say all this with OSC 7501, as Rex's own tools do.
- What a tab has to say while you are not looking at it also goes to the desktop as a notification, which says which tab, session and server it is from: the words above, and any notice the program sends for itself (OSC 9 or 777). Looking at the pane takes its notification back, and clicking the notification shows the pane and puts the keyboard in it. A pane counts as looked at only while its tab is on show and the window is the one in use. One that is out of sight there does not count: behind a zoomed pane, or in a floating layer that another client has minimized. Clicking its notification brings it out. So does picking it from the list of the panes out of sight, which a button at the top opens whenever the tab on show has any: getting to a tab never rearranges it. This needs a notification service on the session's D-Bus, as GNOME, KDE and most desktops have; without one, nothing is sent and nothing else changes.
- **Mute Tab**, in a tab's menu, silences all of that for the tab. Muted tabs are remembered on this computer with the rest, and no server is told.
- The two buttons at the top split the focused pane right or down, as **Ctrl Shift D** and **Ctrl Shift E** do. Drag the line between two panes to give one room from the other, and double-click it to give both the same. There is none to drag while one pane fills the tab, or while a floating pane is on show.
- Click a pane or its header to focus it, or press **Ctrl Shift** and an arrow to move to the pane on that side. The focused pane of a split tab has a blue line along the top of its header. Input and focus take ownership. A pane is the size its owner gives it: one that another client owns keeps that client's size until you click it or type in it. One whose owner has left the server, as a pane made from the command line has, becomes this window's when it is shown.
- **Ctrl Shift Enter** zooms the focused pane, so that it fills the tab, and pressed again restores the others. **Ctrl Shift W** closes it, asking to be pressed again if a program is running in it; a tab closes with its last pane. Pointing at a pane of a split tab shows both as buttons in its header, and the pane menu has them too.
- **Ctrl Tab** and **Ctrl Shift Tab** show the next and the previous tab of the session, round and round. So do **Ctrl PageDown** and **Ctrl PageUp**.
- Drag to select; double-click a word. **Ctrl Shift C** copies, **Ctrl Shift V** pastes, to and from the desktop's clipboard. What you select is also the desktop's selection, which a middle click pastes, here or in any other program. **Shift** overrides a program's mouse capture for selection, the wheel and the middle click.
- The wheel moves through a terminal's history, three lines a notch. **Shift PageUp** and **Shift PageDown** move a screen at a time, **Shift Home** and **Shift End** go to the top and the bottom, and **Ctrl Shift Z** and **Ctrl Shift X** to the previous and the next shell prompt, where the shell marks them (OSC 133). A pane that is scrolled back says so at its foot; click that, or type, to return. A program that fills the screen, such as `less` or an editor, keeps no history: there these keys are the program's, and the wheel sends it arrow keys or, if it tracks the mouse, its own wheel steps.
- Hold **Ctrl** over a link to underline it and be told where it leads, and click to open it with `xdg-open`. A link is an `http`, `https`, `ftp` or `mailto` address, written out in the text or marked by a program (OSC 8). A `file` link is never opened: the terminal may be showing another machine.
- A program can copy to the clipboard (OSC 52) from the pane you are at, while the window is the one you are at. None can read the clipboard.
- **Ctrl Shift N**: new session. **Ctrl Shift T**: new tab. **Ctrl Shift B**: sidebar, or tabs along the top.
- **Ctrl + / - / 0**: terminal font size. With Shift or Alt as well, these go to the terminal's programs. **Esc** clears selection.
- The pane menu also offers copy/paste. **Ctrl C** still interrupts terminal programs.
- Nothing is shown while all is well. A lost connection says so above the last screen, which goes dim, and offers to reconnect. A failed action says so briefly at the top of the terminals; click it to send it away.
- The terminal keeps the colours Rex gives it. The tabs along the top and the headers of split panes take a dark terminal's background, so they read as one surface with it.

## What is here

Several servers at once, over Unix sockets and SSH. Sessions and tabs can be created, renamed, reordered and closed, and panes split, resized, zoomed and closed. Server-defined tabs and splits, live terminal titles, foreground programs and program status, desktop notifications, links, copies made by programs, terminal snapshot restoration, live output, keyboard and mouse input, IME events, copy/paste, local scrollback, and reconnection with fresh snapshots. Unix sockets have a disposable-server integration test. WebSocket/TLS transports are implemented but need remote-host testing. A host name without a scheme, such as a Tailscale name, is tried as Rex tries it: HTTPS on port 443, HTTPS on 7297, then HTTP on 7297.

This first version does not include terminal search, adding or editing hosts from the window, installing Rex on another machine, image rendering, or full Rex keymap/action compatibility. Keyboard layouts and IMEs need broader hands-on testing. It targets Linux; other platforms are not claimed.

## Small by construction

- Each server has one network runtime for control and terminal transport. Each visible pane has a separate Ghostty worker; parsing never runs on the UI thread.
- Ordered terminal bytes use bounded queues with backpressure. Redraw notices coalesce; the UI reads the newest immutable frame.
- Rows that have not changed are shared between frames, also when a scroll moves them. The renderer caches shaped text by row identity; it does not build widgets for individual cells.
- Terminal workers sleep when idle. Hidden tabs release their streams and restore a fresh snapshot when selected again.
- The first frame waits for a complete streamed snapshot. Initial limits are 32 MiB scrollback and about 1 MiB queued input per visible pane. History that arrives with a snapshot is held compressed until it is scrolled to.
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

This additionally needs `rex`, `Xvfb`, `dbus-run-session`, ImageMagick's `import` (for optional screenshots), `xclip`, and the X11/XTest libraries. It checks real typing, Ctrl+C, selection/copy/paste, that copies reach other programs and theirs come back, and typing immediately after clearing a selection. This is X11 validation; Wayland is checked by hand.

For a backend-only run, `tests/live.rs` is opt-in and requires `REX_LITE_TEST_SERVER` pointing to a disposable Unix socket under this project's `work/` directory. It sends input and creates panes/tabs; never aim it at personal sessions.

```sh
REX_LITE_TEST_SERVER="unix://$PWD/work/smoke/server.sock" \
  ./scripts/dev test --test live -- --ignored --nocapture
```

See [THIRD_PARTY.md](THIRD_PARTY.md) for protocol references and adapted terminal code attribution.
