#!/usr/bin/env python3
"""Exercise the actual oconsole binary with a fake BNET Director and a real PTY.

Usage: python3 scripts/test_oconsole_rate_guard.py --binary target/debug/oconsole
No production Director, configuration or catalog is used.
"""
import argparse
import os
import pty
import select
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path

BINARY = Path('target/debug/oconsole').resolve()


class Director:
    def __init__(self):
        self.listener = socket.socket()
        self.listener.bind(('127.0.0.1', 0))
        self.listener.listen(1)
        self.port = self.listener.getsockname()[1]
        self.commands = []
        self.condition = threading.Condition()
        self.failure = None
        self.connection = None
        self.worker = threading.Thread(target=self.run, daemon=True)
        self.worker.start()

    def run(self):
        try:
            self.listener.settimeout(10)
            self.connection, _ = self.listener.accept()
            self.connection.settimeout(15)
            self.receive()  # Console hello; test config has no password/TLS.
            self.send('1000 OK\n')
            self.send('1002 fake Director for rate-guard tests\n')
            dialog = 0
            while True:
                command = self.receive()
                if command is None:
                    break
                if command.startswith('.api'):
                    self.send(-1)
                    continue
                if command == '.help all':
                    self.send('version\tVersion\tversion\t\t0\ndialog\tDialog\tdialog\t\t0\nquit\tQuit\tquit\t\t0\n')
                    self.send(-1)
                    continue
                with self.condition:
                    self.commands.append((command, time.monotonic()))
                    self.condition.notify_all()
                if command == 'dialog':
                    dialog = 1
                    self.send(-24)  # YesNo
                elif dialog:
                    self.send(f'ANSWER {command}\n')
                    dialog += 1
                    self.send({2: -27, 3: -28}.get(dialog, -1))
                    if dialog == 4:
                        dialog = 0
                else:
                    self.send(f'RESULT {command}\n')
                    self.send(-1)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception as error:
            self.failure = error
        finally:
            if self.connection:
                self.connection.close()

    def receive(self):
        def exact(size):
            data = b''
            while len(data) < size:
                chunk = self.connection.recv(size - len(data))
                if not chunk:
                    return None
                data += chunk
            return data
        header = exact(4)
        if header is None:
            return None
        size = struct.unpack('!i', header)[0]
        if size < 0:
            return None
        data = exact(size)
        return data.decode() if data is not None else None

    def send(self, data):
        if isinstance(data, int):
            self.connection.sendall(struct.pack('!i', data))
        else:
            data = data.encode()
            self.connection.sendall(struct.pack('!i', len(data)) + data)

    def wait_commands(self, count):
        with self.condition:
            if not self.condition.wait_for(lambda: len(self.commands) >= count, timeout=7):
                raise AssertionError(f'Expected {count} commands; got {self.commands}; error={self.failure}')
        return self.commands[count - 1][1]

    def close(self):
        if self.connection:
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.listener.close()
        self.worker.join(timeout=2)


class Console:
    def __init__(self, director, terminal=False):
        self.temporary = tempfile.TemporaryDirectory(prefix='oconsole-rate-guard-')
        config = Path(self.temporary.name) / 'oconsole.conf'
        config.write_text('')
        env = dict(os.environ, HOME=self.temporary.name, TERM='xterm', RUST_LOG='off')
        if terminal:
            env.pop('NO_COLOR', None)
            self.master, slave = pty.openpty()
            stdin = stdout = slave
        else:
            env['NO_COLOR'] = '1'
            stdin = stdout = subprocess.PIPE
            self.master = None
        self.process = subprocess.Popen(
            [str(BINARY), '-c', str(config), '-a', '127.0.0.1', '-p', str(director.port), '-w', ''],
            stdin=stdin, stdout=stdout, stderr=subprocess.PIPE, env=env,
        )
        if terminal:
            os.close(slave)
        self.buffers = {}

    def write(self, data):
        if self.master is not None:
            os.write(self.master, data)
        else:
            self.process.stdin.write(data)
            self.process.stdin.flush()

    def until(self, descriptor, marker, timeout=6):
        data = self.buffers.get(descriptor, b'')
        end = time.monotonic() + timeout
        while marker not in data:
            remaining = end - time.monotonic()
            if remaining <= 0 or not select.select([descriptor], [], [], max(0, remaining))[0]:
                raise AssertionError(f'Missing {marker!r} in {data!r}')
            chunk = os.read(descriptor, 65536)
            if not chunk:
                raise AssertionError(f'EOF before {marker!r}: {data!r}')
            data += chunk
        boundary = data.index(marker) + len(marker)
        self.buffers[descriptor] = data[boundary:]
        return data[:boundary]

    def close(self):
        if self.process.poll() is None:
            self.process.kill()
        self.process.wait(timeout=3)
        for stream in [self.process.stdin, self.process.stdout, self.process.stderr]:
            if stream:
                stream.close()
        if self.master is not None:
            os.close(self.master)
        self.temporary.cleanup()


class RateGuardTests(unittest.TestCase):
    def setUp(self):
        self.director = Director()
        self.addCleanup(self.director.close)

    def console(self, terminal=False):
        console = Console(self.director, terminal)
        self.addCleanup(console.close)
        return console

    def test_stdin_burst_is_dropped_and_recovery_does_not_queue_commands(self):
        console = self.console()
        console.write(b'version\nspam-intruder\nversion\n')
        first = self.director.wait_commands(1)
        console.until(console.process.stderr.fileno(), b'Command Rate Guard violation')
        time.sleep(max(0, first + 3.05 - time.monotonic()))
        console.write(b'version\n')
        second = self.director.wait_commands(2)
        console.process.stdin.close()
        console.process.wait(timeout=3)
        stdout = console.process.stdout.read()
        self.assertNotIn(b'Command Rate Guard', stdout)
        self.assertNotIn(b'\x1b', stdout)  # NO_COLOR
        self.assertEqual([c for c, _ in self.director.commands], ['version', 'version'])
        self.assertGreaterEqual(second - first, 3)
        history = (Path(console.temporary.name) / '.oconsole_history').read_text()
        self.assertIn('version', history)
        self.assertNotIn('spam-intruder', history)
        self.assertEqual(console.process.returncode, 0)

    def test_all_dialog_types_reprompt_after_fast_replies(self):
        console = self.console()
        console.write(b'dialog\nyes\n')
        last = self.director.wait_commands(1)
        console.until(console.process.stderr.fileno(), b'Command Rate Guard violation')
        for count, answer in [(2, b'yes'), (3, b'choice'), (4, b'text')]:
            time.sleep(max(0, last + 3.05 - time.monotonic()))
            console.write(answer + b'\n' + (b'intruder\n' if count < 4 else b''))
            last = self.director.wait_commands(count)
            if count < 4:
                console.until(console.process.stderr.fileno(), b'Command Rate Guard violation')
        console.process.stdin.close()
        console.process.wait(timeout=3)
        self.assertEqual([c for c, _ in self.director.commands], ['dialog', 'yes', 'choice', 'text'])
        times = [t for _, t in self.director.commands]
        self.assertTrue(all(b - a >= 3 for a, b in zip(times, times[1:])))
        self.assertNotIn(b'Command Rate Guard', console.process.stdout.read())
        self.assertEqual(console.process.returncode, 0)

    def test_real_terminal_prompt_changes_red_to_green_without_input_loss(self):
        console = self.console(terminal=True)
        console.until(console.master, b'\x1b[32m*UserAgent*@bareos-dir[')
        console.write(b'version\n')
        first = self.director.wait_commands(1)
        console.until(console.master, b'\x1b[31m*UserAgent*@bareos-dir[')
        console.write(b'ver')
        # No Enter or refresh key: timer must redraw the prompt in green itself.
        console.until(console.master, b'\x1b[32m*UserAgent*@bareos-dir[', timeout=5)
        console.write(b'sion\n')
        second = self.director.wait_commands(2)
        console.until(console.master, b'\x1b[31m*UserAgent*@bareos-dir[')
        console.write(b'\x04')  # EOF closes without an extra unguarded quit command.
        console.process.wait(timeout=3)
        self.assertEqual([c for c, _ in self.director.commands], ['version', 'version'])
        self.assertGreaterEqual(second - first, 3)
        self.assertEqual(console.process.returncode, 0)

    def test_multiline_paste_cannot_bypass_guard_in_one_packet(self):
        console = self.console(terminal=True)
        console.until(console.master, b'\x1b[32m*UserAgent*@bareos-dir[')
        console.write(b'\x1b[200~version\nversion\x1b[201~\n')
        console.until(console.process.stderr.fileno(), b'multiline input discarded')
        console.write(b'version\n')
        self.director.wait_commands(1)
        console.until(console.master, b'\x1b[31m*UserAgent*@bareos-dir[')
        console.write(b'\x04')
        console.process.wait(timeout=3)
        self.assertEqual([c for c, _ in self.director.commands], ['version'])
        self.assertEqual(console.process.returncode, 0)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument('--binary', type=Path, default=BINARY)
    args, remaining = parser.parse_known_args()
    BINARY = args.binary.resolve()
    unittest.main(argv=[__file__, *remaining])
