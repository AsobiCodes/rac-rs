# Changelog

## Linux fork v0.4.0

This fork carries rac-rs v0.4.0 over to Linux. The game itself is unchanged: it
is still the original PS2 code, recompiled from your own disc and run on a PS2
CPU core written in Rust. What is new here is the platform, the extras this fork
had, and the parts of v0.4.0 that needed a Linux version.

### Added

- **A native Linux build.** `setup.sh` and `play.sh` replace the Windows
  `Setup.cmd` and `setup.ps1`; your system Rust and Python are used as they are.
  The Docker build, `install-desktop-launcher.sh` and `clean.sh` are here too.
- **Gamepad support.** Xbox-style and PlayStation controllers are read as evdev
  devices, with full analog sticks and triggers, and a controller connected
  while the game runs is picked up without a restart.
- **Xbox and Switch button glyphs.** The prompts can show the buttons as
  pictures instead of the retail's PlayStation shapes. The style is a row on
  both Graphics pages (pause: System; title: the same page), and Xbox is the
  default. The PlayStation style puts the retail's own shapes back.
- **The prompts follow your rebindings and your hands.** A rebound button is
  shown as the button it is really played with, and the prompts switch between
  key names and pad buttons by themselves as you use each.
- **Skip intro.** A row on Options > Graphics > System, on the title screen or
  in the game. Start, Escape and Space also end the logo movies.
- **Save dates.** The emulated console now has a clock: saves take your PC's
  local time, on the memory card too, and the Load Game screen shows it as
  day/month/year.
- **The remembered language.** The game starts in the language you picked, and
  the Language page lists German and Spanish in the game's own order.
- **The title's Graphics page has the in-game page's sections and rows**, the
  Preset row, HUD, Skip intro and Button glyphs included, and a change made in
  one place shows in the other.
- **Texture packs.** PCSX2 texture packs can be switched on in the Mods page.
- **A crash log** in `player/crash.log`, with the run log in `player/log.txt`.

### Changed

- **The game is now upstream's v0.4.0.** Graphics gained volumetric fog, 3D
  clouds, shining stars, tides, Enhanced water on Kalebo III, smoother texture
  detail and more vegetation, plus a Graphics Preset row. Culling Off draws
  past the draw distance; debug mode has F10's split view and F11's HUD toggle.
  Strafe-aiming feels like Ratchet & Clank 2, there is a Mouse speed row, and
  the Modern camera turns with the stick. Upstream's own changelog is below.
- **Vulkan is the only renderer.** DirectX 12 is a Windows thing, so the
  Renderer row is not shown here.
- **Script and native mods need an OmniLoader library built for Linux.** The
  upstream loader is a Windows DLL and its source is not published, so the
  Mods page hides those rows and the game plays exactly as before. Texture
  packs work. Arsenal Mastery stays in the mods folder, switched off.
- **The Windows-only v0.4.0 files are kept under `windows/`** for reference;
  nothing there is built or run here.

### Fixed

- **A controller rebind no longer freezes the pause menu.** The font strip is
  repatched without the GPU capture that v0.4.0 builds inside a GS job.
- **Returning from a level no longer resets the title Graphics rows to Low.**
  The pause magic survived the level change, and the pause page's own sync ran
  over the title page's rows.
- **The title Graphics page shows the real fog and cloud levels.** It read the
  shared block without them, so a preset looked like Classic or Custom.
- **The Button glyphs value no longer shares its byte with fog.** It changed
  the fog too, and a fourth value showed up that read a text id.
- **The intro skip keys work again**, and the title page's Preset row applies
  when you press it, not when you move onto it.

## v0.4.0

- Updating from v0.3.1 takes about 2 to 10 minutes (only the parts that changed are rebuilt). Your saves and settings are kept.
- New: the RAC-RS Launcher (its own download, github.com/AsobiCodes/rac-rs-launcher; Setup offers to install it). Play, change every option, manage texture packs and mods, keep several game versions side by side, profiles, updates with a backup of your current version first.
- New: mods. Script mods run through the OmniLoader mod loader, which now comes with the game. Turn mods on or off in the launcher's Mods page or on the title screen under Options > Mods (not in the pause menu). With no mod switched on the game plays exactly as before.
- New: Arsenal Mastery comes preinstalled, switched off. A fan-remaster style mod: weapons level up like in Ratchet & Clank 3, Nanotech levels raise Ratchet's health, enemies are rebalanced to match. Turn it on in the launcher (Mods page) when you want it; its own settings are there too.
- New: PCSX2 texture packs. Add a pack made for PCSX2 (folder or .zip) in the launcher, or import it from your PCSX2 texture folder. Packs work on world textures, water, sky, chrome and glass, the ship, particles, grass, and the HUD and menus. A pack made for the other disc region mostly works too. Title screen Options > Mods turns packs on or off; while one is on, Pause > Options > Mods has the switches in game too.
- New: DirectX 12 option (Options > Graphics > Screen > Renderer), with HUD Sharp. Vulkan stays the default.
- New: ReShade works with the game (Vulkan and DirectX 12), with depth, and its effects stay under the HUD.
- New: Setup can add the game to Steam.
- Graphics (Enhanced lighting): volumetric fog and 3D clouds per planet, shining stars in starry skies, Gemlik's animated star with its own light and shadows, water seen from above and from below, tides, waves that move around Ratchet and water that rises and falls smoothly, Enhanced water on Kalebo III, smoother texture detail changes, Veldin's light follows the painted sun, more grass and plants on Umbris and in swamps.
- New: Graphics Preset row in Pause > Options (Low / Medium / High / Custom, the same presets as the launcher).
- Culling Off (Options > Graphics) now also draws far-away objects past the draw distance and lets off-screen things cast shadows.
- Debug mode (Pause > Options > Controls, last page): F10 shows the original look and the new one side by side in a split screen (with a texture pack on, the original half keeps the disc's own textures), F11 hides the HUD.
- Strafing while aiming now feels like Ratchet & Clank 2 (start, stop and turn). Clank, the Pyrocitor and the gloves stay in place while you walk backwards.
- Camera: a Mouse speed row next to Camera speed (Camera speed now sets the right stick only). The Modern camera turns with the stick like with the mouse.
- New: Skip intro (Options > Graphics, page System, or in the launcher): starts straight at the title screen without the logo movie (about 4 seconds instead of 18). Off by default.
- Fixed: in 16:9 the HUD no longer smears into the side margins. RivaTuner no longer freezes the game on Vulkan. Setup now works on exFAT drives (and other drives that cannot make file links, such as many USB sticks and SD cards): it installs Rust there without rustup, everything stays inside the game folder.
- Fixed: playing with a controller, the mouse pointer no longer appears in the middle of the screen when you pause; it shows only when you move the mouse. Setup now fits on screen at large Windows display scales (up to 300%).
- Fixed: the oxygen tank no longer shows as a pink block at the screen edge, no smoke trail when wading, no sunlight in roofed caves or under water where the sun cannot reach, and Kerwan's city chasm shows fog again instead of water.
- Fixed: the Enhanced sea surface no longer jiggles near Ratchet (worst at lower frame rates). Mods work on the USA disc too.
- Speed: the busiest game functions run as native PC code: the game logic needs about 9% less CPU time. It plays exactly the same.
- Speed: the graphics need less GPU time: the sea skips wave work far from any shore (Umbris about 30% more fps), the HUD capture no longer waits for the GPU, High fog and clouds use a cheaper grid with the same look (about 10-15 fps more), and fog no longer shades plants hidden behind walls (Vulkan).
- Thank you: the garage on Veldin shows drawings for the project's first supporters.

## v0.3.1

- Fixed: on some AMD graphics cards (reported on a Radeon RX 9070 XT) the game window stayed white and "Not Responding" at the start. On AMD and Intel graphics the HUD capture runs on the processor again, as in v0.2.2, and the game hands its pictures to the screen the way v0.2.2 did. NVIDIA cards keep the faster way from v0.3.0. Because of this, on AMD and Intel graphics HUD Sharp only sharpens the title screen and main menu for now; in game, the on-screen text, icons, menus and shop keep the Original look.
- Fixed: Setup failed to build the game ("cannot find crt2.o" or "cannot find -lkernel32") on PCs that have another MinGW compiler on the PATH (MSYS2, Strawberry Perl, w64devkit and others). Setup now ignores those folders while it runs. Your PC's settings are not changed.
- Fixed: with DirectX 12 on NVIDIA graphics, HUD Sharp now works (the HUD capture uses Vulkan when the PC has it).
- New: the game writes a log file, `player\log.txt` in your rac-rs folder (the run before is kept as `player\log.prev.txt`). If the game freezes, crashes or shows an error, the log says where. Please attach it to bug reports (see "Reporting a problem" in the README).

## v0.3.0

- This update rebuilds the whole game once, so it takes longer than usual: about 10 minutes, up to 20 on slower PCs. Your saves and settings are kept.
- Graphics page (Pause > Options > Graphics) now has three pages: Picture, Effects and Screen. Left / Right on the Page row flips between them. The same page is on the title screen under Options > Graphics.
  - Quality levels Off / Low / Medium / High for ambient occlusion, shadows, water, particles and sun rays. High looks like v0.2; lower levels run faster on slower PCs.
  - New: Global illumination (Off / Low / Medium / High). Light bounces off walls and the ground and softly tints nearby surfaces.
  - New: Waves (On / Off). The sea on Pokitaru, Novalis, Rilgar and Batalia now rolls in waves, calm near the shore and around Ratchet.
  - New: Vegetation (Off / Moderate / Lush): tall grass and flowers on grassy ground.
  - New: Draw distance (Low like the PS2 / Medium / High) and Culling (Off draws everything, for testing; slower).
  - New: Frame smoothing (On by default). On monitors faster than 50 or 60 Hz the game draws in-between pictures, so motion looks smooth. The game itself still runs at its original speed.
  - New: HUD Sharp / Original. Sharp draws the on-screen text and icons, the shop screen and the title menu at your screen's resolution (windows 896 pixels tall or more). Original keeps the PS2 look.
- Water: light patterns on the sea floor and light shafts underwater, reflections of the scenery in pools and the sea, no more flicker or holes at the Pokitaru waterfalls, and no more flickering dark shapes or bands on Rilgar's sea.
- Sun shadows: one sun per planet, and shadows are no longer cut into pieces.
- The pause menus now show their 3D models again: Ratchet and the selected weapon with its ammo on the Weapons page, Ratchet on the Gadgets page, the weapon on Quick Select and the gold bolt on Items.
- Camera: a new Camera row (Pause > Options > Camera Options) with Classic, Static and Smooth. Smooth is the new default: it follows softly, recentres lazily while you run, looks down at drops and pulls in at walls. Static is the modern camera from v0.2. The modern camera no longer goes inside the level when Ratchet jumps up big steps.
- Camera Options also has a camera speed setting (10 to 200) and Lock-on (Auto / On / Off): while strafe-aiming with the right mouse button, a red circle locks on to the nearest enemy.
- Controls: every button can now be changed (Pause > Options > Controls), for the keyboard and for controllers. The menus show your keys instead of PlayStation buttons.
- Debug mode (Pause > Options > Controls, last page, off by default): F1 planet list and travel, F2 free camera, F3 draw distance, F5 cannot die, F6 infinite ammo, F8 +10,000 bolts. Travelling can skip story progress, so save first.
- Clank now stays on Ratchet's back during Thruster-Pack flight, and both legs move when Ratchet walks backwards while strafing.
- Widescreen: the right edge of the picture now fades with the cutscene fades and takes the underwater colour like the rest.
- PAL: the game's own 60 Hz option (title Options > Video) now really runs the game at 60 steps a second.
- Faster Load Game (also after Quit Game), a plain black screen while a level loads, and New Game on a used save slot now asks before overwriting it.
- Sound: no more pop at the end of every movie, and no more crackle on busy or slower PCs.
- Speed: the graphics card now does more of the work (character skinning, culling, frame smoothing), big fights no longer hitch, and there is no more slowdown after many planet trips. The USA pause menus are much faster.
- Setup now compiles the game with a speed profile (profile-guided optimisation) made for your disc, Europe or USA: the game logic needs about 8-13% less CPU time. It plays exactly the same.
- USA disc: everything above works on the USA disc too.

## v0.2.2

- Big speed-up: the game logic now needs 30-50% less CPU time on every planet, so busy places like Metropolis (Kerwan), Blarg, Orxon, Hoven, Gemlik and Veldin no longer slow down or stutter. This helps most on older or slower CPUs. The game plays exactly as before: only how fast the PC runs it has changed.
- The game no longer draws hundreds of frames per second that your monitor cannot show: it now draws at most as many as your monitor's refresh rate, which leaves more CPU time for the game.
- This update rebuilds the whole game once, so it takes longer than usual: about 10 minutes, up to 20 on slower PCs. Your saves and settings are kept.

## v0.2.1

- Every liquid in the game now has an Enhanced look (Lighting and Water set to Enhanced):
  - Aridia's mud is now thick brown mud that oozes slowly and bubbles. Before, it shone like a mirror.
  - Orxon's toxic sludge now swells slowly, with a glossy skin, glowing veins and bubbles that swell and pop.
  - Gaspar's lava now has drifting dark crust with glowing cracks, bubbles, a heat shimmer, and a white-hot glow where it meets rock.
  - Batalia's jungle river now flows, with ripples and reflections.
  - The water in Hoven's abyss and in the flooded tank room on Quartu now get the Enhanced water look.
- Hoven's frozen floor by the doors now stays still like ice instead of rippling like water.
- Hoven's ice (floes, crystals and icicles) now has a cold shine with sparkles (with Lighting Enhanced).
- Distant water no longer flickers when the camera moves, on every planet.
- With Water set to Classic, every liquid still looks like the PS2.
- The Graphics settings are now also on the title screen: Options > Graphics. They are the same settings as Pause > Options > Graphics, and a change made in one place shows in the other.
- The game window's title now shows the game's version (for example "rac-rs v0.2.1").
- Lighting Classic: objects (crates, machines, platforms, enemies, Ratchet) now use the same texture detail as the PS2, so they look as soft as on the PS2 from a distance. Lighting Enhanced keeps the sharper textures.
- Controllers: running with the left stick no longer slows Ratchet down at diagonal angles. A full push now runs in every direction, like on the PS2.
- USA disc: the title screen's Options page showed a Video row that the USA game does not have, so the highlight sat on the wrong row. It now shows Language, Sound and Graphics, and the highlight follows your choice.

## v0.2.0

- This update rebuilds the whole game once, so it takes longer than usual: about 10 minutes, up to 20 on slower PCs. Your saves and settings are kept.
- New Graphics page (Pause > Options > Graphics). Every setting is remembered. With Classic and everything Off, the game looks like the PS2.
- Lighting: Classic or Enhanced. Enhanced adds a soft glow around bright light, richer contrast and colour, and light sharpening.
- Ambient occlusion: soft shading in corners and where objects meet the ground.
- Dynamic shadows: real sun shadows from Ratchet, enemies and scenery.
- Water: Classic or Enhanced. Enhanced water is clear, with reflections, ripples, light patterns on the bottom, foam at the shore and rings where Ratchet wades.
- Particles: Classic or Enhanced. Enhanced smoke fades softly where it meets walls and the ground, and fire and explosions light up their surroundings.
- Sun rays: light shafts from the sun.
- Display mode: windowed or fullscreen.
- USA disc: the US version of the game now works too, at its normal 60 frames per second, with every PC feature. Setup recognises which disc you have. It should play the same as the PAL version, but it has had less testing so far.
- No more dark outlines around trees, palms, cliffs and other scenery (most visible on Novalis, Pokitaru, Eudora and Hoven).
- Smoke and sparks fade into fog like on the PS2, effects no longer go missing for one frame after loading, and the edges of object and sky textures are drawn like on the PS2.
- Menus with the mouse: the cursor no longer overshoots, every row can be clicked (the Graphics page too), and the description panel shows correctly.

## v0.1.3

- Update window: the list of what is new now loads (before, it said "The list of changes could not be loaded"). The **Check for updates** link in Setup works too (before, it always said you were offline).
- Play.cmd now checks for updates when you start the game, like the desktop shortcut and Setup's **Play** button.

## v0.1.2

- Weapon purchase scene: after you buy a weapon, Ratchet now holds it up in the show-off scene (before, his hands were empty). This works for every weapon.
- Veldin (second visit): the Blarg tanks now have their tracks and wheels on both sides. Everything the game draws as a mirrored copy now shows correctly.
- After the final battle: the robot in the portrait panel of the closing page is shown and animated, as on the PS2.
- Glass on Kalebo III and in the Drek fleet now uses the PS2's depth test and texture filtering.
- PlayStation controllers: the PS4 controller (DualShock 4) and the PS5 controller (DualSense) now work directly, over USB or Bluetooth. You no longer need Steam or DS4Windows for them.
- Controllers: the game now finds a controller that you switch on or plug in after starting the game, and finds it again after it disconnects or goes to sleep. Before, a controller only worked if it was already connected when the game started.

## v0.1.1

- Automatic updates: when you start the game, it checks online for a newer version. If there is one, a window shows what is new, and you can click **Update now** or **Not now**. Your saves, your disc setting and the downloaded tools are kept, and only the parts that changed are rebuilt, so most updates take a few minutes. Setup.cmd also has a **Check for updates** link.
- **Repair / rebuild** no longer deletes the generated game code first. It only rewrites files that really changed, so it no longer forces a full rebuild when nothing changed.

## v0.1.0

First public release.
