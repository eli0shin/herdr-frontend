#!/usr/bin/env python3
"""Generate crates/app/src/view/terminal/glyph/tables.rs from a Ghostty checkout.

    python3 -I scripts/gen_ghostty_tables.py GHOSTTY_DIR EMOJI_DATA

GHOSTTY_DIR is a checkout of ghostty-org/ghostty at the tag the renderer ports (v1.4.0). Its
generated src/font/nerd_font_tables.zig, each Nerd Font code point's glyph constraint
(`font/nerd_font_attributes.zig` `getConstraint`), becomes a range table here.

EMOJI_DATA is the Unicode `emoji-data.txt` from the uucode Ghostty pins (its build.zig.zon,
`ucd/emoji/emoji-data.txt`). Its `Emoji_Presentation` code points, which Ghostty draws as emoji
when no variation selector says otherwise (`font/CodepointResolver.zig`), become a range table.

It's a 3-level lookup (`src/unicode/lut.zig`): stage1[cp >> 8] is an offset into stage2,
stage2[offset + (cp & 0xff)] an index into stage3, and stage3 holds the values.
"""

import os
import re
import sys

MAX_CP = 0x10FFFF


def arrays(text):
    """The `stage1`, `stage2` and `stage3` initializers in a generated table file."""
    out = {}
    for name in ("stage1", "stage2", "stage3"):
        m = re.search(name + r": \[\d+\][^=]*= \.\{", text)
        if not m:
            raise SystemExit(f"no {name} in table")
        start = m.end()
        depth = 1
        i = start
        while depth:
            c = text[i]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            i += 1
        out[name] = text[start : i - 1]
    return out


def ints(body):
    return [int(v) for v in re.findall(r"-?\d+", body)]


def ranges(lookup):
    """[(lo, hi, value)] for every run of code points with the same non-null value."""
    out = []
    run = None
    for cp in range(MAX_CP + 1):
        v = lookup(cp)
        if run and v == run[2] and cp == run[1] + 1:
            run[1] = cp
            continue
        if run and run[2] is not None and run[2] is not False:
            out.append(tuple(run))
        run = [cp, cp, v]
    if run and run[2] is not None and run[2] is not False:
        out.append(tuple(run))
    return out


def lut(stage1, stage2, stage3):
    def get(cp):
        return stage3[stage2[stage1[cp >> 8] + (cp & 0xFF)]]

    return get


FIELDS = {
    "size": ("size", lambda v: "Size::" + {"fit": "Fit", "cover": "Cover", "fit_cover1": "FitCover1", "stretch": "Stretch", "none": "None"}[v.lstrip(".")]),
    "height": ("height", lambda v: "Height::" + {"cell": "Cell", "icon": "Icon"}[v.lstrip(".")]),
    "align_horizontal": ("align_horizontal", lambda v: "Align::" + {"none": "None", "start": "Start", "end": "End", "center": "Center", "center1": "Center1"}[v.lstrip(".")]),
    "align_vertical": ("align_vertical", lambda v: "Align::" + {"none": "None", "start": "Start", "end": "End", "center": "Center", "center1": "Center1"}[v.lstrip(".")]),
    "max_constraint_width": ("max_constraint_width", lambda v: v),
    "max_xy_ratio": ("max_xy_ratio", lambda v: f"Some({float(v)!r})"),
}
FLOATS = ("pad_left", "pad_right", "pad_top", "pad_bottom", "relative_width", "relative_height", "relative_x", "relative_y")


def constraint(body):
    """A stage3 struct literal as a Rust `Constraint` expression."""
    parts = []
    for name, value in re.findall(r"\.(\w+) = ([^,]+),", body):
        value = value.strip()
        if name in FIELDS:
            field, conv = FIELDS[name]
            parts.append(f"{field}: {conv(value)}")
        elif name in FLOATS:
            parts.append(f"{name}: {float(value)!r}")
        else:
            raise SystemExit(f"unknown constraint field {name}")
    return "Constraint { " + ", ".join(parts + ["..Constraint::NONE"]) + " }"


def nerd(ghostty):
    text = open(os.path.join(ghostty, "src/font/nerd_font_tables.zig")).read()
    a = arrays(text)
    body = a["stage3"]
    entries = []
    i = 0
    while i < len(body):
        c = body[i]
        if body.startswith("null", i):
            entries.append(None)
            i += 4
        elif c == "." and body.startswith(".{", i):
            depth = 0
            j = i + 1
            while True:
                if body[j] == "{":
                    depth += 1
                elif body[j] == "}":
                    depth -= 1
                    if depth == 0:
                        break
                j += 1
            entries.append(constraint(body[i + 2 : j]))
            i = j + 1
        else:
            i += 1
    distinct = sorted({e for e in entries if e is not None})
    index = {e: k for k, e in enumerate(distinct)}
    stage3 = [None if e is None else index[e] for e in entries]
    return distinct, ranges(lut(ints(a["stage1"]), ints(a["stage2"]), stage3))


def emoji_presentation(path):
    """The `Emoji_Presentation` ranges in emoji-data.txt, merged."""
    out = []
    for line in open(path, encoding="utf-8"):
        data = line.split("#", 1)[0].strip()
        if not data:
            continue
        cps, prop = (part.strip() for part in data.split(";"))
        if prop != "Emoji_Presentation":
            continue
        lo, _, hi = cps.partition("..")
        lo, hi = int(lo, 16), int(hi or lo, 16)
        if out and out[-1][1] + 1 == lo:
            out[-1][1] = hi
        else:
            out.append([lo, hi])
    return sorted(out)


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    ghostty = sys.argv[1]
    emoji = emoji_presentation(sys.argv[2])
    constraints, nerd_ranges = nerd(ghostty)
    here = os.path.dirname(os.path.abspath(__file__))
    out = os.path.join(here, "..", "crates/app/src/view/terminal/glyph/tables.rs")
    with open(out, "w") as f:
        f.write("//! Generated by `scripts/gen_ghostty_tables.py` from Ghostty v1.4.0's\n")
        f.write("//! `src/font/nerd_font_tables.zig` and the `emoji-data.txt` of the uucode it pins.\n")
        f.write("//! Don't edit by hand.\n\n")
        f.write("use super::constraint::{Align, Constraint, Height, Size};\n\n")
        f.write("/// The distinct Nerd Font constraints.\n")
        f.write("#[rustfmt::skip]\n")
        f.write(f"pub(super) static CONSTRAINTS: [Constraint; {len(constraints)}] = [\n")
        for c in constraints:
            f.write(f"    {c},\n")
        f.write("];\n\n")
        f.write("/// Each Nerd Font code point range's constraint, an index into `CONSTRAINTS`, in order.\n")
        f.write("#[rustfmt::skip]\n")
        f.write(f"pub(super) static NERD: [(u32, u32, u16); {len(nerd_ranges)}] = [\n")
        for lo, hi, v in nerd_ranges:
            f.write(f"    (0x{lo:X}, 0x{hi:X}, {v}),\n")
        f.write("];\n\n")
        f.write("/// The code points with the Unicode `Emoji_Presentation` property, as inclusive ranges in order.\n")
        f.write("#[rustfmt::skip]\n")
        f.write(f"pub(super) static EMOJI_PRESENTATION: [(u32, u32); {len(emoji)}] = [\n")
        for lo, hi in emoji:
            f.write(f"    (0x{lo:X}, 0x{hi:X}),\n")
        f.write("];\n")
    print(os.path.normpath(out), f"{len(constraints)} constraints, {len(nerd_ranges)} ranges, {len(emoji)} emoji ranges")


if __name__ == "__main__":
    main()
