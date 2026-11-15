# Rexy

A small, fast, native Linux client for [Rex](https://www.superlogical.com/rex), built with Rust, egui and libghostty.

![Rexy showing a split tab: an editor, a chart drawn in the terminal and a training log, beside a sidebar of sessions on three servers](assets/screenshot.png)

Every Rex server you use, with its sessions and tabs, in one window: a sidebar that tells you what each agent is doing, split panes, and a Ghostty terminal in each of them. Closing the window leaves your sessions running, and connecting never starts or installs a server.

Rexy is unofficial and not affiliated with Superlogical. It is early software for Linux, on Wayland and X11.

## Features

- **Every server in one sidebar.** Your home server and every host it knows of, over a Unix socket, SSH or WebSocket.
- **Agent status at a glance.** Each tab shows its programs, a status dot and a line such as *Working* or *Needs approval*. An activity view gathers every pane that needs you.
- **Notifications that take you there.** Click one to go to the pane that sent it.
- **Splits.** Split, resize and zoom panes, and move them between tabs.
- **A real terminal.** libghostty does the emulation: Kitty graphics, hyperlinks, search through the whole history, and your Ghostty theme.
- **Rex's own keymap**, from your home server's `init.lua`, and a command palette.

## Install

Rexy builds from source, with Rust 1.100 and Zig 0.17.x. Both are pinned for [mise](https://mise.jdx.dev/):

```sh
mise install
./scripts/icons    # optional: program icons
mise exec -- cargo build --release --locked
install -Dm755 target/release/rexy ~/.local/bin/rexy
```

Without mise, put Zig on `PATH` or point `ZIG` at it. A desktop entry and an icon to install are in `assets/`.

The program icons are Superlogical's and are not in this repository. See [assets/icons/NOTICE.md](assets/icons/NOTICE.md) before passing on a build that includes them.

## Usage

```sh
rexy                          # your Rex home, or else this machine's server
rexy --server ssh://studio    # or unix:///path, ws://, wss://, or a host name
```

The sidebar lists that server, then every host it knows of, which `rex hosts add studio ssh://studio` adds to. A server that cannot be reached is retried, so the window can be opened before the server is up. `ssh://` needs `rex` on the other machine and an `ssh` that connects without prompting.

## Keys

These are the defaults, and `rex.bind` in your home server's `init.lua` changes them. The command palette lists every action with its keys.

| Keys | Action |
|---|---|
| **Ctrl Shift K** | Find a session or tab |
| **Ctrl Shift P** | Command palette |
| **Ctrl Shift B** | Sidebar, or tabs along the top |
| **Ctrl Shift N**, **Ctrl Shift T** | New session, new tab |
| **Ctrl Tab**, **Ctrl Shift Tab** | Next and previous tab |
| **Ctrl Shift D**, **Ctrl Alt D** | Split the pane right or down |
| **Ctrl Alt** + arrow | Focus the pane on that side |
| **Ctrl Shift Enter** | Zoom the pane |
| **Ctrl Shift W** | Close the pane |
| **Ctrl Shift C**, **Ctrl Shift V** | Copy and paste |
| **Ctrl Shift F** | Find in the pane |

With the mouse: drag the line between two panes to resize them, drag a pane's header onto a tab to move it there, click beside a server's or a session's name to fold it away, double-click a name to rename it, and right-click anything for its menu. **Shift** overrides a program's mouse capture, and **Ctrl** with a click opens a link.

## Good to know

- **Your tab stays put.** The window shows the tab you chose, not the one another client or the `rex` CLI has the server show.
- **Panes fit the window that shows them.** Showing a tab takes its panes from any other client, which takes them back when it shows the tab.
- **Remote control is off by default.** `rex -C <client> do …` works only after you choose **Allow Remote Control** in the palette. With it on, any client of the server can type into a pane or close things.
- **Programs cannot reach your machine.** They can copy to the clipboard but never read it, `file` links are never opened, and pictures are accepted only as pixels sent inline.
- **State** is kept in `rexy/state.json` under `$XDG_STATE_HOME` or `~/.local/state`.
- **Not yet:** editing hosts from the window, floating panes, moving a pane to another session, animated pictures.

Rexy speaks revision 13 of Rex's control API, and refuses a server that is older or too new.

## Development

Each live pane has its own Ghostty worker thread, so parsing never runs on the UI thread, and the UI draws the newest frame it has. The last sixteen tabs shown on each server stay streamed; any other is restored from a snapshot when shown.

`scripts/dev` runs `cargo` with the local Zig toolchain found for you:

```sh
./scripts/dev test --locked
./scripts/dev clippy --locked --all-targets -- -D warnings
./scripts/dev build --locked && python3 tests/smoke.py
```

`tests/smoke.py` starts its own disposable Rex server, Xvfb display and D-Bus session, and drives the real window. It needs `rex`, `Xvfb`, `dbus-run-session`, `xclip` and ImageMagick. `tests/live.rs` changes the server it is pointed at, so it refuses any that was not made for testing.

## Credits

Rex is made by [Superlogical](https://www.superlogical.com/rex). Terminal emulation is [Ghostty](https://ghostty.org)'s, through [libghostty-rs](https://github.com/Uzaaft/libghostty-rs), and the interface is [egui](https://github.com/emilk/egui). Parts of the terminal metadata handling are adapted from [rex-rune](https://github.com/shuv1337/rex-rune). The fonts are Inter and JetBrains Mono.

See [THIRD_PARTY.md](THIRD_PARTY.md) for licences and attribution.
