#!/usr/bin/env python3
"""Recompresse un PNG produit par `Screenshot::to_png` (blocs non compressés).

L'encodeur intégré à rwdwm n'a aucune dépendance et ne compresse pas ; ce
script réécrit le flux IDAT avec zlib (niveau 9) sans toucher aux pixels.

    python3 tools/optimize_png.py docs/capture-headless.png
"""
import struct
import sys
import zlib


def chunks(data):
    pos = 8
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        yield kind, data[pos + 8 : pos + 8 + length]
        pos += 12 + length


def chunk(kind, body):
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))


def main(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "pas un PNG"
    out = [data[:8]]
    idat = b""
    for kind, body in chunks(data):
        if kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            out.append(chunk(b"IDAT", zlib.compress(zlib.decompress(idat), 9)))
            out.append(chunk(kind, body))
        else:
            out.append(chunk(kind, body))
    result = b"".join(out)
    open(path, "wb").write(result)
    print(f"{path} : {len(data)} -> {len(result)} octets")


if __name__ == "__main__":
    main(sys.argv[1])
