#!/usr/bin/env python3
"""Write the synthetic fixture screens the renderer scenes and the Ghostty pixel diff draw.

    python3 -I make.py

Each screen is the bytes a program would write to an 80x30 terminal, ending with the cursor
parked where the screen's scene expects it. `nvim-splits.bin` is a recording instead; its
command line is in `README.md`.
"""

import os

ESC = "\x1b"
CSI = ESC + "["


def sgr(*codes):
    return CSI + ";".join(str(c) for c in codes) + "m"


RESET = sgr(0)


def colors():
    out = [CSI + "H" + CSI + "2J"]
    out.append("16 colors, fg then bg:\r\n")
    for base in (30, 90):
        for i in range(8):
            out.append(sgr(base + i) + f" fg{base + i - 30:>2} " + RESET)
        out.append("\r\n")
    for base in (40, 100):
        for i in range(8):
            out.append(sgr(base + i) + f" bg{base + i - 40:>2} " + RESET)
        out.append("\r\n")
    out.append("256-color cube and greys:\r\n")
    for row in range(6):
        for i in range(36):
            out.append(sgr(48, 5, 16 + row * 36 + i) + "  ")
        out.append(RESET + "\r\n")
    for i in range(24):
        out.append(sgr(48, 5, 232 + i) + "   ")
    out.append(RESET + "\r\n")
    out.append("Truecolor:\r\n")
    for i in range(80):
        r = int(255 * i / 79)
        out.append(sgr(48, 2, r, 64, 255 - r) + " ")
    out.append(RESET + "\r\n")
    out.append("Styles: ")
    out.append(sgr(1) + "bold" + RESET + " ")
    out.append(sgr(3) + "italic" + RESET + " ")
    out.append(sgr(1, 3) + "bold italic" + RESET + " ")
    out.append(sgr(2) + "faint" + RESET + " ")
    out.append(sgr(4) + "underline" + RESET + " ")
    out.append(sgr("4:2") + "double" + RESET + " ")
    out.append(sgr(9) + "strike" + RESET + " ")
    out.append(sgr(7) + "inverse" + RESET + "\r\n")
    out.append("More: " + sgr(8) + "invisible" + RESET + " ")
    out.append(sgr(53) + "overline" + RESET + " ")
    out.append(sgr(1, 31) + "bold red" + RESET + " ")
    out.append(sgr(3, 32) + "italic green" + RESET + " ")
    out.append(sgr(2, 34) + "faint blue" + RESET + "\r\n")
    out.append("Colors: ")
    out.append(sgr(4, 34) + sgr("58:2::255:128:0") + "orange underline" + RESET + " ")
    out.append(sgr(7, 33) + "inverse yellow" + RESET + " ")
    out.append(sgr(38, 2, 255, 170, 0) + sgr(48, 2, 30, 60, 90) + "24-bit fg on bg" + RESET)
    out.append("\r\n")
    out.append("Text: The quick brown fox jumps over the lazy dog. 0123456789\r\n")
    out.append("Punctuation: ~!@#$%^&*()_+{}|:\"<>? -> => != == <= >= /* */ // www fi fl\r\n")
    out.append("Wide: 日本語のテキスト and 한국어 and emoji 🙂🚀 end\r\n")
    out.append("Marks: café naïve e\u0301 a\u0308 Ωμέγα Привет\r\n")
    out.append("Underlines: ")
    for code, name in (("4:1", "single"), ("4:2", "double"), ("4:3", "curly"), ("4:4", "dotted"), ("4:5", "dashed")):
        out.append(sgr(code) + name + RESET + " ")
    out.append(sgr("4:3") + sgr("58:2::244:71:71") + "error curl" + RESET + " ")
    out.append(sgr("4:4", 33) + sgr("58:5:4") + "dots" + RESET + "\r\n")
    out.append(CSI + "2 q")  # a steady block
    out.append(CSI + "29;1H" + "$ ")
    return "".join(out)


def boxes():
    out = [CSI + "H" + CSI + "2J"]
    out.append("Light, heavy and double lines, corners, tees and crosses:\r\n")
    rows = [
        "┌─┬─┐ ┏━┳━┓ ╔═╦═╗ ╭─┬─╮ ┍━┯━┑ ╒═╤═╕",
        "│ │ │ ┃ ┃ ┃ ║ ║ ║ │ │ │ │ │ │ │ │ │",
        "├─┼─┤ ┣━╋━┫ ╠═╬═╣ ├─┼─┤ ┝━┿━┥ ╞═╪═╡",
        "│ │ │ ┃ ┃ ┃ ║ ║ ║ │ │ │ │ │ │ │ │ │",
        "└─┴─┘ ┗━┻━┛ ╚═╩═╝ ╰─┴─╯ ┕━┷━┙ ╘═╧═╛",
    ]
    for row in rows:
        out.append(row + "\r\n")
    out.append("\r\n")
    out.append("A window with a title, as a TUI draws it:\r\n")
    width = 40
    out.append(sgr(36) + "╭" + "─" * 2 + sgr(1) + " Title " + RESET + sgr(36))
    out.append("─" * (width - 11) + "╮" + RESET + "\r\n")
    for line in ("A line of text inside the box.", "", "Another, in " + sgr(33) + "yellow" + sgr(36) + "."):
        visible = len(line.replace(sgr(33), "").replace(sgr(36), ""))
        out.append(sgr(36) + "│" + RESET + " " + line + " " * (width - 3 - visible) + sgr(36) + "│" + RESET + "\r\n")
    out.append(sgr(36) + "╰" + "─" * (width - 2) + "╯" + RESET + "\r\n")
    out.append("\r\n")
    out.append("Splits: " + "─" * 20 + "┼" + "─" * 20 + "\r\n")
    out.append("Half lines and mixes: ╴╵╶╷ ╸╹╺╻ ╼╽╾╿ ┄┅┆┇ ┈┉┊┋ ╌╍╎╏\r\n")
    out.append("Blocks: █▀▄▌▐░▒▓ ▁▂▃▄▅▆▇█ ▏▎▍▌▋▊▉ ▖▗▘▙▚▛▜▝▞▟\r\n")
    out.append("Braille: ⠁⠃⠇⡇⣇⣧⣷⣿ ⢀⣀⣠⣤⣴⣶⣾⣿\r\n")
    out.append("Powerline: " + sgr(30, 44) + " main " + sgr(34, 42) + "\ue0b0" + sgr(30, 42) + " src " + sgr(32, 49) + "\ue0b0" + RESET + " \ue0b1 \ue0b2 \ue0b3\r\n")
    out.append("Shapes: ◢◣◤◥ ◸◹◺◿ ╱╲╳\r\n")
    out.append(CSI + "29;1H")
    return "".join(out)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    for name, screen in (("colors", colors()), ("boxes", boxes())):
        path = os.path.join(here, name + ".bin")
        with open(path, "wb") as f:
            f.write(screen.encode())
        print(path)


if __name__ == "__main__":
    main()
