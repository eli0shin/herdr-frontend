#!/usr/bin/env python3
"""Record real herdr-app layout documents for herdr-shared's golden tests
(crates/herdr-shared/tests/fixtures/documents).

It reads, and only reads, the user's running core: `machine.list`, then `doc.get herdr-app` for
this machine and for every connected machine that isn't temporary. It never writes a document,
label or terminal. Run it as `python3 -I scripts/record_shared_fixtures.py`, then review the diff.

Each fixture is one document's `value`, byte for byte in the core's own format (sorted keys, no
spaces). Space and tab names and space folders are personal, so they are replaced, keeping empty
ones empty: the script first checks that re-printing the untouched value gives back the core's
exact bytes, so the replacement changes nothing but those strings. Ids and terminal addresses are
kept. Fixtures are numbered (`machine-0.json` is this machine) so machine names stay out.
"""

import json
import os
import subprocess
import sys

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "herdr-shared", "tests", "fixtures", "documents")


def core_format(value):
    # As serde_json prints a Value: sorted keys, no spaces, non-ASCII as is.
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


class Conn:
    def __init__(self):
        self.p = subprocess.Popen(["hc", "connect"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.next = 0
        self.call("hello", {"generation": 1})

    def call(self, method, params):
        request = {"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}
        self.next += 1
        self.p.stdin.write(json.dumps(request).encode() + b"\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline().decode()
        reply = json.loads(line)
        assert reply["id"] == request["id"], reply
        if "error" in reply:
            raise RuntimeError(f"{method}: {reply['error']}")
        return line.rstrip("\n"), reply["result"]

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=10)


def scrub(value):
    names = {"n": 0}

    def name(text, kind):
        if not text:
            return text
        names["n"] += 1
        return f"{kind} {names['n']}"

    for i, space in enumerate(value.get("spaces", [])):
        space["name"] = name(space.get("name", ""), "space")
        if space.get("folder"):
            space["folder"] = f"/home/user/folder-{i}"
        for tab in space.get("tabs", []):
            tab["name"] = name(tab.get("name", ""), "tab")
    return value


def main():
    conn = Conn()
    _, machines = conn.call("machine.list", {})
    targets = [None] + sorted(
        m["name"] for m in machines["machines"] if m.get("state") == "connected" and not m.get("temporary")
    )
    documents = []
    for on in targets:
        params = {"name": "herdr-app"} | ({"on": on} if on else {})
        line, result = conn.call("doc.get", params)
        value = result["value"]
        if value is None:
            continue
        # The re-printed value must be the core's own bytes, or scrubbing would change more.
        assert '"value":' + core_format(value) in line, f"{on or 'this machine'}: not the core's format"
        documents.append(core_format(scrub(value)))
    conn.close()
    os.makedirs(OUT, exist_ok=True)
    for old in os.listdir(OUT):
        if old.startswith("machine-") and old.endswith(".json"):
            os.remove(os.path.join(OUT, old))
    for i, text in enumerate(documents):
        with open(os.path.join(OUT, f"machine-{i}.json"), "w", encoding="utf-8") as f:
            f.write(text + "\n")
    print(f"wrote {len(documents)} documents to {os.path.relpath(OUT)}", file=sys.stderr)


if __name__ == "__main__":
    main()
