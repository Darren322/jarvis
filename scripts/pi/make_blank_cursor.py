#!/usr/bin/env python3
"""Make a fully transparent Xcursor theme (default: ~/.local/share/icons/jarvis-blank).

labwc has no setting that hides the pointer for good (its HideCursor action only
hides it until the next pointer event), so the kiosk uses a theme whose every cursor
is one transparent pixel. Every name in Adwaita's cursors directory becomes a link to
that one file. Select it with XCURSOR_THEME=jarvis-blank (scripts/pi/jarvis-kiosk.sh
for Chromium, ~/.config/labwc/environment for labwc itself). Standard library only.
"""
import os
import struct
import sys

SOURCE_NAMES_DIR = "/usr/share/icons/Adwaita/cursors"
THEME_NAME = "jarvis-blank"
IMAGE_CHUNK_TYPE = 0xFFFD0002


def blank_cursor_file():
    pixel = struct.pack("<I", 0)  # one fully transparent ARGB pixel
    chunk = struct.pack("<IIIIIIIII", 36, IMAGE_CHUNK_TYPE, 1, 1, 1, 1, 0, 0, 0) + pixel
    header = struct.pack("<4sIII", b"Xcur", 16, 0x10000, 1)
    toc = struct.pack("<III", IMAGE_CHUNK_TYPE, 1, 16 + 12)
    return header + toc + chunk


def main():
    root = sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/.local/share/icons")
    cursors = os.path.join(root, THEME_NAME, "cursors")
    os.makedirs(cursors, exist_ok=True)
    blank = os.path.join(cursors, "blank")
    with open(blank, "wb") as f:
        f.write(blank_cursor_file())
    with open(os.path.join(root, THEME_NAME, "index.theme"), "w") as f:
        f.write(f"[Icon Theme]\nName={THEME_NAME}\nComment=Invisible pointer for the Jarvis kiosk\n")
    count = 0
    for name in sorted(os.listdir(SOURCE_NAMES_DIR)):
        link = os.path.join(cursors, name)
        if os.path.lexists(link):
            os.remove(link)
        os.symlink("blank", link)
        count += 1
    print(f"{THEME_NAME}: {count} cursor names in {cursors}")


if __name__ == "__main__":
    main()
