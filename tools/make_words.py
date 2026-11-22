#!/usr/bin/env python3
"""Build src/words.bin: sorted, unique, uppercase, five bytes per word."""

from pathlib import Path

root = Path(__file__).resolve().parent.parent
words = set()
for line in (root / "src/words.txt").read_text().splitlines():
    word = line.strip().upper()
    if len(word) == 5 and word.isascii() and word.isalpha():
        words.add(word)
data = b"".join(word.encode("ascii") for word in sorted(words))
if len(data) % 5:
    raise SystemExit("dictionary is not a whole number of five-byte records")
(root / "src/words.bin").write_bytes(data)
print(f"words.bin: {len(words)} words, {len(data)} bytes")
