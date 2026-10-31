# Bolt Multiplier

Ratchet & Clank 2-style bolt multipliers for Ratchet & Clank 1. Defeat enemies without taking damage to build a multiplier, then earn extra bolts as you collect them. Taking damage resets the streak. A compact bolt icon and multiplier appear beneath the game's bolt counter when you collect bolts, and disappear after a short time. The mod uses the RAC-RS OmniLoader extension and supports PAL and NTSC.

## Kills and multipliers

* Every 5 consecutive enemy kills raises the multiplier by one: 5 kills = x2, 10 = x3, 15 = x4, and so on. The default maximum is x20, reached at 95 kills. Both the kills-per-level interval and maximum multiplier can be changed in Mod Options.
* Taking actual damage resets the streak and multiplier to x1. Invulnerable or zero-damage hits do not reset it.
* The multiplier applies to eligible positive bolt gains during gameplay. At x4, collecting 10 bolts awards 40 instead. Disabling **Enable bolt multiplier** stops the bonus awards but leaves kill tracking and the display active.
* The streak resets on level and save transitions; it is not stored in the save file.

## On-screen display

The multiplier appears with the game's bolt icon below the regular bolt counter. It is hidden at x1 and appears briefly when your bolt count increases. The multiplier flashes when it levels up. The position, size, spacing and translucent near-black background are tuned to match the native HUD and are not exposed as settings.

## Settings

The launcher's Mod Options expose:

* **Kills per multiplier level** (default 5, range 1–100): kills needed for each increase.
* **Maximum multiplier** (default x20, range x1–x30): the highest multiplier allowed.
* **Enable bolt multiplier** (on): grant extra bolts, or track streaks without changing the balance.
* **Largest eligible bolt gain per tick** (default 250, range 1–10,000): gains above this value are not multiplied.
* **Show multiplier HUD** (on): show or hide the popup.
* **HUD display duration (ticks)** (default 100, range 30–600): how long the popup stays visible after a bolt gain.

## Bolt pickup detection

RAC-RS currently exposes the bolt balance but not a dedicated bolt-pickup event to mods. The mod detects increases in the balance from one frame to the next, adds the multiplier bonus to eligible increases during gameplay, and ignores gains larger than the configured safety limit. This means unusually large legitimate pickups may not receive a bonus, while some scripted rewards during gameplay could still qualify. The limit is a safeguard, not a guaranteed way to distinguish pickups from scripted awards.

## Installation

Place the `bolt-multiplier` directory in the RAC-RS `mods` folder and enable **Bolt Multiplier** in the launcher. The mod requires an RAC-RS build with the OmniLoader APIs used for enemy detection, player damage, bolt balance and HUD drawing. Settings are managed through the launcher's Mod Options.
