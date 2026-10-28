# Changelog

## v0.4.1

- Updating from v0.4.0 takes about 2 to 10 minutes (only the parts that changed are rebuilt). Your saves and settings are kept.
- New: Linux (x86-64, issue #6). With the RAC-RS Launcher (its own download, now also for Linux) installing is one click: it downloads the game, asks for your own disc and builds it for you, with no programming tools and no sudo. Without the Launcher: unpack the game, run "sh setup.sh", then "./play.sh" (or build it in a Docker container). An application-menu entry and gamepad support (Xbox, DualShock 4, DualSense) are included. The game starts with Vulkan and writes a run log and a crash log in the player folder. The same recorded fight gives identical game memory on Linux and Windows. Linux support is new: it was tested on Ubuntu and Arch Linux under WSL only, so please report what you find. See the Linux section of the README. Thanks to DeSynkro for the Linux port (pull request #38), which these parts were taken from; the rest of that pull request is not part of this release.
- New: "Start skips cutscenes" (Pause > Options > Graphics, page System), on by default. Start now skips story movies and the planet arrival and landing scenes, also on a controller (a controller could not skip movies at all before). Off gives the original rule back (Start plus L1, R1, L2 and R2; Esc and Space still skip). Ship takeoff stays unskippable, like the original (issues #34 and #23).
- New: Display page (Pause > Options > Graphics, page Display): HUD position (4:3 or Corners, which puts the nanotech bar and the bolt counter in the corners of a wide window), HUD scale (75 to 150), FPS limit (default: your screen's refresh rate) and Render scale (50 to 200 percent of the window: lower runs faster on weak PCs, higher is sharper) (issue #33).
- New: texture packs now also replace the HUD, the pause and Options menus, the vendor, the text and the title screen (v0.4.0 listed these, but only the world textures were replaced). Where the processor draws the HUD (integrated graphics) a pack's colours and art show at the PS2's picture size (issue #32).
- New: texture packs in the BC7 format (what texconv and similar tools write) now load. Before, most of them were skipped and the original pictures stayed.
- Fixed: big PNG texture packs (several GB) no longer leave part of a planet with the original textures: they are kept compressed in memory so the whole pack fits (on Novalis 495 of 609 pictures were replaced before, now all 609) (issue #32).
- Fixed: a texture pack folder copied by hand into the mods folder (a PCSX2 textures folder, loose pictures, any folder name) now shows up in Options > Mods and can be switched on.
- Fixed: with a texture pack on, a level no longer freezes the picture for a moment while the pack loads, a damaged picture in a pack no longer crashes the game (it is skipped), and leaf and branch shadows no longer go missing, with slower speed, for a whole visit on planets whose pack textures arrive late.
- Fixed: German and Spanish were swapped in the title screen's Language list (issue #26).
- Fixed: the language you choose is remembered. Before, the game started in English every time (issue #11).
- Changed: Culling is On for everyone: it is reset to On once with this update, every Graphics Preset now sets it On, and turning it off shows a warning (it gives no gameplay benefit and puts a heavy load on your hardware). You can still switch it off.
- Fixed: with Culling Off, objects the game hides on purpose stay hidden (on Veldin, grey wall blocks appeared).
- Fixed: with the Classic camera, aiming while strafing turned the view the opposite way from normal play; both now match the original. With the Modern camera the right stick no longer flips direction in free look or after the game's own camera takes over, and it no longer zooms the pause map the wrong way (issues #25 and #24).
- Fixed: remapping a button no longer moves other actions to other buttons. One button can now do several things, the Controls page shows both names (for example "B / Circle"), and fire on the R2 trigger still works while aiming (issue #24).
- Fixed: controller sticks no longer need a big push before Ratchet moves (the PC dead zone and the game's own were stacked on each other). Walking starts with a small push and running with a bit over half, like in PCSX2 (issue #30).
- Fixed: a rare one-picture glitch of Ratchet's legs when the strafe direction flips while aiming.
- Fixed: the pause menu's secret button codes (they need L1 and R2 held together) work again on a controller: since v0.4.0 R2 was taken as the aim button even in menus; it is now the aim button only while you play.
- Fixed: the Gadgetron vendor and Clank boarding and leaving the ship ran in slow motion, with the voices ahead of the animation, at high resolutions or on slower graphics cards. They now keep the game's speed (4K vendor with a 25 fps limit: 51 to 61 game steps a second instead of 28 to 33) (issue #37).
- Fixed: movie sound no longer drifts and goes silent on a busy PC (Drek's intro movie), and movies no longer end with a click (issue #7).
- Fixed: the screen no longer switches off while you play with a controller only. Windows may sleep again once the game window loses focus or is minimised.
- Fixed: closing the game with Task Manager (or a power cut) during the first seconds no longer switches the graphics card's HUD capture, and with it HUD Sharp, off for good; it now takes two failed starts in a row, and the message says how to switch it back on.
- Fixed: the RivaTuner (RTSS) message no longer appears after crashes that RivaTuner did not cause.
- Fixed: DirectX 12 without the extra shader compiler file could crash at the start with fog on, and black streaks could appear in the stars. If a graphics effect is ever refused by the graphics driver, the game now switches that effect off and keeps running. The long first start on DirectX 12 without the compiler file now shows a notice.
- Fixed: switching Lighting from Enhanced back to Classic left the game's fog faded to about a third of its strength until you left the level.
- Fixed: with the PC fog on, the item model in the Gadgetron vendor now takes the planet's fog like the rest of the picture.
- Fixed: with the PC fog on, Kerwan's fog (the sheet at water level and the fog under the buildings) and the fog on other planets with a sea could switch off for a moment and come back whenever no sea was near the camera. It now stays on.
- Fixed: with Dynamic shadows on, animated objects just outside the picture now cast their shadow in the right pose.
- Fixed: with Frame smoothing on, a very fast turn of the Modern camera no longer shows a one-picture flick; it is blended over two pictures. A rare ReShade crash is fixed.
- Fixed: script mods (Arsenal Mastery) were switched off in big fights. A slow moment now only gives one warning; a mod stops only if it stays far over its time. Log lines from mods are also much cheaper. A mod can no longer write into the game's own program code by mistake.
- New: Arsenal Mastery 0.3.3 (faster; Ammo: Survival no longer gives a shot back for Glove of Doom kills; mini bombs of a cluster are not split again; an error in one hit no longer switches XP and weapon scaling off). It now has its own page, github.com/AsobiCodes/arsenal-mastery, and the launcher (v0.1.2, its own download) can update mods from there.
- Fixed (USA disc): Hoven's Enhanced water now also appears with the free camera, moving water wakes run at the USA disc's 60 Hz like the rest of the game, and every PAL-to-USA memory address the port uses was checked on all 19 planets, New Game and Quit Game to the title screen.
- Speed: shops, pause menus and big fights at 4K stutter less: on NVIDIA cards the HUD capture no longer waits behind the picture, and the HUD capture needs far fewer graphics calls in the vendor (4K vendor: 75 to 86 fps, worst frames 48 to 39 ms) (issue #28).
- Fixed: with HUD position Corners, help prompts (vendor, ship pads, Infobots) are no longer torn or stretched across the screen, also in their first and last moments with Frame smoothing on, and the race bar and position panel sit whole in the top-left corner.
- Fixed: the Galactic Map no longer ignores the keys or a controller while the mouse pointer rests over the planet list (the pointer kept pulling the selection back, issue #16). Pointing at a planet now picks the planet under the pointer (the list's real row size and scrolling are used), also in the Help, Goodies and Cinematics lists, and a bump of a hidden mouse no longer moves a menu selection.
- Fixed: the water on Hoven (the river and the abyss) and the sea on Kalebo III no longer vanish at some camera angles after you arrive from another planet.
- Fixed: with Enhanced water, calm patches from the previous planet's coast could stay on the open sea of the next planet with a sea; the shore marks are now cleared when you change planet.
- Fixed: with Clouds High, the sky seen from under water showed the bare backdrop instead of the game's own sky layers.
- Fixed: with Fog High, Hoven's chasms (under the bridges and between the cliffs) and some gaps in its sky no longer show flat blue fog sheets; you see the dark rock, the river and the distant mountains like on the PS2.
- Fixed: on a wide screen, parts of Hoven's ice floor no longer disappear at the screen edge while the camera turns (the game only checks its own 4:3 view before it skips drawing them; the port now checks the whole wide view). Gaspar's second lava layer and the ship's shadow on the ground had the same fault and are fixed too (issue #40).
- Thank you: to everyone who reported problems after v0.4.0 on GitHub and YouTube; most of the fixes above come from your reports.

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
