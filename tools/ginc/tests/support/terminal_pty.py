import os
import pty
import select
import subprocess
import sys
import termios
import time


def read_until(master, process, data, expected):
    deadline = time.monotonic() + 5
    while expected not in data and time.monotonic() < deadline:
        ready, _, _ = select.select([master], [], [], 0.05)
        if ready:
            data += os.read(master, 65536)
        if process.poll() is not None:
            break
    assert expected in data, (expected, data, process.poll())
    return data


def assert_restored(slave, original):
    restored = termios.tcgetattr(slave)
    if sys.platform == "darwin":
        # XNU sets PENDIN when switching back to canonical input.
        restored[3] &= ~termios.PENDIN
        original = original.copy()
        original[3] &= ~termios.PENDIN
    assert restored == original, (restored, original)


def exercise(executable, exit_key):
    master, slave = pty.openpty()
    original = termios.tcgetattr(slave)
    process = subprocess.Popen(
        [executable], stdin=slave, stdout=slave, stderr=subprocess.PIPE
    )
    try:
        data = read_until(master, process, b"", b"\x1b[6;11H\x1b[33m@")
        assert b"\x1b[?1049h\x1b[?25l" in data
        assert b"\x1b[36mGin terminal" in data
        raw = termios.tcgetattr(slave)
        assert not raw[3] & (termios.ICANON | termios.ECHO | termios.ISIG)
        assert not raw[1] & termios.OPOST
        assert raw[6][termios.VMIN] == 0
        assert raw[6][termios.VTIME] == 1
        for sequence, position in [
            (b"\x1b[A", b"\x1b[5;11H\x1b[33m@"),
            (b"\x1b[C", b"\x1b[5;12H\x1b[33m@"),
            (b"\x1bOB", b"\x1b[6;12H\x1b[33m@"),
            (b"\x1b[D", b"\x1b[6;11H\x1b[33m@"),
        ]:
            for byte in sequence:
                os.write(master, bytes([byte]))
                time.sleep(0.01)
            data = read_until(master, process, b"", position)
        os.write(master, exit_key)
        data = read_until(master, process, b"", b"\x1b[0m\x1b[?25h\x1b[?1049l")
        assert process.wait(timeout=5) == 0
        assert process.stderr.read() == b""
        assert_restored(slave, original)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        termios.tcsetattr(slave, termios.TCSANOW, original)
        os.close(master)
        os.close(slave)


def failed_enter(executable):
    master, slave = pty.openpty()
    original = termios.tcgetattr(slave)
    process = subprocess.Popen(
        [executable], stdin=slave, stdout=slave, stderr=subprocess.PIPE
    )
    try:
        data = read_until(master, process, b"", b"\x1b[0m\x1b[?25h\x1b[?1049l")
        assert b"\x1b[?1049h" not in data
        assert process.wait(timeout=5) == 0
        assert process.stderr.read() == b""
        assert_restored(slave, original)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        termios.tcsetattr(slave, termios.TCSANOW, original)
        os.close(master)
        os.close(slave)


def settings_copy(executable):
    master, slave = pty.openpty()
    original = termios.tcgetattr(slave)
    process = subprocess.Popen(
        [executable], stdin=slave, stdout=slave, stderr=subprocess.PIPE
    )
    try:
        assert process.wait(timeout=5) == 0
        assert process.stderr.read() == b""
        assert_restored(slave, original)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)
        termios.tcsetattr(slave, termios.TCSANOW, original)
        os.close(master)
        os.close(slave)


if sys.argv[2] == "demo":
    exercise(sys.argv[1], b"q")
    exercise(sys.argv[1], b"\x03")
    exercise(sys.argv[1], b"\x1b[\x03")
elif sys.argv[2] == "failed_enter":
    failed_enter(sys.argv[1])
else:
    assert sys.argv[2] == "settings_copy"
    settings_copy(sys.argv[1])
