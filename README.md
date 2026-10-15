# rac-rs

An unofficial PC port of **Ratchet & Clank** (PlayStation 2, 2002), written in Rust. It runs the full game, start to credits, from **your own disc**.

> **No game data is included.** This repository contains no game code, graphics, sound, text or other game files.
> The setup reads your own copy of the disc and creates everything it needs on your PC. You need to own the game.

Project site: https://asobicodes.dev

## Status

- The whole game is playable from power-on to the end credits, including New Game+.
- Gameplay follows the original PS2 game one to one. On top of that, the PC version has widescreen, keyboard and mouse controls, gamepad support, and an optional modern camera (mouse look and strafing).
- Windows only for now.
- Known issues are listed under [Known issues](#known-issues) below.

## How it works

The original game code runs on a PS2 CPU core written in Rust. When you run setup, it translates the code from your disc into native Rust ("static recompilation"). Rewritten Rust replaces the original functions one at a time. Graphics, sound, movies, input and saving are native PC code.

## What you need

- **The game disc:** Ratchet & Clank, **PAL / Europe, v1.00** (serial SCES-50916). Make an `.iso` image of **your own** disc. Other versions (US, Japan, Platinum re-releases) are not supported yet. Setup checks the image.
- **Windows 10 or 11, 64-bit**, with about **5 GB of free disk space** (the finished install takes about 3.5 GB) and an internet connection for the first setup.

That's all. Setup downloads anything else it needs into its own folder.

## Setup

1. Click the green **Code** button above, choose **Download ZIP**, and unzip it anywhere.
2. Double-click **Setup.cmd**. A window opens (no black console window).
3. Click **Browse** and pick your `.iso` file. The window checks it and shows a green **Disc OK**. Then click **Install**.
4. Wait. The window shows the steps and a progress bar (about 15 minutes; you can keep using your PC). When it says **Ready to play**, click **Play**.

Next time, double-click the **Ratchet & Clank (rac-rs)** shortcut on your desktop, or open Setup.cmd and click **Play**. The same window also has **Repair / rebuild** if something breaks. If anything goes wrong the window says which step failed and has a **Copy details** button. For troubleshooting, `Setup.cmd -Console` runs setup in a console window instead.

Setup does the rest by itself:

- **Tools:** it downloads its own private copy of Rust, Python and the GNU assembler into the `toolchain` folder, using the official free packages (fixed versions, each download checked). Nothing is installed on your PC: no admin rights, no Windows prompts, no changes to your system, and any Rust or Python you already have is left alone. This download is about 220 MB and runs only once; the folder takes about 1.2 GB on disk.
- **Disc check:** it checks your disc image.
- **Build:** it generates the game code from your disc and builds the game. This takes about 12 minutes on a 24-thread CPU, or roughly half an hour on a 4-core CPU.
- **Shortcut:** it puts a shortcut on your desktop.

Your saves are kept in the `player` folder.

**Do not share or upload the generated files or the built game.** They contain code generated from the game disc. Everything setup creates stays inside this folder.

## Controls

The game's own button prompts show the keyboard and mouse keys. You can change the camera style and language in the game's Options menu. Gamepads (XInput) work too; with the modern camera on, hold R2 to strafe.

## Known issues

- Weapon models are missing in the cutscene when you buy a weapon.
- The tank on the second Veldin visit is missing its right-side wheels.
- Clank's jetpack can drift out of sync with Ratchet.
- Some glass surfaces can flicker.
- The portrait panel on the screen after the final battle is empty.

## Legal

This is a fan project for preservation and education. It is **not affiliated with, endorsed by or sponsored by Sony Interactive Entertainment or Insomniac Games**. "Ratchet & Clank" is a trademark of Sony Interactive Entertainment LLC. All game content belongs to its owners.

The repository contains only original source code and tools. It contains no copyrighted game data, and it does not tell you how to get the game other than from a disc you own.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). Credits for third-party work are in [NOTICE.md](NOTICE.md).
