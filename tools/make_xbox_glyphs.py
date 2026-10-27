#!/usr/bin/env python3
"""make_xbox_glyphs.py - the Xbox button artwork as the game can use it.

The button prompts can show the buttons the way an Xbox pad does. The pictures
come from `xboxglyphs/` (Xbox Buttons (c) 2020 by ArksDigital, CC BY 4.0) and
this script cuts them down to what the game's font can hold: one 16x16 cell per
button, the size of a cell in the retail's 256x128 font sheet.

The cells are written as RGBA (xbox.rgba, switch.rgba), for the fork's own
menus, which draw the font it decoded off the disc. The retail's own prompts
are drawn out of the graphics memory, where the font sheet is 8-bit indexed
with a 256-colour palette that the text already uses most of, so the fork
quantises the artwork against that palette itself when it puts the pictures
into the sheet.

The Switch set is the Xbox set with the four face buttons the other way round:
the Switch's bottom button is B, its right one A, its top X and its left one Y.

Run it from the repository root:

    python3 tools/make_xbox_glyphs.py

It needs Pillow (the same library the other tools here use).
"""

import os
import sys

try:
    from PIL import Image
except ImportError:
    sys.exit("make_xbox_glyphs.py: Pillow is needed (python3 -m pip install pillow)")

# The retail's button cells are 24 wide and 16 tall. The art sits in a 16-wide window at x 2: the
# text after a button glyph starts 18 columns in, so the rest of the cell is overdrawn by it.
CELL_W = 24
CELL_H = 16
ART_X = 2
ART_W = 16
# The sheet is 256x128 with 16x16 cells, so the button cells sit on a 24 pitch.
PITCH = 24
# The band's top row (v = 96 in the sheet) holds the button cells.
BAND_V = 96

# The retail's glyph codes and where each cell sits in the sheet. The order is
# the sheet's: Cross, Square, Triangle, Circle, L1, L2, R1, R2, then the sticks.
CELLS = [
    ("cross", 0),    # 0x10
    ("square", 24),  # 0x13
    ("triangle", 48),  # 0x12
    ("circle", 72),  # 0x11
    ("l1", 96),      # 0x14
    ("l2", 120),     # 0x16
    ("r1", 144),     # 0x15
    ("r2", 168),     # 0x17
    ("rstick", 208),  # 0x18
    ("lstick", 232),  # 0x19
]

# The Xbox picture for each cell, and the Switch one: the Xbox's with the four
# face buttons swapped.
XBOX = {
    "cross": "a.png",
    "square": "x.png",
    "triangle": "y.png",
    "circle": "b.png",
    "l1": "lb.png",
    "l2": "lt.png",
    "r1": "rb.png",
    "r2": "rt.png",
    "rstick": "rs.png",
    "lstick": "ls.png",
}
SWITCH = dict(XBOX)
SWITCH["cross"] = XBOX["circle"]      # the Switch's bottom button is B
SWITCH["circle"] = XBOX["cross"]      # its right one is A
SWITCH["triangle"] = XBOX["square"]   # its top one is X
SWITCH["square"] = XBOX["triangle"]   # its left one is Y

def load_cell(src_dir, name):
    """The picture as a 24x16 RGBA cell, scaled down from the original.

    The picture keeps its own proportions: it is scaled to the cell's height and centred in the
    16-wide window the retail's own art is in, so a narrow trigger stays narrow and a disc stays
    round. A picture wider than the cell (a bumper) fills it.
    """
    path = os.path.join(src_dir, name)
    if not os.path.isfile(path):
        sys.exit(f"make_xbox_glyphs.py: {path} is missing")
    im = Image.open(path).convert("RGBA")
    # The artwork is a shape with a transparent background. Scaling it in straight alpha keeps the
    # edge from picking up the black of the background.
    w = min(CELL_W, max(1, round(im.width * (CELL_H / im.height))))
    small = im.resize((w, CELL_H), Image.LANCZOS)
    cell = Image.new("RGBA", (CELL_W, CELL_H), (0, 0, 0, 0))
    cell.alpha_composite(small, (max(0, ART_X + (ART_W - w) // 2), 0))
    raw = cell.tobytes()
    return [tuple(raw[i : i + 4]) for i in range(0, len(raw), 4)]


def write_rgba(path, cells):
    """The cells as RGBA, row by row, cell by cell."""
    with open(path, "wb") as f:
        for cell in cells:
            for px in cell:
                f.write(bytes(px))


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    src = os.path.join(root, "xboxglyphs")
    out = os.path.join(root, "rac-rs", "assets", "glyphs")
    os.makedirs(out, exist_ok=True)

    for name, table in (("xbox", XBOX), ("switch", SWITCH)):
        cells = [load_cell(src, table[key]) for key, _ in CELLS]
        write_rgba(os.path.join(out, f"{name}.rgba"), cells)
        print(f"  {name}.rgba: {len(cells)} cells of {CELL_W}x{CELL_H}")


if __name__ == "__main__":
    main()
