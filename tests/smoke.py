#!/usr/bin/env python3
"""Run live and native tests with a private Rex server, Xvfb, and D-Bus session.

Build first: ./scripts/dev build --locked
Run: python3 tests/smoke.py [--binary target/release/rex-lite] [--screenshot /absolute/path.png]
Requires rex, Xvfb, dbus-run-session, import, xclip, libX11, and libXtst.
"""
import argparse
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default='target/debug/rex-lite')
parser.add_argument('--screenshot')
args = parser.parse_args()
repo = Path(__file__).resolve().parents[1]
binary = (repo / args.binary).resolve()
assert binary.is_file(), 'build the client before running this test'
(repo / 'work').mkdir(exist_ok=True)
processes = []

def start(argv, **kwargs):
    process = subprocess.Popen(argv, start_new_session=True, **kwargs)
    processes.append(process)
    return process

def stop(process):
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=3)

with tempfile.TemporaryDirectory(prefix='rex-lite-smoke-', dir=repo / 'work') as directory:
    work = Path(directory)
    config = work / 'init.lua'
    config.write_text('')
    endpoint = 'unix://' + str(work / 'server.sock')
    log_path = repo / 'work' / 'last-smoke.log'
    with log_path.open('w') as log:
        try:
            server = start(['rex', 'server', 'run', '--listen', endpoint,
                '--state-dir', str(work / 'state'), '--config', str(config), '--restore=false'],
                stdout=log, stderr=log)
            deadline = time.monotonic() + 10
            while not (work / 'server.sock').exists():
                if server.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError('private Rex server did not start')
                time.sleep(.05)
            cli = ['rex', '-S', endpoint, '--autostart=false']
            fixture = json.loads(subprocess.check_output(cli + ['new', 'rex-lite smoke',
                '--window', 'Terminal', '--shell=none', '--keep-open', '-c', str(repo),
                '--json', '--', '/bin/sh'], text=True))
            # The window keeps what it remembers here, away from the user's own.
            env = dict(os.environ, REX_LITE_TEST_SERVER=endpoint, XDG_STATE_HOME=str(work / 'local'))
            subprocess.run(['./scripts/dev', 'test', '--locked', '--test', 'live', '--',
                '--ignored', '--nocapture'], cwd=repo, env=env, check=True)
            display_read, display_write = os.pipe()
            try:
                start(['Xvfb', '-displayfd', str(display_write), '-screen', '0',
                    '1280x900x24', '-nolisten', 'tcp', '-noreset'], pass_fds=(display_write,),
                    stdout=log, stderr=log)
                os.close(display_write)
                display_write = None
                if not select.select([display_read], [], [], 10)[0]:
                    raise RuntimeError('private Xvfb did not start')
                number = os.read(display_read, 64).decode().strip()
                assert number.isdigit(), 'invalid Xvfb display number'
                display = ':' + number
            finally:
                os.close(display_read)
                if display_write is not None:
                    os.close(display_write)
            # Without these a Wayland desktop would get the window instead of Xvfb.
            app_env = {key: value for key, value in env.items()
                if key not in ('WAYLAND_DISPLAY', 'WAYLAND_SOCKET')}
            app = start(['dbus-run-session', '--', str(binary), '--server', endpoint],
                cwd=repo, env=dict(app_env, DISPLAY=display, LIBGL_ALWAYS_SOFTWARE='1'),
                stdout=log, stderr=log)
            time.sleep(1)
            command = ['python3', 'tests/native_input.py', '--display', display,
                '--server', endpoint, '--session', fixture['session_id'], '--pane',
                fixture['initial_windows'][0]['block_ids'][0]]
            if args.screenshot:
                command += ['--screenshot', str(Path(args.screenshot).resolve())]
            subprocess.run(command, cwd=repo, check=True, timeout=60)
            # This measures an isolated software-rendered window, not the real desktop GPU.
            children = Path(f'/proc/{app.pid}/task/{app.pid}/children').read_text().split()
            for pid in children:
                if Path(f'/proc/{pid}/comm').read_text().strip() != 'rex-lite':
                    continue
                def ticks():
                    fields = Path(f'/proc/{pid}/stat').read_text().split(') ', 1)[1].split()
                    return int(fields[11]) + int(fields[12])
                before = ticks()
                time.sleep(2)
                cpu = (ticks() - before) / os.sysconf('SC_CLK_TCK') / 2 * 100
                rss = next(line for line in Path(f'/proc/{pid}/status').read_text().splitlines()
                    if line.startswith('VmRSS:'))
                print(f'Isolated software-rendered idle sample: {cpu:.1f}% CPU; {rss}')
            print('Live and native smoke tests passed; disposable processes cleaned up.')
        except Exception:
            print(log_path.read_text())
            raise
        finally:
            for process in reversed(processes):
                stop(process)
