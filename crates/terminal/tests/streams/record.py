#!/usr/bin/env python3
"""Record what a program writes to its terminal, for the replay tests in ../replay.rs.

    python3 -I record.py OUT.bin COLS ROWS -- PROGRAM [ARGS...] < SCRIPT

The program runs on a fresh pty of COLS x ROWS with TERM=xterm-256color, in a scratch HOME and
working directory, so no user config or path ends up in the recording. SCRIPT is one step per
line: `sleep SECONDS`, or `send TEXT` where TEXT is a Python string literal (escapes allowed).
Every byte the program writes is saved to OUT.bin. The few startup queries in ANSWERS get a
canned answer, as a dark terminal would give, so a program doesn't stall or warn waiting for
one; the replay tests check what our own terminal answers to the same queries.

Each recording's command line is in ../replay.rs next to the test that replays it.
"""

import ast
import fcntl
import os
import pty
import select
import shutil
import struct
import sys
import tempfile
import termios
import time


# Query -> canned answer. Only the ones programs wait on at startup.
ANSWERS = {
    b"\x1b[c": b"\x1b[?62;22c",
    b"\x1b[5n": b"\x1b[0n",
    b"\x1b[?u": b"\x1b[?0u",
    b"\x1b]11;?\x07": b"\x1b]11;rgb:1111/1111/1111\x07",
}


def main() -> None:
    out, cols, rows = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    if sys.argv[4] != "--":
        sys.exit("usage: record.py OUT.bin COLS ROWS -- PROGRAM [ARGS...] < SCRIPT")
    argv = sys.argv[5:]
    steps = [line.split(" ", 1) for line in sys.stdin.read().splitlines() if line.strip()]

    fixtures = os.path.dirname(os.path.abspath(__file__))
    scratch = tempfile.mkdtemp(prefix="record-")
    for name in os.listdir(fixtures):
        if name.endswith((".txt", ".py")) and name != "record.py":
            shutil.copy(os.path.join(fixtures, name), scratch)
    env = {
        "TERM": "xterm-256color",
        "HOME": scratch,
        "XDG_CONFIG_HOME": os.path.join(scratch, "config"),
        "XDG_DATA_HOME": os.path.join(scratch, "data"),
        "XDG_STATE_HOME": os.path.join(scratch, "state"),
        "XDG_CACHE_HOME": os.path.join(scratch, "cache"),
        "PATH": "/opt/homebrew/bin:/usr/bin:/bin",
        "LANG": "en_US.UTF-8",
        "LESSHISTFILE": "-",
    }

    pid, fd = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        os.chdir(scratch)
        os.execvpe(argv[0], argv, env)

    recorded = bytearray()

    def drain(seconds: float) -> None:
        end = time.monotonic() + seconds
        while (left := end - time.monotonic()) > 0:
            ready, _, _ = select.select([fd], [], [], left)
            if not ready:
                return
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            recorded.extend(chunk)
            for query, answer in ANSWERS.items():
                for _ in range(chunk.count(query)):
                    os.write(fd, answer)

    for verb, arg in steps:
        if verb == "sleep":
            drain(float(arg))
        elif verb == "send":
            try:
                os.write(fd, ast.literal_eval(arg).encode())
            except OSError:
                sys.exit(f"the program exited early; it wrote: {bytes(recorded)!r}")
        else:
            sys.exit(f"unknown step {verb!r}")
    drain(1.0)
    try:
        os.kill(pid, 9)
    except ProcessLookupError:
        pass
    os.waitpid(pid, 0)
    shutil.rmtree(scratch)
    with open(out, "wb") as f:
        f.write(recorded)
    print(f"{out}: {len(recorded)} bytes")


if __name__ == "__main__":
    main()
