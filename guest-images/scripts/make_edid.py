#!/usr/bin/env python3
"""Write a small EDID 1.3 block for a virtual gaming monitor.

A GPU passed through to a VM usually has no screen plugged in, and with no
screen there is nothing to capture (docs/sunshine-moonlight-analysis.md 1.7).
dcl-display-setup gives the GPU this EDID so X runs a real mode on it.

Modes (detailed timings, CEA-861 / CVT reduced blanking):
  1920x1080 @ 60 (preferred), 1920x1080 @ 120, 2560x1440 @ 60

    make_edid.py OUT.bin
"""

from __future__ import annotations

import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class Timing:
    pclk_khz: int
    h_active: int
    h_front: int
    h_sync: int
    h_back: int
    v_active: int
    v_front: int
    v_sync: int
    v_back: int

    @property
    def refresh_hz(self) -> float:
        h_total = self.h_active + self.h_front + self.h_sync + self.h_back
        v_total = self.v_active + self.v_front + self.v_sync + self.v_back
        return self.pclk_khz * 1000 / (h_total * v_total)


MODES = (
    Timing(148_500, 1920, 88, 44, 148, 1080, 4, 5, 36),  # CEA VIC 16
    Timing(297_000, 1920, 88, 44, 148, 1080, 4, 5, 36),  # CEA VIC 63
    Timing(241_500, 2560, 48, 32, 80, 1440, 3, 5, 33),  # CVT-RB
)
SIZE_MM = (600, 340)


def _dtd(t: Timing) -> bytes:
    h_blank = t.h_front + t.h_sync + t.h_back
    v_blank = t.v_front + t.v_sync + t.v_back
    pclk = t.pclk_khz // 10
    w, h = SIZE_MM
    return bytes(
        [
            pclk & 0xFF,
            pclk >> 8,
            t.h_active & 0xFF,
            h_blank & 0xFF,
            ((t.h_active >> 8) << 4) | (h_blank >> 8),
            t.v_active & 0xFF,
            v_blank & 0xFF,
            ((t.v_active >> 8) << 4) | (v_blank >> 8),
            t.h_front & 0xFF,
            t.h_sync & 0xFF,
            ((t.v_front & 0xF) << 4) | (t.v_sync & 0xF),
            ((t.h_front >> 8) << 6) | ((t.h_sync >> 8) << 4) | ((t.v_front >> 4) << 2) | (t.v_sync >> 4),
            w & 0xFF,
            h & 0xFF,
            ((w >> 8) << 4) | (h >> 8),
            0,
            0,
            0x1E,  # digital separate sync, +hsync +vsync
        ]
    )


def _name(text: str) -> bytes:
    body = text.encode("ascii")[:13]
    if len(body) < 13:
        body += b"\n" + b" " * (12 - len(body))
    return bytes([0, 0, 0, 0xFC, 0]) + body


def edid() -> bytes:
    maker = (ord("D") - 64) << 10 | (ord("C") - 64) << 5 | (ord("L") - 64)
    out = bytearray(b"\x00\xff\xff\xff\xff\xff\xff\x00")
    out += maker.to_bytes(2, "big")
    out += (0x0001).to_bytes(2, "little")  # product
    out += (1).to_bytes(4, "little")  # serial
    out += bytes([1, 2026 - 1990, 1, 3])  # week, year, EDID 1.3
    out += bytes([0x80, SIZE_MM[0] // 10, SIZE_MM[1] // 10, 120, 0x0A])  # digital, cm, gamma 2.2, RGB + preferred
    out += bytes([0xEE, 0x91, 0xA3, 0x54, 0x4C, 0x99, 0x26, 0x0F, 0x50, 0x54])  # sRGB-like chromaticity
    out += bytes([0x21, 0x08, 0x00])  # 640x480, 800x600, 1024x768 @ 60
    out += b"\x01\x01" * 8  # no standard timings
    for t in MODES:
        out += _dtd(t)
    out += _name("DCL Stream")
    out += b"\x00"  # no extension blocks
    out.append((-sum(out)) & 0xFF)
    assert len(out) == 128
    return bytes(out)


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    with open(argv[0], "wb") as f:
        f.write(edid())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
