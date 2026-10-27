# Changelog

## v0.1.1
- Updating now offers to back up your current version first. The tick "Back up your current version first" is on by default (the choice is remembered): the old version is copied next to your install, shows up as its own entry in the Library (marked "Backup"), plays at once with its own saves and settings, and never updates. Delete it in the Library when you do not need it ("Delete backup", asks twice, goes to the Recycle Bin).
- Texture packs are not copied again: when the drive allows it they are shared with the backup (hard links), so a 6 GB pack costs no extra space. On a drive that cannot share files the pack is left out and the backup plays with the original textures.
- If the drive has not enough room, the tick is greyed out with the reason and the update runs without a backup. A backup can be cancelled while it is made: it is removed and the update is not started. If the backup fails, the update is not started and nothing is changed.
- If an update fails after the backup was made, the error screen offers "Play the backup (vX)".
- Script mods (like Arsenal Mastery) show on the Mods page under "Script mods", next to "Texture packs": a switch to turn each on or off, priority arrows, and "Mod settings" with the mod's own settings (switches, choices, sliders and number fields, text fields, key bindings that listen for the next key or mouse button, an "Advanced" group), preset buttons and "Reset to defaults". Values are saved to `mods\settings.json` (kept per install, other mods' and unknown entries are never lost); on/off goes to the profile's `active.txt` by the mod's `id`. A mod whose omni-mod.json has a problem is listed with the reason and cannot be switched on. The "Load mods" switch governs script mods. Needs game version 0.4.0 (older versions grey the switch with the reason). Changes apply the next time the game starts.
- Mod rows put their buttons under the text when the window is narrow, so a long description never runs under them.
- Settings: a new "Mouse speed" row under "Camera speed" (needs game version 0.4.0). From game version 0.4.0 on, Camera speed sets the right stick only and the mouse has its own speed (50 = the mouse speed you had with the default camera speed).
- Settings: a new "Skip intro (go straight to the title screen)" row under Gameplay (On / Off, default Off). It writes `intro=skip` or `intro=play` into the profile's `pc_options.txt`; other lines of the file are kept. Needs game version 0.4.0 (older versions grey it with the reason).
- A small "Support the project" link (a heart) at the bottom of the left column opens the project's Ko-fi page in your browser when you click it. Nothing opens by itself.

## v0.1.0
First release.
- Install the PC port of Ratchet & Clank from your own disc: the launcher downloads the game release from AsobiCodes/rac-rs (SHA-256 checked), asks for your disc image (.iso, PAL or USA v1.00) and builds the game once (about 15 minutes).
- Library of installs: several game versions side by side (from 0.2.1), each keeps its version. Profiles per install, each with its own settings and mods.
- Settings page: the game's options, plus "Graphics quality" presets Low / Medium / High / Custom. High is the game's own default picture; Low and Medium turn the costly options down (draw distance, frame smoothing, HUD sharpness, shadows, water, particles and so on). A preset never touches camera, controls, language, window mode or the graphics system; change any option afterwards and the selector shows Custom.
- Mods page ("bring your own pack"): add a texture pack as a folder or .zip, by drag and drop, or import it from your PCSX2 texture folder. Big packs (6 GB and more) are read and copied in the background with a progress bar and Cancel. Per-pack switches and priority order, plus two master switches, "Load texture packs" and "Load mods", that turn a whole kind off without losing the per-pack choices.
- A pack made for the other disc region is marked: most world textures still match; menus, HUD and text may stay original.
- Options and features an older game version does not have are greyed out (texture packs and the graphics-system choice need game version 0.3.5).
- "Open saves folder" and "Open log folder" (with "Report a problem"): the log folder opens with `log.txt` selected, the file to attach to a bug report.
- Updates: the launcher checks for a new launcher and a new game version and asks before it installs either.
- Windows first; Linux groundwork only (it compiles, there is no Linux install yet).
