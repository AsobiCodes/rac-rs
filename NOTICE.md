# Notices and credits

rac-rs is licensed under GPL-3.0-or-later (see LICENSE). It builds on the published knowledge and work listed below.

## rac1-decomp

Several level-file readers began as Rust ports of the Python extraction tools in rac1-decomp: terrain, ties, shrubs, sky, level, table-of-contents and collision. The rac1-decomp tools are released under this licence (its LICENSE file, quoted in full; `src/libgcc/` refers to that repository):

```
MIT License

Copyright (c) 2026 Kryštof "Lynder063" Malinda

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

The files in `src/libgcc/` that
come from GCC keep their original license (GPL v2 with the libgcc linking
exception), as stated in each file's header.

*Ratchet & Clank* is a trademark of Sony Interactive Entertainment. This
project is not affiliated with or endorsed by Sony or Insomniac Games.
```

## Wrench

The Wrench modding tools and their documentation (the Ratchet & Clank modding community) describe the game's file formats. rac1-decomp's readers follow them, and so do ours.

## PCSX2

The sound (SPU2), graphics (GS/VU) and CPU behaviour in this project was written against the PCSX2 emulator as a hardware-behaviour reference. PCSX2 is licensed under GPL-3.0-or-later; rac-rs uses the same licence.

## Hardware documentation

psx-spx (Martin Korth / nocash) documents the SPU ADPCM and Gaussian interpolation tables. These are properties of the PlayStation hardware.

## Rust libraries

rac-rs uses crates from crates.io, including wgpu, winit, cpal, rayon, bytemuck and pollster, under the MIT, Apache-2.0, Zlib, BSD and ISC licences. Cargo downloads them at build time. Their licence texts ship with each crate.
