#!/usr/bin/env python3
"""Convertit une police BDF à chasse fixe 8x16 en table binaire compacte.

Usage : tools/gen_font.py spleen-8x16.bdf crates/rwdwm-core/assets/spleen-8x16.bin

Format de sortie (petit-boutiste) :
    u32  nombre de glyphes N
    N x { u32 point de code ; 16 octets : une ligne par octet, bit 7 = pixel de gauche }
Les entrées sont triées par point de code (recherche dichotomique à l'exécution).
"""
import struct
import sys

W, H = 8, 16


def parse(path):
    glyphs = {}
    ascent = 12
    cp = bbx = rows = None
    in_bitmap = False
    for line in open(path, encoding="latin-1"):
        parts = line.split()
        if not parts:
            continue
        key = parts[0]
        if key == "FONT_ASCENT":
            ascent = int(parts[1])
        elif key == "ENCODING":
            cp = int(parts[1])
        elif key == "BBX":
            bbx = tuple(int(v) for v in parts[1:5])
        elif key == "BITMAP":
            in_bitmap, rows = True, []
        elif key == "ENDCHAR":
            in_bitmap = False
            if cp is not None and cp >= 0 and bbx is not None:
                bw, bh, xoff, yoff = bbx
                cell = [0] * H
                top = ascent - (yoff + bh)
                for i, value in enumerate(rows):
                    y = top + i
                    if 0 <= y < H:
                        # Les lignes BDF sont alignées à gauche sur des octets entiers.
                        bits = value >> (max(0, (bw + 7) // 8 * 8 - 8))
                        cell[y] = (bits >> xoff) & 0xFF if xoff >= 0 else (bits << -xoff) & 0xFF
                glyphs[cp] = bytes(cell)
            cp = bbx = rows = None
        elif in_bitmap:
            rows.append(int(key, 16))
    return glyphs


def main():
    src, dst = sys.argv[1], sys.argv[2]
    glyphs = parse(src)
    with open(dst, "wb") as out:
        out.write(struct.pack("<I", len(glyphs)))
        for cp in sorted(glyphs):
            out.write(struct.pack("<I", cp))
            out.write(glyphs[cp])
    print(f"{len(glyphs)} glyphes -> {dst}")


if __name__ == "__main__":
    main()
