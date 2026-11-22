#!/usr/bin/env python3
"""Draw the Gri Wordle app icon and write it as a multi-size Windows .ico.

The mark is a 2x3 Wordle board spelling WORDLE in green, yellow and gray tiles. It is drawn as
pixels and box-filtered down, so the build has no font or image-library dependency.
"""

from __future__ import annotations

import argparse
import struct
from pathlib import Path

SIZE = 256
TILE = 70
GAP = 8
GRID_LEFT = (SIZE - (2 * TILE + GAP)) // 2
GRID_TOP = (SIZE - (3 * TILE + 2 * GAP)) // 2
RADIUS = 9
COLORS = [
    (69, 112, 82),    # green
    (181, 154, 74),   # yellow
    (112, 112, 112),  # gray
]
INK = [
    (255, 255, 255),
    (24, 24, 24),
    (255, 255, 255),
]
GLYPHS = {
    "W": ("10001", "10001", "10001", "10101", "10101", "11011", "10001"),
    "O": ("01110", "10001", "10001", "10001", "10001", "10001", "01110"),
    "R": ("11110", "10001", "10001", "11110", "10100", "10010", "10001"),
    "D": ("11110", "10001", "10001", "10001", "10001", "10001", "11110"),
    "L": ("10000", "10000", "10000", "10000", "10000", "10000", "11111"),
    "E": ("11111", "10000", "10000", "11110", "10000", "10000", "11111"),
}


def rounded_tile(image: list[list[tuple[int, int, int]]], x0: int, y0: int, color: tuple[int, int, int]) -> None:
    for y in range(TILE):
        for x in range(TILE):
            if RADIUS <= x < TILE - RADIUS or RADIUS <= y < TILE - RADIUS:
                inside = True
            else:
                cx = RADIUS if x < RADIUS else TILE - 1 - RADIUS
                cy = RADIUS if y < RADIUS else TILE - 1 - RADIUS
                inside = (x - cx) ** 2 + (y - cy) ** 2 <= RADIUS**2
            if inside:
                image[y0 + y][x0 + x] = color


def draw_letter(image: list[list[tuple[int, int, int]]], x0: int, y0: int, letter: str, ink: tuple[int, int, int]) -> None:
    rows = GLYPHS[letter]
    cell = 9
    block = 8
    width = len(rows[0]) * cell
    height = len(rows) * cell
    left = x0 + (TILE - width) // 2
    top = y0 + (TILE - height) // 2
    for y, row in enumerate(rows):
        for x, bit in enumerate(row):
            if bit != "1":
                continue
            for dy in range(block):
                for dx in range(block):
                    image[top + y * cell + dy][left + x * cell + dx] = ink


def render(letters: str) -> list[list[tuple[int, int, int]]]:
    if len(letters) != 6 or any(c not in GLYPHS for c in letters):
        raise SystemExit("the icon needs exactly six letters from WORDLE's alphabet")
    image = [[(255, 255, 255)] * SIZE for _ in range(SIZE)]
    for i, letter in enumerate(letters):
        col, row = i % 2, i // 2
        x0 = GRID_LEFT + col * (TILE + GAP)
        y0 = GRID_TOP + row * (TILE + GAP)
        style = i % len(COLORS)
        rounded_tile(image, x0, y0, COLORS[style])
        draw_letter(image, x0, y0, letter, INK[style])
    return image


def downscale(image: list[list[tuple[int, int, int]]], size: int) -> list[list[tuple[int, int, int]]]:
    scale = SIZE // size
    out: list[list[tuple[int, int, int]]] = []
    for y in range(size):
        row = []
        for x in range(size):
            r = g = b = 0
            for sy in range(scale):
                for sx in range(scale):
                    pr, pg, pb = image[y * scale + sy][x * scale + sx]
                    r += pr
                    g += pg
                    b += pb
            count = scale * scale
            row.append((r // count, g // count, b // count))
        out.append(row)
    return out


def ico_entry(image: list[list[tuple[int, int, int]]], size: int) -> bytes:
    pixels = bytearray()
    for y in range(size - 1, -1, -1):
        for x in range(size):
            r, g, b = image[y][x]
            pixels.extend((b, g, r, 255))
    mask_stride = ((size + 31) // 32) * 4
    mask = bytes(mask_stride * size)
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, size * size * 4, 0, 0, 0, 0)
    return header + pixels + mask


def build(letters: str) -> bytes:
    master = render(letters)
    entries = []
    payload_offset = 6 + 16 * 5
    for size in (16, 32, 48, 64, 128):
        data = ico_entry(downscale(master, size), size)
        entries.append((size, len(data), payload_offset))
        payload_offset += len(data)
    out = bytearray(struct.pack("<HHH", 0, 1, len(entries)))
    for size, length, offset in entries:
        out.extend(struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, length, offset))
    for size in (16, 32, 48, 64, 128):
        out.extend(ico_entry(downscale(master, size), size))
    return bytes(out)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--letters", default="WORDLE")
    parser.add_argument("--out", default="icon.ico")
    args = parser.parse_args()
    data = build(args.letters.upper())
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(data)
    print(f"{out}: {len(data)} bytes, sizes 16 32 48 64 128")


if __name__ == "__main__":
    main()
