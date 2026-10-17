# Changelog

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
