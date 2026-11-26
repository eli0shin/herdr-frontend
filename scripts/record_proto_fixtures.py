#!/usr/bin/env python3
"""Record real herdr-core messages for herdr-proto's tests (crates/herdr-proto/tests/fixtures).

It starts its own isolated core (HC_SESSION=test-*, scratch XDG dirs, every inherited HC_* variable
removed), drives one scripted session against it, and stops that core with `hc server stop` under
the same environment. It never talks to the user's core. Run it as `python3 -I
scripts/record_proto_fixtures.py`, then review the diff: the fixtures must not hold personal data.
The core's name is this machine's host name, so it is written as `fixture-host`.

Fixtures written:
- calls.jsonl: one object per call, `{"request": ..., "reply": ...}`, exactly as sent and received.
- events.jsonl: every line of the events stream, in order, after the subscribe reply (first line).
- missed.jsonl: a second subscriber's stream that fell behind and got `missed`.
- frames.bin: the raw framed bytes the core sent on the first connection, after the hello line,
  until the final `terminal.list`.
- frames.jsonl: the same frames, one `{channel, kind, len}` per frame, to check the decoder against.
- close.jsonl: the JSON reasons on the CLOSE frames that ended attach streams.
- schema.json: `rpc.discover`, the core's OpenRPC schema.
"""

import base64
import json
import os
import queue
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time

DATA, CREDIT, CLOSE, RESIZE, FOCUS, FACTS, AUTO = range(7)
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "herdr-proto", "tests", "fixtures")


def isolated_env(scratch):
    env = {k: v for k, v in os.environ.items() if not k.startswith("HC_")}
    env["HC_SESSION"] = "test-hfr6"
    # A kept terminal's replacement shell is the core's $SHELL: keep it plain.
    env["SHELL"] = "/bin/sh"
    for var in ("XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
        path = os.path.join(scratch, var.split("_")[1].lower())
        os.makedirs(path, mode=0o700)
        env[var] = path
    assert env["HC_SESSION"].startswith("test-")
    return env


class Conn:
    def __init__(self, env, record=False):
        self.p = subprocess.Popen(["hc", "connect"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env)
        self.hello_request = {"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {"framed": True, "generation": 1}}
        self.p.stdin.write(json.dumps(self.hello_request).encode() + b"\n")
        self.p.stdin.flush()
        self.hello_line = self.p.stdout.readline()
        self.hello = json.loads(self.hello_line)["result"]
        self.next = 1
        self.replies = {}
        self.chan = {}
        self.credit_channels = set()
        self.lock = threading.Lock()
        self.raw = bytearray() if record else None
        self.frames = [] if record else None
        threading.Thread(target=self.reader, daemon=True).start()

    def send(self, ch, kind, payload=b""):
        with self.lock:
            self.p.stdin.write(struct.pack("<IBI", ch, kind, len(payload)) + payload)
            self.p.stdin.flush()

    def reader(self):
        out = self.p.stdout
        while True:
            h = out.read(9)
            if len(h) < 9:
                return
            ch, kind, n = struct.unpack("<IBI", h)
            payload = out.read(n)
            if self.raw is not None:
                self.raw += h + payload
                self.frames.append({"channel": ch, "kind": kind, "len": n})
            if ch == 0:
                msg = json.loads(payload)
                q = self.replies.pop(msg.get("id"), None)
                if q:
                    q.put(payload)
            else:
                if kind == DATA and ch in self.credit_channels:
                    self.send(ch, CREDIT, struct.pack("<I", n))
                self.chan.setdefault(ch, queue.Queue()).put((kind, payload))

    def stop_recording(self):
        raw, frames = bytes(self.raw), list(self.frames)
        self.raw = self.frames = None
        return raw, frames

    def call(self, method, params=None, log=None):
        i = self.next
        self.next += 1
        q = queue.Queue()
        self.replies[i] = q
        request = {"jsonrpc": "2.0", "id": i, "method": method, "params": params if params is not None else {}}
        self.send(0, DATA, json.dumps(request).encode())
        raw = q.get(timeout=30)
        reply = json.loads(raw)
        if log is not None:
            log.append({"request": request, "reply": reply})
        return reply.get("result", reply.get("error"))

    def drain(self, ch, secs):
        q = self.chan.setdefault(ch, queue.Queue())
        end, got = time.time() + secs, []
        while time.time() < end:
            try:
                got.append(q.get(timeout=max(0.01, end - time.time())))
            except queue.Empty:
                pass
        return got


class Lines:
    """Event lines from one subscription, split across DATA frames."""

    def __init__(self, conn, channel):
        self.conn, self.channel, self.pending, self.lines = conn, channel, b"", []

    def pump(self, secs):
        for kind, payload in self.conn.drain(self.channel, secs):
            if kind != DATA:
                continue
            self.pending += payload
            while b"\n" in self.pending:
                line, self.pending = self.pending.split(b"\n", 1)
                if line:
                    self.lines.append(json.loads(line))

    def wait_for(self, pred, secs, what):
        end = time.time() + secs
        while time.time() < end:
            if any(pred(e) for e in self.lines):
                return
            self.pump(0.2)
        print([(e.get("method"), e["params"].get("type"), e["params"].get("terminal")) for e in self.lines], file=sys.stderr)
        sys.exit(f"timed out waiting for {what}")


def event(kind, terminal=None):
    return lambda e: e.get("method") == "event" and e["params"]["type"] == kind and (
        terminal is None or e["params"].get("terminal") == terminal)


def typed(conn, channel, text):
    conn.send(channel, DATA, text.encode())


def main():
    # Short: the core's socket lives under it, and macOS caps socket paths at 103 bytes.
    scratch = tempfile.mkdtemp(prefix="hfr6-", dir="/tmp")
    env = isolated_env(scratch)
    work = os.path.join(scratch, "work")
    os.makedirs(work)
    calls = []
    try:
        record(env, work, calls)
    finally:
        subprocess.run(["hc", "server", "stop"], env=env, check=False, capture_output=True)
        shutil.rmtree(scratch, ignore_errors=True)


def record(env, work, calls):
    a = Conn(env, record=True)
    calls.append({"request": a.hello_request, "reply": json.loads(a.hello_line)})

    sub = a.call("events.subscribe", {}, calls)
    events = Lines(a, sub["channel"])
    a.credit_channels.add(sub["channel"])

    a.call("server.status", {}, calls)
    a.call("templates.list", {}, calls)
    a.call("machine.list", {}, calls)
    a.call("terminal.list", {}, calls)
    schema = a.call("rpc.discover", {})

    # t1: a kept shell, attached, driven through status, title, size, foreground, labels and keep.
    t1 = a.call("terminal.run", {
        "argv": ["sh"], "cwd": work, "keep": True, "size": {"cols": 80, "rows": 24},
        "env": {"TERM": "xterm-256color", "COLORTERM": "truecolor", "TERM_PROGRAM": "herdr", "PS1": "$ "},
        "labels": {"herdr.space": "sp0123456789abcdef0123456789abcdef", "herdr.tab": "tb0123456789abcdef0123456789abcdef",
                   "herdr.seen": "0"},
        "display": {"fg": "#c0c5ce", "bg": "#1b2b34", "cursor": "#c0c5ce", "cell": [8, 17]},
    }, calls)["address"]
    events.wait_for(event("created", t1), 5, "created")
    v1 = a.call("terminal.attach", {"terminal": t1, "cols": 100, "rows": 30, "history": 2000}, calls)
    ch1 = v1["channel"]
    a.credit_channels.add(ch1)
    events.wait_for(event("size", t1), 5, "size after attach")

    b = Conn(env)
    calls.append({"request": b.hello_request, "reply": json.loads(b.hello_line)})
    ob = b.call("terminal.attach", {"terminal": t1, "cols": 40, "rows": 10, "observe": True,
                                    "resize_notifications": True}, calls)
    b.credit_channels.add(ob["channel"])

    a.send(ch1, RESIZE, struct.pack("<HH", 90, 25))
    events.wait_for(lambda e: event("size", t1)(e) and e["params"]["data"]["cols"] == 90, 5, "size after RESIZE")
    a.send(ch1, FOCUS, b"\x01")
    a.send(ch1, FACTS, json.dumps({"fg": "#c0c5ce", "bg": "#1b2b34", "cursor": "#c0c5ce", "cell": [8, 17]}).encode())

    typed(a, ch1, "printf '\\033]2;fixture title\\007'\n")
    events.wait_for(event("title", t1), 5, "title")
    typed(a, ch1, "printf '\\033]7501;state=working:app=fixture:progress=40\\033\\\\'\n")
    events.wait_for(event("status", t1), 5, "status working")
    typed(a, ch1, "printf '\\033]7501;state=working:id=sub:app=fixture\\033\\\\'\n")
    typed(a, ch1, "printf '\\033]7501;state=blocked:kind=question:app=fixture\\033\\\\'\n")
    events.wait_for(event("turn", t1), 5, "turn")
    typed(a, ch1, "printf '\\033]7501;state=done:app=fixture\\033\\\\'\n")
    typed(a, ch1, "sleep 2\n")
    events.wait_for(event("foreground", t1), 5, "foreground")
    events.pump(2.5)

    a.call("terminal.label", {"terminal": t1, "labels": {"fixture": "1", "herdr.seen": "1"}}, calls)
    a.call("terminal.label", {"terminal": t1, "labels": {"fixture": ""}}, calls)
    a.call("terminal.keep", {"terminal": t1, "keep": False}, calls)
    a.call("terminal.keep", {"terminal": t1, "keep": True}, calls)
    a.call("terminal.get", {"terminal": t1}, calls)
    a.call("terminal.read", {"terminal": t1, "lines": 5}, calls)
    a.call("terminal.keys", {"terminal": t1, "keys": ["e", "c", "h", "o", "space", "k", "enter"]}, calls)
    a.call("terminal.send", {"terminal": t1, "text": "echo sent"}, calls)
    a.call("terminal.interrupt", {"terminal": t1, "timeout": 1.0}, calls)
    a.call("terminal.put", {"terminal": t1, "name": "note.txt", "data": base64.b64encode(b"fixture\n").decode()}, calls)
    events.pump(1)

    # Loss: a kept terminal whose program dies by a signal stays live, then gets a new shell.
    pid = a.call("terminal.get", {"terminal": t1})["pid"]
    os.kill(pid, signal.SIGKILL)
    events.wait_for(event("lost", t1), 10, "lost")
    events.wait_for(event("restarted", t1), 30, "restarted")
    lost_close = [p for k, p in a.drain(ch1, 0.5) if k == CLOSE]

    # t2: exits with a code, then its record is removed.
    t2 = a.call("terminal.run", {"argv": ["sh", "-c", "exit 3"], "cwd": work}, calls)["address"]
    events.wait_for(event("finished", t2), 10, "finished t2")
    a.call("terminal.wait", {"terminal": t2, "timeout": 2.0}, calls)
    a.call("terminal.rm", {"terminal": t2}, calls)
    events.wait_for(event("removed", t2), 5, "removed t2")

    # t3: killed while attached.
    t3 = a.call("terminal.run", {"argv": ["sh", "-c", "sleep 600"], "cwd": work, "name": "fixture-sleeper"}, calls)["address"]
    v3 = a.call("terminal.attach", {"terminal": t3, "cols": 80, "rows": 24, "resumed": True}, calls)
    a.credit_channels.add(v3["channel"])
    a.call("terminal.kill", {"terminal": t3}, calls)
    events.wait_for(event("finished", t3), 10, "finished t3")
    killed_close = [p for k, p in a.drain(v3["channel"], 0.5) if k == CLOSE]

    a.call("terminal.run", {"argv": ["echo", "hi"], "cwd": work, "wait": True, "timeout": 10.0}, calls)

    a.call("machine.exec", {"argv": ["echo", "hi"], "cwd": work}, calls)
    path = os.path.join(work, "file.txt")
    a.call("file.write", {"path": path, "data": base64.b64encode(b"one\ntwo\n").decode()}, calls)
    a.call("file.edit", {"path": path, "edits": [{"old": "two", "new": "three"}]}, calls)
    a.call("file.read", {"path": path}, calls)

    a.call("doc.get", {"name": "fixture-doc"}, calls)
    v = a.call("doc.set", {"name": "fixture-doc", "value": {"n": 1, "later": {"kept": True}}, "version": 0}, calls)["version"]
    a.call("doc.set", {"name": "fixture-doc", "value": {"n": 2}, "version": 0}, calls)
    a.call("doc.get", {"name": "fixture-doc"}, calls)
    a.call("doc.set", {"name": "fixture-doc", "value": None, "version": v}, calls)
    events.wait_for(event("doc"), 5, "doc")

    a.call("nope.nope", {}, calls)
    a.call("terminal.get", {"terminal": "t999"}, calls)
    a.call("terminal.attach", {"terminal": t1}, calls)
    a.call("doc.get", {"name": "x", "on": "nowhere"}, calls)

    events.pump(1.5)
    a.call("terminal.list", {"all": True}, calls)
    raw, frames = a.stop_recording()
    # Heartbeats come every 15 s on a quiet stream.
    events.wait_for(lambda e: e.get("method") == "heartbeat", 20, "heartbeat")
    event_lines = list(events.lines)

    # A second subscriber that never credits falls behind and gets `missed`.
    c = Conn(env)
    sub_c = c.call("events.subscribe", {})
    big = "x" * 60_000
    for i in range(120):
        a.call("terminal.label", {"terminal": t1, "labels": {"fixture.big": f"{i}{big}"}})
    a.call("terminal.label", {"terminal": t1, "labels": {"fixture.big": ""}})
    c.send(sub_c["channel"], CREDIT, struct.pack("<I", 64 << 20))
    c.credit_channels.add(sub_c["channel"])
    missed = Lines(c, sub_c["channel"])
    missed.wait_for(lambda e: e.get("method") == "missed", 15, "missed")
    missed.pump(0.5)

    host = a.hello["core"]["name"]
    write(host, calls, event_lines, sub, missed, sub_c, raw, frames, lost_close + killed_close, schema)


def shrink(value):
    """The `missed` test only needs the shape: cut the 60 kB label values down to their counter."""
    if isinstance(value, dict):
        return {k: shrink(v) for k, v in value.items()}
    if isinstance(value, list):
        return [shrink(v) for v in value]
    if isinstance(value, str) and len(value) > 1000:
        return value.rstrip("x") + "x"
    return value


def scrub(text, host):
    """This machine's name is the core's name: don't commit it."""
    return text.replace(json.dumps(host), json.dumps("fixture-host"))


def dump(value, host):
    return scrub(json.dumps(value, separators=(",", ":")), host) + "\n"


def write(host, calls, event_lines, sub, missed, sub_c, raw, frames, closes, schema):
    os.makedirs(OUT, exist_ok=True)
    with open(os.path.join(OUT, "schema.json"), "w") as f:
        f.write(scrub(json.dumps(schema, indent=1, sort_keys=True), host) + "\n")
    with open(os.path.join(OUT, "calls.jsonl"), "w") as f:
        for call in calls:
            f.write(dump(call, host))
    with open(os.path.join(OUT, "events.jsonl"), "w") as f:
        f.write(dump({"subscribe": sub}, host))
        for line in event_lines:
            f.write(dump(line, host))
    with open(os.path.join(OUT, "missed.jsonl"), "w") as f:
        f.write(dump({"subscribe": shrink(sub_c)}, host))
        for line in missed.lines:
            f.write(dump(shrink(line), host))
    # Raw bytes keep their lengths: pad the name to the same length instead of replacing it.
    name = json.dumps(host).encode()
    padded = json.dumps("fixture-host"[: len(host)].ljust(len(host), "x")).encode()
    with open(os.path.join(OUT, "frames.bin"), "wb") as f:
        f.write(raw.replace(name, padded))
    with open(os.path.join(OUT, "frames.jsonl"), "w") as f:
        for frame in frames:
            f.write(dump(frame, host))
    with open(os.path.join(OUT, "close.jsonl"), "w") as f:
        for payload in closes:
            f.write(payload.decode() + "\n")
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
