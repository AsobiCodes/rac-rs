# Arsenal Mastery

Ratchet & Clank 3 style weapon levels for Ratchet & Clank 1. Every weapon, and the wrench, earns XP from the damage it deals and grows through levels V1 to V5. The enemies are rebalanced so the game stays a fight. XP is saved per save slot (a new game starts at zero, New Game+ keeps what you had). It needs a game with the OmniLoader extension: on another loader the mod logs one line and does nothing. The mod reads the game only through the named fields of the port's binding; a perk whose fields a port does not name yet is switched off with one log line, the rest of the mod keeps working.

## XP and levels
- A weapon earns 10 XP for every point of health it takes off an enemy (overkill does not count), plus a bonus for the last hit. A 3-health enemy killed by one weapon gives 45 XP. Crates do not count.
- Levels (XP, before the XP rate): V2 750, V3 2,250, V4 5,000, V5 10,000. Weapons you get late in the game need less (three quarters, or half for the last ones), so they can catch up. Levelling is slow on purpose: expect to reach V5 with your favourite weapons, not with all of them, in one playthrough.
- Each level makes the weapon hit harder: about x1.2 at V2, x1.5 at V3, x1.9 at V4 and x2.5 at V5.
- The Weapons page of the pause menu shows every weapon's level and XP bar, and the wrench's in the header. When a weapon levels up, its icon and the new level pop up on screen for a moment.

## New Game+ and Endless
- In New Game+ weapons can go on to V6 Mega, V7 Giga and V8 Omega (Mega is bought, see below; the five weapons without a gold version earn it with XP).
- With the Endless level cap, levels go up to V99, each one a little stronger and a little slower to earn.
- Enemies are tougher in New Game+ (they take 2.5x less damage than in the first playthrough), so Mega to Omega weapons still have a fight; with Nanotech levels on, their hits also take twice as much health.

## Nanotech levels
Health works as in Ratchet & Clank 3. You start with 10 health, and every hit you score with any weapon or the wrench earns nanotech XP, a third of what the weapon gets (10/3 per point of enemy health). Each level adds 1 health and fills it at once, as in Ratchet & Clank 3. Levels come often early and slower later: 20 health at 2,000 XP, 30 at 5,000, 50 at 14,500, 70 at 30,000 and the most, 100 health, at 72,500. In New Game+ it goes on to 200 health, 1,100 XP a level. The Premium and Ultra Nanotech upgrades you buy on Orxon still count: x1.25 and x2 health.
- Enemy hits take more health the further you are in the game: 3 on Veldin, about 10 on Eudora, 20 on the last planets (x2 in New Game+, x1.3 with Hard rebalance). A falling or crushing death is still a death.
- The orbs are replaced by the number: the nanotech icon and your health, with a bar of what is left, where the orbs were. It turns red under a quarter. A nanotech pickup gives back the share of one orb, a death refills it all.
- Your nanotech XP is saved per save slot (a new game starts again at 10 health, New Game+ keeps it).

## Enemy rebalance
Enemies take less damage the further you are in the game, to match your levelled weapons: Off, Standard, or Hard (stronger enemies, and every second hit on Ratchet costs one more health orb, or, with Nanotech levels, enemy hits take 30% more health).

## Settings
XP rate (0.5x to 3x), Level cap, Enemy rebalance, Weapon power from levels, Gold version at V5, Extra projectiles and perks, Ammo, XP bars and level-up icons, Nanotech levels, and "Start every weapon at" (a sandbox option for trying high levels).

Advanced (in the launcher's Mod settings, under "Advanced"), each x0.25 to x4, x1.0 = as designed:
- **Nanotech XP rate**: how fast nanotech levels come (weapon XP keeps its own XP rate).
- **Weapon damage**: the damage your weapons and the wrench deal, on top of weapon levels and the rebalance.
- **Damage you take**: the damage enemy hits do to Ratchet. With Nanotech levels it scales the health a hit takes; with the orbs, the part of an orb carries over (x0.5 = every second hit is free, x1.5 = 1, 2, 1, 2 orbs, x2 = two orbs a hit).

The settings are saved in the game's `mods\settings.json` under `"arsenal-mastery"` (the launcher writes it; you can also edit it by hand while the game is closed), for example:
```json
{"format":1,"mods":{"arsenal-mastery":{"xp_rate":1.5,"nanotech_xp_rate":2.0,"weapon_damage":1.25,"enemy_damage":0.75}}}
```
A setting you leave out, or a value of the wrong kind or outside its range, uses its default.

Presets: **Up Your Arsenal** (the defaults), **Just the XP** (bars and XP only, nothing in the game changes, the game's own orbs), **Survival** (Hard enemies and Survival ammo), **Endless** (V99 cap).

## Gold weapons (V5)
With "Gold version at V5" on, an owned weapon that has a gold version in the game turns gold when it reaches V5: gold icon, gold name, gold glow and the game's own gold stats, all together. A gold version earned this way costs no gold bolts, so every gold bolt you found stays free for Mega. Gold weapons you bought in the game yourself are never touched. Turn the setting off and the gold the mod gave is taken back (the ones you bought stay).

## Mega at the Gold Weapon Cases (New Game+)
In New Game+, the gold weapon case of a weapon that is at V5 offers Mega, bought the same way you buy a gold weapon in the original game: you need the bolts and 4 spare gold bolts. The case, the talk prompt and the fanfare are the game's own, so the offer text is the game's gold text. The price in bolts is the weapon's normal gold price (10,000 to 60,000), and every Mega uses up 4 of your gold bolts: the 40 gold bolts in the game pay for all ten. The Gold Bolts page of the pause menu shows what is left. A case that was already empty when you walked in (for example after you bought the gold weapon the normal way in this room) opens again after you leave the room and come back. The five weapons with no case (Taunter, Walloper, Visibomb, Drone Device, R.Y.N.O.) reach Mega by XP instead. Mega needs "Weapon power from levels", "Gold version at V5" or "Extra projectiles (perks)" to be on.

## Ammo capacity and Survival
With "Weapon power from levels" on, a weapon holds more ammo as it levels: 125 percent from V3, 150 percent from V5, 175 percent from V6. With "Ammo: Survival", ammo crates give half of their ammo (at least 1), every kill gives 1 shot back to the weapon you used (not the Glove of Doom: its agents kill by themselves), and vendors sell the full amount as always. With "Normal" crates and kills work as in the original game.
