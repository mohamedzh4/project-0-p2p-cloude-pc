#!/usr/bin/env python3
"""Give the passed-through GPU a screen before the desktop starts (Pro).

Runs as root before the display manager. A passthrough GPU normally has no
monitor plugged in; X then starts on nothing (or on QEMU's emulated VGA) and
the stream shows nothing. This:

1. finds the passthrough GPU (PCI display class, not QEMU's 1234:1111 VGA);
2. writes an X config that pins X to that GPU by bus ID;
3. if no monitor is connected, gives it a virtual one:
   - NVIDIA: ConnectedMonitor + CustomEDID in the X config,
   - AMD / Intel: EDID override + forced connector through DRM debugfs.

A real monitor or an HDMI dummy plug is left alone. Paths can be moved
under DCL_DISPLAY_ROOT for tests.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

ROOT = Path(os.environ.get("DCL_DISPLAY_ROOT", "/"))
EDID = Path(os.environ.get("DCL_DISPLAY_EDID", "/usr/lib/firmware/edid/dcl-gaming.bin"))
XORG_CONF = "etc/X11/xorg.conf.d/20-dcl-gpu.conf"

VENDORS = {"0x10de": "nvidia", "0x1002": "amd", "0x8086": "intel"}
EMULATED = {("0x1234", "0x1111"), ("0x1b36", "0x0100"), ("0x1af4", "0x1050")}  # QEMU VGA, QXL, virtio-gpu


def _read(p: Path) -> str:
    try:
        return p.read_text(encoding="utf-8").strip()
    except OSError:
        return ""


def find_gpus(root: Path = ROOT) -> list[tuple[str, str]]:
    """(PCI address, vendor name) of real display devices, NVIDIA first."""
    found = []
    devices = root / "sys/bus/pci/devices"
    if not devices.is_dir():
        return []
    for dev in sorted(devices.iterdir()):
        if not _read(dev / "class").startswith("0x03"):
            continue
        ids = (_read(dev / "vendor"), _read(dev / "device"))
        if ids in EMULATED or ids[0] not in VENDORS:
            continue
        found.append((dev.name, VENDORS[ids[0]]))
    found.sort(key=lambda g: (g[1] != "nvidia", g[0]))
    return found


def bus_id(address: str) -> str:
    """0000:01:00.0 -> PCI:1:0:0 (X wants decimal)."""
    _, bus, rest = address.split(":")
    dev, fn = rest.split(".")
    return f"PCI:{int(bus, 16)}:{int(dev, 16)}:{int(fn, 16)}"


def connectors(address: str, root: Path = ROOT) -> list[tuple[str, str, str]]:
    """(card, connector, status) for DRM connectors of this GPU."""
    out = []
    drm = root / "sys/class/drm"
    if not drm.is_dir():
        return out
    for entry in sorted(drm.iterdir()):
        name = entry.name
        if "-" not in name or not name.startswith("card"):
            continue
        card, conn = name.split("-", 1)
        device = drm / card / "device"
        try:
            if device.resolve().name != address:
                continue
        except OSError:
            continue
        out.append((card, conn, _read(entry / "status")))
    return out


def xorg_config(address: str, vendor: str, virtual_monitor: bool) -> str:
    driver = {"nvidia": "nvidia", "amd": "amdgpu", "intel": "modesetting"}[vendor]
    lines = [
        "# Written by dcl-display-setup at boot. Pins X to the passthrough GPU.",
        'Section "Device"',
        '    Identifier "dcl-gpu"',
        f'    Driver "{driver}"',
        f'    BusID "{bus_id(address)}"',
    ]
    if vendor == "nvidia":
        lines.append('    Option "AllowEmptyInitialConfiguration" "true"')
        if virtual_monitor:
            lines += [
                '    Option "ConnectedMonitor" "DFP-0"',
                f'    Option "CustomEDID" "DFP-0:{EDID}"',
                '    Option "ModeValidation" "AllowNonEdidModes"',
            ]
    lines += [
        "EndSection",
        'Section "Screen"',
        '    Identifier "dcl-screen"',
        '    Device "dcl-gpu"',
        "    DefaultDepth 24",
        '    SubSection "Display"',
        "        Depth 24",
        '        Modes "1920x1080"',
        "    EndSubSection",
        "EndSection",
        'Section "ServerLayout"',
        '    Identifier "dcl-layout"',
        '    Screen "dcl-screen"',
        "EndSection",
        "",
    ]
    return "\n".join(lines)


def force_virtual_monitor(card: str, connector: str, root: Path = ROOT) -> bool:
    """AMD/Intel: EDID override and force-on through DRM debugfs."""
    minor = card.removeprefix("card")
    base = root / "sys/kernel/debug/dri" / minor / connector
    try:
        (base / "edid_override").write_bytes(EDID.read_bytes())
        (base / "force").write_text("on\n", encoding="utf-8")
    except OSError as e:
        print(f"dcl-display-setup: could not force {card}-{connector}: {e}", file=sys.stderr)
        return False
    return True


def main(root: Path = ROOT) -> int:
    gpus = find_gpus(root)
    if not gpus:
        print("dcl-display-setup: no passthrough GPU; leaving X alone")
        return 0
    address, vendor = gpus[0]
    conns = connectors(address, root)
    has_screen = any(status == "connected" for _, _, status in conns)
    virtual = not has_screen
    if virtual and vendor != "nvidia":
        # The NVIDIA driver takes the EDID from the X config; others need it in DRM.
        if conns:
            card, conn, _ = conns[0]
            virtual = force_virtual_monitor(card, conn, root)
        else:
            print("dcl-display-setup: no DRM connector found for the GPU", file=sys.stderr)
    conf = root / XORG_CONF
    conf.parent.mkdir(parents=True, exist_ok=True)
    conf.write_text(xorg_config(address, vendor, virtual), encoding="utf-8")
    what = "virtual monitor" if virtual else ("its own monitor" if has_screen else "no monitor")
    print(f"dcl-display-setup: X on {vendor} GPU {address} with {what}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
