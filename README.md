<p align="center"><img src="rac-rs.png" alt="Ratchet &amp; Clank" width="600"></p>

# rac-rs for Linux

**Fellow Linux users! This one is for you!**

Ratchet & Clank (PlayStation 2, 2002) on Linux, from power-on to the end
credits, built from **your own disc**. The game runs its own original code,
recompiled for your CPU.

This is a fork of **[rac-rs](https://github.com/AsobiCodes/rac-rs)** by
[AsobiCodes](https://github.com/AsobiCodes), a remarkable piece of work that
made the whole game playable on a PC in the first place. It was built for
Windows. This fork carries it over to Linux, adds what Linux players were
missing, and smooths the edges for anyone who would rather just play the
game than fight their way through a build. Its base is upstream **v0.4.0**;
what that brought is in [What's new from upstream
0.4.0](#whats-new-from-upstream-040).

> **No game data is included.** This repository contains no game code,
> graphics, sound, text or other game files. The setup reads your own copy of
> the disc and creates everything it needs on your PC. You need to own the
> game.

## What this fork adds

- **A Docker build.** `sudo docker compose up` builds the game in a container
  that already has everything it needs, so there is nothing to install on your
  machine. It also builds the game against an older C library, so what it
  produces runs on more systems, a Steam Deck included.
- **A native Linux build.** One script, no PowerShell, no Windows toolchain,
  and no 1.2 GB private copy of Rust.
- **Gamepad support.** Xbox-style and PlayStation controllers, over USB or
  Bluetooth, read as evdev devices. Full analog sticks and triggers, and a
  controller plugged in while the game is running is picked up straight away.
- **Prompts that follow your hands.** Mouse and keyboard shows the key names,
  a controller shows the button names, and the game switches between them by
  itself as you use each. A controller's buttons can be shown as the PlayStation
  shapes the game came with, as Xbox pictures, or as the Nintendo Switch's,
  where the four face buttons are the other way round. Pick the one you want in
  the Graphics menu.
- **The controller skips cutscenes.** Start skips, like Spacebar does. The
  Insomniac intro at the start of the game can be skipped too, at last.
- **A crash log.** Panics and errors are written to `player/crash.log`, so you
  can read what went wrong even when the game was started from the application
  menu, where there was no terminal to read.
- **A kinder build.** Drop your disc image in the `rom` folder and run one
  script. It finds the disc, checks it, and builds the game. The build log
  lands in `generated/build.log`, the build is `nice`d so your desktop stays
  usable, and `CARGO_BUILD_JOBS` caps how many cores it takes.
- **Saves show when you saved.** The console has no clock, so the game used to
  write a blank date. It now takes your PC's local time, on the memory card
  too, and the Load Game screen shows it as day/month/year.
- **Your language is kept.** The game starts in the language you picked, and
  the Language page lists German and Spanish in the game's own order.
- **The title's Graphics page matches the in-game one.** Options > Graphics on
  the title screen has the same sections and rows as Pause > Options >
  Graphics: the Preset row, the HUD, Skip intro and the Button glyphs style
  included.
- **Vulkan only, on purpose.** The DirectX 12 renderer is a Windows thing; here
  the game uses Vulkan, and the Renderer row is not shown.
- **Texture packs work; script mods wait for a loader.** PCSX2 texture packs
  can be turned on in the game's Mods page. Script and native mods need an
  OmniLoader library built for Linux, which upstream does not publish yet, so
  the Mods page leaves those rows out.
- **An optional launcher (work in progress).** A small desktop app that installs
  and repairs the game from your disc, keeps a library of installs, starts the
  game, edits its options and profiles, imports texture packs, makes manual
  backups, and adds application-menu entries. `./launcher.sh` builds it the
  first time and starts it; the game works the same without it.

## What's new from upstream 0.4.0

This fork is now based on upstream **v0.4.0**. Everything it added that can run
on Linux is here:

- **New graphics:** volumetric fog and 3D clouds on every planet, shining stars
  in the starry skies, tides and waves that move around Ratchet, Enhanced water
  on Kalebo III, smoother texture detail changes, more vegetation on Umbris and
  in swamps, and Veldin's light following its painted sun.
- **A Graphics Preset row** (Low / Medium / High / Custom) on the Graphics
  page, the same presets the Windows launcher shows.
- **Culling Off** now also draws far-away objects past the draw distance. The
  debug mode's F10 split view and F11 HUD toggle are there too.
- **Strafing like Ratchet & Clank 2**, a Mouse speed row next to Camera speed,
  and the Modern camera turning with the stick.
- **Skip intro** on Options > Graphics > System (and on the title screen's
  Graphics page); the skip keys end the intro too.
- **PCSX2 texture packs**, and the Mods pages (see below).
- **Speed:** the busiest game functions now run as native PC code (about 9% less
  CPU), the fog and clouds cost less, and the HUD capture no longer waits for
  the graphics card.

The Windows-only parts of v0.4.0 are not in this fork: the DirectX 12 renderer,
ReShade, the Windows launcher and the Steam integration. This fork has its own
launcher instead (below). What is and is not here is listed in
[`docs/linux-parity.md`](docs/linux-parity.md).

## Status

- The whole game is playable from power-on to the end credits, including
  New Game+.
- Gameplay follows the original PS2 game one to one. On top of that, the PC
  version has widescreen, keyboard and mouse controls, gamepad support, and
  three camera styles to choose from.
- Optional modern graphics on the Graphics page (Pause > Options > Graphics):
  enhanced lighting with volumetric fog, 3D clouds and shining stars, global
  illumination, ambient occlusion, dynamic shadows, clear reflective water with
  waves and tides, vegetation, enhanced smoke, fire and explosions, sun rays,
  frame smoothing and a sharp HUD, each with quality levels, plus a Graphics
  Preset row (Low / Medium / High / Custom). The Classic settings keep the
  original PS2 look.
- The same Graphics page is on the title screen under Options, with the same
  sections and rows (including Preset, HUD, Skip intro and Button glyphs).
- PCSX2 texture packs work (title Options > Mods). Script mods need a Linux
  OmniLoader build, see [Known issues](#known-issues).
- An optional launcher (work in progress) installs and repairs the game,
  manages installs, options, profiles, texture packs and backups; see
  [Launcher](#launcher-optional-work-in-progress).
- Every button can be rebound, for the keyboard and for controllers, on the
  Controls page (Pause > Options > Controls).
- Plays from the PAL / Europe disc and, since v0.2, from the USA disc.
- Known issues are listed under [Known issues](#known-issues) below.

## How it works

The game runs on its own original code. When you run the setup, it translates
the PS2 code from your disc into native Rust ("static recompilation"), and
that code runs on a PS2 CPU core written in Rust. Because the game logic is
the original code, gameplay behaves exactly like the PS2 version.

Everything around the game code is native PC code written for this port:
graphics, sound, movies, input, saving, and the PC features. Where a fix or
a PC feature needs it, an original function is rewritten in readable Rust.
Changes are tested against the original: test runs play the same frames on
both versions and compare the game's memory.

## Goal

A faithful, fully playable PC version of Ratchet & Clank that you build from
your own disc, with PC improvements on top. The game logic stays the
original code. Rewriting the whole game from scratch is not a goal of this
project.

## What you need

- **The game disc:** Ratchet & Clank, **PAL / Europe v1.00** (serial
  SCES-50916) or **USA v1.00** (serial SCUS-97199). Make an `.iso` image of
  **your own** disc. Other versions (Japan, Platinum and Greatest Hits
  re-releases) are not supported yet. The setup checks the image.
- **About the USA version:** it plays the same as the PAL version, but it has
  had less testing so far.
- **A 64-bit Linux** with about **5 GB of free disk space** (the finished
  install takes about 3.5 GB).

To play it you also want a Vulkan driver (`vulkan-radeon`, `vulkan-intel` or
the proprietary one) and working sound. A gamepad is optional. Vulkan is the
only renderer this fork offers.

> **Tested on CachyOS with KDE Plasma.** Other distributions and desktop
> environments may need extra packages or a small tweak. If something does
> not work for you, please open an issue and say what you are on.

### Requirements

**Docker and Docker Compose**, and nothing else. The build runs in a
container that already has every tool it needs:

```sh
git clone https://github.com/DeSynkro/rac-rs-linux
cd rac-rs-linux
cp /path/to/your-disc.iso rom/
sudo docker compose up
```

The `sudo` is only there because the container installs packages and writes
to your folder as root. If your user is in the `docker` group, plain
`docker compose up` does exactly the same thing and you can leave the `sudo`
out:

```sh
sudo usermod -aG docker $USER
```

Log out and back in for that to take effect. Being in the `docker` group
means you can do anything root can, so only do it if you are comfortable with
that.

Either way the container hands the files it created back to you when it
finishes, so nothing is left owned by root.

**Or, if you would rather build it yourself natively,** these packages:

| | Arch | Debian / Ubuntu | Fedora |
|---|---|---|---|
| Rust 1.85 or newer | `rust` | `rustup` | `rust` |
| Python 3.8 or newer | `python` | `python3` | `python3` |
| A C compiler, for linking | `gcc` | `build-essential` | `gcc` |
| pkg-config | `pkgconf` | `pkg-config` | `pkgconf-pkg-config` |
| ALSA headers | `alsa-lib` | `libasound2-dev` | `alsa-lib-devel` |
| libudev headers | `systemd-libs` | `libudev-dev` | `systemd-devel` |

```sh
# Arch
sudo pacman -S --needed rust python gcc pkgconf alsa-lib systemd-libs

# Debian / Ubuntu
sudo apt install rustup python3 build-essential pkg-config libasound2-dev libudev-dev

# Fedora
sudo dnf install rust python3 gcc pkgconf-pkg-config alsa-lib-devel systemd-devel
```

Nothing else. The setup script is plain POSIX shell, so it runs under whatever
`/bin/sh` is on your system: bash on Arch and Fedora, dash on Debian and
Ubuntu.

## Setup

The block above is the quick way. These are the details.

1. Download or clone this repository and unpack it anywhere.
2. Put your disc image in the **`rom`** folder. Any name ending in `.iso`
   works. If you have more than one, the setup asks you which to use.
3. Build the game.

   With Docker:

   ```sh
   sudo docker compose up
   ```

   Or without Docker, if you installed the packages above:

   ```sh
   ./setup.sh
   ```

   Either way it checks the disc, generates the game code from it, and builds
   the game. This takes about 8 minutes on a 24-thread CPU, or roughly half an
   hour on a 4-core CPU. You can keep using your computer while it runs.

The build stays on screen when it is done, so you can read the log. Set
`CARGO_BUILD_JOBS=4` to use fewer cores, or pass `--no-pause` to skip the
wait. With Docker, put it in front of the command, as
`sudo CARGO_BUILD_JOBS=4 docker compose up`. A full log is written to
`generated/build.log`.

Next time, run **`./play.sh`**.

The build does the rest by itself:

- **Disc check:** it checks your disc image.
- **Build:** it generates the game code from your disc and builds the game.

To add a "Ratchet & Clank (rac-rs)" entry to your application menu, run
**`./install-desktop-entry.sh`**. It writes one small file to your home folder
and nothing else, and it works whichever way you built the game.

If you want the optional launcher, run **`./launcher.sh`**: it builds the
launcher the first time and starts it. See
[Launcher](#launcher-optional-work-in-progress) below.

**Do not share or upload the generated files or the built game.** They contain
code generated from the game disc. Everything the setup creates stays inside
this folder.

## Cleaning up

Docker is only used to build the game. Nothing about running it needs
Docker, so once you have it built you can take away everything the build
left behind.

### Docker

To remove the container, the network and the image the build pulled:

```sh
cd rac-rs-linux
sudo docker compose down --rmi all --volumes
```

Leave out the `sudo` here too if your user is in the `docker` group.

The image is roughly 550 MB to download, and more than that on disk, so it is
worth taking away. Leave out `--rmi all` if you would rather keep it, so the
next build does not have to download it again. Your game stays where it is, in
`build/release/rac-rs`, and keeps working.

If you installed Docker only to build this and want it gone again:

```sh
sudo pacman -Rns docker docker-compose
sudo rm -rf /var/lib/docker
```

The second command deletes everything Docker has on your machine, not just
this project: other images, other containers and other volumes go with it.
Remove it only when you are sure. The `docker` group is left behind as
well; `sudo groupdel docker` takes that away too.

For anything else you have built with Docker, `docker system prune` clears
out stopped containers, unused networks and dangling images, and
`docker system prune -a` also removes the images no container is using.

### The build files

The build also leaves the game code it generated from your disc, and the
compiler's working files, a couple of gigabytes in all. The game does not
need them to run, so **`./clean.sh`** removes them and keeps what it needs:

```sh
./clean.sh
```

It leaves `build/release/rac-rs` and `generated/play.cfg` in place, so the
game still starts, and it does not touch your disc, your saves or the
source code. Run `./setup.sh` again and it generates what it needs, if you
want to build once more.

To do it by hand:

```sh
rm -rf oracle/rec oracle/game
rm -f oracle/src/vu0_prog*.rs oracle/src/skinff_*.rs
rm -f rac-rs/src/gsfull/vu1_prog*.rs rac-rs/src/gsfull/vu1aot_gen.rs
rm -f rac-rs/src/particles/orb_tables.rs
find build -mindepth 1 -maxdepth 1 ! -name release -exec rm -rf {} +
find build/release -mindepth 1 -maxdepth 1 ! -name rac-rs -exec rm -rf {} +
find generated -mindepth 1 -maxdepth 1 ! -name play.cfg -exec rm -rf {} +
```

The last line keeps `generated/play.cfg`, which is where the game looks for
your disc. Without it the game will not start.

## Portability

The game is a normal Linux program. It runs on any 64-bit Linux that has the
same C library, or a newer one, than the system it was built on.

*Worth noting, because it decides where the game will run.
Programs usually link against the C library of the machine it was built on.
Every new version of that library adds its own version of the functions in
it, so a program built on a current desktop asks for functions that an older
system has never heard of. On an older system it then refuses to start,
with a message about a missing library version.*

The Docker build sidesteps that. Its container is Debian 12, whose C library
is older than a current desktop's, so the game it produces asks for less and
runs on more:

| System | C library | The Docker build | A hand build on a current desktop |
|---|---|---|---|
| Ubuntu 22.04 | 2.35 | runs | will not start |
| Debian 12 | 2.36 | runs | will not start |
| Fedora 37 and newer | 2.37 and up | runs | will not start |
| SteamOS 3, the Steam Deck | 2.41 | runs | will not start |
| A current desktop | 2.44 | runs | runs |

A game you build yourself always runs on the machine you built it on. It just
does not travel.

## Playing

Run `./play.sh`. It is a plain shell script, so there is nothing to install to
play. The game starts from your disc, the same way the original did, logos and
all.

If the game stops with an error or a panic, that is written to
**`player/crash.log`**, so you can read it afterwards even if you started the
game from the application menu, where there was no terminal to read.

The whole run is written to **`player/log.txt`** as well: your system and
graphics card, what the game did up to the point it stopped, and, when it
froze, where it got stuck. The run before it is kept as `player/log.prev.txt`.

The game draws the two-dimensional layer, the HUD and the menus, with the
graphics card. On a graphics card or driver the game cannot test it may freeze
while that capture opens. If that happens, the game notices on the next start
and writes `gscap_gpu=off` into `player/pc_options.txt`, then draws the layer on
the processor instead. Write `gscap_gpu=auto` there to have the graphics card
try again.

## Launcher (optional, work in progress)

The fork includes its own launcher: a small desktop app that keeps a list of
installs, starts the game, edits its options, switches profiles and installs
texture packs. It is optional; everything it does can also be done in the
game's own menus (packs included), and `play.sh` keeps starting the game
directly.

```sh
./launcher.sh
```

The first start builds it (Rust and a C compiler are needed), then opens the
window:

- **Library:** "Add existing install..." and pick this folder, or another
  built copy. Several installs can live side by side; removing one from the
  list never deletes files.
- **New install** takes a downloaded or cloned copy of the game's source
  (it must hold `VERSION` and `setup.sh`), asks for your disc and builds the
  game, showing the same progress as `setup.sh`. **Repair / rebuild** does the
  same for an existing install.
- **Play** starts the game with the selected profile's options and mods.
- **Settings** edits the game's own options. **Mods** installs a texture pack
  from a `.zip` or folder, or imports one from PCSX2 itself. The game reads the
  same `mods/` folder, so the in-game Mods page and the launcher agree.
- **Back up this install** makes a playable copy next to it; the copy is in the
  Library and can be deleted from there.
- The game page has "Open saves folder" and "Open log folder".

Application-menu entries: `./install-desktop-entry.sh` adds the game;
`./launcher/install-desktop-entry.sh` adds the launcher (build it first).

The launcher does not check for or install updates: this is a fork, so a new
version is a new download or `git pull`. Put the newer source in the install
folder and use Repair / rebuild; the launcher notices that the source is newer
than the build. Script mods still need a Linux OmniLoader. Build instructions,
the data folders and the code map are in
[`launcher/README.md`](launcher/README.md).

## Updating

This fork does not have an automatic updater. Pull the changes instead:

```sh
git pull
./setup.sh
```

Your saves, your disc setting and your downloaded crates are kept, and only
the parts that changed are rebuilt.

## Controls

The game's own button prompts show the keyboard and mouse keys, or the
controller's buttons, whichever you used last. You can change the camera
style, the language and every button in the game's Options menu (Pause >
Options > Controls for the buttons).

Controllers work too: Xbox-style controllers and PlayStation controllers
(PS4 DualShock 4 and PS5 DualSense, over USB or Bluetooth), also when you
switch one on or plug it in after the game has started.

A controller's buttons are shown the way your pad labels them, and you pick
the style in the Graphics menu, on the title screen (Options > Graphics) or in
the game (Pause > Options > Graphics > System > Button glyphs):

| | PlayStation | Xbox | Switch |
|---|---|---|---|
| The bottom button | Cross | A | B |
| The right button | Circle | B | A |
| The top button | Triangle | Y | X |
| The left button | Square | X | Y |
| The shoulders | L1, R1 | LB, RB | LB, RB |
| The triggers | L2, R2 | LT, RT | LT, RT |
| The sticks | L3, R3 | L3, R3 | L3, R3 |

PlayStation shows the shapes the game came with. Xbox and Switch show the
buttons as pictures, the green A disc and the rest, with the four face buttons
the other way round for the Switch, whose bottom button is B and whose right
one is A. Xbox is the default.

The style is for a controller: with the keyboard the prompts name the buttons
by key, as they always have.

A button you have rebound is shown as the button it is really played with. If
you put the Circle on the RT, the prompts say RT and show the RT's picture, and
the RT is what presses it, in the menus as well as in the game.

| | Classic | Static | Smooth |
|---|---|---|---|
| Camera | the original PS2 follow camera | mouse look and strafing | mouse look, softer |
| R2 | does nothing | aims | aims |
| L2 | does nothing | nothing | nothing |

With the camera on, hold R2 to strafe. Smooth is the default: it follows
softly, recentres lazily while you run, looks down at drops and pulls in at
walls. The camera speed, the mouse speed and the lock-on circle are on the same
page. The camera's "Reversed" options turn the right stick; the mouse keeps its
own direction.

The intro can be skipped while it plays (Start, Escape or Space) and turned off
for good with Options > Graphics > System > Skip intro, on the title screen or
in the game.

The title screen's Graphics page is the game's own Video Options page: use
Cross / A to step a row's value and Up / Down to move between rows. The pause
menu's Graphics page also takes Left / Right on its Page row and on the rows.

## Known issues

- The faint afterimage trail behind Ratchet during Thruster-Pack flight is not
  drawn yet.
- Some glass surfaces can flicker.
- Script and native mods need an OmniLoader library built for Linux. Upstream
  only publishes the Windows DLL, so the Mods page hides those rows and plays
  exactly as before. Arsenal Mastery stays in the mods folder, switched off,
  until a Linux loader exists.
- The title screen's Graphics page steps a value with Cross / A; the D-pad's
  Left / Right does not step values there (that page is the game's own Video
  Options page). In game, the Graphics page takes Left / Right as well.
- DirectX 12 and ReShade are Windows-only and are not part of this fork. See
  [`docs/linux-parity.md`](docs/linux-parity.md).
- The launcher hides script mods until a Linux OmniLoader exists. Updates are
  manual: download the newer fork and use Repair / rebuild.

## Reporting a problem

If the game freezes, crashes or looks wrong, please
[open an issue](https://github.com/DeSynkro/rac-rs-linux/issues) and attach
`player/log.txt` from your game folder. The game writes it every time it runs.
If the game froze, wait about fifteen seconds before you close it: the log then
also says where it got stuck. The previous run is kept as `player/log.prev.txt`,
so if you already restarted the game, attach that file instead. The log stays
on your computer; the game sends it nowhere.

## For developers

The unit tests read the game data off your disc. They look for it at a
fixed path in the game folder, so make a link to it:

```sh
ln -s "/path/to/Ratchet & Clank.iso" \
      "Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"
```

Then `cargo test --release --manifest-path rac-rs/Cargo.toml -p rac-rs --lib`
runs the suite. Five tests want savestates from the upstream project's own test
data, which is not part of a public checkout, so they fail here:
`retail::memcard::card_dump_round_trips` and four `omni::pose_read` tests.
Everything else passes. The file and function size caps are enforced by the
same suite; `RAC_SIZE_CAPS_WRITE=1` lowers `rac-rs/SIZE_CAPS.txt` after a split.

The launcher is a separate crate with its own lockfile:
`cd launcher && cargo test`. Its build and code map are in
[`launcher/README.md`](launcher/README.md).

## Credits
**[rac-rs](https://github.com/AsobiCodes/rac-rs)** by
[AsobiCodes](https://github.com/AsobiCodes) is the original project, and
essentially all of it. The Linux port, the gamepad support, the Xbox button
glyphs and the cutscene skip are the work of
**[DeSynkro](https://github.com/DeSynkro)**, who did it for Linux users and
for anyone who would rather just play the game than fight a Windows
installer.

The optional launcher started as
[AsobiCodes' rac-rs-launcher](https://github.com/AsobiCodes/rac-rs-launcher);
the Linux version is part of this fork (see [`launcher/`](launcher/)).

[OpenCode](https://opencode.ai) was used for a portion of the work.

The original project's `NOTICE.md` and `LICENSE` are kept, and the credits for
the third-party work it builds on are in [NOTICE.md](NOTICE.md).

The Xbox button artwork in [`xboxglyphs/`](xboxglyphs/README.md) is part of
that work: the fork's own drawings of the buttons, not Microsoft's.

## Legal

This is a fan project for preservation and education. It is **not affiliated
with, endorsed by or sponsored by Sony Interactive Entertainment or
Insomniac Games**. "Ratchet & Clank" is a trademark of Sony Interactive
Entertainment LLC. All game content belongs to its owners. "Xbox" is a
trademark of Microsoft; the button artwork in `xboxglyphs/` is the fork's own
drawing of the buttons, not Microsoft's.

The repository contains only original source code and tools, apart from the
Xbox button artwork in [`xboxglyphs/`](xboxglyphs/README.md), which is the
fork's own work. It contains no copyrighted game data, and it does not tell
you how to get the game other than from a disc you own.

The Windows installer, the automatic updater and the Windows release of the
game live in the [upstream repository](https://github.com/AsobiCodes/rac-rs),
not here. The v0.4.0 Windows-only files this fork does not use are kept for
reference under `windows/`; nothing there is built or run on Linux.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE). Credits for third-party work are
in [NOTICE.md](NOTICE.md).
