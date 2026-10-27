# Xbox button artwork

The button prompts in the game can show the buttons the way an Xbox pad does
instead of the retail's own PlayStation glyphs. These are the pictures they use.

| the retail's glyph | code | the picture | the Xbox's name |
|---|---|---|---|
| Cross | `0x10` | `a.png` | A |
| Circle | `0x11` | `b.png` | B |
| Triangle | `0x12` | `y.png` | Y |
| Square | `0x13` | `x.png` | X |
| L1 | `0x14` | `lb.png` | LB |
| R1 | `0x15` | `rb.png` | RB |
| L2 | `0x16` | `lt.png` | LT |
| R2 | `0x17` | `rt.png` | RT |
| right stick | `0x18` | `rs.png` | R3 |
| left stick | `0x19` | `ls.png` | L3 |

The pictures keep their own shape, and the game scales each one to the size of
the prompt's button cell (24 by 16 pixels, the size of the retail's own button
cells) and puts it in the font's cells, so a picture can be replaced with
anything of the same kind. `tools/make_xbox_glyphs.py` makes the small copies
the game reads (`rac-rs/assets/glyphs/`).

The rest of the set (the D-pad, the stick directions and the Menu, View, Back
and Start buttons) has no cell in the retail's font, so it is not in this
folder.

## Licence

The pictures are the fork's own work, drawn for it by
[DeSynkro](https://github.com/DeSynkro), and are under the repository's own
licence (GPL-3.0-or-later, see [LICENSE](../LICENSE)). The Xbox name and the
buttons themselves are Microsoft's trademarks: these are the fork's own
drawings of them, not Microsoft's artwork.
