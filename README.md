# rac-rs

An unofficial PC port of **Ratchet & Clank** (PlayStation 2, 2002), written in Rust. It runs the full game, start to credits, from **your own disc**.

> **No game data is included.** This repository contains no game code, graphics, sound, text or other game files.
> The setup reads your own copy of the disc and creates everything it needs on your PC. You need to own the game.

Project site: https://asobicodes.dev
Launcher (optional app to play, update, and manage texture packs and mods): https://github.com/AsobiCodes/rac-rs-launcher
Support the project (no pressure tho!): https://ko-fi.com/asobicodes

## Status

- The whole game is playable from power-on to the end credits, including New Game+.
- Gameplay follows the original PS2 game one to one. On top of that, the PC version has widescreen, keyboard and mouse controls, gamepad support, and an optional modern camera (mouse look and strafing).
- Optional modern graphics on the Graphics page (Pause > Options > Graphics): enhanced lighting with volumetric fog, 3D clouds and shining stars, global illumination, ambient occlusion, dynamic shadows, clear reflective water with waves and tides, vegetation, enhanced smoke, fire and explosions, sun rays, frame smoothing and a sharp HUD, each with quality levels, plus a Graphics Preset row (Low / Medium / High / Custom). The Classic settings keep the original PS2 look.
- Two renderers: Vulkan (the default) and DirectX 12 (Options > Graphics > Screen > Renderer). ReShade works with both.
- **RAC-RS Launcher** (since v0.4): a separate app to play, change every option, manage texture packs and mods, keep several game versions side by side, and update with a backup first. See [Launcher](#launcher).
- **Mods** (since v0.4): script mods through the bundled OmniLoader mod loader. **Arsenal Mastery** comes preinstalled and switched off. See [Mods and texture packs](#mods-and-texture-packs).
- **PCSX2 texture packs** (since v0.4): packs made for PCSX2 work as they are.
- Plays from the PAL / Europe disc and, since v0.2, from the USA disc.
- Windows 10 and 11, and since v0.4.1 Linux (x86-64). See [Linux](#linux).
- Known issues are listed under [Known issues](#known-issues) below.

## How it works

The game runs on its own original code. When you run setup, it translates the PS2 code from your disc into native Rust ("static recompilation"), and that code runs on a PS2 CPU core written in Rust. Because the game logic is the original code, gameplay behaves exactly like the PS2 version.

Everything around the game code is native PC code written for this port: graphics, sound, movies, input, saving, and the PC features. Where a fix, a PC feature or speed needs it, an original function is rewritten in readable Rust. Changes are tested against the original: test runs play the same frames on both versions and compare the game's memory.

## Goal

A faithful, fully playable PC version of Ratchet & Clank that you build from your own disc, with PC improvements on top. The game logic stays the original code. Rewriting the whole game from scratch is not a goal of this project.

## What you need

- **The game disc:** Ratchet & Clank, **PAL / Europe v1.00** (serial SCES-50916) or **USA v1.00** (serial SCUS-97199). Make an `.iso` image of **your own** disc. Other versions (Japan, Platinum and Greatest Hits re-releases) are not supported yet. Setup checks the image.
- **About the USA version:** it should play the same as the PAL version (at 60 frames per second, like the original US game), but it has had less testing so far. If something looks or behaves differently from the PAL version, please open an issue here on GitHub.
- **Windows 10 or 11, 64-bit**, with about **5 GB of free disk space** (the finished install takes about 3.5 GB) and an internet connection for the first setup.

That's all. Setup downloads anything else it needs into its own folder.

## Setup

1. Download **rac-rs-0.4.1.zip** from the [latest release](https://github.com/AsobiCodes/rac-rs/releases/latest) (or click the green **Code** button above and choose **Download ZIP**), and unzip it anywhere. Drives formatted as exFAT, USB sticks and SD cards work too.
2. Double-click **Setup.cmd**. A window opens (no black console window).
3. Click **Browse** and pick your `.iso` file. The window checks it and shows a green **Disc OK**. Two boxes below choose whether to also install the **RAC-RS Launcher** and whether to **add the game to Steam** (close Steam first). Then click **Install**.
4. Wait. The window shows the steps and a progress bar (about 15 minutes; you can keep using your PC). When it says **Ready to play**, click **Play**.

Next time, double-click the **Ratchet & Clank (rac-rs)** shortcut on your desktop (it opens the launcher if you installed it, otherwise the game), or open Setup.cmd and click **Play**. The same window also has **Repair / rebuild** if something breaks. If anything goes wrong the window says which step failed and has a **Copy details** button. For troubleshooting, `Setup.cmd -Console` runs setup in a console window instead.

Setup does the rest by itself:

- **Tools:** it downloads its own private copy of Rust, Python and the GNU assembler into the `toolchain` folder, using the official free packages (fixed versions, each download checked). Nothing is installed on your PC: no admin rights, no Windows prompts, no changes to your system, and any Rust or Python you already have is left alone. This download is about 220 MB and runs only once; the folder takes about 1.2 GB on disk.
- **Disc check:** it checks your disc image.
- **Build:** it generates the game code from your disc and builds the game. This takes about 12 minutes on a 24-thread CPU, or roughly half an hour on a 4-core CPU.
- **Shortcut:** it puts a shortcut on your desktop.

Your saves are kept in the `player` folder.

**Do not share or upload the generated files or the built game.** They contain code generated from the game disc. Everything setup creates stays inside this folder.

## Linux

Since v0.4.1, rac-rs also runs on Linux (x86-64). It is the same game code as on Windows, built from your own disc, and a recorded fight gives identical game memory on both. **Linux support is new**: it has been tested on Ubuntu 24.04 and Arch Linux (under WSL, with software Vulkan). Real graphics drivers and other distributions are not tested yet, so please report what you find.

Like on Windows, you do not install any programming tools. `setup.sh` downloads its own private copy of Rust 1.99, Python 3.14 and a C linker (zig) into the game folder (`toolchain`, about 250 MB to download and 1 GB on disk), plus the Rust libraries that `cargo` fetches from crates.io (about 200 MB). There is no compiler, `pkg-config`, development package or `sudo` involved. You need:

- A 64-bit Intel/AMD Linux with glibc 2.28 or newer (every mainstream distribution since 2019, SteamOS 3 included), about 7 GB of free space on a Linux drive (ext4, btrfs, ...; not an SD card formatted as FAT or exFAT, which cannot run programs or make links; `setup.sh` checks this) and an internet connection for the first setup.
- A Vulkan-capable graphics driver (Mesa for AMD/Intel, the NVIDIA driver for NVIDIA). On Linux the game uses Vulkan only; there is no Renderer row in the Options menu.
- Ordinary run-time libraries, which any desktop install already has (a minimal or server install may not): ALSA (sound), udev (it is what lets the game see gamepads), the Vulkan loader, the keyboard library xkbcommon and the X11 window libraries (Xlib, Xcursor, Xi; on a pure Wayland desktop `libwayland-client` instead). `setup.sh` looks for them first and prints the install line for your distribution if one is missing. Without one of the window libraries the game stops at start with "Failed to load one of xlib's shared libraries". The packages:
  - Debian / Ubuntu: `sudo apt install libasound2t64 libudev1 libvulkan1 libxkbcommon-x11-0 libx11-6 libx11-xcb1 libxcursor1 libxi6` (`libasound2` instead of `libasound2t64` on Ubuntu 22.04, Debian 12 and older)
  - Arch: `sudo pacman -S alsa-lib systemd-libs vulkan-icd-loader libxkbcommon-x11 libx11 libxcursor libxi`
  - Fedora: `sudo dnf install alsa-lib systemd-libs vulkan-loader libxkbcommon-x11 libX11 libX11-xcb libXcursor libXi`
  - SteamOS has all of them.
- Your Ratchet & Clank disc image: PAL/Europe v1.00 (SCES-50916) or USA v1.00 (SCUS-97199). Put it in the `rom` folder, or give its path.

Build and play (in the unzipped game folder; the Launcher does the same with one click):

```
sh setup.sh                          # finds the .iso in rom/, downloads the tools, checks the disc, generates the game code, builds (takes a while)
sh setup.sh --iso "/path/to/disc.iso"
./play.sh                            # starts the game
```

`setup.sh` writes only inside the game folder (also its temporary files and caches; not with `--system-toolchain` below); it downloads only the tools above and the Rust libraries from crates.io, and checks every tool download against a fixed SHA-256. It ignores your `RUSTFLAGS`, compiler wrapper (for example sccache) and `~/.curlrc` / `~/.wgetrc`. It prints no compiler warnings, only errors. The first build takes 10 to 30 minutes and about 6 GB of memory (it uses fewer parallel jobs on a PC with little memory; `--cargo-jobs N` sets it). Run it again after an update (the Launcher's Update does that for you). `./clean.sh` removes the generated game code, the build files and the downloaded tools (several GB) once the game is built and keeps the game itself (and your `--system-toolchain` choice). `./install-desktop-entry.sh` adds "Ratchet & Clank (rac-rs)" to your application menu (one file in `~/.local/share/applications`).

Advanced: `sh setup.sh --system-toolchain` builds with your own Rust (1.90 or newer; 1.99 is best), `python3`, a C compiler, `pkg-config` and the ALSA and udev headers (`libasound2-dev`, `libudev-dev` / `alsa-lib`, `systemd-libs` / `alsa-lib-devel`, `systemd-devel`) instead of downloading anything; it is remembered until you pass `--pinned-toolchain`. `sudo docker compose up` is a third way: it builds the game in a Debian 12 container.

Gamepads: Xbox-style controllers, DualShock 4 and DualSense are read through the kernel's evdev interface (the SDL controller database that ships inside the `gilrs` library decides which button is which). A controller plugged in while the game runs is picked up. The first controller found is used.

Differences from Windows, for now:

- The Linux build has no profile-guided speed boost (PGO), so it can run slower than the Windows build on the same machine.
- Mods written as scripts (the OmniLoader mods, for example Arsenal Mastery) need the OmniLoader library, which only exists for Windows so far. Without it the game hides those mod rows; texture packs work as before.
- The update window and the Windows-only options (Renderer, RivaTuner, ReShade) do not exist. To update, use the Launcher, or unpack the new release over the game folder and run `sh setup.sh` again (it notices the new version and rebuilds what changed).
- The game writes `player/log.txt` (the run log when started from a menu or file manager) and `player/crash.log` (panics) next to the saves.

## Launcher

The **RAC-RS Launcher** ([AsobiCodes/rac-rs-launcher](https://github.com/AsobiCodes/rac-rs-launcher)) is an optional app next to the game. Setup offers to install it; you can also download it from its own release page. From the launcher you can:

- start the game, and change every option (graphics, camera, controls, Skip intro) before you play,
- add, import and switch PCSX2 texture packs, and turn mods on or off with their own settings,
- keep several game versions side by side and use profiles,
- update the game: it backs up your current version first.

The game works the same without the launcher; everything it changes can also be set in the game's own menus.

## Mods and texture packs

- **Script mods** run through **OmniLoader**, this project's own mod loader, which comes with the game (`bundled\omniloader.dll`). Turn mods on or off in the launcher's Mods page or on the title screen under Options > Mods. With no mod switched on, the game plays exactly as before.
- **The mod format comes from OpenRAC.** OmniLoader is this project's own code, written from the public, MIT-licensed mod SDK of [OpenRAC](https://github.com/OpenRAC/OpenRAC) (another Ratchet & Clank PC project). It follows OpenRAC's script-mod format and Luau API, so a mod folder with OpenRAC's `openrac-mod.json` is read too. No code of OpenRAC's game port is used. Credit and the SDK licence are in [NOTICE.md](NOTICE.md).
- **Arsenal Mastery** is preinstalled and switched off: a fan-remaster style mod where weapons level up like in Ratchet & Clank 3, Nanotech levels raise Ratchet's health and enemies are rebalanced to match. Switch it on in the launcher when you want it.
- **PCSX2 texture packs:** add a pack made for PCSX2 (a folder or a .zip) in the launcher, or import it from your PCSX2 texture folder. Packs cover world textures, water, sky, chrome and glass, the ship, particles, grass, and the HUD and menus. A pack made for the other disc region mostly works too. Turn packs on or off on the title screen under Options > Mods; while one is on, Pause > Options > Mods has the switches in game too.

## Making mods

- **Script mods** (Luau, run by OmniLoader): [docs/MODDING.md](docs/MODDING.md) is a step-by-step guide with a 5-minute first mod, hooks, fields, drawing and per-save data. The preinstalled [Arsenal Mastery](bundled/mods/arsenal-mastery) is a full worked example.
- **Reference**: [docs/MODDING_REFERENCE.md](docs/MODDING_REFERENCE.md) lists every hook id, game function and field.
- **Texture packs** (PCSX2 format): [docs/TEXTURE_PACKS.md](docs/TEXTURE_PACKS.md) shows how players add and switch packs and how to make one.

## Updating

When you start the game, it checks online whether a newer version is out. If there is one, a window shows what is new. Click **Update now** to update, or **Not now** to just play. With the launcher, updates also show up there, and the launcher backs up your current version first. Your saves, your settings, your disc setting and the downloaded tools are kept, and only the parts that changed are rebuilt. Most updates take a few minutes; a few take about as long as the first build (about 15 minutes). You can also check by hand: open Setup.cmd and click **Check for updates**. The check only downloads a small version file from GitHub; nothing about you or your game is sent.

If you downloaded v0.1.0, it has no automatic updates. Download the new ZIP, unzip it over your old folder (replace the files) and double-click **Setup.cmd**. If the window says "Finish the update", click **Update**; otherwise click **Repair / rebuild**. Either way the game is rebuilt once, completely, and your saves and disc settings are kept.

What changed in each version is in [CHANGELOG.md](CHANGELOG.md).

## Controls

The game's own button prompts show the keyboard and mouse keys. You can change the camera style, the language and every button in the game's Options menu (Pause > Options > Controls for the buttons; Camera speed and Mouse speed are separate rows). Controllers work too: Xbox-style controllers (XInput) and PlayStation controllers (PS4 DualShock 4 and PS5 DualSense, over USB or Bluetooth), also when you switch one on or plug it in after the game has started. With the modern camera on, hold R2 to strafe.

## Known issues

- The faint afterimage trail behind Ratchet during Thruster-Pack flight is not drawn yet.
- Some glass surfaces can flicker.
- On AMD and Intel graphics, HUD Sharp only sharpens the title screen and main menu for now. In game, the on-screen text, icons, menus and shop keep the Original look.

## Reporting a problem

If the game freezes, crashes or looks wrong, please [open an issue](https://github.com/AsobiCodes/rac-rs/issues) and attach the file `player\log.txt` from your rac-rs folder. The game writes it every time it runs. It lists your Windows version, processor and graphics card, and what the game did up to the problem. If the game froze, wait about 15 seconds before you close it: the log then also says where it got stuck. The previous run is kept as `player\log.prev.txt`, so if you already restarted the game, attach that file instead. If you use mods or a texture pack, say which ones, and try once with them switched off. The log stays on your PC; the game sends it nowhere.

## Legal

This is a fan project for preservation and education. It is **not affiliated with, endorsed by or sponsored by Sony Interactive Entertainment or Insomniac Games**. "Ratchet & Clank" is a trademark of Sony Interactive Entertainment LLC. All game content belongs to its owners.

The repository contains only original source code and tools. It contains no copyrighted game data, and it does not tell you how to get the game other than from a disc you own.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). Credits for third-party work (including OmniLoader's Luau script engine) are in [NOTICE.md](NOTICE.md).
