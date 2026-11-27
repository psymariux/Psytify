#!/usr/bin/env python3
"""README art (docs/assets/banner.svg and preview.svg) drawn from the pixel sprite sources.

Every shape is a crisp rect on an integer grid: the sprites are read from design/assets/pixel/src
and the colours from the manifest tokens, so the README follows the design when a sprite changes.

Usage: python3 -I design/tools/readme_art.py
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "design/assets/pixel/src"
OUT = ROOT / "docs/assets"

TOKENS = {
    "o": "#0e0d0c", "k": "#1a1917", "c": "#2a2826", "i": "#3d3a37", "s": "#78746d",
    "m": "#9a958d", "g": "#bdb8af", "w": "#ece8e0", "p": "#ffffff", "l": "#c6f24e",
}
BLACK, CHROME, RAISED, TEXT, MUTED, LIME = "#000000", "#0e0d0c", "#1a1917", "#ece8e0", "#9a958d", "#c6f24e"


def grid(path: str, frame: int = 0) -> list[str]:
    frames: list[list[str]] = []
    for line in (SRC / path).read_text().splitlines():
        if line.startswith("@frame"):
            frames.append([])
        elif frames and line and not line.startswith("# "):
            frames[-1].append(line)
    return frames[min(frame, len(frames) - 1)]


def rects(rows: list[str], x: int, y: int, scale: int, tint: str | None = None) -> str:
    """One <rect> per horizontal run of equal colour."""
    out = []
    for r, row in enumerate(rows):
        for match in re.finditer(r"(.)\1*", row):
            ch = match.group(0)[0]
            if ch == ".":
                continue
            colour = tint if tint and ch == "#" else TOKENS.get(ch, tint or TEXT)
            out.append(
                f'<rect x="{x + match.start() * scale}" y="{y + r * scale}" '
                f'width="{len(match.group(0)) * scale}" height="{scale}" fill="{colour}"/>'
            )
    return "".join(out)


def text(x: int, y: int, size: int, fill: str, content: str, weight: int = 500, anchor: str = "start") -> str:
    return (
        f'<text x="{x}" y="{y}" fill="{fill}" font-size="{size}" font-weight="{weight}" '
        f'text-anchor="{anchor}">{content}</text>'
    )


FONT = "Inter, 'Segoe UI', -apple-system, Helvetica, Arial, sans-serif"
MONO = "'Departure Mono', 'JetBrains Mono', ui-monospace, Menlo, Consolas, monospace"


def eq_bars(x: int, y: int, scale: int) -> str:
    """Three lime bars that bounce (SMIL; static where animation is unsupported)."""
    spec = [(0, "8;4;10;6;8", "0.9s"), (3, "4;10;5;9;4", "1.1s"), (6, "10;6;8;3;10", "1.0s")]
    out = []
    for i, (dx, values, dur) in enumerate(spec):
        heights = [int(v) * scale for v in values.split(";")]
        ys = [y + 10 * scale - h for h in heights]
        out.append(
            f'<rect x="{x + dx * scale}" y="{ys[0]}" width="{2 * scale}" height="{heights[0]}" fill="{LIME}">'
            f'<animate attributeName="height" values="{";".join(map(str, heights))}" dur="{dur}" repeatCount="indefinite"/>'
            f'<animate attributeName="y" values="{";".join(map(str, ys))}" dur="{dur}" repeatCount="indefinite"/></rect>'
        )
    return "".join(out)


def banner() -> str:
    w, h = 1280, 320
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
        f'shape-rendering="crispEdges" role="img" aria-label="Psytify"><title>Psytify</title>',
        f'<rect width="{w}" height="{h}" fill="{BLACK}"/>',
        f'<rect y="{h - 8}" width="{w}" height="8" fill="{CHROME}"/>',
        rects(grid("motifs/app_icon.txt"), 96, 64, 12),
        f'<g font-family="{MONO}">',
        text(336, 168, 88, TEXT, "Psytify", 400),
        "</g>",
        f'<g font-family="{FONT}">',
        text(340, 216, 20, MUTED, "A fast, lightweight desktop and Android music player, written in Rust."),
        "</g>",
        eq_bars(1120, 56, 8),
        rects(grid("mascot/nastro_idle.txt"), 1130, 150, 4),
        "</svg>",
    ]
    return "".join(parts)


def preview() -> str:
    w, h = 1280, 720
    side, bar, top = 200, 72, 40
    p = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
        f'shape-rendering="crispEdges" role="img" aria-label="Illustration of the Psytify desktop interface">'
        "<title>Psytify interface</title>",
        f'<rect width="{w}" height="{h}" fill="{BLACK}"/>',
        f'<rect width="{w}" height="{top}" fill="{CHROME}"/>',
        rects(grid("motifs/mark.txt"), 14, 8, 2, TEXT),
        f'<g font-family="{MONO}">{text(48, 26, 16, TEXT, "Psytify", 400)}</g>',
        f'<rect x="200" y="6" width="360" height="28" fill="{BLACK}" stroke="{MUTED}"/>',
        rects(grid("icons/search.txt"), 208, 10, 2, MUTED),
        f'<g font-family="{FONT}">{text(238, 25, 13, MUTED, "Search tracks, albums, artists")}</g>',
        f'<rect y="{top}" width="{side}" height="{h - top - bar}" fill="{CHROME}"/>',
        f'<rect x="{side}" y="{top}" width="1" height="{h - top - bar}" fill="#2a2826"/>',
    ]
    nav = [("home_fill", "Home", True), ("search", "Search", False), ("library", "Library", False),
           ("bell_ring", "News", False), ("download", "Downloads", False), ("people", "Jam", False),
           ("account", "Friends", False), ("settings", "Settings", False)]
    for n, (icon, label, active) in enumerate(nav):
        y = top + 12 + n * 34
        if active:
            p.append(f'<rect x="0" y="{y}" width="3" height="28" fill="{LIME}"/>')
        p.append(rects(grid(f"icons/{icon}.txt"), 16, y + 2, 2, TEXT if active else MUTED))
        p.append(f'<g font-family="{FONT}">{text(52, y + 19, 14, TEXT if active else MUTED, label, 600 if active else 500)}</g>')
    mx = side + 28
    p.append(f'<g font-family="{MONO}">{text(mx, top + 52, 26, TEXT, "Home", 400)}</g>')
    tiles = ["Liked songs", "Night drive", "Tape hiss", "Slow lights", "Open roads", "Low tide", "Static", "Afterglow"]
    for n, name in enumerate(tiles):
        x = mx + (n % 2) * 480
        y = top + 76 + (n // 2) * 52
        p.append(f'<rect x="{x}" y="{y}" width="468" height="44" fill="{RAISED}"/>')
        if n == 0:
            p.append(rects(grid("icons/heart_fill.txt"), x + 10, y + 10, 2, TEXT))
        else:
            p.append(rects(grid("motifs/album_placeholder.txt"), x, y, 2))
        p.append(f'<g font-family="{FONT}">{text(x + 64, y + 27, 14, TEXT, name, 600)}</g>')
    ry = top + 76 + 4 * 52 + 36
    p.append(f'<g font-family="{FONT}">{text(mx, ry, 18, TEXT, "Recently added", 700)}</g>')
    for n, (title, artist) in enumerate([("Glass Rooms", "Hollow Pines"), ("Paper Moon", "Ada Lund"),
                                         ("Signal Fade", "Kite and Co."), ("Long Way Round", "Mara Vey")]):
        y = ry + 16 + n * 48
        p.append(rects(grid("motifs/album_placeholder.txt"), mx, y, 2))
        p.append(f'<g font-family="{FONT}">{text(mx + 48, y + 19, 14, TEXT, title, 600)}{text(mx + 48, y + 36, 12, MUTED, artist)}'
                 f'{text(w - 36, y + 26, 12, MUTED, f"{3 + n}:{10 + n * 7:02d}", anchor="end")}</g>')
        if n == 0:
            p.append(eq_bars(w - 84, y + 14, 2))
    by = h - bar
    p.append(f'<rect y="{by}" width="{w}" height="{bar}" fill="{CHROME}"/><rect y="{by}" width="{w}" height="1" fill="#2a2826"/>')
    p.append(rects(grid("motifs/album_placeholder.txt"), 16, by + 12, 2))
    p.append(f'<g font-family="{FONT}">{text(80, by + 32, 14, TEXT, "Glass Rooms", 600)}{text(80, by + 50, 12, MUTED, "Hollow Pines")}</g>')
    cx = 520
    for n, (icon, tint, size) in enumerate([("shuffle", MUTED, 2), ("previous", MUTED, 2), ("play", BLACK, 2), ("next", MUTED, 2), ("repeat", LIME, 2)]):
        x = cx + n * 44
        if icon == "play":
            p.append(f'<rect x="{x - 8}" y="{by + 10}" width="40" height="40" fill="{TEXT}"/>')
        p.append(rects(grid(f"icons/{icon}.txt"), x, by + 18 if icon != "play" else by + 18, size, tint))
    p.append(f'<rect x="440" y="{by + 58}" width="400" height="4" fill="#2a2826"/><rect x="440" y="{by + 58}" width="150" height="4" fill="{TEXT}"/>')
    for n, icon in enumerate(["video", "lyrics", "queue", "volume", "fullscreen"]):
        p.append(rects(grid(f"icons/{icon}.txt"), 1000 + n * 44, by + 24, 2, MUTED))
    p.append("</svg>")
    return "".join(p)


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "banner.svg").write_text(banner() + "\n")
    (OUT / "preview.svg").write_text(preview() + "\n")
    print("wrote docs/assets/banner.svg and preview.svg")


if __name__ == "__main__":
    main()
