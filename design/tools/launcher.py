#!/usr/bin/env python3
"""Psytify launcher icons from the pixel sprites (Nastro's face is the brand mark).

Reads the sprite grids through sprites.py and writes, with nearest-neighbour integer
scaling only (no smoothing, no fractional scale):

  android-app/app/src/main/res/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher.png
      48/72/96/144/192 px, `app_icon` on deep_black. Scale = the largest integer that
      leaves at least 10 % padding on each side; the art is centred on whole pixels.
  android-app/app/src/main/res/drawable/ic_stat_psytify.xml
      notification icon: the `mark` sprite as a vector of rects, white on transparent
      (24 dp viewport, 2 dp per art pixel, so every density lands on whole pixels).
  design/assets/psytify.svg
      desktop icon: `app_icon` as crisp rects on deep_black (128 viewBox, 6 per art pixel).

Usage: python3 -I design/tools/launcher.py
Requires Pillow. Output is deterministic.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

from PIL import Image

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parents[1]

ANDROID_SIZES = {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}
PADDING = 0.10  # minimum padding per side, as a fraction of the canvas
SVG_VIEW = 128
SVG_SCALE = 6
STAT_VIEW = 24
STAT_SCALE = 2

Pixels = dict[tuple[int, int], tuple[int, int, int]]


def load_sprites():
    """Imports sprites.py by path (works under `python3 -I`, which drops the script dir)."""
    spec = importlib.util.spec_from_file_location("psytify_sprites", TOOLS / "sprites.py")
    if spec is None or spec.loader is None:
        sys.exit(f"cannot load {TOOLS / 'sprites.py'}")
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


def sprite_pixels(name: str, tint: str = "warm_white") -> Pixels:
    """Frame 0 of a sprite as {(x, y): rgb}, cropped to its art bounding box."""
    sprites = load_sprites()
    manifest, items, lint = sprites.load()
    if lint.errors:
        raise SystemExit("sprite lint errors: run sprites.py --check")
    sprite = next(s for s in items if s.name == name)
    pal = manifest["palettes"][sprite.palette]
    raw: Pixels = {}
    for y, row in enumerate(sprite.frames[0].rows):
        for x, ch in enumerate(row):
            role = pal[ch]
            if role != "transparent":
                raw[(x, y)] = sprites.role_rgb(manifest, role, tint)
    x0 = min(x for x, _ in raw)
    y0 = min(y for _, y in raw)
    return {(x - x0, y - y0): c for (x, y), c in raw.items()}


def size_of(px: Pixels) -> tuple[int, int]:
    return max(x for x, _ in px) + 1, max(y for _, y in px) + 1


def token_rgb(name: str) -> tuple[int, int, int]:
    sprites = load_sprites()
    manifest, _, _ = sprites.load()
    return sprites.hex_rgb(manifest["tokens"][name])


def rects(px: Pixels) -> dict[tuple[int, int, int], list[tuple[int, int, int, int]]]:
    """Greedy merge into rectangles per colour: horizontal runs, then equal runs stacked."""
    by_color: dict[tuple[int, int, int], list[tuple[int, int, int, int]]] = {}
    w, h = size_of(px)
    for color in sorted(set(px.values())):
        runs: list[tuple[int, int, int]] = []  # (y, x, length)
        for y in range(h):
            x = 0
            while x < w:
                if px.get((x, y)) == color:
                    start = x
                    while px.get((x, y)) == color:
                        x += 1
                    runs.append((y, start, x - start))
                else:
                    x += 1
        merged: list[list[int]] = []  # [x, y, w, h]
        open_: dict[tuple[int, int], list[int]] = {}
        for y, x, length in runs:
            prev = open_.get((x, length))
            if prev is not None and prev[1] + prev[3] == y:
                prev[3] += 1
            else:
                prev = [x, y, length, 1]
                merged.append(prev)
                open_[(x, length)] = prev
        by_color[color] = [(m[0], m[1], m[2], m[3]) for m in merged]
    return by_color


def fit_scale(art_w: int, art_h: int, canvas: int) -> int:
    scale = int(canvas * (1 - 2 * PADDING)) // max(art_w, art_h)
    return max(1, scale)


def android_png(px: Pixels, canvas: int, bg: tuple[int, int, int]) -> Image.Image:
    w, h = size_of(px)
    scale = fit_scale(w, h, canvas)
    ox, oy = (canvas - w * scale) // 2, (canvas - h * scale) // 2
    if (canvas - w * scale) % 2 or (canvas - h * scale) % 2:
        raise SystemExit(f"{canvas}px: art does not centre on whole pixels at scale {scale}")
    img = Image.new("RGBA", (canvas, canvas), bg + (255,))
    for (x, y), c in px.items():
        for dy in range(scale):
            for dx in range(scale):
                img.putpixel((ox + x * scale + dx, oy + y * scale + dy), c + (255,))
    return img


def stat_vector(px: Pixels) -> str:
    w, h = size_of(px)
    ox, oy = (STAT_VIEW - w * STAT_SCALE) // 2, (STAT_VIEW - h * STAT_SCALE) // 2
    paths = []
    for color_rects in rects(px).values():
        for x, y, rw, rh in color_rects:
            paths.append(
                f"M{ox + x * STAT_SCALE},{oy + y * STAT_SCALE}"
                f"h{rw * STAT_SCALE}v{rh * STAT_SCALE}h-{rw * STAT_SCALE}z"
            )
    return (
        '<?xml version="1.0" encoding="utf-8"?>\n'
        "<!-- Marchio di Psytify (la faccia di Nastro, sprite `mark`) per la barra di stato: Android lo vuole\n"
        "     bianco su trasparente. Generato da design/tools/launcher.py: non modificarlo a mano. -->\n"
        '<vector xmlns:android="http://schemas.android.com/apk/res/android"\n'
        f'    android:width="{STAT_VIEW}dp" android:height="{STAT_VIEW}dp" '
        f'android:viewportWidth="{STAT_VIEW}" android:viewportHeight="{STAT_VIEW}">\n'
        '    <path android:fillColor="#FFFFFFFF"\n'
        f'        android:pathData="{"".join(paths)}" />\n'
        "</vector>\n"
    )


def desktop_svg(px: Pixels, bg: tuple[int, int, int]) -> str:
    w, h = size_of(px)
    ox, oy = (SVG_VIEW - w * SVG_SCALE) // 2, (SVG_VIEW - h * SVG_SCALE) // 2
    hexc = lambda c: "#%02x%02x%02x" % c
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SVG_VIEW} {SVG_VIEW}" '
        'shape-rendering="crispEdges">',
        "<!-- Nastro's face (sprite `app_icon`). Generated by design/tools/launcher.py: do not edit by hand. -->",
        f'<rect width="{SVG_VIEW}" height="{SVG_VIEW}" fill="{hexc(bg)}"/>',
    ]
    for color, color_rects in rects(px).items():
        out.append(f'<g fill="{hexc(color)}">')
        for x, y, rw, rh in color_rects:
            out.append(
                f'<rect x="{ox + x * SVG_SCALE}" y="{oy + y * SVG_SCALE}" '
                f'width="{rw * SVG_SCALE}" height="{rh * SVG_SCALE}"/>'
            )
        out.append("</g>")
    out.append("</svg>")
    return "\n".join(out) + "\n"


def main() -> int:
    bg = token_rgb("deep_black")
    face = sprite_pixels("app_icon")
    for density, canvas in ANDROID_SIZES.items():
        out = REPO / "android-app" / "app" / "src" / "main" / "res" / f"mipmap-{density}" / "ic_launcher.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        android_png(face, canvas, bg).save(out, optimize=False)
        print(f"wrote {out.relative_to(REPO)} ({canvas}px)")
    mark = sprite_pixels("mark", "pure_white")
    stat = REPO / "android-app" / "app" / "src" / "main" / "res" / "drawable" / "ic_stat_psytify.xml"
    stat.parent.mkdir(parents=True, exist_ok=True)
    stat.write_text(stat_vector(mark), encoding="utf-8")
    print(f"wrote {stat.relative_to(REPO)}")
    svg = REPO / "design" / "assets" / "psytify.svg"
    svg.write_text(desktop_svg(face, bg), encoding="utf-8")
    print(f"wrote {svg.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
