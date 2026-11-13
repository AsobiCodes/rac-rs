# External localization packs (proposal, API 1)

This interface separates translations from the RAC-RS executable. The engine ships no community translations, fonts or disc resources. It does not edit the ISO and does not execute a pack's Python or native preparation tools. Changes to a pack take effect after restarting the game.

## Install and select

A pack is a folder inside the existing `mods` directory. Keep the usual mod manifest (`mod.json` or `omni-mod.json`), texture layout and `active.txt` activation. Add `localization.json`. The first enabled supported localization in mod priority order wins; other mods keep their priority and activation. Master switches are respected. The Language page lists installed packs; selecting a pack disables the other language providers for the next launch. Selecting an original language disables providers. A restart notice is shown because decoded resources and GPU textures are cached.

A minimal data-only example is in `examples/localization-demo`. It contains newly written example text, not material from a game disc. No Lua hook is necessary to load localization data.

## localization.json

```json
{
  "format": 1,
  "language": "example-Latn",
  "name": "Example language",
  "base": "en",
  "strings": "strings.json",
  "prepared_cache": false,
  "font_assets": "translation_assets.bin",
  "movie_subtitles": "movie-subtitles",
  "overlay_glyphs": "overlay-glyphs.json",
  "force_subtitles": false,
  "sharp_small_ui": false,
  "serials": ["SCES-50916"],
  "thousands_separator": ","
}
```

`format` is the interface major version, not a RAC-RS release number. Unknown versions are ignored with a diagnostic. Extra fields are allowed for future optional extensions. Language identifiers are descriptive labels, not new PS2 language indices. `base` selects one of `en`, `fr`, `de`, `es`, `it` for resources not replaced by the pack. Non-ASCII names are supported. Optional `serials` restricts the provider to matching disc serials; unsupported discs disable all localization hooks. Files are relative to the pack; absolute paths, parent traversal, alternate Windows prefixes and symlink escapes are rejected.

`prepared_cache: true` reads `overlay.json` and `ui.json` from `<player>/localization-cache/<mod-id>` instead. An external installer can build private replacement resources from the owner's own matching disc. Do not redistribute that cache. Missing or malformed private data disables the provider and falls back to original disc/text/font behavior with a diagnostic. The host never runs the installer itself. A pack can contain its own source translation JSON and prepare any native resource format offline: the engine does not prescribe a translation editor or compiler.

## Modern GUI and achievements

`ui.json` is an object mapping original display strings to UTF-8 translations. Lookups happen before measuring/wrapping and, for dynamic messages, before filling `{}` parameters. A missing entry displays the original string. Keys are display text in the pack's base language, independent of Rust paths and addresses; when upstream introduces or rewords text it needs a new translation, not new integration code.

For existing catalogs with translation IDs, optional `ui-extra.json` maps display strings to IDs in the `strings` file's `strings` object. Translated `{}` placeholders and control tokens must be retained. Pixel-overlay achievement text uses an optional `overlay-glyphs.json`, e.g. `{"Á":{"base":"A","mark":[2,4]}}`: an ASCII base and two 5-bit rows above it. This is a small legacy overlay font, not a general Unicode shaping system. The modern GUI retains its existing vector-font renderer.

Optional `count_forms` provides explicit count-dependent shop nouns, e.g. `{"bolts":{"1":"unit","other":"units"}}`. Exact decimal keys take precedence over `other`. This deliberately does not introduce a new general-purpose pluralization library. `thousands_separator` is a string, allowing a non-breaking space or an empty separator.

## Retail text and images: private sector overlays

`overlay.json` has `format: 1` and a `segments` array of `{ "offset": 2048, "length": 2048, "file": "replacement.bin" }`. Offsets and lengths are bytes, sector aligned. Segments must not overlap. Individual files are limited to 64 MiB, total resident data to 256 MiB. Files must match the declared length. Read filters cover both asset reads and the emulated game's disc reads. The original disc stays read-only; original I/O remains unchanged with no active localization.

Compile text banks, scene subtitles and embedded image plates offline, preserving their original allocation limits and PS2 control bytes. Different disc editions need different overlays; the compiler/installer must validate the owner's disc. This interface does not make PAL offsets portable to NTSC. Texture replacements continue to use the existing PCSX2 hash naming. `images-hd.json` marks the existing high-resolution plate convention; logical positions remain the original ones while the texture resolution may increase.

## Fonts

Reuse the normal texture-pack pipeline for the three HD font pages. `translation_assets.bin` is the existing bounded `RCLA0001` format:

- magic (8 bytes), character count (u32 LE, at most 40), then Unicode scalar (u32 LE) + game code (u8, 0x80..0xa7);
- metric-table count (u32 LE, at most 3); each entry has size (u32 LE, exactly 928), original bytes and replacement bytes;
- texture count (u32 LE, at most 1024); each entry has original index hash and RGBA hash (u64 LE each), width and height (u32 LE), edit count (u32 LE), then edits: pixel index (u32 LE), palette index (u8), RGBA (4 bytes).

Supported small atlases are 256x128 and 512x64. The original table/texture signatures guard replacements. Metric discovery is finite, spread over bounded chunks at resource loads; it never restarts on a periodic timer and does not scan RAM for translation strings. Per-font composed accents and controller icons retain separate slots. `sharp_small_ui` opts into the 256-pixel intermediate-target mirror used by classic Help and Moves panels so HD glyphs survive the intermediate render. It does not change a pack's PNG pixels.

This API extends the existing Latin bitmap renderer; it does not promise arbitrary CJK coverage or RTL shaping. New scripts may need a separate renderer change.

## Movie subtitles

The existing `RCSUB001` renderer is reused. `movie-subtitles/map.json` maps the decimal start LBA to a `.bin` filename. Both PAL 50 Hz and 60 Hz movie LBAs can point to the same track. No video is re-encoded.

A track starts with `RCSUB001`, then repeats start_ms, end_ms, compressed_length (u32 LE each) and a zlib payload of premultiplied RGBA at 512x416. Cue intervals must be ordered, non-overlapping and non-empty. Track files are limited to 8 MiB. Decoding is limited to one complete 512x416 RGBA plane. ASS/libass is used by the external preparation/calibration tool, not linked into the game. The GPU overlays cues on the same presentation clock as movie frames, scaling within the movie rectangle. Brightness/contrast and the existing v0.5.1 movie lifecycle remain intact.

Providing film tracks explicitly enables them independently of the retail subtitle switch. `force_subtitles` also enables retail dialogue subtitles for the active PAL localization. Player-calibrated track sets can be selected with the existing `movie-subtitles-custom/current.txt` 32-hex-character directory convention. These files belong to the external pack, not this repository.

## Compatibility and publication

The code targets upstream 0.5.1. Installers can query `rac-rs localization-api`; it prints `{"localization":1}` and exits before updater, disc, GPU, mods or player initialization. Once this interface is merged and maintained upstream, packs targeting API 1 do not need to ship an EXE or replay Rust patches for each release. This cannot guarantee compatibility with arbitrary future engine or disc changes. Breaking protocol changes need a new `format` major version; preserve API 1 or diagnose unsupported packs rather than interpreting them as a newer format.

Do not commit extracted disc text, original atlases, modified disc resources, movie frames, compiled games, or privately licensed fonts. Ship translation sources and your own permitted artwork separately; generate private disc overlays on the user's machine. Respect third-party font licenses independently of this source code's GPL license.
