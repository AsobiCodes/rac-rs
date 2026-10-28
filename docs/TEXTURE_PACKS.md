# Texture packs in RAC-RS

RAC-RS loads the same texture packs as the PCSX2 emulator, unchanged. A pack is a set of pictures (PNG or DDS files) whose file names say which game texture they replace. No game file is touched; the pack is read while you play. Script mods are a different thing: see [MODDING.md](MODDING.md).

Everything here is in v0.4.0 unless marked "(since v0.4.1)".

## For players: add a pack and switch it on

**With the RAC-RS Launcher** (Mods page, "Texture packs"):

- Add a pack as a **folder or .zip** (also by drag and drop), or **import it from your PCSX2 texture folder** (`Documents\PCSX2\textures\<SERIAL>\replacements`, or a `<SERIAL>.zip` there). 7z and rar files must be unpacked first.
- Big packs (6 GB and more) are copied in the background with a progress bar and a Cancel button.
- Each pack has an on/off switch and priority arrows (the pack higher in the list wins when two replace the same texture). A master switch "Load texture packs" turns all packs off without forgetting your choices. A pack made for the other disc region is marked.
- The launcher copies the pack into the game's `mods` folder and writes a small `mod.json` for it. Changes take effect at the next start of the game.

**In the game**: title screen Options > Mods, and Pause > Options > Mods (shown while a pack is on). Both have the "Texture packs" master switch and one On/Off row per installed pack. A change applies at the **next planet** you load.

**By hand**: put the pack in `<your RAC-RS folder>\mods\<name>\textures\` (the pictures can be in sub-folders), optionally with a `mod.json`, and add `<name>` as a line in `mods\active.txt` (one id per line, first line = highest priority, `#` starts a comment). With no `mod.json` the folder is still treated as a texture pack. Since v0.4.1 the layout may be any PCSX2 one (see below) and the pack shows up in Options > Mods by itself.

`mod.json` (all fields optional):

```json
{
  "format": 1, "id": "my-hd-pack", "name": "My HD pack", "author": "You", "version": "1.0",
  "description": "What it does.", "game": "rac1", "kind": "textures",
  "serials": ["SCES-50916"], "source": "pcsx2", "mip_names": "both"
}
```

`kind` must be `textures` and `game` must be `rac1` or the folder is skipped. `serials` is information only: a pack is never refused because of the disc (see "Other disc region"). `mip_names` is `single` (every mip level has its own file name), `chain` (the name of a mip is the hash of the chain from that level up, as current PCSX2 with hardware mipmapping writes it) or `both` (the default; `single` is tried first).

## What a pack can replace

**v0.4.0**: the world textures (ground, walls, objects, characters) and the extra textures outside the world tables: effects, shadows, glows, water sheets and the ocean's animation frames, particles, chrome and glass (metal and glass surfaces), the sky, the ship and warp textures, and the colour of grass (taken from the HD ground texture). The HUD, menus, text and title screen were **not** replaced in v0.4.0, although the v0.4.0 notes listed them.

**Since v0.4.1**: also the HUD, the pause and Options menus, the vendor, the text (font) and the title screen (logo, "Press Start", menu panels). Where the processor draws the HUD (integrated graphics) a pack's colours and art show at the PS2's picture size, not sharper. BC7 DDS packs (what texconv and similar tools write) load; before, most BC7 files were skipped. Very big PNG packs (several GB) keep working: a texture that does not fit the memory budget is stored compressed (BC3) instead of being dropped. A damaged picture in a pack is skipped (one log line) instead of crashing.

## Which files work

- **Names**: exactly PCSX2's replacement names, case-insensitive extension `.png` or `.dds`:
  `<texture hash>[-<palette hash>][-r<W>x<H>]-<bits>` where the hashes are hexadecimal, `<bits>` is 8 hex digits and `r<W>x<H>` appears only for region textures. Example: `1a2b3c4d5e6f7081-00000007.png`. Files with a `-mipN` suffix are not loaded (PCSX2 never loads them either; put the mips inside a DDS).
- **PNG**: 8-bit RGB and RGBA, plus grey, grey with alpha, palette and 16-bit; not interlaced. Alpha follows the PS2 scale (see "Known limits").
- **DDS**: BC1, BC2, BC3, uncompressed 24 and 32 bit, and BC7 (since v0.4.1). Mip levels stored in the file are kept.
- **Folder layouts**: v0.4.0 reads `mods\<id>\textures\` (sub-folders allowed). Since v0.4.1 a folder copied in as it is also works when it holds `replacements\` folders, a `<SERIAL>\replacements\` folder, a whole PCSX2 `textures` folder, or the pictures loose in the pack folder: the game looks for files with PCSX2 names anywhere below the folder. Loose pictures directly in `mods\` (not in a folder) are not a pack; put them in a folder of their own.
- **Other disc region**: the names are content hashes, so a pack made for the USA disc matches most world textures of the PAL disc and the other way round. The game also tries the matching PAL/USA twin name when a pack lacks a name. Menus, HUD and text from the other region may stay original.

## For modders: make a pack

1. **Dump**: in PCSX2 turn on texture dumping (Settings > Graphics > Texture Replacement; menu names vary a little between PCSX2 versions) and play. PCSX2 writes the textures it draws into `textures\<SERIAL>\dumps`. RAC-RS produces the same names, so a pack made this way fits.
2. **Replace**: copy the pictures you want to change into `textures\<SERIAL>\replacements` and edit them. **Keep the file names exactly.** Keep the picture's proportions; a bigger picture (HD) is fine and is scaled to the texture. Delete the files you did not change: only files that exist replace anything.
3. **Check in PCSX2**: turn on "Load Textures" and look. A pack that works in PCSX2 works here.
4. **Pack it**: put the `replacements` folder (or the folder with the pictures) into a folder named for your pack, with an optional `mod.json`, and zip it. Players add it in the launcher or unzip it into `mods`.
5. **Share only your own work.** Dumps are the game's pictures: do not upload dumped, unchanged or only upscaled files. Ship pictures you painted or made yourself.

Check the result in the run log (`player\log.txt`): lines starting with `texpack:` say which packs loaded and what was skipped (for example a folder without textures, or a pack of another game).

## Known limits

- **Changes apply at the next planet**, not at once.
- **Memory**: the world pictures of a level are held in memory; the budget is 1536 MB by default (`RAC_TEXPACK_MB` raises or lowers it). Over the budget, a texture keeps the original picture in v0.4.0; since v0.4.1 it is stored compressed (BC3) instead.
- **No `-mipN` files**, no animated textures other than the frames the game already has, no model or sound replacement.
- **Alpha**: PCSX2 packs use the PS2 alpha scale (128 = opaque); RAC-RS converts it.
- **HUD pictures on the processor path** (since v0.4.1): shown at the PS2's size; `RAC_TEXPACK_HUD=0` keeps the HUD and menus original while a pack is on.
- 7z and rar packs must be unpacked before the launcher can add them.
- The game does not dump textures itself; use PCSX2 for the dump step.
