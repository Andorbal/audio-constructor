#!/usr/bin/env python3
"""Draw the application icon and write it in every size the installers use.

    python3 scripts/make_icon.py          # rewrites packaging/icons/

The icon is two node cards joined by an audio wire, in the editor's dark
theme colors (docs/UX.md "Look"): an amber wire leaves a sine on the left
card and lands on the right one. It's drawn at four times the largest size
and scaled down, which anti-aliases every edge.

Writes `icon-<size>.png` for 16 to 1024 pixels, `icon.ico` (Windows, 16 to
256), and `icon.icns` (macOS). Needs Pillow; the outputs are committed, so
nothing else needs it.
"""

from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "packaging" / "icons"

SIZES = [16, 32, 48, 64, 128, 256, 512, 1024]

# The dark palette (crates/ac-app/src/theme.rs) and the audio socket color.
BG_TOP = (0x26, 0x29, 0x31)
BG_BOTTOM = (0x15, 0x16, 0x1A)
CARD = (0x2C, 0x30, 0x38)
CARD_OUTLINE = (0x4A, 0x50, 0x5B)
HEADER = (0x34, 0x4A, 0x66)
AUDIO = (0xF0, 0xA0, 0x30)
TEXT = (0xF2, 0xF4, 0xF7)


def bezier(p0, p1, p2, p3, steps=200):
    pts = []
    for i in range(steps + 1):
        t = i / steps
        u = 1 - t
        x = u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0]
        y = u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]
        pts.append((x, y))
    return pts


def thick_line(draw: ImageDraw.ImageDraw, pts, width, fill):
    """A polyline with round joins: segments plus a disc at every point."""
    r = width / 2
    draw.line(pts, fill=fill, width=int(width))
    for x, y in pts[:: max(1, len(pts) // 60)] + [pts[-1]]:
        draw.ellipse((x - r, y - r, x + r, y + r), fill=fill)


def draw(size: int) -> Image.Image:
    s = size / 1024
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))

    # Background: a rounded square with a vertical gradient, inset the way
    # platform icon grids expect (macOS leaves ~10 % around the shape).
    inset = 100 * s
    radius = 185 * s
    grad = Image.new("RGBA", (size, size))
    gd = ImageDraw.Draw(grad)
    for y in range(size):
        t = y / max(1, size - 1)
        c = tuple(round(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t) for i in range(3))
        gd.line([(0, y), (size, y)], fill=c + (255,))
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (inset, inset, size - inset, size - inset), radius=radius, fill=255
    )
    img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)

    def card(x0, y0, x1, y1):
        r = 46 * s
        d.rounded_rectangle((x0, y0, x1, y1), radius=r, fill=CARD, outline=CARD_OUTLINE, width=max(1, round(10 * s)))
        # The header strip.
        d.rounded_rectangle((x0, y0, x1, y0 + 70 * s), radius=r, fill=HEADER)
        d.rectangle((x0, y0 + 40 * s, x1, y0 + 70 * s), fill=HEADER)

    left = (165 * s, 240 * s, 455 * s, 530 * s)
    right = (569 * s, 494 * s, 859 * s, 784 * s)

    # The wire goes under the cards' edges, from the left card's output
    # socket to the right card's input socket.
    out_socket = (left[2], 440 * s)
    in_socket = (right[0], 680 * s)
    pts = bezier(out_socket, (out_socket[0] + 160 * s, out_socket[1]), (in_socket[0] - 160 * s, in_socket[1]), in_socket)

    card(*left)
    card(*right)
    thick_line(d, pts, 34 * s, AUDIO)

    # A sine on the left card.
    sx0, sx1 = left[0] + 50 * s, left[2] - 60 * s
    cy, amp = 400 * s, 60 * s
    wave = [
        (sx0 + (sx1 - sx0) * i / 120, cy - amp * math.sin(2 * math.pi * i / 120))
        for i in range(121)
    ]
    thick_line(d, wave, 26 * s, TEXT)

    # Bars (a level meter) on the right card.
    bx = right[0] + 60 * s
    for i, h in enumerate([120, 180, 90]):
        x = bx + i * 70 * s
        d.rounded_rectangle((x, 730 * s - h * s, x + 42 * s, 730 * s), radius=10 * s, fill=TEXT)

    # Sockets: amber discs with a dark ring.
    for x, y in (out_socket, in_socket):
        r = 40 * s
        d.ellipse((x - r, y - r, x + r, y + r), fill=BG_BOTTOM)
        r = 28 * s
        d.ellipse((x - r, y - r, x + r, y + r), fill=AUDIO)
    return img


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    master = draw(4096).resize((1024, 1024), Image.LANCZOS)
    images = {}
    for size in SIZES:
        # Small sizes come from a sharper redraw at 4x their own size, which
        # keeps lines from thinning to nothing.
        img = master if size == 1024 else draw(size * 4).resize((size, size), Image.LANCZOS)
        img.save(OUT / f"icon-{size}.png", optimize=True)
        images[size] = img
    images[256].save(OUT / "icon.ico", sizes=[(n, n) for n in (16, 32, 48, 64, 128, 256)])
    master.save(OUT / "icon.icns")
    print(f"wrote {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
