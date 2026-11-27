#!/usr/bin/env python3
"""Psytify pixel sprites: text grids -> indexed PNG.

Source of truth: design/assets/pixel/manifest.toml (tokens, palettes, sprite list)
and one .txt grid per sprite (one character = one art pixel, frames separated by
'@frame' header lines). This script lints the grids, then writes:

  export/1x/<name>.png        indexed PNG, frames side by side, native art pixels
  export/sprites.json         frame size, count, durations, palette roles
  export/contact/<group>@4x.png   review sheets (4 device px per art pixel)
  export/preview/<name>@4x.gif    animated previews for multi-frame sprites

Usage:
  python3 -I design/tools/sprites.py            lint + export
  python3 -I design/tools/sprites.py --check    lint only, exit 1 on errors

Requires Pillow. Output is deterministic (no timestamps in the PNGs).
"""

from __future__ import annotations

import json
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1] / "assets" / "pixel"
EXPORT = ROOT / "export"
CONTACT_SCALE = 4  # device px per art pixel in the contact sheets
REAL_SCALE = 2  # 1 art pixel = 2 dp = 2 device px at 1x: the real on-screen size
SHEET_MAX_W = 1400  # contact sheet width before wrapping


def hex_rgb(value: str) -> tuple[int, int, int]:
    v = value.lstrip("#")
    return int(v[0:2], 16), int(v[2:4], 16), int(v[4:6], 16)


@dataclass
class Frame:
    rows: list[str]
    ms: int


@dataclass
class Sprite:
    name: str
    group: str
    src: Path
    size: tuple[int, int]
    palette: str
    frame_ms: int
    max_colors: int
    allow_isolated: list[tuple[int, int]]
    tint_preview: list[str]
    note: str
    frames: list[Frame] = field(default_factory=list)


class Lint:
    def __init__(self) -> None:
        self.errors: list[str] = []
        self.warnings: list[str] = []

    def error(self, msg: str) -> None:
        self.errors.append(msg)

    def warn(self, msg: str) -> None:
        self.warnings.append(msg)


def parse_grid(sprite: Sprite, lint: Lint) -> None:
    """Reads '@frame [ms=N]' blocks. Lines starting with '# ' are comments."""
    text = sprite.src.read_text(encoding="utf-8")
    current: list[str] | None = None
    ms = sprite.frame_ms
    for raw in text.splitlines():
        line = raw.rstrip()
        if line.startswith("# "):
            # Comment: '#' followed by a space. Pixel rows never contain spaces,
            # so a row of mask ink ('#') is still read as pixels.
            continue
        if line.startswith("@frame"):
            if current is not None:
                sprite.frames.append(Frame(current, ms))
            current = []
            ms = sprite.frame_ms
            for part in line.split()[1:]:
                if part.startswith("ms="):
                    ms = int(part[3:])
            continue
        if not line:
            continue
        if current is None:
            current = []
        current.append(line)
    if current is not None:
        sprite.frames.append(Frame(current, ms))
    if not sprite.frames:
        lint.error(f"{sprite.name}: no frames")


def neighbours(rows: list[str], x: int, y: int, diagonal: bool):
    steps = [(-1, 0), (1, 0), (0, -1), (0, 1)]
    if diagonal:
        steps += [(-1, -1), (1, -1), (-1, 1), (1, 1)]
    h, w = len(rows), len(rows[0])
    for dx, dy in steps:
        nx, ny = x + dx, y + dy
        if 0 <= nx < w and 0 <= ny < h:
            yield rows[ny][nx]


def lint_sprite(sprite: Sprite, palette: dict[str, str], lint: Lint) -> None:
    w, h = sprite.size
    used: set[str] = set()
    for i, frame in enumerate(sprite.frames):
        tag = f"{sprite.name}[{i}]"
        if len(frame.rows) != h:
            lint.error(f"{tag}: {len(frame.rows)} rows, expected {h}")
            continue
        for y, row in enumerate(frame.rows):
            if len(row) != w:
                lint.error(f"{tag}: row {y} has {len(row)} columns, expected {w}")
        if any(len(r) != w for r in frame.rows):
            continue
        islands = 0
        for y, row in enumerate(frame.rows):
            for x, ch in enumerate(row):
                if ch not in palette:
                    lint.error(f"{tag}: unknown character {ch!r} at {x},{y}")
                    continue
                if palette[ch] == "transparent":
                    continue
                used.add(ch)
                around = list(neighbours(frame.rows, x, y, diagonal=True))
                if all(palette.get(c) == "transparent" for c in around):
                    if (x, y) not in sprite.allow_isolated:
                        lint.error(f"{tag}: orphan pixel {ch!r} at {x},{y} (no neighbour)")
                if ch not in neighbours(frame.rows, x, y, diagonal=True):
                    islands += 1
        if islands:
            lint.warn(f"{tag}: {islands} single-pixel colour island(s) (check they are deliberate)")
    if len(used) > sprite.max_colors:
        lint.error(f"{sprite.name}: {len(used)} colours, limit {sprite.max_colors}")


def load() -> tuple[dict, list[Sprite], Lint]:
    manifest = tomllib.loads((ROOT / "manifest.toml").read_text(encoding="utf-8"))
    lint = Lint()
    sprites: list[Sprite] = []
    for entry in manifest["sprite"]:
        sprite = Sprite(
            name=entry["name"],
            group=entry["group"],
            src=ROOT / entry["src"],
            size=(entry["size"][0], entry["size"][1]),
            palette=entry["palette"],
            frame_ms=entry.get("frame_ms", 0),
            max_colors=entry.get("max_colors", manifest["max_colors"]),
            allow_isolated=[tuple(p) for p in entry.get("allow_isolated", [])],
            tint_preview=entry.get("tint_preview", ["warm_white"]),
            note=entry.get("note", ""),
        )
        if sprite.palette not in manifest["palettes"]:
            lint.error(f"{sprite.name}: unknown palette {sprite.palette}")
            continue
        if not sprite.src.exists():
            lint.error(f"{sprite.name}: missing {sprite.src}")
            continue
        parse_grid(sprite, lint)
        lint_sprite(sprite, manifest["palettes"][sprite.palette], lint)
        sprites.append(sprite)
    for name, palette in manifest["palettes"].items():
        for ch, role in palette.items():
            if role not in ("transparent", "tint") and role not in manifest["tokens"]:
                lint.error(f"palette {name}: {ch!r} maps to unknown token {role}")
    return manifest, sprites, lint


def palette_order(manifest: dict, sprite: Sprite) -> list[tuple[str, str]]:
    """Index order: transparent first, then the palette in manifest order."""
    pal = manifest["palettes"][sprite.palette]
    items = [(ch, role) for ch, role in pal.items() if role == "transparent"]
    items += [(ch, role) for ch, role in pal.items() if role != "transparent"]
    return items


def role_rgb(manifest: dict, role: str, tint: str) -> tuple[int, int, int]:
    if role == "tint":
        return hex_rgb(manifest["tokens"][tint])
    if role == "transparent":
        return (0, 0, 0)
    return hex_rgb(manifest["tokens"][role])


def export_png(manifest: dict, sprite: Sprite) -> dict:
    w, h = sprite.size
    order = palette_order(manifest, sprite)
    index = {ch: i for i, (ch, _) in enumerate(order)}
    img = Image.new("P", (w * len(sprite.frames), h), 0)
    flat: list[int] = []
    for _, role in order:
        flat.extend(role_rgb(manifest, role, "warm_white"))
    img.putpalette(flat + [0] * (768 - len(flat)))
    px = img.load()
    for f, frame in enumerate(sprite.frames):
        for y, row in enumerate(frame.rows):
            for x, ch in enumerate(row):
                px[f * w + x, y] = index[ch]
    out = EXPORT / "1x" / f"{sprite.name}.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    img.save(out, transparency=0, optimize=False)
    return {
        "name": sprite.name,
        "group": sprite.group,
        "file": f"1x/{sprite.name}.png",
        "frame_size": [w, h],
        "frames": len(sprite.frames),
        "durations_ms": [fr.ms for fr in sprite.frames],
        "palette": [role for _, role in order],
        "note": sprite.note,
    }


def render(manifest: dict, sprite: Sprite, frame: Frame, scale: int, tint: str,
           bg: tuple[int, int, int], grid: bool = False) -> Image.Image:
    w, h = sprite.size
    pal = manifest["palettes"][sprite.palette]
    img = Image.new("RGB", (w * scale, h * scale), bg)
    draw = ImageDraw.Draw(img)
    for y, row in enumerate(frame.rows):
        for x, ch in enumerate(row):
            role = pal[ch]
            if role == "transparent":
                continue
            c = role_rgb(manifest, role, tint)
            draw.rectangle([x * scale, y * scale, (x + 1) * scale - 1, (y + 1) * scale - 1], fill=c)
    if grid and scale >= 4:
        line = tuple(min(255, v + 18) for v in bg)
        for gx in range(0, w * scale, scale):
            for gy in range(0, h * scale, scale):
                if img.getpixel((gx, gy)) == bg:
                    img.putpixel((gx, gy), line)
    return img


def contact_sheets(manifest: dict, sprites: list[Sprite]) -> list[Path]:
    font = ImageFont.load_default()
    surfaces = [("deep_black", manifest["tokens"]["deep_black"]),
                ("charcoal", manifest["tokens"]["charcoal"])]
    out_paths: list[Path] = []
    groups: dict[str, list[Sprite]] = {}
    for s in sprites:
        groups.setdefault(s.group, []).append(s)
    for group, items in groups.items():
        pad = 16
        label_w = 150
        rows: list[Image.Image] = []
        for sprite in items:
            w, h = sprite.size
            cells: list[Image.Image] = []
            for tint in sprite.tint_preview:
                for _, bg_hex in surfaces:
                    bg = hex_rgb(bg_hex)
                    for frame in sprite.frames:
                        cells.append(render(manifest, sprite, frame, CONTACT_SCALE, tint, bg, grid=True))
            real = [render(manifest, sprite, fr, REAL_SCALE, sprite.tint_preview[0],
                           hex_rgb(manifest["tokens"]["deep_black"])) for fr in sprite.frames]
            per_line = max(1, min(len(cells), 1100 // (w * CONTACT_SCALE + 8)))
            lines = [cells[i:i + per_line] for i in range(0, len(cells), per_line)]
            cw = w * CONTACT_SCALE + 8
            ch = h * CONTACT_SCALE + 8
            real_h = h * REAL_SCALE + 8
            block_w = label_w + per_line * cw + pad
            block_h = len(lines) * ch + real_h + 22
            block = Image.new("RGB", (block_w, block_h), (40, 40, 40))
            d = ImageDraw.Draw(block)
            d.text((6, 6), sprite.name, fill=(236, 232, 224), font=font)
            d.text((6, 20), f"{w}x{h} art px", fill=(154, 149, 141), font=font)
            d.text((6, 34), f"{len(sprite.frames)} frame(s)", fill=(154, 149, 141), font=font)
            if len(sprite.frames) > 1:
                d.text((6, 48), "ms " + ",".join(str(fr.ms) for fr in sprite.frames)[:22],
                       fill=(154, 149, 141), font=font)
            for li, line in enumerate(lines):
                for ci, cell in enumerate(line):
                    block.paste(cell, (label_w + ci * cw, 4 + li * ch))
            y0 = 4 + len(lines) * ch + 4
            d.text((6, y0 + 2), "real size (1x)", fill=(154, 149, 141), font=font)
            for i, cell in enumerate(real):
                block.paste(cell, (label_w + i * (w * REAL_SCALE + 8), y0))
            rows.append(block)
        # Flow the blocks left to right, wrapping at SHEET_MAX_W.
        placed: list[tuple[Image.Image, int, int]] = []
        x, y, line_h = 0, 30, 0
        for r in rows:
            if x and x + r.width > SHEET_MAX_W:
                x, y, line_h = 0, y + line_h + 6, 0
            placed.append((r, x, y))
            x += r.width + 6
            line_h = max(line_h, r.height)
        sheet_w = max(px + r.width for r, px, _ in placed)
        sheet_h = y + line_h + 6
        sheet = Image.new("RGB", (sheet_w, sheet_h), (24, 24, 24))
        ImageDraw.Draw(sheet).text((6, 8), f"Psytify pixel sprites / {group} / 4x contact sheet "
                                            "(each frame on deep_black, then on charcoal)",
                                   fill=(236, 232, 224), font=font)
        for r, px, py in placed:
            sheet.paste(r, (px, py))
        out = EXPORT / "contact" / f"{group}@4x.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        sheet.save(out)
        out_paths.append(out)
    return out_paths


def previews(manifest: dict, sprites: list[Sprite]) -> None:
    bg = hex_rgb(manifest["tokens"]["deep_black"])
    for sprite in sprites:
        if len(sprite.frames) < 2:
            continue
        frames = [render(manifest, sprite, fr, CONTACT_SCALE, sprite.tint_preview[-1], bg)
                  for fr in sprite.frames]
        out = EXPORT / "preview" / f"{sprite.name}@4x.gif"
        out.parent.mkdir(parents=True, exist_ok=True)
        frames[0].save(out, save_all=True, append_images=frames[1:], loop=0,
                       duration=[fr.ms for fr in sprite.frames], disposal=2)


def main() -> int:
    manifest, sprites, lint = load()
    for w in lint.warnings:
        print(f"warning: {w}")
    for e in lint.errors:
        print(f"error: {e}")
    if lint.errors:
        print(f"{len(lint.errors)} error(s)")
        return 1
    if "--check" in sys.argv:
        print(f"ok: {len(sprites)} sprites")
        return 0
    meta = [export_png(manifest, s) for s in sprites]
    (EXPORT / "sprites.json").write_text(
        json.dumps({"ap_dp": manifest["ap_dp"], "sprites": meta}, indent=2) + "\n", encoding="utf-8")
    sheets = contact_sheets(manifest, sprites)
    previews(manifest, sprites)
    print(f"ok: {len(sprites)} sprites exported")
    for s in sheets:
        print(f"contact sheet: {s.relative_to(ROOT.parents[2])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
