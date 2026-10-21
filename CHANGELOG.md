# Changelog

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
