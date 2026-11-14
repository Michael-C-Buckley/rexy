#!/usr/bin/env python3
"""Opt-in X11 smoke check on an existing isolated display and disposable Rex pane.

Requires libX11, libXtst, and the Rex CLI. Never uses the default display/server.
"""
import argparse
import ctypes as C
import ctypes.util
import subprocess
import time

CONTROL, SHIFT, RETURN, ESCAPE = 0xffe3, 0xffe1, 0xff0d, 0xff1b  # X keysyms

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--display', required=True)
parser.add_argument('--server', required=True)
parser.add_argument('--session', required=True)
parser.add_argument('--pane', required=True)
parser.add_argument('--screenshot')
args = parser.parse_args()
if not (args.server.startswith('unix://') and 'rex-lite' in args.server and '/work/' in args.server):
    parser.error('requires an explicit disposable rex-lite socket under work/')

x11 = C.CDLL(ctypes.util.find_library('X11'))
xtest = C.CDLL(ctypes.util.find_library('Xtst'))
x11.XOpenDisplay.argtypes = [C.c_char_p]
x11.XOpenDisplay.restype = C.c_void_p
x11.XDefaultRootWindow.argtypes = [C.c_void_p]
x11.XDefaultRootWindow.restype = C.c_ulong
x11.XQueryTree.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.POINTER(C.c_ulong)), C.POINTER(C.c_uint)]
x11.XFetchName.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_char_p)]
x11.XFree.argtypes = [C.c_void_p]
x11.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
x11.XFlush.argtypes = [C.c_void_p]
x11.XKeysymToKeycode.argtypes = [C.c_void_p, C.c_ulong]
x11.XKeysymToKeycode.restype = C.c_uint
x11.XKeycodeToKeysym.argtypes = [C.c_void_p, C.c_ubyte, C.c_int]
x11.XKeycodeToKeysym.restype = C.c_ulong
xtest.XTestFakeKeyEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
xtest.XTestFakeButtonEvent.argtypes = [C.c_void_p, C.c_uint, C.c_int, C.c_ulong]
xtest.XTestFakeMotionEvent.argtypes = [C.c_void_p, C.c_int, C.c_int, C.c_int, C.c_ulong]

display = x11.XOpenDisplay(args.display.encode())
assert display, 'cannot open explicit test display'
root, parent, children, count = C.c_ulong(), C.c_ulong(), C.POINTER(C.c_ulong)(), C.c_uint()
assert x11.XQueryTree(display, x11.XDefaultRootWindow(display), C.byref(root), C.byref(parent), C.byref(children), C.byref(count))
window = None
for i in range(count.value):
    name = C.c_char_p()
    if x11.XFetchName(display, children[i], C.byref(name)) and name.value:
        title = name.value.decode(errors='replace')
        x11.XFree(name)
        if title == 'Rex Lite':
            window = children[i]
            break
x11.XFree(children)
assert window, 'Rex Lite test window not found'
x11.XSetInputFocus(display, window, 2, 0)

def key(sym, down):
    code = x11.XKeysymToKeycode(display, sym)
    assert code, f'no X keycode for {sym}'
    xtest.XTestFakeKeyEvent(display, code, int(down), 0)
    x11.XFlush(display)

def chord(sym, ctrl=False, shift=False):
    if ctrl: key(CONTROL, True)
    if shift: key(SHIFT, True)
    key(sym, True)
    key(sym, False)
    if shift: key(SHIFT, False)
    if ctrl: key(CONTROL, False)
    time.sleep(.02)

def type_text(text):
    for char in text:
        sym = ord(char)
        code = x11.XKeysymToKeycode(display, sym)
        chord(sym, shift=x11.XKeycodeToKeysym(display, code, 0) != sym)

def click(px, py, twice=False):
    xtest.XTestFakeMotionEvent(display, -1, px, py, 0)
    for _ in range(2 if twice else 1):
        xtest.XTestFakeButtonEvent(display, 1, 1, 0)
        xtest.XTestFakeButtonEvent(display, 1, 0, 0)
        x11.XFlush(display)
        time.sleep(.07)
    time.sleep(.15)

def capture():
    return subprocess.check_output(['rex', '-S', args.server, '--autostart=false', 'capture', '-s', args.session, '-b', args.pane, '--trim'], text=True)

def expect(label, predicate):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        if predicate(capture()):
            print(label + ': passed', flush=True)
            return
        time.sleep(.1)
    raise AssertionError(label + ': ' + capture())

# The terminal fills the window to the right of the 240 px sidebar.
click(400, 140)
chord(ord('c'), ctrl=True)
nonce = str(time.monotonic_ns())[-8:]
type_text(f"printf 'native-%s\\n' {nonce}")
chord(RETURN)
expect('native printable input', lambda text: f'native-{nonce}' in text)

type_text('sleep 20')
chord(RETURN)
time.sleep(.25)
chord(ord('c'), ctrl=True)
type_text(f"printf 'interrupt-%s\\n' {nonce}")
chord(RETURN)
expect('native Ctrl+C interrupts', lambda text: f'interrupt-{nonce}' in text)
# Clear scrollback/screen and print a stable word at the first terminal row.
type_text("printf '\\033[3J\\033[2J\\033[HCLIP%s\\n' BOARD")
chord(RETURN)
expect('clipboard fixture', lambda text: text.strip().startswith('CLIPBOARD'))
click(290, 18, twice=True)
chord(ord('c'), ctrl=True, shift=True)
chord(ESCAPE)
type_text('cat')
chord(RETURN)
expect('first character after leaving selection', lambda text: text.rstrip().endswith('$ cat'))
chord(ord('v'), ctrl=True, shift=True)
chord(RETURN)
expect('native selection copy and bracketed paste', lambda text: text.count('CLIPBOARD') >= 3)
chord(ord('c'), ctrl=True)

# A program tracking the mouse gets the clicks on its pane.
type_text("printf '\\033[?1000h\\033[?1006h'; cat -v")
chord(RETURN)
click(400, 300)
expect('mouse click reaches a tracking program', lambda text: text.count('[<0;') == 2)
chord(ord('c'), ctrl=True)
print('Native X11 keyboard, mouse and clipboard smoke passed.', flush=True)

if args.screenshot:
    subprocess.run(['import', '-display', args.display, '-window', str(window), args.screenshot], check=True)
