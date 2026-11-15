# rex-lite review findings

A ten-reviewer pass over the code, the Rex protocol, the libraries and the running app. All ten reports are in. Nothing in the repo has been changed apart from this file.

Reviewers' working files worth keeping were copied from the temporary scratchpad to the git-ignored `work/review/`: the backend tests with a fake server that reproduce 7.2–7.6, the two-line ordering fix for 7.2, the compiled core-concision patch, the terminal probes behind section 9, and the UX screenshots and driver (`work/review/ux/`).

---

## Verdict

Taking the five suspicions in turn:

| Suspicion | Verdict |
|---|---|
| Code that can be seriously cleaned up and simplified | **Mostly no, in lines.** Two reviewers read every line: about 250 hand-removable code lines out of 9,900, and one dead item in the crate. A formatter setting removes 1,089 lines. The real cost is naming (see the last section). |
| Not using the libraries to their fullest | **Mostly no.** egui and libghostty are used well, and the big hand-rolled pieces beat the library alternatives when measured. Real under-use: libghostty's incremental snapshot decoder (9.2), live-history compression (9.3), and a handful of Rex methods (5.4, 5.5, and the table in section 5). |
| Weird decisions | **Yes, a few, all in how the client relates to the server:** shortcuts follow the server on show rather than home (5.1), pane focus never follows the server (5.2), the ownership etiquette (1.6), eight warm tabs per server (1.10, 5.7), and the pick-numbering for tab selection (1.5). |
| Strange self-imposed limits | **Yes.** Remote hosts abandoned after three tries (1.1); the working directory needing OSC 7 (5.4); new panes opening in `$HOME` (5.5); no host editing; no moving or floating panes; the first frame waiting for the whole snapshot (9.2). |
| Bad UX | **Yes, in specific places.** Ctrl K taken from the shell, tab names lost to status words in the top strip, the work list naming only the session, a palette that hides its commands, a near-invisible rename selection, and the app's private wording (section 10). |

On top of those, the review found about twenty real bugs, most of them a few lines each.

### Recommended order

**1. Fix the bugs.** Fifteen of the seventeen are fixed: commit `773a2f2` on branch `review-fixes`, plus a second pass (`217b34a`) that redid four of them properly. Format, clippy, 144 unit tests and the live smoke test pass. Two were set aside.

| Bug | Ref | Status |
|---|---|---|
| Cursor reset corrupts the stream and overrides a program's cursor | 9.1 | fixed; test updated |
| Keys typed into a new split are dropped until it draws | 1.2 | fixed; checked by hand. A pane whose feed has failed still refuses typing |
| Remote hosts that are not SSH are abandoned after three tries | 1.1 | fixed; test and README updated |
| Escape is spelled `escape`; bindings on it never fire | 2.1 | fixed; tests updated |
| A failed view request leaves a session with zero tabs and no error | 7.3 | fixed, then redone with typed errors (7.6): it is said once, in the server's words without its code, and asked again after 2 s and then every 15 s; the test now checks the re-ask |
| Stale desktop notifications from answers overtaking news | 7.2 | fixed; new test; live test passes. A request from another client that arrives while connecting is now kept and passed on once connected (new test); the first fix refused it |
| A pane's title depends on how its state was reached | 7.4 | fixed; test extended |
| A dropped client still connects and bootstraps | 7.5 | fixed; new test |
| A click on another server's row sends that frame's keys to the wrong server | 4.3 | fixed: the window turns to the other server at the next frame |
| Rename and Close Tab from the palette do nothing when the row is not drawn | 4.4 | fixed, then redone: with the work list showing, the field and the question appear in the row over the terminals and the list stays. A session's only tab asks "Close tab?" on the session's row, and pressing it closes the tab, not the session. Checked by hand |
| Extra mouse buttons arrive as left clicks | 2.3 | fixed |
| Keypad navigation keys arrive as digits with NumLock off | 2.2 | fixed; new test |
| A bound key after a sequence prefix goes raw to the terminal | 2.4 | fixed |
| Shortcut hints show shadowed keys | 2.5 | fixed; test extended |
| A URL that wraps past the bottom of the view opens truncated | 9 | fixed properly after the first commit: the frame now carries the part of a wrapped word that is out of sight above or below the view, so the whole address resolves. Uncommitted |
| A cleared selection leaves one highlighted cell | 10.21 | **not fixed.** The library gives no sign that selected text was erased, and a one-cell selection can be a real one. Needs a look inside Ghostty's selection tracking |
| `session.select` for a just-created session is refused | notes | **not a bug of that kind.** On a window that is being drawn it works at once (checked). The refusal was the window being out of sight: it takes nothing then, and refuses after 3 s |

Second pass, from 7.6: request errors are now a type (`wire::Failed`: refused with a code and words, too large, or unanswered), which also removed the comparison of an error's text to a constant (7.6b) and keeps the reason when the server closes during setup (7.6a, new test). Still open from 7.6: the connection state on `View` is a string and a flag (7.6d). SSH hosts are still left alone after three tries, by decision: repeated logins can get an address blocked.

The new connection tests use a small fake server added to `tests/unit/backend.rs` (the start of step 3). Each was checked to fail with its fix undone.

**2. Delete what is dead or stale.** *Done: commit `7363806`, and the formatter setting in `ba10b86` (54 files, 1,116 fewer lines).* The `winit` line is gone and `resvg` is at 0.50: eight fewer packages in the lock file, and on a private headless GNOME Wayland session the window comes up undecorated with no helper processes. The `winit` dependency (4.1, 8.1) and then the `resvg` pin (8.2); `Item.more` and `Item.align` (section 3); the three README statements that contradict the code (section 8). Add `rustfmt.toml` in a commit of its own.

**3. Add the fake-server test for the connection state machine (7.1)** before touching anything in step 4. *Two tests are in (`2626c56`): picking a tab, and feeding, taking and giving up the panes of a tab on show. They state today's behaviour and will change with the two decisions that rewrite it.* The harness already exists in `work/review/`.

**4. Decide the five behaviour questions.** *Decided on 2026-11-14, as recommended: shortcuts follow home (done, `2626c56`); pane focus stays the window's own (no change); a tab's panes are taken when it comes on show (to do); a picked tab stays picked (done, unstaged); remote control sits behind a setting, off at first (done, `2626c56`).* Each one removes machinery:

| Decision | Ref | Removes |
|---|---|---|
| Shortcuts follow home, not the server on show | 5.1 | per-server keymap handling |
| Pane focus follows the server | 5.2 | ~45 lines of client geometry |
| Take ownership when a tab comes on show | 1.6 | ~70 lines of request/claim logic |
| A picked tab stays picked until you pick another | 1.5 | ~80 lines and five state fields |
| Gate remote control behind a setting | 5.3 | nothing; closes a clipboard-reading hole on shared servers |

**5. Draw the first frame from the snapshot's prefix (9.2).** With cold tabs showing almost at once, the eight-warm-tabs machinery can shrink or go.

**6. Cheap features:** working directory from the process (5.4) and into new panes (5.5); move panes and split left/up; hosts from the window; live-history compression (9.3); highlight every find match; a scrollbar.

**7. UX polish (section 10) and a naming pass (last section).**

### Not confirmed — needs your desktop

- Whether the `winit` feature really spawns four helper processes at startup on Wayland (4.1).
- Whether hover is stuck after dragging or resizing the window until the next click (4.2).
- Whether Ctrl Alt Left/Right and Ctrl Alt Shift Left/Right reach the app on GNOME, or the shell takes them for workspaces (10.3).
- Whether wrapped prompts are left as garbage after resizes with your real prompt, compared against Ghostty (10.22).

How to read the confidence tags:

- **checked** — I (the lead) read the cited code myself and it holds.
- **verified** — the reviewer read both sides (rex-lite and the library or `rex api describe`), or ran something.
- **inferred** — reasoned from one side only; needs confirming before acting.

Paths are relative to the repo root. `renux:` paths are in the renux repo (`zenodea/renux`, HEAD 5adca66).

---

## Cross-cutting notes

- **The shape of the problem is not size.** Both concision passes read every line and agree: the UI half has about 180 defensible hand-removable lines out of 5,097, the core half about 70 out of 4,806. No dead functions were found in the core. The cost is in `backend.rs` state machinery, a handful of real bugs, a few places where the client departs from Rex's model, and naming.
- **The single largest line saving is a formatter setting.** There is no `rustfmt.toml`; `use_small_heuristics = "Max"` removes 1,089 lines repo-wide (537 in `src`, 548 in tests) with no behaviour change. Both concision reviewers measured it independently.
- **The backend is not bloated and its concurrency shape is right.** `backend.rs` is six jobs in one file, four of them sharing one 20-field struct, with little duplication or defensive code. It is already a single-owner actor. The reviewer found six reproducible bugs in it and no unit test for its state machine (7.1); that test gap is what makes every simplification in sections 1 and 5 risky.
- **My early worry about cloning the whole model every frame was wrong.** Measured in release with 120 panes: 18 µs per server. Leave it.
- **Mechanically the code is clean.** One dead item in the whole crate, no panics reachable from server data, clippy and fmt pass, 138 tests pass in about a second. The dependency list is lean apart from one stale entry (`winit`, 8.1) and one heavy one (`url`, 8.3).
- **libghostty is used well.** The terminal core follows the library's intended flow, and the two big hand-rolled pieces (`ApcParser`, the row reader) were measured against the library alternatives and win. There is one real bug (9.1) and two places the library is under-used: the first frame waits for the whole snapshot when it could draw after 1% of it (9.2), and history written live is never compressed (9.3).
- **The client fits the protocol well at the wire level.** The gaps are semantic: shortcuts follow the server on show rather than home (5.1), pane focus never follows the server (5.2), any client can drive the window (5.3), and the working directory needs OSC 7 when the server already reports it (5.4).
- **Two premises turned out wrong.** renux is not smaller (about 25.5K lines of Zig against rex-lite's 12.3K), and it does not embed Ghostty's surface: like rex-lite it paints cells itself from libghostty-vt. Where it is cleaner is a few state-model choices (sections 1.3, 1.4, 1.7, 1.8).
- **Where rex-lite is growing the way rex-rune grew** is `backend.rs` and the activity/notification stack (`Block.records`, `heard.rs`, `agents.rs`, `notify.rs`, about 800 lines), not the UI chrome.
- **Found while using it:** `rex -C studio do session.select session_id=…` failed once with "Rex Lite cannot do that just now" and worked two seconds later. This turned out to be the hidden-window rule, not a problem with new sessions: a window that is not being drawn takes no requests and refuses them after 3 s. Whether `session.select` should work on a hidden window is a decision, since it only changes what the window will show.

---

## 1. Comparison with renux

### Bugs and limits a user would hit

**1.1 Any host that is not a local Unix socket is abandoned after three tries.** *checked*
- `src/backend.rs:335-341`: `near` is true only when every endpoint is a single Unix socket. `src/backend.rs:300-308`: otherwise `retry()` returns `None` after 2 s and 15 s.
- The stated reason (a refused login may get the address blocked) applies only to `ssh://`. A Tailscale or `wss://` host that drops for more than about 20 s (suspend, Wi-Fi change) stays "Disconnected" until Retry is pressed.
- renux retries forever, 1 s doubling to 30 s.
- Fix: give up only when the endpoint is `ssh://`. About 5 lines.

**1.2 Keys typed into a pane that has not drawn its first frame are dropped.** *checked*
- `src/backend.rs:1516-1526` refuses any engaging input unless the terminal has a frame and no error, with the toast "Terminal is not ready; input was not sent".
- A new split takes focus as soon as its view arrives (`src/ui.rs:381-383`) but its snapshot comes later, so keys typed straight after Ctrl Shift D are lost. The same happens while one stream reconnects.
- `client.key` needs only the control connection. renux sends unconditionally.
- Fix: drop the readiness gate for keys, text and paste; keep it for mouse coordinates. About 10 lines removed.

**1.3 New splits and tabs do not inherit the working directory.** *verified*
- `shell()` passes no options (`src/backend.rs:1977-1979`) though `Block.folder` is already tracked (`:597`).
- `rex api describe block_creator.list` shows `cwd` in the shell flavour's options. renux passes the anchor pane's folder.
- Fix: about 20 lines.

**1.4 No fallback when a snapshot will not decode.** *renux side verified; rex-lite effort inferred*
- A decode error fails the stream and retries for ever (`src/terminal.rs:454-458`, `src/backend.rs:1578-1603`). The bindings are pinned to a git rev, so a server whose snapshot format has moved leaves every pane blank.
- renux creates an empty terminal and fills it from `com.superlogical.terminal.format {format: "vt"}`.
- Fix: about 40–60 lines.

### State machinery that a different choice would remove

**1.5 Tab selection: "pinned or follow" would replace the pick-numbering machinery.** *verified; a behaviour decision*
- rex-lite: `State.picks` (`src/backend.rs:78-82`), numbered actions (`:232-247`), `settled`/`unsettled`/`making`/`made` (`:561-569`, `:957-974`, `:1012-1035`), `View::settle(answered)` (`src/model.rs:93-110`). All of it follows the server's active tab while holding an optimistic choice.
- renux: one optional local tab id. Shown tab = the local id if it still exists, else the server's active one. Picking a tab sets it and tells the server as a courtesy.
- Cost of renux's way: a tab switch made by another client or the CLI does not move a window that has a pinned tab.
- Saving: about 80 lines and five state fields.
- Reviewers disagree here. The backend reviewer judged the `picks`/`settled`/`unsettled` protocol (about 70 lines) "earned" for what it does. It is only removable by accepting renux's behaviour, so this is a product decision, not a cleanup.

**1.6 Ownership is one line in renux and the largest block of special cases in `backend.rs`.** *verified both sides; which is right is a judgement*
- rex-lite: resize without taking (`src/backend.rs:1429-1444`); `own_block` "request" for others' panes on show, then `client.list`, then "take" of abandoned ones (`:1142-1190`, `:1667-1677`); `claim()` before input, focus and zoom (`:1262-1269`, `:1357-1379`, `:1527-1529`); `session.disown_block` on leaving show (`:1121-1133`); `Pane.owner` threaded into the UI.
- User-visible result: "a pane another client owns keeps that client's size until you click it".
- renux: every size it sends is `resize` plus `own_block` "take", debounced 200 ms.
- Middle path: one "take" for the visible tab's blocks when it comes on show, keep the disown on leaving. Removes request, `client.list`, `abandoned` and the per-input `claim`. About 70 lines. Cost: showing a tab resizes it for other viewers at once.
- Open question: what the official Rex apps do. That decides whether the simpler way is polite.

**1.7 Stay attached, and the attach/detach serialisation goes.** *code verified; effect inferred*
- `join()` runs one attach or detach at a time, with `joining`, `Reply::Left` and an `attached` vector (`src/backend.rs:1041-1072`, `:986-1005`). The race it avoids is real.
- renux never detaches. With `event_scope: all`, attachment buys only data tickets, a place in the ownership line and presence in `session.list_clients`.
- Saving: about 30 lines. Cost: the window stays listed in sessions it has visited.

**1.8 Keep the server's split tree instead of re-deriving it from rectangles.** *verified*
- rex-lite flattens the layout at parse (`src/backend.rs:1731-1758`), then recovers structure by geometry: `Border::parts`, `share()`, `nudged`, and `balanced` with a row-counting heuristic and 0.001 slacks (`src/model.rs:196-327`).
- renux walks the tree with the server's own `ratio`: about 45 lines for the same three features.
- Saving: about 80 lines net, and exact for nested layouts.

### Reconnect and hidden tabs

**1.9 Reconnect throws away more than it needs to, and departs from the API note.** *code checked; server support for resume inferred*
- On a resumed connection, a `session.detach` per previously attached session is awaited before `session.bootstrap` (`src/backend.rs:659-673`). `rex api describe session.bootstrap`: "Send it as the first request on a new connection."
- The `warm` list lives in `Connection`, not `Link` (`src/backend.rs:572`), so every warm tab cold-starts after a reconnect.
- The data handshake discards the resume token (`src/wire.rs:358-377`) and each stream attempt builds a new `TerminalHandle` (`src/backend.rs:1637`): scroll position, selection and find state are lost, and up to 32 MiB of snapshot is fetched again per pane.
- renux keeps the terminal and sends a RESUME frame, falling back to a fresh ticket on reject.
- Fix: move `warm` into `Link` and pass all attached ids to bootstrap (about 20 lines, removes the detach loop). Resume needs the worker to outlive a stream attempt (about 100 lines).
- The protocol reviewer ran the reconnect: a resumed client is still attached to every session it had joined, and bootstrap does not detach the others. So the detaches are needed for today's "no tab kept fed across connections" choice and do work; keeping `warm` across connections removes both them and the ordering wart.

**1.10 Hidden warm terminals keep building frames.** *checked*
- The `shown` flag gates only the UI wake (`src/backend.rs:1629-1636`). The worker publishes at up to 60 fps regardless (`src/terminal.rs:510-520`), for up to 8 tabs of panes.
- renux builds frames only at draw time.
- Fix: pass `shown` to the worker, skip `publish` while hidden, owe one frame on show. About 15 lines.

### Self-imposed limits

**1.11 "No adding or editing hosts from the window."** *API verified*
- `rex api list` has `hosts.put` and `hosts.remove`, and rex-lite already follows `hosts_changed`. A palette entry and two fields would do it. Do not copy renux's private host list.

**1.12 Colours: a pane another client owns keeps that client's look.** *verified*
- rex-lite paints whatever theme the server relays (`src/terminal.rs:574-590`). renux applies the local theme locally whatever the owner.
- Hybrid: keep sending `set_theme` (programs that query colours then get matching answers) and ignore relayed themes when a local Ghostty theme exists. About 10 lines.

**1.13 Hard pin to API revision 13.** *verified; why 13 is needed is not determined*
- The hello sets both `api_revision` and `api_min_revision` to 13 and anything else is refused for good (`src/wire.rs:29`, `:449-453`, `:482-490`). The installed server already reports revision 14.
- renux says 10 with no minimum and warns once.
- The protocol reviewer ran this: a revision-14 server welcomes the client with `api_revision: 13`, so today's servers are not refused. The pin only bites on servers older than 13. Reasonable as it stands.

### Smaller

- **README contradicts itself on hidden tabs.** "Hidden tabs release their streams and restore a fresh snapshot" and "each visible pane has a separate Ghostty worker", against the eight warm tabs elsewhere in the README and at `src/backend.rs:50`. *verified*
- `src/wire.rs:98-160`: 60 lines mapping close codes to prose; renux passes the server's reason through. *judgement*
- `src/backend.rs:554`, `:859-862`, `:933-955`: per-terminal news is buffered until four answers arrive; renux applies in arrival order with a three-state flag. *judgement*

### Where rex-lite is ahead — do not copy renux

- **Parsing off the UI thread with bounded backpressure.** renux decodes snapshots and writes VT on the GTK main thread. If rex-rune's fault was lock-ups, this is the property to keep.
- **Keymap correctness:** declares its actions to the server (so `rex -C … actions/do` works), honours exclusive and blocked modes, sends `prefix` for Lua-bound sequences.
- **Event application:** status records merged from events; `session_moved` applied locally. renux re-reads.
- **Input failures are reported**; hosts of every kind with fallbacks plus SSH tunnels; Kitty file and shared-memory media refused; clipboard reads chunked and capped; border drags latest-wins; last screen kept readable after loss; clean goodbye, request timeouts and ping liveness; no hand-rolled WebSocket; a real test suite.

### Features renux has that rex-lite lacks

Floating layers; moving and swapping panes by drag or key; detached blocks; overview grid; per-pane text zoom; smooth scroll; file drop pasted as a quoted path; folder browser; settings window and theme editor; Tailscale control; GTK input method with preedit and a Latin-layout fallback for Ctrl chords; pre-connecting a session's other tabs.

---

## 2. Terminal UI: painting, keys, pointer, keymap, commands

### Bugs

**2.1 Escape is spelled `escape`; the server spells it `esc`, so no keymap binding on Escape can match.** *checked*
- `src/ui/input.rs:222-225` builds the name from `Key::name().to_lowercase()`. `src/ui/keymap.rs:221-223` compares against `"escape"`. `tests/unit/ui_input.rs:271` asserts it.
- The server normalised `…+escape` to `…+esc` when asked. `ctrl+esc`, `shift+esc`, or `esc` bound inside a mode never fire; `pane.send_key {key:"esc"}` is refused.
- About 100 other spellings were probed and all match.
- Fix: about 5 lines.

**2.2 With NumLock off, keypad arrows, Home, End, PgUp, PgDn and Insert reach the server as `digit_N` with empty text.** *client side verified; server effect inferred*
- `src/ui/input.rs:106-109` takes the code from the physical key; egui-winit maps `Numpad8` to `Num8` and drops the keypad location.
- Fix: when the logical key is not printable, take the code from the logical key. About 3 lines. Keypad identity itself is out of reach through egui.

**2.3 Extra mouse buttons are reported to programs as left clicks.** *checked*
- `src/ui/pointer.rs:225-231`: `_ => "left"` catches the thumb buttons.
- Fix: about 4 lines.

**2.4 A key that does not continue a pending sequence goes raw to the terminal even if it is bound on its own.** *verified; partly tested intent*
- `src/ui/keymap.rs:227-235,249`, `src/ui/keys.rs:157-158`. After `ctrl+b`, `ctrl+shift+t` is sent to the program instead of opening a tab.
- Fix: about 6 lines.

**2.5 Shortcut hints advertise keys that are shadowed.** *verified*
- `src/ui/keymap.rs:123-130` returns the first binding with that action name without checking it is top of its key's stack. Tooltips, menus and the palette can show a key that does something else.
- Fix: about 6 lines.

**2.6 Directed actions the client advertises but cannot do are forwarded to the server.** *client side verified*
- `pane.split` with `left`, `up` or no direction becomes `not_supported` and is sent on as `client.action` (`src/ui/commands.rs:90-95,147-153,253`, `src/ui/keys.rs:196-201`).
- Fix: about 3 lines.

**2.7 `pane.send_key` parsing has three gaps.** *verified*
- `src/ui/input.rs:235-251`: cannot parse `+` or `ctrl++`; sends a press with no release; refuses `esc`.

### Waste

**2.8 Input RPCs are sent regardless of what the terminal asked for.** *client side verified; that the server drops them is inferred*
- Motion: a `block.event` on every cell change while hovering any pane with any tracking mode (`src/ui/pointer.rs:113-134`); `Frame` carries only a bool.
- Keys: every release and every lone modifier press goes as `client.key` (`src/ui/input.rs:88-98,141-148`).
- The local Ghostty mirror knows the tracking mode and Kitty keyboard flags. Putting both in `Frame` and gating on them halves per-keystroke RPCs for ordinary shells. About 20 lines.

**2.9 Hidden tabs keep their paint caches and picture textures.** *verified; bounded*
- `src/ui.rs:485` keeps a `PaneView` for every tab of the session; `pictures.retain` runs only inside `paint`. About 5 lines to clear.

**2.10 With the palette or find field open, the pane's cursor keeps blinking and repainting.** *verified*
- `src/ui/pane.rs:254`. One line.

### Rendering

**2.11 The rendering approach is right for the size.** *measured with the repo's own benchmark, release, 200×60*

| Screen | Shape | Paint |
|---|---|---|
| Coloured text, everything redrawn | 5.0 ms | 5.1 ms |
| Coloured text, one line scrolls | 0.21 ms | 0.125 ms |
| Shell's short lines, all redrawn | 0.32 ms | 0.34 ms |
| Wide glyphs and clusters | 0.35 ms | 0.46 ms |

- A mesh, atlas or GL-callback renderer would not pay. The row cache is sound and bounded, and nothing spins when idle.
- Only if the 5 ms case ever matters: one layout job per ASCII stretch instead of per look-run (`src/ui/render.rs:354-394`), about 30 lines.

**2.12 Nerd Font icons are drawn from egui's bundled icon fonts, then clipped to a cell with a draw call each.** *font coverage verified; visual result inferred*
- `src/ui/style.rs:194-204` puts egui's fallbacks ahead of system fonts; `emoji-icon-font` covers part of the Nerd Font range. `src/ui/render.rs:175-185` clips anything wider than its cells.
- Fix: drop Ubuntu-Light and emoji-icon-font from the monospace stacks; shrink-to-fit instead of clip. About 15 lines added, 10 removed.

**2.13 `boxes.rs` earns its place but stops short.** *coverage verified; seams inferred*
- Double lines (U+2550–256C) and Powerline triangles are left to the font, so `╔═║` frames will seam between rows while single-line ones do not. About 40 lines to cover them.

### Structure

- **Key tables:** three hand tables map keys to characters (`src/ui/input.rs:286-365`, `src/backend.rs:1940-1975`). The backend one exists only because `Action::Key` does not carry the unshifted character `input.rs` already knows; carrying it deletes about 30 lines. About 25 keys egui has and the server names are dropped silently (CapsLock, PrintScreen, Menu, media keys).
- **Command names are string literals in about 25 places** outside the table, failing silently on a typo. An enum would fix that; it is a correctness gain, not fewer lines.
- **`App::terminal_pane` is 340 lines** (`src/ui/pane.rs:37-376`); `PaneView::paint` is 180 and ends its cursor block in an early `return` that would skip anything added below.
- **`keys.rs` / `input.rs` / `keymap.rs` / `commands.rs`** split one subject four ways: dispatch (`run`, `answer`) sits in `keys.rs`, Rex key spelling sits in `input.rs`.
- **Keymap diagnostics are discarded** (`src/ui/keymap.rs:62-89`): a bad `init.lua` binding is invisible in the app. A toast is about 8 lines.

### Sound — leave alone

Key, mouse, paste and focus encoding is the server's (so Ghostty's encoders are rightly unused); the row cache; repaint discipline; the "without typing twice" logic; the IME workaround in `Keyboard::settle`; Super tracking; clipboard key handling; `resolve` against the `client.keymap` contract; wheel accumulation.

---

## 3. UI concision (`src/ui.rs` and `src/ui/*`)

**Bottom line:** 6,289 lines = 5,097 code, 888 comment, 304 blank. Hand edits worth defending remove about **180 code lines (3.5%)**. One formatter setting removes 294 more. The cost here is naming, not volume.

### Reductions, largest first

| # | What | Where | Saves | Status |
|---|---|---|---|---|
| 1 | No `rustfmt.toml`, so everything over 60 columns breaks. `use_small_heuristics = "Max"` | whole crate | 294 in UI, 537 in all of `src` | measured |
| 2 | `KEYS` re-types 40 command names already in `COMMANDS`; make it one table | `src/ui/commands.rs:28-144` | ~35 | compiled |
| 3 | `Item.more` and `Item.align` are never set by any caller | `src/ui/style.rs:541-543,553,619,662-678,699-702,740-742` | ~25 | grep |
| 4 | Toolbar: `makers` loop, `names` array, `bar` duplicating `titlebar` | `src/ui/toolbar.rs`, `src/ui/frame.rs:43-57` | ~19 | part compiled |
| 5 | "Show a session or one of its tabs" written three times | `src/ui/rows.rs:25-31`, `commands.rs:339-342`, `remembered.rs:77-83` | ~9 | estimate |
| 6 | Eleven `let font = FontId::proportional(..)` lines; make consts | several | ~9 | compiled |
| 7 | `ground` and `popover`: one-line functions with one caller | `src/ui/style.rs:156-167` | 8 | estimate |
| 8 | Smaller items (render partition, veil corners, `tabs()`, `modifier_bits`, buttons' hover fill, pills, `rule` as egui's `Separator`, palette heads) | several | ~50 | mixed |

### Checked and rejected (do not propose again)

Deriving the window title from the viewport (X11 returns an empty title, so it would resend every frame); serde derive for the state file (no direct serde dependency, and the damaged-file test needs per-field leniency); a shared overlay helper for the lost-connection note and the toast (net zero); merging `toolbar()` and `header()`; dropping `eased`; deriving `key_code` from `Key::name()`.

### Already tight — leave alone

`boxes.rs`, `motion.rs`, `raise.rs`, `borders.rs`, `pointer.rs`, `close.rs`, `find.rs`, `keymap.rs`, `keys.rs`. No module merge is worth it for lines.

---

## 4. Shell and chrome (egui/eframe 0.37)

**Bottom line:** the shell already uses egui broadly and correctly (panels, `ScrollArea`, `Popup`, `TextEdit`, drag-and-drop, `animate_bool`, viewport commands, `request_repaint_after`). `style.rs` is mostly real design, not re-derived `Visuals`. There is little under-use to fix.

### Worth acting on

**4.1 The `winit` `wayland-csd-adwaita` feature is dead, and appears to cost four child processes at startup on Wayland.** *verified by reading; not run on a compositor*
- `Cargo.toml:36-37` enables it "to draw the title bar", but `src/main.rs:93` sets `with_decorations(false)` and `src/ui/frame.rs` draws the frame.
- With the feature on, winit still constructs a hidden `sctk_adwaita::AdwaitaFrame`, whose constructor synchronously runs `dbus-send` twice, `gsettings`, and `fc-match`.
- Fix: delete the `winit` line. That also removes sctk-adwaita and ab_glyph from the build, and the stated reason for pinning resvg 0.46.

**4.2 After the compositor takes a window move or resize, egui's primary button may stay "down" until the next click.** *egui half shown by headless simulation; needs confirming on a real desktop*
- `src/ui/frame.rs:49-50,120-122` send `StartDrag` / `BeginResize`; nothing resets the pointer afterwards, and neither winit nor egui synthesises a release.
- Consequences until the user clicks once: rows do not light or offer their × and + buttons (the click meant for × selects the row instead), no hover fills or tooltips, pane animation suppressed.
- Fix: push a synthetic button release on the next `PointerGone` after either command. About 10 lines.

**4.3 `self.current` is switched mid-frame while `view` still points at the old server.** *verified by reading; a one-frame window*
- `src/ui.rs:376` binds the view; the sidebar (`src/ui/rows.rs:236-240`, `src/ui/agents.rs:134-136`) can call `open()`, which changes `self.current`. For the rest of that frame a resize or a key for an old-server pane is sent to the new server's client.
- Fix: queue the switch as `todo` already does. About 10 lines.

**4.4 "Rename Tab", "Rename Session" and "Close Tab" from the palette or a key do nothing visible when the target row is not drawn.** *verified by reading*
- In the activity-list sidebar the rename field is created, never shown, and dropped (`src/ui/commands.rs:353-361`, `src/ui.rs:492-496`).
- For a lone-tab session, Close Tab asks under the tab's row id but only the session's row is drawn: no "Close?" appears, and a second invocation within 4 s closes it.
- Fix: about 6 lines.

### Lower priority

- **Every frame deep-clones every server's whole model** (`src/ui.rs:360-363` → `src/backend.rs:197-221`), including on cursor-blink frames. The backend reviewer measured it: 18 µs per server with 120 panes, in release. Not worth changing.
- **`rows.rs` `session()` and `tab()` are two near-parallel builds of the same row** (`:134-220`, `:243-332`), and the pane header builds it a third time. One parameterised builder would save about 30–40 lines. *inferred*
- **Metrics are bare literals.** Colours are centralised in `style.rs`; row heights and insets are not (`28.0` in five places, `32.0` in six, the `12.0` inset in five). Menu entries are 30 high while palette rows are 32, which looks accidental.
- **Labels:** `.selectable(false)` is repeated six times; setting `interaction.selectable_labels = false` once replaces them. The server heading (`src/ui/sidebar.rs:157`) is selectable today, probably by accident.
- **"How long it has been so" restarts when a hidden window is shown again**, because the stamp is taken when `ui()` first notices (`src/ui/heard.rs:53-57`). *inferred*
- **README:97** still lists "moving panes by key" as absent though the Controls section documents it.

### Hand-rolled pieces that should stay

The two-line `item()` row (egui's atoms cannot do it); the sidebar resize edge; drag reorder; `motion.rs` (egui's animation has no easing or retarget); close-confirmation and toasts; the window frame; `raise.rs` (winit applies a foreign activation token only at window creation, and `focus_window` is empty on Wayland); the resvg icon pipeline; the state file (eframe persistence would mean RON and losing per-field fallback).

---

## 5. Rex protocol fit

**Bottom line:** the client fits the protocol well at the wire level. It uses 35 methods correctly. The gaps are four places where it departs from Rex's model, one small bug (the Escape spelling, 2.1, found independently here too), and a batch of cheap server features it ignores.

Claims marked *ran* were probed against a disposable server, never the real one.

### Where the client departs from Rex's model

**5.1 Shortcuts follow the server on show; Rex's model is that they follow home.** *verified by reading both sides*
- `src/ui/keys.rs:75-77` resolves keys from `views[current].keymap`, and every server is asked for a keymap (`src/ui.rs:62-75`).
- `rex home --help`: home is "the server this device follows for hosts and shortcuts." `client.action` has `servers` / `server` fields and a `client_call_requested` event so a config action on home can reach another server.
- Effect: with a remote host's session on show, the user's own `init.lua` rebinds, sequences, modes and custom actions are replaced by that host's config. Lua actions using `rex.server()` cannot work.
- Cheap half: always resolve from home's keymap and ask only home. About ±10 lines.
- Full: send config actions to home with `server`/`servers` and handle `client_call_requested`. About 80–120 lines.

**5.2 Pane focus is window-local and never follows the server's `focused_block_id` while the focused pane stays on screen.** *verified by reading; may be deliberate*
- `src/ui.rs:384-388`, `src/ui/pane.rs:215-223`.
- Effect: `rex focus BLOCK` from the CLI, a Lua action that calls `focus_direction`, or another client's focus change does not move the keyboard here. Tab selection does follow the server, so the two are inconsistent.
- If focus followed the server, `Window::beside` and the next/previous logic could become `session.focus_direction` and `session.focus_next_pane` (about 45 lines plus tests removed).
- Against: local focus is instant, and following the server lets another attached client steal the keyboard. The comment at `src/ui.rs:379-380` says the current behaviour is on purpose.

**5.3 Any client on the server can drive this window; Rex's own app gates this behind Remote Control, off by default.** *verified by reading both sides*
- `src/wire.rs:662-668` drops the event's `requester`; `src/ui/keys.rs:219-239` performs whatever is asked, including `pane.paste` (which pastes the desktop clipboard into a pane), `pane.send_key` and `pane.close`.
- Harmless on a single-user server. On a shared server, or a tailnet with members allowed, another person can run `client.perform pane.paste` and read the clipboard from the terminal.
- Fix: a remembered setting that answers `refused` when off, optionally always allowing `session.select`. About 10–15 lines.

**5.4 "The shell has to report its directory (OSC 7)" is self-imposed.** *ran*
- `src/wire.rs:683-686` keeps only program and pid from `process_changed`; the folder comes only from `pwd` (`src/backend.rs:46,803,948`). `model::folder` guesses home from `/home/<anyone>`.
- `process_changed` carries `foreground.cwd`, and `process` returns `home`. In a `/bin/sh` pane with no OSC 7, `cd /tmp` produced `process_changed` with `cwd` = `/tmp` while `pwd` returned `{}`.
- Fix: take the folder from `foreground.cwd` and `~` from `home`. The `pwd` call, `News::Folder`, the `file://` stripping and the home guess all go. Net about −10 lines and the limitation disappears.

**5.5 New tabs and splits ignore the shell's creation options.** *ran*
- `src/backend.rs:1977-1979` sends a bare flavour. Every new pane opens in `$HOME` at 80×24 with built-in colours, then is resized and themed afterwards.
- `cwd`, `size` and `theme` are creation options and are honoured.
- Fix: pass `cwd` (from 5.4) and `theme` (already held). About 10 lines. This is 1.3 with the mechanism confirmed.

### Smaller protocol points

**5.6 The stale-owner workaround is needed, but its presence test is too wide.** *departed-owner case ran; the other inferred*
- `src/backend.rs:1177-1190,1672-1677`: `own_block request`, then `client.list {}`, then `take` for owners no longer present. A block made by `rex new` does keep the departed CLI client as owner, so the workaround is justified.
- Gap: a creator still connected but never attached (`rex run --wait`, an agent's long CLI call) counts as present, so the pane stays at 80×24 until clicked.
- Fix: pass `{"session_id": …}` to `client.list`. One line.

**5.7 The warm set is per server and survives leaving that server.** *verified by reading*
- `Client::leave` only disowns; `join` keeps any session with a warm tab (`src/backend.rs:1056-1068`). So up to 8 streams and Ghostty workers per server stay live, and the window stays listed as attached, even for servers not on show.
- The number 8 and the per-server scope are choices; the protocol forces only that a data stream needs an attachment.
- Fix: make the warm set window-wide, or drop a server's on `leave`. A few lines.

**5.8 Hosts: the list is narrower than Rex's.** *verified by reading*
- `rex hosts --help`: "The hosts of this machine's own server always count too, whatever your home is." The window lists only home's hosts (`src/ui.rs:221-238`), so with a remote home the local server never appears.
- `host_id` is not read, so a home that lists itself would show twice.

**5.9 Declared client actions carry no argument schema.** *ran: nothing is broken*
- `src/ui/commands.rs:192-201`. Schemas for the six actions that take arguments are about 15 lines; they make `rex -C … actions` list them and let the server reject bad calls.

**5.10 `seq` is ignored.** *ran*
- Every response and event carries `seq`; `src/wire.rs:626-671,792-805` drop it. The view path is correct without it (revision-gated). The one exception is a `session_moved` delivered before the bootstrap response. Tiny race; about 10 lines; low priority.

**5.11 One avoidable round trip per pane at connect.** *ran*
- `session.bootstrap` can return `data_tickets` for visible blocks; the client calls `data.connect` per stream (`src/backend.rs:1614-1625`). About 15 lines; only matters on slow links.

### Cheap features the README lists as missing

| Feature | How | Size | Status |
|---|---|---|---|
| Move a pane left/right/up/down | `session.swap_blocks` with the neighbour `beside()` already finds | ~12 lines | ran |
| Split left / up | `"side": "before"` at `src/backend.rs:1337` | ~6 lines | read |
| Float, minimise, dock a pane | `session.float_block`, `set_layer_state`, `move_block`; the client already draws floating layers | 40–60 lines without drag-to-move | ran |
| Drag borders while a floating pane shows | `resize_pane` works "in any window, active or not" (`src/backend.rs:1772-1776` blocks it) | few lines | read |
| Add / remove hosts | `hosts.put`, `hosts.remove`; `hosts_changed` already handled | 50–70 lines | read |
| Close session, close other tabs/panes, reload config | existing methods; config reload is Unix-socket only | trivial each | read |

`terminal.clear` and `terminal.reset` are already reachable from the palette.

Not cheap yet: upload-on-drop and download. The byte-stream framing (`rex.data.bytes.v1`) is not in `rex api describe` or the public docs.

### README policies judged

| Policy | Verdict |
|---|---|
| Revision 13 gate | Self-imposed pin, reasonable (see 1.13) |
| "A pane is the size its owner gives it" | Forced by the protocol; request-on-show, take-on-click is a sound, polite choice |
| "Counts as looked at only while…" | Client policy; the server has no seen-state, so nothing is duplicated |
| "Connecting never starts or installs a server" | Deliberate difference from the CLI |
| "`REX_SERVER` is not read" | Matches `rex home --help` |
| "Failed input is reported and never replayed" | Sound |
| Remote hosts tried twice more then left | Self-imposed and broader than its reason (see 1.1) |

### Sound — leave alone

The view-refresh revision logic (`Held`, exactly what `session.view` prescribes); `event_scope: "all"`; the per-terminal detail calls (no batch method exists); keymap request shape and client-side sequence and mode resolution ("waiting is the client's job"); input ordering; `clipboard_read` paging; naming borders by the first pane of each side; `focus_block` to reveal; default-label detection; connection probing order and the data handshake; `ssh.rs`.

---

## 6. Core concision (backend, wire, terminal, model, notify, ssh, program, link, theme, main)

**Bottom line:** 6,008 lines = 4,806 code, 891 comment, 311 blank. No dead functions, no one-implementation traits worth removing, no history-narrating comments. Hand edits worth defending remove about **70 code lines**. The formatter setting removes 247 here. The cost is vocabulary, not line count.

The items marked *compiled* were applied together to a scratch copy that passes `cargo check --tests` and `cargo test --lib` (138 passed).

### Reductions, largest first

| # | What | Where | Saves | Status |
|---|---|---|---|---|
| 1 | `rustfmt.toml` with `use_small_heuristics = "Max"` | whole crate | 247 here; 1,089 repo-wide | measured |
| 2 | `CursorShape` re-declares Ghostty's `CursorVisualStyle` | `src/terminal.rs:54-60,882-887` | 12 | compiled |
| 3 | Pane geometry as `at: [f32; 4]` instead of `x, y, width, height` (`Border.room` already is) | `src/model.rs:466-469` and users | 14 | compiled |
| 4 | `Ended` Display as a string table | `src/wire.rs:98-160` | 10 | compiled |
| 5 | `Pane { … }` spells out seven defaults; use `..Pane::default()` | `src/backend.rs:1794-1813` | 7 | compiled |
| 6 | Same thread-plus-runtime block twice, failing three different ways | `src/backend.rs:170-189`, `src/notify.rs:139-154` | ~10 | estimate |
| 7 | `Shared::status(message, connected)` is passed `false` by every caller | `src/backend.rs:110-115` | ~5 | estimate |
| 8 | `retain` with a side effect → `extract_if` | `src/backend.rs:1135-1141` | 4 | compiled |
| 9 | `Client::terminal` returns an error string both callers discard | `src/backend.rs:286-292` | 3 | compiled |
| 10 | `text` closure defined four times; three aliases for `Result<Value, String>`; cell-to-text loop twice; `geteuid` fallback twice | several | ~10 | estimate |

### Checked and rejected (do not propose again)

- **serde derive for the wire types.** Would save about 30 lines in `wire::Event` but lengthen `parse_windows` and `layout`, add a direct dependency, and fail harder on odd payloads. `value["a"]["b"].as_str()` is the lean form here.
- **Merging the six pane-keyed maps.** They have three different lifetimes; merging any pair saves at most 5 lines.
- **Removing `Wake`'s `pending` flag.** It is what bounds repaint requests for a hidden window.
- **Splitting the long functions** (`answered` 125 lines, `reconcile` 115, `perform` 140, `input` 112, `read` 120, `frame` 118). Each is one match or one pass; splitting adds signatures and saves nothing.

### Already tight — leave alone

`terminal/find.rs`, `terminal/request.rs`, `terminal/select.rs`, `ssh.rs`, `theme.rs`, `link.rs`, `main.rs`, `program.rs`, `notify.rs`. In `wire.rs`: `targets`, `handshake`, `control_loop`, the `Rpc`/`Request`/`Pending` trio. In `terminal.rs`: `ApcParser`, `Input`, `read()`, `Renderer::frame` (rename the locals, do not restructure). In `backend.rs`: `Held`, `retry`, `run`, `run_connected`, `join`, `show`, `layout`.

---

## 7. Backend internals (`backend.rs`, `wire.rs`, `ssh.rs`, `notify.rs`, the UI boundary)

**Bottom line:** the backend is already a single-owner actor and is not bloated. No different concurrency shape is clearly better. The reviewer found six demonstrable bugs, five reproduced against a 70-line in-process fake server. The densest 880 lines have no unit test.

### How it is put together

- **UI thread:** each frame takes a snapshot per server (lock, clone the `View`, clear the one-shot fields). Writes go into a bounded queue of actions, with the selection written optimistically under the same lock.
- **One I/O thread per server**, each with its own single-threaded tokio runtime. `Connection` (20 fields, `&mut self`) selects over control-task end, events, replies and actions.
- **Tasks on that runtime:** the control socket's loop; one stream task per fed pane; a border mover; short detached tasks (refusal timers, ownership claims, clipboard fetch).
- **One terminal thread per fed pane** (libghostty), fed by a bounded queue with async backpressure.
- **One notification thread** (zbus).
- **Shared state:** one `Mutex<State>` per server, plus a few atomics. No lock is held across an await at any of the 33 lock sites.

### Why `backend.rs` is 2,019 lines

| Responsibility | Lines |
|---|---|
| UI handle and shared state | 290 |
| Connection lifecycle and retry | 175 |
| Model sync (open, ask, view, event, answered) | 620 |
| Attach, warm tabs, streams, ownership | 340 |
| Action → request encoding | 340 |
| JSON → model decoding | 225 |
| Probe | 35 |

A split along those seams (`backend.rs` about 500, `backend/connection.rs` about 800, `backend/stream.rs` about 150, and a pure `protocol` module for all encoding and decoding) saves roughly **zero lines net**. The gain is seams and testability. It moves 1.6K lines guarded only by live tests, so 7.1 comes first.

### Bugs

**7.1 The `Connection` state machine has no unit test.** *verified; both the backend and the deps reviewers flagged it*
- No test builds a `Connection`. `open`, `event`, `answered`, `show`, `join`, `reconcile`, `perform`, `input` (`src/backend.rs:653-1535`, about 880 lines, three of the eleven functions over 100 lines) run only under the ignored `tests/live.rs` and the smoke script.
- A fake server of about 70 lines (a Unix listener with scripted replies, modelled on `serve` in `tests/unit/wire.rs:541`) makes it testable. It was built and used to reproduce 7.2, 7.3, 7.5 and 7.6. A copy is in `work/review/backend-tests-with-fake-server.rs` (the new part starts at line 564).

**7.2 An answer can be handled before news the server sent ahead of it, raising stale desktop notifications.** *reproduced; fix tried on unit tests only*
- `src/backend.rs:441-453`: the select is unbiased, and events are gated `if connected` while replies are not.
- At connect, deterministically: wire order blocked → working → answers(working) produced a "Waiting" notification for a pane that ends up Working. In steady state: 17 of 60 bursts.
- Trial fix: `biased;` and dropping `, if connected` (two lines, in `work/review/ordering-fix.diff`). Result: 0 of 60, existing tests still pass. Not run against a live server. Caveat: a perform request could then reach the UI just before `connected`.

**7.3 A `session.view` that fails is never asked for again; at connect the session shows zero tabs and no error.** *reproduced; code checked*
- `src/backend.rs:975-985` clears `asking`, then acts only on `Ok`. The request timeout takes the same path.
- Cause: errors are strings, so "the session just ended" cannot be told from anything else (7.6).
- Fix: about 10 lines once errors are typed.

**7.4 A pane's title depends on how its state was reached.** *reproduced; code checked*
- `fill` (`src/backend.rs:1875-1880`) has no final `else`. At a prompt the title is "zsh ~/code"; when a titleless program takes the foreground, the live view keeps "zsh ~/code" while a freshly parsed view says "Terminal".
- The existing test builds a fresh pane each time, so it cannot see this.
- Fix: about 5 lines.

**7.5 A `Client` dropped while connecting still connects and bootstraps.** *reproduced*
- `src/backend.rs:131-141` only closes the queue, which is read only once connected. Reached by "Connect" with a new address and by host-list changes.
- Fix: select the connect phase against the action queue. About 6 lines.

**7.6 Errors are strings at every boundary, and the reason for a close during setup is lost.** *(a) reproduced; the rest by reading*
- (a) `Connection::open` is awaited without watching the control task (`src/backend.rs:425`). A close during bootstrap shows "Rex control connection ended: channel closed" although the plain words for the real reason exist at `src/wire.rs:110`.
- (b) `src/backend.rs:1718` compares an error's text to a constant.
- (c) `src/wire.rs:713-722` flattens `{code, message}` into one string, which causes 7.3 and puts raw protocol codes in toasts.
- (d) `src/ui.rs:317` strips a prefix off a string built in `backend.rs`; `View.status` and `connected` can disagree.
- Fix: a small `RpcError` enum and a connection-state enum on `View`. About +30/−15.

### Smaller

- **`Pane.status` is a string with four writers, and its emptiness is read as "live"** (`src/backend.rs:210-214,369,1590-1595,1807-1810`; read at `src/ui/pane.rs:54`). Any view refresh wipes "Reconnecting terminal: why". A per-pane enum would fix it. *inferred*
- **A repaint can be lost when a warm tab comes on show** (`src/backend.rs:1036` notifies before `shown` is set at `:1146`). Two lines. *inferred*
- **`Client::drop` blocks the UI thread for up to 400 ms per connected server**, one after another (`src/backend.rs:131-141`), mid-frame on a host-list change and at exit. About 8 lines to close all queues first and wait once.
- **ssh authenticates twice per connect** and again on every reconnect (`src/ssh.rs:97-111,174-185`). Caching the remote socket path is about 6 lines.
- **Typing waits behind request bursts** on a slow link: connect issues a view per session plus four requests per terminal through the same 64-slot queue that carries input. *inferred; not measured*
- **A stream task that panics is never noticed**, and detached tasks outlive the connection. One `JoinSet` owned by `Connection` would cover both.
- **The one-shot fields in `View` are an event queue stored as state** (`src/model.rs:11-23`). `error`, `made` and `copied` are `Option`, so the later of two between frames overwrites the earlier; clone-then-clear copies up to 8 MiB of clipboard text under the lock. A `Vec` taken with `mem::take` is the same size and correct.
- **The subprotocol check is redundant** (`src/wire.rs:273-280`); tungstenite 0.30 already errors on it. −8 lines. *verified in source*
- **`input`'s wildcard arm** (`src/backend.rs:1409-1506`) silently drops any future `Action` variant.

### Sound — leave alone

Lock discipline; the disconnect protocol (input cannot leak into a replacement connection, and this is tested); wake coalescing, bounded queues and backpressure; `control_loop` (tungstenite gives no correlation, timeouts or keepalive); flume in `terminal.rs` (it needs async send with blocking `recv_timeout`, which neither tokio nor std offers); the raw message stream in `notify.rs`; `ssh.rs`; a thread per server and per pane.

---

## 8. Dependencies, build and tests

**Bottom line:** the suspicion of accumulated cruft is mostly not borne out mechanically. One stale dependency, two large untested areas, a few doc statements that no longer match the code, and some build friction.

### Gates

| Command | Result |
|---|---|
| `./scripts/dev clippy --locked --all-targets -- -D warnings` | passes, no warnings |
| `./scripts/dev fmt --all -- --check` | passes |
| `./scripts/dev test --locked` | 138 lib + 1 bin pass; lib tests 1.02 s |
| Clean release build | 92 s wall, 374 units, 24.3 MB binary |

### Dependencies

**8.1 The direct `winit` dependency is stale.** *verified by build and by two reviewers independently*
- See 4.1 for the mechanism. Removing it was built in a scratch copy: 277 → 272 crates, binary −220 KB.

**8.2 Removing it unpins `resvg`.** *inferred from crate metadata; not built*
- `Cargo.toml:22-23` holds resvg at 0.46 to "share tiny-skia with the title bar's". 0.50 would remove four duplicate crate versions that exist only because of the pin.

**8.3 `url` costs 27 of 277 crates.** *crate count verified; replacement inferred*
- One user, `src/wire.rs`. `http::Uri` is already compiled through tungstenite.
- Saving: 27 crates, no wall-time gain. Cost: non-ASCII host names stop working and about 40 lines are rewritten. A judgement call.

**Everything else earns its place.** `anyhow`, `eframe` (features are lean), `flume`, `futures-util`, `libc`, `percent-encoding`, `raw-window-handle`, `rustls`, `serde_json`, `tokio` (all seven features used), `tokio-tungstenite`, the three wayland crates (winit has no API for a foreign activation token), `zbus`. Small trims: `flume` and `futures-util` without default features drop one crate each.

**serde derive would not shrink the code.** It is already compiled through zbus, but parsing is deliberately tolerant per field. Three reviewers reached this independently.

### Build

**8.4 The dev profile carries full debuginfo for every dependency.** *measured*
- `target/debug` is 25 GB; the debug binary is 491 MB. Adding `debug = false` under `[profile.dev.package."*"]` gives 101 MB while keeping the project's own debuginfo. Disk only; rebuild time is unchanged.

**8.5 Zig discovery is fragile.** *verified*
- The version is written in three places (`mise.toml:2`, `scripts/dev:5` with a hard-coded x86_64 path, `README.md:38,45`). Nothing in the repo populates `work/toolchains/`.
- Plain `cargo` and `./scripts/dev` invalidate each other's build, because `ZIG` is in the build script's rerun triggers. Plain `cargo check` fails with "failed to execute zig build".
- Fix: one mechanism, and have `scripts/dev` fail with a message when no Zig is found.

**8.6 Clean-build wall time is bounded by Zig, not crate count.** *measured*
- The libghostty build script takes 66 s of the 92. Dropping crates saves CPU and download, not wall time.

**8.7 `strip = "debuginfo"` keeps 3.6 MB of symbol tables.** *measured*
- `strip = true` gives 20.7 MB. The cost is symbol names in panic backtraces.

**Where the 24.3 MB goes:** egui/eframe/winit/wayland 3.1 MB of code; the text stack 2.1; std 2.1; libghostty 1.95; TLS 1.05; rex-lite itself 0.83; resvg 0.83. Data: 156 icon SVGs 1.41 MB, egui's default fonts 1.42 MB, Inter 0.88, JetBrains Mono 0.30. Nothing else is cheap to drop.

### Tests

**8.8 About 2,500 of 6,289 UI lines have no test module.** *verified*
- `ui.rs`, `pane.rs`, `rows.rs`, `keys.rs`, `sidebar.rs`, `toolbar.rs`, `frame.rs`, `heard.rs`, `find.rs`, `raise.rs`.
- A headless egui harness already exists (`tests/unit/ui_style.rs:11-40`), so this is a choice, not a limit.

**8.9 About 200 lines of benchmark live in the unit tests** (`tests/unit/terminal.rs:752-928`, `ui_render.rs:140-165`) and install a global allocator that all 138 tests then run under. Move them to `benches/` or behind a feature.

**8.10 `tests/unit/home.rs` (one test) forces a second 423 MB test binary** for `main.rs`. Moving `home()` into the lib removes it.

**8.11 Timing assertions.** Eight "nothing drawn within 80 ms" checks in the terminal tests can pass vacuously on a slow machine. Stable in 7 of 7 runs under load.

The `#[path]` test arrangement is sound: it widens no visibility. Tests are behaviour-level.

### Docs against code

Twenty spot-checked README claims hold, including every key chord. Three do not:

- `README.md:104` says hidden tabs release their streams; eight tabs are kept fed.
- `README.md:101,105` say "each visible pane" has a worker; every fed pane has one.
- `src/backend.rs:1981` calls `--probe` read-only, but it attaches to a session and can take ownerless panes. *inferred from reading*

**The README restates the key table by hand.** Controls is 13.3 KB of the 20.8 KB file and repeats every default chord from `KEYS` in prose, with nothing tying the two together. Generate that part from the table, or point at the palette.

### Small

- XDG lookup is written three ways; with `XDG_STATE_HOME=` set empty, state would go to a relative path (`src/ui/remembered.rs:24-28`). One six-line helper.
- No CI and no `[lints]` table; the three gates are manual.
- `scripts/icons` pulls from a personal GitHub repo pinned by commit, without a checksum.
- Long functions: `terminal_pane` 278 code lines, `item` 153, `paint` 145, `palette` 138, `App::ui` 132, `perform` 129, `deed` 124, `answered` 113. Large types: `App` 22 fields, `Connection` 21, `Item` 19, `Action` 22 variants.

---

## 9. Terminal core and libghostty use

**Bottom line:** the core is in better shape than suspected. It follows the library's intended flow closely, and the two big hand-rolled pieces are justified by measurement. One real bug, two places where the library is under-used.

Timings are release builds, 200×60 grid. Probe code is in `work/review/terminal-tests-with-probes.rs` (after the `probes` marker) and `work/review/terminal-probe-live.rs`.

### Bug

**9.1 The cursor reset in `configure()` is obsolete, overrides an explicit request, and can corrupt the stream.** *reproduced; code checked*
- `src/terminal.rs:435-442`: after `set_default_cursor_blink(Some(true))` it builds a throwaway render state and, if the cursor is a steady block, writes `ESC[0 q`. The comment says the snapshot "cannot say whether that was asked for".
- The snapshot does say: its header carries `cursor_is_default`, and `set_default_cursor_blink` re-resolves a default cursor at once. With those lines removed, a plain shell's cursor still blinks, and a program that asked for a steady block keeps it.
- Harm today, one: a steady block a program asked for is made to blink.
- Harm today, two: the write goes into a parser that may be mid-sequence. With `CSI 2 q` in effect and a split snapshot, the worker drew `last1;2;3mX` for `lastX` (split inside a colour sequence), `a���` for `a界` (split inside UTF-8), and leaked a title onto the screen.
- Fix: delete lines 439-442 and correct the comment. About −5 lines and one fewer FFI object per pane. The test at `tests/unit/terminal.rs:446-452` pins the old premise and needs updating.

### Under-use of the library

**9.2 The first frame waits for the whole snapshot.** *measured*
- `src/terminal.rs:414-423` uses the one-shot `decoder.decode()`. Every tab outside the eight kept warm restores afresh when selected.
- The library has `Decoder::ready()`, which returns a renderable terminal after the screen prefix, then an incremental decoder for history pages and pictures.
- On a full 32 MiB history (a 21 MiB snapshot): ready after 219 KiB read, first frame at 0.27 ms. The one-shot restore takes 52.6 ms of CPU after every byte has arrived. Over SSH that is 1% of the bytes before the first frame instead of 100%.
- The change must also re-apply the scrollback limit after the finish, redraw once for pictures, and keep the size cap. About 12 lines.
- This would let the eight-warm-tabs machinery (1.10, 5.7) shrink or go: a cold tab would show almost at once.

**9.3 Only restored history is compressed.** *measured*
- `src/terminal.rs:421` is the only compression call. The library's `compress(Incremental)` with an activity token is its documented idle-time flow.
- A pane that restored and then received the same amount live: 0 of 84 pages compressed, 32.6 MiB resident. Select All opens every page for good.
- Incremental compression of a full live history: 94 steps, 38 ms in total, longest step 1.16 ms, 32.6 → 15.2 MiB.
- Fix: about 10 lines in the worker's idle arm. Matters with up to eight warm tabs times their panes.

### Smaller

- **A URL that wraps past the bottom of the view is opened truncated** (`src/link.rs:50`). Refuse when the last visible row still wraps. 1–2 lines. *checked*
- **The snapshot size cap can contradict the picture cap** (`src/terminal.rs:42,47,415,418`). The decoder is told to drop pictures over 128 MiB per screen, but the 256 MiB read limit rejects the whole snapshot first, and the attach is then retried every 15 s, downloading again each time. *inferred*
- **The worker's real error can be reported as "terminal worker stopped"** (`src/terminal.rs:234-237` vs `:255-262`): the queue closes before the error is stored. About 2 lines.
- **Each output byte is copied two or three times before parsing** (`src/wire.rs:342`, `src/terminal.rs:249-250,372`). Not a bottleneck.
- **OSC 8 links are read per cell through a slow path** (`src/terminal.rs:789-803`): 660 µs per frame for a screen of links against 64 µs for the same words. A library gap, not a misuse; fine as is.

### Library features that map to visible gaps

| Gap | Library call |
|---|---|
| Find highlights only the selected match | `Snapshot::viewport_matches` highlights every match in view |
| Kitty animations sit on their first frame | `kitty_graphics_animation_tick` is never called |
| No scrollbar | `scrollbar()` gives total, offset and length; `ScrollViewport::Top` / `Row` |
| Bar cursor at a prompt uses the server's asynchronous idle flag | `is_cursor_at_prompt()` |
| Cursor blink is hard-coded on | `set_default_cursor_style` could follow Ghostty's `cursor-style` settings |
| Theme colours must be six hex digits (`src/theme.rs:46-50`) | `Rgb::parse` accepts named colours |

### Measured and justified — keep

- **The hand-tuned row reader** (`src/terminal.rs:696-815`, 120 lines): 103–109 µs per frame, against 307 µs for the naive reader and 174 µs for the library's batched getter. The naive one is about 60 lines shorter and would still hold 60 fps, so this is a choice, not a need.
- **`ApcParser`** (`src/terminal.rs:342-409`): the library route (`on_unknown_sequence`) drops payloads containing certain UTF-8 bytes and corrupts the stream when metadata arrives mid-sequence.
- **The git pin** (`Cargo.toml:16-18`): the latest release (0.2.2) has no snapshot or search module and lacks nine other items in use. The pinned rev is upstream's current head. A published fork exists but is not a drop-in.
- **One thread per pane** is forced: the terminal type is not `Send`, and the decoder wants a blocking reader.
- **Frame building** matches the library's dirty-row model; rows are shared across scrolls (7 allocations per frame).
- **Constants:** the 1 s render hold is the library's own suggestion; 32 MiB scrollback is a client cap on top of the server's history; 128 MiB of pictures per screen is below Ghostty's default.

Worst-case memory to keep in mind: eight warm tabs times their panes, each up to 32 MiB of history plus 2 × 128 MiB of pictures, with every shown picture also held as an RGBA copy and a GPU texture.

---

## 10. Hands-on UX

The release build was run on a private X display against disposable Rex servers and driven with synthetic input. Screenshots are in `work/review/ux/`. There was no compositor, so title-bar dragging, rounded corners, focus loss, Wayland, desktop notifications, Kitty pictures and input methods were not exercised. *observed* means the reviewer saw it; I looked at the screenshots for 10.4, 10.5, 10.7 and 10.22 and they show what is described.

### Problems, most damaging first

**10.1 New tabs and splits do not start in the focused pane's directory.** *observed* (`64-split-cwd.png`)
- `cd /usr/share`, then Ctrl Shift D and Ctrl Shift T: both new shells opened in `~`. Ghostty, GNOME Terminal and VS Code all inherit. Mechanism and fix in 5.4 and 5.5.

**10.2 Ctrl K is taken from the shell.** *observed*
- It opens the palette; the shell never sees it. Ctrl K is kill-to-end-of-line in readline, zsh and emacs, and cut-line in nano.
- Fix: drop the Ctrl K default. Ctrl Shift P and the Search field open the same list.

**10.3 Pane focus and resize keys collide with GNOME defaults.** *from reading; not observable without a desktop*
- Ctrl Alt Left/Right switch workspace on stock GNOME, and Ctrl Alt Shift Left/Right move the window to another workspace. On Wayland the shell takes them first.

**10.4 Tabs along the top lose their names to status words.** *observed* (`38-tabs-top.png`)
- A tab named `tests` reporting 60% reads `… 60%` with about 330 px of empty strip beside it. A tab reporting a question shows only its icon and `Question`.
- The README says the name stays and the status becomes a dot only when room is short.
- Fix: give the name priority and let tabs grow when the strip has room.

**10.5 The "what is at work" list names only the session.** *observed* (`48-agents.png`)
- Statuses from three tabs of `webapp` give three rows all titled `webapp`. The tooltip never gives the tab name.
- A Done row vanishes when clicked and the rows below slide under the pointer.
- Fix: title rows `session › tab`.

**10.6 The palette hides its commands and reopens where it was scrolled.** *observed* (`05-palette-open.png`, `76-palette-reopen.png`)
- About ten rows show, sessions and tabs first, so no command is visible until you type or scroll.
- After scrolling, Esc and reopening, it opens at the old offset with the picked row off-screen.
- The hovered row and the picked row are nearly the same grey.
- With several servers, `New Session "name"` does not say which server it will be created on.
- It opens with the current session picked, so Enter does nothing; picking the previous session would make it a toggle.

**10.7 Names mislead when a program sets no title.** *observed* (`04-server-split.png`)
- A pane running htop, `cat`, `sleep` or `nvim -u NONE` is called `shell`: the Neovim icon sits over the word "shell". A pane running `less` kept the stale `zsh /etc` (this is 7.4).
- Three fresh tabs are all `zsh …/home`, and three-level paths lose their `~`.
- Fix: fall back to the foreground program's name.

**10.8 Find puts the match under its own field and highlights only one.** *observed* (`17-find-next.png`)
- The match lands on the top row, where the find field covers the right of that line. Another match five rows down is not marked.
- No previous, next or close buttons, and reopening forgets the last query.
- The library can highlight every match in view (section 9).

**10.9 The rename field's selected text is nearly unreadable.** *observed, measured* (`89z.png`)
- The name is pre-selected as teal on teal, 1.49:1.
- An unnamed tab's field is pre-filled with its whole terminal title, scrolled to the end.

**10.10 The close confirmation does not explain itself.** *observed* (`32d-menu-close.png`)
- Choosing Close Tab on a tab running nvim turns the entry red with the same words, which reads as a pressed state.
- Nothing says what is running, so `×` sometimes closes and sometimes becomes `Close?` for no visible reason.
- One click on `×` closed an idle multi-tab session at once, with no undo, and that `×` sits beside `+`.
- Fix: "Close — nvim is running"; always confirm a session of more than one tab.

**10.11 The sidebar has three row kinds that are hard to tell apart.** *observed* (`03-scene-initial.png`)
- A one-tab session is an icon row, a multi-tab session is a small dim heading, and its tabs are icon rows indented by 10 px. A session and a tab look like the same thing.
- A one-tab session's menu mixes Rename (the session), Mute Tab and Close Session.

**10.12 Floating pills sit on terminal text.** *observed* (`43-mode-hint.png`, `45-toast.png`, `54-lost.png`)
- The key-hint row covers the prompt line for as long as a mode is on. "Scrolled back" covers the end of the last row. The error toast covers pane header buttons. The lost-connection banner covers the first two lines.
- Fix: give the hint row and the banner their own strip; put "Scrolled back" in the pane header.

**10.13 No scrollbar or position while scrolled back.** *observed*

**10.14 The lost-connection banner hides that it is retrying.** *observed* (`54-lost.png`)
- "· retrying" appears only in the tooltip. The sidebar keeps showing Working / 60% as live, and typed keys vanish silently. The automatic reconnect itself worked.

**10.15 The failed-action toast does not look like an error.** *observed* (`45-toast.png`)
- A neutral grey pill reading `internal: init.lua:15: …`, with no red, no icon and no action name.

**10.16 The top tab strip clips without saying so.** *observed* (`66-small-top.png`)
- At 640 px it shows about one and a half of four tabs, hard-cut, with no fade, arrow or count.

**10.17 Tabs-on-top mode has no button for the work list**, and the action has no default key. *observed*

**10.18 "Panes out of sight" lists identical names** and the button has no count. *observed*

**10.19 The UI's wording is the README's voice, not the usual terms.** *observed* (`19-pane-menu.png`)
- "What is at work, on every server", "Show What Is at Work", "Nothing is at work.", "Panes out of sight", "Find a session, a tab or something to do".
- The pane menu mixes "Copy selection" with Title Case entries. The Search, work-list and server `+` tooltips carry no key.
- Suggested: "Activity", "Hidden panes", "Search sessions, tabs and commands (Ctrl+Shift+P)".

**10.20 Some dim text is too dim.** *measured from pixels*
- Sidebar second lines are about 4.1:1 at roughly 12 px. Disabled toolbar buttons are 1.6:1 and look absent rather than disabled. Other secondary text is fine (5.7–6.8:1).

**10.21 A selection whose text is cleared leaves one highlighted cell**, and Ctrl Shift C then copies one character. *observed*

**10.22 Wrapped zsh prompts are left as garbage after resizes.** *observed; low confidence it is the app's fault* (`48-agents.png`, `42-seq-hint.png`)
- In panes narrow enough for the prompt to wrap, after font-size changes and a sidebar toggle the prompt line read `adadadaadaada ada /ada /home/…`. A one-line prompt came through clean. Worth one comparison against Ghostty with a real prompt.

**10.23 The Controls section reads as a list of rules to memorise.** *from reading*
- Close has three outcomes with no visible reason. A pane's size and colours belong to its "owner" and nothing on screen marks one. Eight tabs are "kept fed"; others "take a moment". Done/Error/Bell stay "until you have looked", with exceptions. A local server is retried for ever, a remote one twice. Enter in find goes backwards. Four pointer modifiers.
- Each rule is defensible; together they are a lot of invisible state for a lean client. Either surface each one (an owner badge, "loading" on a cold tab) or remove the rule, which several of the decisions in the recommended order do.

### What works well — keep

- **First run:** "No Terminals / New Session" is clear and one click gives a working shell.
- **The look:** quiet dark cards, Inter and JetBrains Mono, a thin outline on the focused pane. htop, a split nvim and `less` all render cleanly.
- **Status in the sidebar:** coloured dot on the icon, the words, the program's line, and age or percent.
- **Hover buttons** reserve their space so rows do not jump, and have tooltips with keys.
- **Palette basics:** key chips, fast narrowing, and an unmatched name offers a new session.
- **Inline `Close?`:** non-modal, at the place clicked, and it reverts by itself.
- **Pane handling:** drag-reorder shows an insertion line; border drag highlights; double-click equalises; zoom adds a button for hidden panes.
- **The key-hint row** for sequences and modes.
- **Selection:** word, line, block and wrapped-URL selection all work; multi-line paste is bracketed.
- **Reconnect:** the last screen stays readable and selectable while dimmed, and the window reconnected by itself.
- **Remembered state** came back after a restart.

---

## Things reviewers did that you should know

- **Modification times:** one reviewer ran `touch` on `src/main.rs` and `src/backend.rs` by mistake. Content is unchanged; their times are now 22:16:19 rather than the originals.
- **`target/`:** for about two minutes `target/debug/rex-lite` was built from a reviewer's scratch copy of the sources. Debug and release were rebuilt from the real repo afterwards.
- **Git history:** the UX reviewer typed `git log --oneline | head -40` into a disposable pane while building a scene and saw five commit subjects. It cleared the pane and used none of it.
- **Your real Rex server** was only ever asked read-only questions (`rex api list`, `rex api describe`, help, and one `rex keymap --request` query). Every probe that created sessions or sent input ran against a disposable server, all of which were stopped and removed.

---

## Naming and readability (from all reports)

The same complaint came back independently from four reviewers and from my own read: the plain-verb, past-participle naming is consistent, but it reuses a few stems for unrelated things and leans on doc comments to say what a name means. Roughly one comment in five in the core exists to decode a one-word name.

### Three vocabularies for one thing

- **`Window` means tab.** `struct Window` is documented as `/// A tab.` (`src/model.rs:150`). In `backend.rs` and `model.rs` the Window family appears 81 times and "tab" 86 times. Picking `Tab` deletes four comments.
- **block / pane / terminal** for one id: 84 / 187 / 117 occurrences in `backend.rs`.
- **join vs attach** (42 vs 20), **"on show" vs `selected_*`** (27 vs 15), **"fed" vs stream** (13 vs 22).

### One word, many things

- **`held` in `backend.rs`: 80 uses, at least 10 referents**, including `struct Held`, `Connection.held` (a session id), a `&mut Session`, buffered news, and a `(session, tab)` pair.
- **`asked` / `asking`: 68 and 26 uses.** `View.asked` is client-action requests; `Connection.asked` is panes whose ownership was requested; `Connection.asking` is per-block info queries; `Held.asking` is a bool.
- **`shown`:** `src/backend.rs:1236-1237` defines two closures in a row, both named `shown`.
- **Type and constant names repeated across files:** `Shared` (backend, terminal), `Request` (wire, terminal, notify), `Notify` (a local alias and tokio's, both used in `backend.rs`), `PATIENCE` (3 s in notify, 20 s in ssh), `REPEAT` (10 s in notify, 500 ms in select), `backend::Link` vs `link::Link`, `status` in seven roles.

### Names that force a comment — a clearer name deletes or halves it

| Now | Where | Suggested |
|---|---|---|
| `hosts`, `keys` (they are hashes) | `src/backend.rs:556,558` | `hosts_hash`, `keymap_hash` |
| `last`, `rest()` | `src/backend.rs:77,87` | `frozen`, `freeze()` |
| `picks`, `settled`, `unsettled` | `src/backend.rs:81,563-564` | `select_seq`, `answered_pick`, `picks_in_flight` |
| `making`, `made` | `src/backend.rs:567,569` | `created_session`, `reveal_pane` |
| `Held { revision, heard, asking }`, `want()`, `take()` | `src/backend.rs:502-534` | `ViewAge { have, latest, fetching }`, `should_fetch()`, `accept()` |
| `UNTAKEN`, `FULL` | `src/backend.rs:40-41` | `CLIENT_CALL_TIMEOUT`, `WHOLE_TAB` |
| `list`, `fill`, `state`, `divide`, `copy` (free functions) | `src/backend.rs:1681,1867,1835,1708,1551` | `apply_session_event`, `decorate_panes`, `activity_of`, `spawn_border_mover`, `fetch_copied` |
| `pictured`, `known`, `caught`, `held`, `owed` | `src/terminal.rs:490-492,823-838` | `placements_gen`, `rows_by_id`, `hold_frame_at`, `sync_hold_since`, `frame_due` |
| `status()` (it is the worker's error) | `src/terminal.rs:269` | `error()` |
| `parts()`, `sent()`, `unshown()`, `every()` | `src/model.rs:231,215,87,240` | `contains()`, `clamped_move()`, `first_session_to_select()`, `all_panes()` |

### In the UI

- **Ask and Tell are crossed.** `Deed::Ask(action)` ends in `App::tell`; `Deed::Tell(pane, request)` calls `App::ask` (`src/ui/commands.rs:204-233,451,471`, `src/ui.rs:183-191`, `src/ui/pane.rs:379`). The `ask` stem has 84 uses across 11 files in six senses.
- **`shown`: 96 uses in 14 files.** In `App::ui` it is rebound four times (`src/ui.rs:405-454`): terminal colours, the central panel's response, animated rectangles, a loop variable.
- **`held`: 67 uses** (keys down, last key, pointer down, hovered, modifiers). **`kept`: 48 uses** (settings, "pointer still near", a sentinel galley, a closure).
- **A field and a method share a name:** `if self.agents { return self.agents(ui, views); }` (`src/ui/sidebar.rs:59-60`); likewise `self.find`.
- **Positional tuples read as `.0/.1/.2`:** `Command = (&str, &str, bool)`, `Keymap.served`, `Keys.modes`, `CachedRow.decorations` (a 5-tuple), `lines()` returning `(Activity, String, Option<String>, String)`, `Deed::Close(Id, bool, Action)`.
- **Private words for standard notions:** `spread` (maximised), `ground` (background), `tile` (icon), `worn` (badge), `Standing` (command context), `Deed` (effect), `doomed` (close id), `Kept` (settings), `caps` (keycaps).
- **Comments:** about 40–50 of the 888 UI comment lines restate code or compensate for a name. The rest carry real reasons and should stay.
- **Helpers have drifted from their modules:** `App::switcher` (a toolbar button) is in `palette.rs`; `App::heading` is in `sidebar.rs` and used by the palette; `App::button` is in `commands.rs`; the paint helpers `dot` and `worn` are in `heard.rs` and imported by six modules.
