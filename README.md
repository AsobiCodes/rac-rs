# rac-rs

An unofficial PC port of **Ratchet & Clank** (PlayStation 2, 2002), written in Rust. It runs the full game, start to credits, from **your own disc**.

> **No game data is included.** This repository contains no game code, graphics, sound, text or other game files.
> The setup reads your own copy of the disc and creates everything it needs on your PC. You need to own the game.

Project site: https://asobicodes.dev
Support the project (no pressure tho!): https://ko-fi.com/Manage/

## Status

- The whole game is playable from power-on to the end credits, including New Game+.
- Gameplay follows the original PS2 game one to one. On top of that, the PC version has widescreen, keyboard and mouse controls, gamepad support, and an optional modern camera (mouse look and strafing).
- Optional modern graphics on the Graphics page (Pause > Options > Graphics): enhanced lighting, ambient occlusion, dynamic shadows, clear reflective water, enhanced smoke, fire and explosions, and sun rays. The Classic settings keep the original PS2 look.
- Plays from the PAL / Europe disc and, since v0.2, from the USA disc.
- Windows only for now.
- Known issues are listed under [Known issues](#known-issues) below.

## How it works

The game runs on its own original code. When you run setup, it translates the PS2 code from your disc into native Rust ("static recompilation"), and that code runs on a PS2 CPU core written in Rust. Because the game logic is the original code, gameplay behaves exactly like the PS2 version.

Everything around the game code is native PC code written for this port: graphics, sound, movies, input, saving, and the PC features. Where a fix or a PC feature needs it, an original function is rewritten in readable Rust. Changes are tested against the original: test runs play the same frames on both versions and compare the game's memory.

## Goal

A faithful, fully playable PC version of Ratchet & Clank that you build from your own disc, with PC improvements on top. The game logic stays the original code. Rewriting the whole game from scratch is not a goal of this project.

## What you need

- **The game disc:** Ratchet & Clank, **PAL / Europe v1.00** (serial SCES-50916) or **USA v1.00** (serial SCUS-97199). Make an `.iso` image of **your own** disc. Other versions (Japan, Platinum and Greatest Hits re-releases) are not supported yet. Setup checks the image.
- **About the USA version:** it should play the same as the PAL version (at 60 frames per second, like the original US game), but it has had less testing so far. If something looks or behaves differently from the PAL version, please open an issue here on GitHub.
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

## Updating

When you start the game, it checks online whether a newer version is out. If there is one, a window shows what is new. Click **Update now** to update, or **Not now** to just play. Your saves, your disc setting and the downloaded tools are kept, and only the parts that changed are rebuilt. Most updates take a few minutes; a few take about as long as the first build (about 15 minutes). You can also check by hand: open Setup.cmd and click **Check for updates**. The check only downloads a small version file from GitHub; nothing about you or your game is sent.

If you downloaded v0.1.0, it has no automatic updates. Download the new ZIP, unzip it over your old folder (replace the files) and double-click **Setup.cmd**. If the window says "Finish the update", click **Update**; otherwise click **Repair / rebuild**. Either way the game is rebuilt once, completely, and your saves and disc settings are kept.

## Controls

The game's own button prompts show the keyboard and mouse keys. You can change the camera style and language in the game's Options menu. Controllers work too: Xbox-style controllers (XInput) and PlayStation controllers (PS4 DualShock 4 and PS5 DualSense, over USB or Bluetooth), also when you switch one on or plug it in after the game has started. With the modern camera on, hold R2 to strafe.

## Known issues

- Clank's jetpack can drift out of sync with Ratchet.
- Some glass surfaces can flicker.

## Legal

This is a fan project for preservation and education. It is **not affiliated with, endorsed by or sponsored by Sony Interactive Entertainment or Insomniac Games**. "Ratchet & Clank" is a trademark of Sony Interactive Entertainment LLC. All game content belongs to its owners.

The repository contains only original source code and tools. It contains no copyrighted game data, and it does not tell you how to get the game other than from a disc you own.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). Credits for third-party work are in [NOTICE.md](NOTICE.md).
