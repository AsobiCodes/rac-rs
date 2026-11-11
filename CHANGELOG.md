# Changelog

## v0.5.1

- Updating from v0.5.0 keeps your saves and settings. Your key settings and controller settings are converted to the new controls on the first start (nothing to redo).
- Linux: update with one click from the RAC-RS Launcher, or unpack the new release over the game folder and run `sh setup.sh` again. The new RAC-RS Launcher v0.1.6 has rows for the settings that moved out of the game's menu (DLSS preset, Underwater look, Edge smoothing (MSAA), Waves).

### Controls

- New: keyboard and mouse are their own system now (issues #77 and #78). Every game action (Jump, Wrench, Fire, Quick Select, Look, Crouch and the others) has two slots, and each slot can be a key, a mouse button (left, right, middle, and the side buttons Mouse 4 and Mouse 5) or the mouse wheel. Modern menu: Options > Controls is a table (action, 1st, 2nd): Enter or a click captures a slot, Tab picks the 2nd slot, Del clears it, and Pause always keeps at least one key. Classic menu: the Controls pages do the same row by row. On-screen hints and tutorial boxes name the key or mouse button you bound.
- New: the controller is remapped the same way. Every game action (Jump, Wrench, Fire, Quick Select, Look, Crouch, Map, Pause, the d-pad, L3 / R3, Aim) has two controller slots, each any of the 16 buttons. Modern menu: Options > Controls has a switch "Keyboard & mouse | Controller" over the same table. Classic menu: the pages Controller 1/3 to 3/3 (Cross = 1st slot, Square = 2nd slot, L3 or Del = clear). A button used by two actions shows "x2", and Aim takes over its button while you aim. Menus keep the standard layout whatever you remap (Cross / A confirms everywhere); only gameplay follows your remap.
- New: the button pictures in gameplay texts follow your controller remap: tutorial and hint boxes, Helpdesk pages, the L1 / R1 of the Quick Select ring, and the pad picture in Help > Controls (changed labels are rewritten in the game's own lettering, with extra lines such as "D-Up: OmniWrench" for buttons the picture has no label for).
- New: a mouse invert of its own (asked for by DeSynkro in pull request #48). Options > Controls, PC page: "Mouse up/down" and "Mouse left/right", each Like camera (the default, as before), Normal or Inverted. The camera's own Reversed options still flip the controller.

### New graphics options

- New: fire light v2 (Lighting Enhanced, Global illumination Low, Medium or High). Every fire is now a real light: a warm glow of the fire's real size and flame colour (redder as it dies down) that fades naturally over about 8 to 10 m. Near a fire, shaded sides and the sun's shadows light up in its colour on the ground, walls, ships, characters, grass, fur and water; farther away the scene keeps its own shade. Faces turned to the flames glow, charred parts stay dark, and very bright light goes from orange to yellow to white instead of turning Ratchet pink. The light flickers and sways with the flames and stands still while the game is paused. Small flames give a small light too, and the flames of Clank's Thruster-Pack give a small warm glow on Ratchet's back and the ground under him. This replaces the v0.5.0 fire light.
- New: fire shadows. The strongest fires near you cast soft shadows from the flames themselves, with a blurry edge that widens with distance, so a wreck or Ratchet throws a shadow away from the fire. Global illumination High: up to 4 fires, Medium: 1, Low: none. They fade in and out instead of popping. Dynamic shadows Off also turns the fire shadows off (the fire light stays). Fire shadows cost frame rate only next to fires; Global illumination Medium costs less than High. Lighting Classic and Global illumination Off look exactly as before.
- New: heat haze (Lighting Enhanced with Fire Medium or High). The air above fires, burning enemies, fireballs, the Pyrocitor's jet and lava wobbles slightly, like real heat. It stands still while the game is paused, stays off under water and costs almost nothing. Fire Low or Off and Lighting Classic: no haze.
- New: Options > Graphics > Frame Generation (after Upscale quality; classic menu: the Screen page, after Renderer). The graphics card adds generated frames between the real ones, so the frame rate shown goes up 2, 3 or 4 times; the HUD is kept out of the generated frames. It follows the Upscaler: with FSR it offers FSR 2x (AMD frame generation, works on any recent graphics card, NVIDIA too), with DLSS it offers DLSS 2x, 3x and 4x (NVIDIA RTX 40: 2x; RTX 50: up to 4x). Off is the default and gives exactly the game of before. Works with Vulkan and DirectX 12. The first time you switch it on the game needs a restart; after that, switching between Off, 2x, 3x and 4x applies at once. It pauses while the picture has black bars (an Aspect ratio other than Auto, or a fullscreen Resolution smaller than the screen) and comes back when the bars go. If it cannot start on a PC, the game falls back to the normal picture and stops offering it.
- New: Options > Display > Resolution: Desktop (the default, as before) or one of your monitor's sizes. In a window it sets the window size; in Fullscreen the whole game, HUD included, is drawn at that size and scaled to your screen, without a monitor mode switch, so alt-tab, DLSS / FSR and frame generation keep working. It applies 0.7 seconds after your last press, so there is no flicker while you scroll through the list.
- New: Options > Display > Picture > Brightness and Contrast (0 to 100, default 50 = the picture of before). Brightness lifts or darkens the shadows and mid-tones (black and white stay); Contrast is a soft S-curve (nothing clips to pure black or white). They change the game picture and the movies, in Lighting Classic and Enhanced and with every upscaler; the HUD and menus keep their colours. They change live while you move the slider.
- Changed: DLSS preset. Auto now uses preset K (DLSS 4; L at Ultra performance) instead of M: the DLSS step costs about 0.85 ms instead of about 1.4 ms a frame at 2560x1440 DLSS Quality (Veldin 176 -> 197 fps on an RTX 5070 Ti). M, L, K and J can be picked in the RAC-RS Launcher (Settings > Screen and HUD > DLSS preset; GitHub issue #84). DLSS needs an NVIDIA RTX graphics card.
- Changed: Options > Graphics > Effects > Metal is one row now: Off, Low, Medium or High (was On / Off; Metal On becomes High). High draws the floor reflection of shiny metal at half of v0.5.0's width and height (softer), Medium at a third, Low at a quarter. The sharp mirror of v0.5.0 is no longer offered. Applies at once. The classic Graphics page has the same row.
- Changed: less gloss on painted metal, such as Novalis' copper floors. Its glossy coat now shows the sky's colour at the floor's own brightness, so a dark floor in the shade no longer gets a pale sheen with sharp edges. Polished, mirror and brushed metal, the floor reflections and the Metal reflections slider are unchanged.
- Changed: clouds fitted to each planet's own sky, compared with the PS2 picture. Batalia: a flat dark grey overcast (was bright blue-white heaps). Hoven and Oltanis: a dark green-grey storm (was bright white). Gaspar: a golden sky with faint bands (was dark brown heaps). Novalis: deep blue again (was washed out to grey-teal). Veldin's second visit: the game's own painted orange-red sky. Veldin, Kerwan, Eudora, Rilgar, Umbris, Orxon, Pokitaru and Kalebo III already matched and are unchanged.
- Changed: denser fur. Ratchet's fur is a closed coat with no bald gaps between the tufts, shade deep inside the fur, lighter tips that stand up a little and a soft sheen along the hair. Fur High draws 20 layers with Soft and 32 with Fluffy (was 16 and 24); Low and Medium are unchanged.

### Speed

- Faster: MSAA is now automatic: off while DLSS or FSR runs (they smooth edges themselves; about 5 to 10 % more frames at DLSS native), 4x otherwise. The RAC-RS Launcher's Edge smoothing (MSAA) row can keep 4x on with the upscalers.
- Faster: the new fire shadows are made cheap for the processor: all shadow passes share one set-up and the fire shadow work runs on a helper thread. Near Novalis' burning buildings at 2560x1440 with DLSS Quality the graphics card no longer waits about 4 ms a frame for the processor.
- Faster: near burning fires with fire shadows on, at 1440p or more, on NVIDIA with Vulkan, buildings and rocks that end up hidden behind others are no longer lit first. Same picture.
- Faster: the short hitch about 2 seconds after a level starts is shorter (shader warm-up about 300 ms -> about 115 ms on our PC), and levels without volumetric clouds or fog skip that warm-up.
- Faster: the floor reflection of shiny metal is drawn smaller (see the Metal row).

### Menus

- Changed: a shorter Options screen (modern menu). The MSAA row is gone (it is automatic now). DLSS preset and Underwater look are set in the RAC-RS Launcher. Waves has no row either: Water sets the waves itself (Medium and High = moving waves, Classic and Low = none, as the Low / Medium / High presets always did); the launcher can still set Waves on its own. Supporter drawings moved from Graphics > More effects to Game > Extras. Rows that need another setting are hidden instead of greyed out: with Lighting Classic the effect rows disappear (the banner on top says why), with Upscaler Off the Upscale quality and Frame Generation rows disappear, Lock-on strength hides with Lock-on Off and the Metal reflections slider hides with Metal Off. The classic Graphics page keeps all its rows, Waves included.
- New: the modern Help tab has the game's Help topics in the modern style, each with its own text and picture (it was empty in v0.5.0).
- New: the classic menu has the rows that were modern-only: Aspect ratio and Upscaler (Display page), Quick Select pauses the game (System page).
- New: the Galactic Map (modern menu) is animated: the galaxy painting slowly turns (one turn in 8 minutes), the core breathes, stars twinkle and the selected planet's ring pulses. The planet markers stay put. The classic menu is unchanged.

### Gameplay

- New: Game > Gameplay > Fast travel (default On). Planet trips go from takeoff through a short black screen straight to the landing (about 12 seconds shorter), and loading a save from the title screen (about 12 seconds shorter) or from the pause menu to another planet (about 3 seconds shorter) skips the flight the same way. No warp sound plays over the black screen. Off gives the original flights.

### Fixes

Problems of v0.5.0 that are fixed. (Problems in the new v0.5.1 features that were found and fixed before this release are not listed.)

- Fixed: the travel sequence after a played movie stayed black (issue #74).
- Fixed: Eudora music (issue #74): the music could go silent after arriving on a planet, or not start at all when its start fell on the same moment as a sound hush (fast travel, mouse in a menu).
- Fixed: classic menus (issue #79): the olive / gold frame around every classic menu box is back, also while a menu flies in, changes page or closes. Classic hint boxes no longer cut long German and French words.
- Fixed: far floors (issue #79): water pools and lava now reach as far as your Draw distance allows (Low, Medium, High) instead of a fixed distance, so on wide screens (21:9) far floors should no longer end early. The sea keeps the original distance. We could not see the problem on our own PC, so please tell us if you still do.
- Fixed: with the Classic camera, pressing strafe aim no longer makes the camera jump for one frame (the Raised aim too), also when you press it again while the camera is still swinging back.
- Fixed: Kerwan stairs: with the Modern camera far down the stairs, the near steps no longer lose a wedge. The Classic camera is unchanged.
- Fixed: a button still held when a menu opens or closes does nothing until you let go (no accidental buy at a vendor).
- Fixed: enemies that die in an explosion, and scenery explosions such as the three in the Gemlik base, no longer leave fire on the ground or burnt grass; they show the game's own explosion again. Only your own weapons' explosions get the realistic fire: Bomb Glove, Devastator, Mine Glove, Visibomb, R.Y.N.O., Glove of Doom, Drone Device, Decoy Glove, the Suck Cannon's flung enemy and the Pyrocitor.
- Fixed: Fur: the front of Ratchet's chest has fur again, and the plate and strap of his back harness no longer grow fur.
- Fixed: Novalis cave: the water of the broken pipe no longer flickers dark and bright, and with Culling On the water reflections no longer flicker or lose the cave ceiling and far walls. Reflections now show everything they can show on every level, with Culling On exactly like Culling Off.
- Fixed: Batalia: the grey haze over the sea and the island bases is gone; a light haze stays far away, as on the PS2.
- Fixed: Global illumination on levels with water: ground and walls seen through volumetric fog or smoke were taken for water and got no bounce light. They are now lit as on the other levels.
- Fixed: DirectX 12 only: far-away textures (rocks, buildings, tree tops, grass, fur) were always drawn at full sharpness, which gave over-sharp, jagged leaf edges on tree tops. They now match Vulkan and the PS2. Vulkan is unchanged.

### Known issues

- Windows: the game built by Setup does not follow the PS2 code to the last digit in gameplay. Replaying exactly the same button presses gives the same game for a moment, then tiny rounding differences (for example in Ratchet's position) grow. Linux builds are not affected. It was the same in v0.5.0; a fix is planned.
- Issue #70 (needs the reporter's exact spot), #24 (same as on the PS2) and #31 (the PS2 game has no rumble) are not changed.
- The first start of a new version pauses for about 8 seconds at the first level while shaders compile (once). On the first start after an update or a graphics driver update, fire shadows can show up a few seconds late while they are prepared.
- Some rarer fires (enemy flamethrowers, ship thruster flames, lava) do not give the new fire light yet.
- DLSS preset, Underwater look and Edge smoothing (MSAA) are set only in the RAC-RS Launcher.
- Not tried on a real Steam Deck. Not checked on the USA disc: the modern planet map for every planet, and a planet the map does not name ("????").

## v0.5.0

- This update rebuilds the whole game once, so it takes longer than usual: about 10 minutes, up to 20 on slower PCs. Your saves and settings are kept.
- Linux: update with one click from the RAC-RS Launcher, or unpack the new release over the game folder and run `sh setup.sh` again. The new RAC-RS Launcher v0.1.5 has an Achievements tab and the new graphics rows.

### New graphics options

- New: Fire, Smoke, Fur and Metal. They need Lighting set to Enhanced (Pause > Options > Graphics); then the Fur, Fur style, Metal, Fire and Smoke rows appear. Each has quality levels, so you can trade looks for speed.
- New: Fire. Burning buildings and level fires are drawn as animated flame sprites without flicker. Explosions (Bomb Glove, R.Y.N.O., Devastator and others) leave fire on the ground and burn the grass; small shots such as the Blaster leave none. The Pyrocitor is unchanged.
- New: fire lights the scene. The level's fires and the effect fire throw warm, slowly swelling and sinking light on the walls, ground, Ratchet and nearby enemies. How many fires and weapon shots give light follows the Global illumination setting (Off, Low, Medium, High). Weapon shots (Pyrocitor, Blaster, Devastator, Tesla Claw, R.Y.N.O.) light up what is nearby too.
- New: volumetric smoke. Smoke now rises in drifting, spreading columns instead of round lumps standing still, and the columns stay steady while the camera moves.
- New: shiny Metal with soft reflections and a real floor mirror. A Metal reflections slider (default 75 %) sets how strong it is. Metal costs some graphics-card time, so turn it Off on weak PCs.
- New: Fur on Ratchet, with a Fur style row: Soft (short, close to the PS2 look; the default) or Fluffy (long and plush).
- New: Upscaler row (Graphics > Resolution): Off, AMD FidelityFX Super Resolution (FSR) or NVIDIA DLSS (DLSS 4.5), with Upscale quality (Native AA, Quality, Balanced, Performance). The game is drawn smaller and scaled up to the window; menus and the HUD stay sharp. It switches without a restart, works with Vulkan and DirectX 12 (Windows only), and Off gives the same picture as before. FSR and DLSS need the vendors' own DLL files, which come with the game download; without them the row stays Off. DLSS needs an NVIDIA RTX graphics card.
- New: Display > Screen > Aspect ratio (issue #45): Auto (default, the picture fills the window as before), 4:3, 16:9, 16:10, 21:9 or 32:9. Any other shape draws the whole game (world, HUD, menus, movies, mouse) in the largest box of that shape inside the window, with black bars around it, and switches at once. 4:3 is the shape of the PS2 picture. ReShade is off while bars show. Modern menu style only.
- New: Graphics > World > Underwater look (issue #64): On (default) or Off. Off turns off our underwater effects (fog colour, light patterns on the floor, light shafts, wobble) and shows the PS2's own underwater picture; the water surface keeps your Water setting. Waves and Underwater look are greyed out while Water is Classic, with a button to switch Water to High.
- New: every Graphics row shows a "speed impact" meter, measured on the graphics card at three places in the game (not guessed).
- New: Graphics > More effects > Supporter drawings (Off by default). On = thank-you drawings for the Ko-fi supporters on the walls of the garage on Veldin, on every visit. Thank you: Scrolt, korixx, reTokyo, khalifax10, DeSynkro, DarkBlaze, Smoe05, eardrummer, Chris, Ryan Palmer, fleece, Justintime4u2bu, Ga, THUND3R_WOLF, MK, Ratchetenjoyer, Tonyswan and Luke4600.

### Menus

- New: modern menus. The title screen, pause menu (tabs, Help, Options, Goodies, Weapons / Gadgets / Items), Gadgetron vendor, Galactic Map, Save / Load, Quick Select (a ring of 8 slots with the game's own item pictures; Q / E or L1 / R1 turn it), Options > Controls (rebind keys) and Options > Mods are redrawn as modern pages. A Menu style row (Graphics) switches back to Classic, which stays as the PS2 menus.
- New: the modern pause menu has a Map tab. A click on it (or Enter / Cross) closes the pause menu and opens the modern planet map and mission list. Pause > Options (Modern style) has Display, Graphics and Game rows that open the Options screen on that tab.
- New: Quit to Desktop, the last row of Options (pause menu and title screen). It asks first, saves the achievements, then closes.
- New: mouse menus: hovering selects a row, a click selects an item at once, you can click other tabs from inside a sub-page, and click Buy, Cancel or the hint bar. Button hints show your keyboard keys on keyboard or mouse and the A/B/X/Y or PlayStation symbols only on a gamepad.
- Modern menus follow the game's language (a German save gets German menus). The new menus have texts in English, French, German, Spanish and Italian; the French, Spanish and Italian ones have not been checked by native speakers yet.
- Texture packs now also reach the modern menus, the vendor and Quick Select, and the classic vendor's item icons (issue #66).

### Gameplay and controls

- New: Camera > Strafe aiming: a fourth choice, Steady (issues #73 and #27), like Ratchet & Clank 2. While you hold the strafe aim (right mouse / R2) the camera stays at its normal height behind Ratchet and does not swing; he faces where the camera looks and side-steps or walks backwards, and there is no aim circle. Shots fly level from his chest along the camera; the Bomb Glove, Mine Glove, Devastator and R.Y.N.O. keep the game's own targeting. The default stays Raised.
- New: Camera > Aiming > Lock-on strength slider. The lock-on no longer picks targets beyond the weapon's reach (also for the Blaster with the crosshair off); 100 % is the lock as before, within the reach.
- New: Game > Gameplay > Auto-equip gadgets (issue #76), Off by default. With it On, pressing Circle (fire) at a Swingshot target, a Trespasser lock or a Hydrodisplacer pad takes out the right gadget by itself, and your weapon comes back afterwards.
- New: Game > Gameplay > Wrench throw: Classic (the original) or Moving (like Ratchet & Clank 3: you can run once the wrench has left his hand, and it flies where he aims, even while strafing backwards). Default Classic.
- New: Game > Gameplay > Quick Select pauses the game (issues #46 and #67), Off by default. With it On, enemies, shots and the world stand still while the Quick Select ring is open.
- New: the mouse turns the view in first-person (Triangle look) in both camera styles (issue #71).
- New: lower mouse lag (issue #63): the camera on screen already turns by the mouse movement the game has not used yet.
- Fixed: the oxygen meter moves to the window's edge together with the other boxes with HUD position Corners, but only when nothing else is drawn there (issue #65).
- New: Hide HUD (Debug mode, F11) also hides the mods' layer, such as Arsenal Mastery's weapon level, during gameplay.

### Achievements

- New: Achievements, 35 of them (suggested by khalifax10). They show as toasts when you earn them, and in the Achievements tab of the modern pause menu (F6 while paused in any menu style). The launcher has an Achievements tab too.

### Fixes

- Fixed: the Galactic Map could get stuck (no exit, no travel) and the game could crash on the way to or while landing on a planet. A clean-up of the game thread's call stack could run in the middle of a menu hook; it now waits for a safe moment. This could happen in earlier versions too.
- Fixed: random game sounds after a normal game start (issue #62, PAL and USA): every random sound came out the same, for example Ratchet's third-hit grunt was always the same deep one. They now vary like on the PS2 (about one third hit in three has no grunt). Saves loaded in-game were not affected.
- Fixed: gold bolt pickup (issue #69): his weapon or wrench no longer shows in his hand while he holds up a gold bolt, as on the PS2.
- Fixed: smoother picture at 120 to 240 Hz (frame pacing, issues #28 and #57).
- Fixed: R.Y.N.O.: the occasional big fire when a shot hit a wall is gone.
- Fixed: Kalebo III: the floor no longer shines far brighter than on the other planets. Drek's fleet: no more bright sun patches on the ship floors (there is no sun in space).
- Fixed: Novalis water cave: the water no longer shows a pale sky haze. Pole and thin-object shadows are fixed.
- Fixed: Switching Water to Classic while under water could leave fog, clouds, smoke and global illumination switched off on every level until Water was set back to Low or higher.
- Fixed: Galactic Map with Render scale below 100 %: the galaxy painting was drawn too big.
- Fixed: the mouse on the title screen aimed at twice the real place with DLSS / FSR on or a render scale below 100 %.
- Fixed: HD texture pack: Gemlik no longer crashes ("device lost").
- Fixed: menu sound crackle: the paused game's frozen sound level is removed before it reaches your speakers, so short Windows sound stalls no longer click.
- Fixed: Shop: a fast double click on Buy no longer buys without the confirm box. Vendor and Galactic Map: scrolling with the wheel and then moving the mouse no longer runs the list back to the top. "Buy all ammo" is always the top row of the vendor list.
- Fixed: German title Options: accented letters no longer show as boxes. Menu texts: the "speed impact" label no longer runs into its bars in German; 14 other menu text fixes.
- Fixed: modern Load Game / New Game: Up / Down and the d-pad move between save slots; save slot dates no longer show "95/00/00".
- Fixed: a menu page that stayed blank for 1.5 seconds could lock the mouse; it now falls back to the classic page.
- Fixed: split view (F10) crashed with the upscaler on.

### Known issues

- Issue #70 (needs the reporter's exact spot), #24 (same as on the PS2) and #31 (the PS2 game has no rumble) are not changed.
- With the Classic camera, any strafe aim causes a one-frame camera jump when you press it (the Raised aim has it too).
- The modern Help tab is empty; Space opens the classic Helpdesk.
- The first start of a new version pauses for about 8 seconds at the first level while shaders compile (once).
- The upscaler, Quick Select pause and Aspect ratio rows are in the modern menu style only.
- Not tried on a real Steam Deck. Not checked on the USA disc: the modern planet map for every planet, and a planet the map does not name ("????").

## v0.4.3

- Updating from v0.4.2 keeps your saves and settings.
- Linux: update with one click from the RAC-RS Launcher, or unpack the new release over the game folder and run `sh setup.sh` again. The new RAC-RS Launcher v0.1.4 fits the Steam Deck screen in Gaming Mode (launcher issue #1).
- Fixed: Qwark's TV in Blackwater City showed a still green screen instead of his face (issue #29). Clank's glass dome and the blue "G" Gadgetron signs (Aridia, Hoven, Umbris) now glow closer to the PS2.
- Fixed: the turret shots on Umbris were barely visible, and only from some angles (issue #29). Each shot is now a bright glowing streak from every angle, as on the PS2. The same rule makes the green force-field panels on Gemlik Base and the cyan bolts at Drek's Fleet a little brighter, also as on the PS2.
- Fixed: the ring waves of the Taunter enemies on Blarg were invisible (issue #43).
- Fixed: the thrown wrench vanished while you held L1 to look around (issue #51).
- Fixed: the blue underwater tint and the muffled sound could stay on after Ratchet surfaced with the Modern camera (issue #39).
- Fixed: USA disc: the sound of the movies could drift or stop, for example while the window was dragged (issue #7).
- Fixed: Rilgar at night: the clouds looked like grey heaps pasted onto the sky. They are now a thin, soft overcast with the stars showing through, like the clouds on Orxon.
- Speed: the title screen, the Options pages and the Gadgetron vendors need much less processor time, for weak PCs such as the Steam Deck (issue #50). The title screen's game code is now built from your disc like the rest of the game, and the menu and shop pictures are drawn faster. Not yet tried on a real Steam Deck.
- Fixed: Wine and Proton (Linux, Steam Deck): the game could find no graphics card and wrote no log (issue #49). Not yet tried on a real Steam Deck.
- Fixed: setup no longer hangs when a download server stops sending. After 30 seconds without data it tries again or moves on to a backup server, and on Windows the GNU assembler files can now also come from four official MSYS2 backup servers. The Linux setup uses the same 30-second rule.

## v0.4.2

- Updating from v0.4.1 keeps your saves and settings.
- Speed: v0.4.2 is a speed update for weaker PCs and laptops. The graphics card has much less work in the busy parts of every planet, and the frame rate holds steadier (fewer single slow frames). The picture and the game itself stay exactly as before.
- Speed: the game no longer draws what you cannot see. Hidden and far-away objects used to be sent to the graphics card as millions of invisible, squashed triangles, in the main view, in the water reflection and under the water. The sun shadows (Enhanced lighting) now only draw objects that can cast into each shadow area, the water skips steps whose result was thrown away anyway, and a few picture effects skip work that cannot change a pixel. The picture is the same, pixel for pixel, and the game plays exactly the same.
- Speed: fewer stutters. The HUD picture (health, bolts, menus) is sent to the graphics card by a helper thread, on NVIDIA cards the last steps of each frame also run on a helper thread (AMD and Intel graphics, and so the Steam Deck, do not get that part), Ratchet's drawing no longer stalls a frame when the camera comes very close to him or in the first seconds after the game starts, and the frames that also run a game step are cheaper in busy fights.
- Speed: steadier frames on slower PCs and laptops. A frame no longer stops to wait for the effects (water, sparks, smoke) of the last game step before it starts its own drawing work, and Pokitaru's waterfalls cost less to shade. The picture and the game stay exactly as before.
- Speed: less work in every frame with water, waterfalls or many moving objects. On open water the game no longer works out the shoreline fade for every pixel (far from the shore it always comes out the same), Pokitaru's waterfalls are matched up from frame to frame with much cheaper maths (their cost per frame went down by about 40% on a test laptop), the poses of the moving objects go to the graphics card in one copy instead of three, Ratchet's occasional self-check no longer holds up a frame, and the ground is not re-posed when it cannot have changed. The picture and the game stay exactly as before. On a test laptop (RTX 3060) the average frame rate on Pokitaru's sea went from about 94 to about 96 fps.
- Fixed: on NVIDIA graphics the game could crash while closing (the helper thread that shows the frames was stopped after the window was already gone).
- New: Linux launcher download that starts with a double-click. The RAC-RS Launcher for Linux now comes as a .tar.gz folder: extract it and double-click the launcher (no chmod, no "allow executing as program"). Installing the game is still one click in the launcher. The launcher copies itself to a safe place, so its application-menu entry keeps working when you delete the download. Linux support is still new, so please report what you find.
- New: Linux, "Add to Steam" (for the Steam Deck). Settings > Advanced > Steam in the launcher (or "sh setup.sh --add-to-steam") puts the game into your Steam library as a non-Steam game, so it starts from Gaming Mode with the controller; the same button takes it out again. Steam must be closed first. It is written but not yet tried on a real Steam Deck.
- New: Linux, script mods (like Arsenal Mastery) now work. The mod loader for Linux comes with the game.
- New: Linux, the game now tells you when a newer version is out (a notice window; update with the launcher).
- Fixed: Linux, a game that did not start gave no sign of why. The launcher now checks the system libraries the game needs before Play and names a missing one with the install line for your distribution, and the game shows a window with the reason when it stops at start (when zenity, kdialog or xmessage is installed). The launcher starts the game with its run log on, so "Open log folder" always finds the log.
- New: Linux, while the launcher builds or updates the game it keeps the PC from going to sleep, the install page estimates the build time from your processor and memory, and the screen stays on while you play. Not yet tried on every desktop.
- New: Linux, the launcher's Mods > "Import from PCSX2" now also looks in PCSX2's Linux texture folders, the Flatpak one included.
- New: "Strafe aiming" (Pause or title > Options > Camera): Raised, Shoulder or Off. Raised is the strafe aim of today and stays the default. Shoulder aims over Ratchet's shoulder. Off turns strafing off.
- New: "Quick Select mouse" (Pause > Options > Graphics, page System), off by default. Hold Triangle (T or the middle mouse button), point the mouse toward a weapon on the wheel and let go to equip it. All eight directions work and the highlight follows the mouse smoothly (issue #13).
- New: "Stick dead zone" (Pause > Options > Graphics, page System): High, Medium or Low. High is the game's own; Medium and Low react to smaller stick pushes (issue #30).
- New: "Button glyphs" (Pause > Options > Graphics, page System): the button pictures in the game's text can show PlayStation symbols (the default, as in the original), Letters (A, B, X, Y) or Letters (Switch). The pictures are drawn for this project.
- New: the title screen's Graphics page now has the same sections and rows as the pause menu's.
- New: Esc, Space, Enter or a controller's Start skip the boot logo movies at the start. A key you are already holding when the game starts does not count.
- Fixed: tutorial pop-ups show your keyboard keys when you play on keyboard (for example "C" in the camera pop-up on Kerwan), not controller buttons (issue #36). With a controller they show the controller buttons as before.
- Fixed: Load Game showed 95/00/00 as the date of every save. Saves you make from now on show the PC's date and time.
- Changed: the mouse in the pause menu. The highlight on its pages (Options, Help, Camera, Sound, Subtitles, Weapons, Items and Gadgets) follows the pointer at once instead of crawling there row by row, a click acts at once, and the Sound sliders can be clicked or dragged.
- Changed: in shops with eight or more icons the mouse does not pick items for now (use the keys or a controller); smaller shops and the pause menu pages work with the mouse.
- Fixed: a stray mouse click can no longer answer the Quit Game or the save-overwrite question by accident.
- Changed: Arsenal Mastery 0.3.4 (preinstalled, off): the setting and the preset called "Up Your Arsenal" are now called "Standard" and "Default". Nothing else changes.

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
- Fixed: with Culling Off, objects the game hides on purpose stay hidden (on Veldin, grey wall blocks appeared).

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
