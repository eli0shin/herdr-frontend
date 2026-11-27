#!/usr/bin/env python3
"""The screen `kitty-icat.bin` records: test images shown with `kitten icat` in each layer.

    python3 -I kitty-images.py             # writes photo.png, dots.png and glass.png here
    python3 -I kitty-images.py show KITTEN # writes them, then draws the screen with KITTEN

`show` is what `record.py` runs (`../images.rs` has the command line). It lays out an 80 x 30
screen of 17 x 37 px cells (JetBrains Mono 14 pt at 2x) and runs `KITTEN icat` for each image,
with text around them, then sends a few commands of its own and parks the cursor on a prompt.

The images are drawn here, with no imaging library, so the recording needs nothing installed but
`kitten`:

- photo.png: 136 x 74 RGBA (8 x 2 cells of 17 x 37 px), a red-to-blue gradient over green,
  opaque, with a white frame one pixel wide.
- dots.png: 34 x 37 RGB (2 x 1 cells), a checkerboard of 1-pixel black and yellow squares, so
  any resampling shows.
- glass.png: 68 x 37 RGBA (4 x 1 cells), white at half alpha with an opaque magenta left half.

It then sends what icat can't: a crop of an image with a pixel offset into its cell, raw
RGB and RGBA with zlib compression, and an image that is placed and then deleted.
"""

import base64
import struct
import subprocess
import sys
import zlib

ESC = "\x1b"


def png(width, height, channels, pixel):
    """A PNG of `pixel(x, y)` tuples: 3 channels for RGB, 4 for RGBA."""
    kind = {3: 2, 4: 6}[channels]
    raw = bytearray()
    for y in range(height):
        raw.append(0)
        for x in range(width):
            raw.extend(pixel(x, y))

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, kind, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def photo(x, y):
    if x == 0 or y == 0 or x == 135 or y == 73:
        return (255, 255, 255, 255)
    return (int(255 * (135 - x) / 135), 160, int(255 * x / 135), 255)


def dots(x, y):
    return (0, 0, 0) if (x + y) % 2 else (255, 220, 0)


def glass(x, y):
    return (255, 0, 255, 255) if x < 34 else (255, 255, 255, 128)


def apc(keys, payload=b""):
    data = base64.standard_b64encode(payload).decode()
    return f"{ESC}_G{keys};{data}{ESC}\\" if data else f"{ESC}_G{keys}{ESC}\\"


def commands():
    out = []
    # A 32 x 32 RGB image: four 16 x 16 quadrants, red, green, blue and white.
    quads = bytearray()
    for y in range(32):
        for x in range(32):
            quads.extend([(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 255)][(y >= 16) * 2 + (x >= 16)])
    out.append(apc("a=t,f=24,s=32,v=32,i=40,q=2", bytes(quads)))
    # Its green quadrant, stretched over 3 x 1 cells, 5 px right and 6 px down in the first one.
    out.append(f"{ESC}[22;2H" + apc("a=p,i=40,x=16,y=0,w=16,h=16,c=3,r=1,X=5,Y=6,C=1,q=2"))
    # The whole image again, zlib-compressed RGBA this time, at its own size.
    rgba = bytes(b for i in range(0, len(quads), 3) for b in (*quads[i:i + 3], 255))
    out.append(f"{ESC}[22;8H" + apc("a=T,f=32,o=z,s=32,v=32,i=41,C=1,q=2", zlib.compress(rgba)))
    # Placed, then deleted with its image: nothing is left to draw.
    out.append(f"{ESC}[22;12H" + apc("a=T,f=24,s=32,v=32,i=42,C=1,q=2", bytes(quads)))
    out.append(apc("a=d,d=I,i=42,q=2"))
    return "".join(out)


def at(row, col, text=""):
    """`text` at 0-based `row` and `col`."""
    return f"{ESC}[{row + 1};{col + 1}H{text}"


def icat(kitten, *args):
    """`kitten icat` on this terminal, sending pixels in the escape codes (as over SSH)."""
    sys.stdout.flush()
    subprocess.run([kitten, "icat", "--transfer-mode=stream", "--stdin=no",
                    "--use-window-size=80,30,1360,1110", *args], check=True)


def show(kitten):
    out = sys.stdout
    red, blue, reset = f"{ESC}[41m", f"{ESC}[44m", f"{ESC}[0m"
    out.write(f"{ESC}[H{ESC}[2J" + at(0, 0, "kitty graphics, kitten icat 0.51.1 (row 22 by hand)"))
    # photo.png at its own size at the cursor, and scaled by icat into 16 x 4 cells.
    out.write(at(1, 0))
    icat(kitten, "--align=left", "photo.png")
    icat(kitten, "--place=16x4@10x1", "--scale-up", "photo.png")
    # Below the text (z -1): over the cell backgrounds, under the letters.
    out.write(at(1, 28, f"below text {red} red {reset} on it"))
    out.write(at(2, 28, f"{blue}  blue cells  {reset} more"))
    out.write(at(3, 28, "the letters stay on top"))
    icat(kitten, "--z-index=-1", "--place=20x3@28x1", "--scale-up", "photo.png")
    # Below the backgrounds (z under INT32_MIN / 2): the red and blue cells hide it.
    out.write(at(1, 52, f"below bg {red}  red  {reset}"))
    out.write(at(2, 52, f"{blue}   blue  cells   {reset}"))
    out.write(at(3, 52, "only default cells"))
    icat(kitten, "--z-index=--1", "--place=20x3@52x1", "--scale-up", "photo.png")
    # Over the text (z 0, icat's default): half-transparent white, opaque magenta.
    out.write(at(6, 0, "above text: GLASSY covers me, and dots here:"))
    icat(kitten, "--place=4x1@12x6", "glass.png")
    icat(kitten, "--place=2x1@46x6", "dots.png")
    # Unicode placeholders: the image moves with the placeholder cells, like text.
    out.write(at(9, 0, "placeholders:"))
    out.write(at(10, 0))
    icat(kitten, "--unicode-placeholder", "--align=left", "photo.png")
    out.write(at(10, 20))
    icat(kitten, "--unicode-placeholder", "--place=16x4@20x10", "--scale-up", "photo.png")
    out.write(at(20, 0, "by hand: crop + offset, zlib RGBA, deleted:"))
    out.write(commands())
    out.write(at(29, 0, "$ "))
    out.flush()


def write_images():
    for name, image in (("photo.png", png(136, 74, 4, photo)), ("dots.png", png(34, 37, 3, dots)),
                        ("glass.png", png(68, 37, 4, glass))):
        with open(name, "wb") as f:
            f.write(image)


def main():
    write_images()
    if sys.argv[1:2] == ["show"]:
        show(sys.argv[2])


main()
