# Modding reference: hook ids, game functions, fields

Companion to [MODDING.md](MODDING.md). These are the names a script mod can use on Ratchet & Clank 1 in RAC-RS. They come from the mod binding that ships with the game (`omni\rac1.json` is built into the exe; a copy next to the exe, if present, wins). The binding of v0.4.0 and the one of v0.4.1 are the same, so every table below is valid from v0.4.0.

Counts: 17 base hook points, 57 `rac1.omni.*` hook points, 136 moby update hook points, 15 callable game functions, 95 fields, 23 record types.

How to read the tables:

- `u32` and `i32` are whole numbers (an address in game memory is a `u32`), `f32` is a decimal number, `void` means "returns nothing". A signature like `void(f32,u32)` reads "returns nothing, takes a decimal and a whole number".
- A hook function gets the arguments in the order shown. With `omni.hook` the function also gets `original` in front of them: `function(original, a, b)`.
- **moby** = an object that lives in the level and updates itself every tick (enemy, crate, shot, your held weapon). A moby argument is its address, a number. It is good only until the end of the tick; check it with `omni.moby_valid(addr)` before you write to one you kept.
- A hook point can fire only on levels that contain that game function (weapon code, for one, lives in the level data). `omni.has("<id>")` asks whether this build can hook it on the disc in use. List the ids you take in your manifest under `hooks` (required) or `optional_hooks` (the mod loads without them).
- Disc differences are handled for you: PAL and NTSC use the same ids, field names and record names.

Contents: [Base hook points](#base-hook-points) - [rac1.omni hook points](#rac1omni-hook-points) - [Moby update hook points](#moby-update-hook-points) - [Game functions](#game-functions) - [Fields](#fields) - [Records](#records-for-omnistruct) - [Skeleton joints](#skeleton-joints-for-pose-overrides) - [Events, modes, pause pages, icons](#events-modes-pause-pages-icons)

## Base hook points

The same ids as on OpenRAC, so a mod that uses only these can run on both. They are stable.

| Id | Signature | Parameters | What the game does there |
|---|---|---|---|
| `rac1.pad.process_report` | `void(u32,u32,i32)` | pad, report, size | One pad report into the pad's state (PadState, at pad), each poll, before the tick reads it. |
| `rac1.player.update_motion_input` | `void()` |  | The stick read into the hero, early in his update each tick. |
| `rac1.player.turn_to_heading` | `void(i32,f32,f32,f32)` | mode, accel, damping, limit | Eases the hero's facing toward his move yaw. |
| `rac1.player.set_planar_velocity` | `void(f32)` | angle | The hero's velocity at his ground speed along an angle; an angle above pi means his facing. |
| `rac1.camera.follow.apply_overrides` | `void(u32)` | camera | The follow camera's per-tick requests from the hero's state; the camera moby's update calls it before it places the eye. |
| `rac1.player.advance_animation` | `void()` |  | Ratchet's animation moved on by his animation speed, once a tick after his state's update. |
| `rac1.item.give` | `void(i32,i32)` | id, equip | Gives the player item id: sets its owned and obtained bytes, shows its banner, gives its starting ammo, marks it bought in the vendor's stock and adds it to the quick select; equip non-zero ... |
| `rac1.planet.unlock` | `void(i32)` | planet | Unlocks a planet's coordinates: its flag (one byte per level), the galaxy list, and its banner unless it is the planet the player is on. |
| `rac1.vendor.init_level_stock` | `void()` |  | The vendor's stock at each level start: gives the Bomb Glove while the player has none, keeps the stock's known items and adds the level's new weapon. |
| `rac1.vendor.build_item_list` | `void(i32)` | skip_unflagged | The vendor menu's list, from the stock and the items owned: each time the menu opens and after each purchase. skip_unflagged leaves out what is for sale (the menu opened from the pause ... |
| `rac1.gold_bolt.count_collected` | `i32()` |  | The gold bolts collected, at most 40: what the pause screen shows; the unspent ones are this less 4 per gold weapon. |
| `rac1.player.take_damage` | `void(i32)` | amount | Takes hit points off the hero's health (Hero_TakeDamage): at most one a call, none for 0, then the HUD's health bank. |
| `rac1.combat.get_hit` | `u32(u32,i32,i32)` | moby, mask, keep | The hit pending on moby: the record of the last hit posted to it when its flags match mask, else 0, dropping the hit unless keep. |
| `rac1.weapon.take_ammo` | `i32(i32,i32)` | item, amount | Takes amount of ammo from item for a shot: 1 when taken, 0 when there is too little. |
| `rac1.level.load_gameplay` | `u32(i32)` | full | Loads a level's gameplay file into the level heap, once per level start: the settings, the placed mobys and their pvars, the spare moby pool, the paths and the rest; returns the heap cursor. |
| `rac1.player.approach_speed` | `void(f32,f32)` | up, down | Eases the hero's ground speed toward his target speed by up when below it and by down when above, each tick he moves on foot, in every level. |
| `rac1.gold_bolt.count_spare` | `i32()` |  | The gold bolts still to spend: clamp(collected - 4 * gold weapons owned, 0, 40), where collected = rac1.gold_bolt.count_collected and gold weapons = the nonzero bytes of the gold byte table. |

## rac1.omni hook points

Added by RAC-RS and OmniLoader for gameplay mods (weapons, ammo, wrench, damage, shots, vendor, HUD). Only RAC-RS has them, so take them as `optional_hooks` or guard with `omni.has` if the mod should also load elsewhere. Several of these ids were added late: the id list below is what v0.4.0 ships.

| Id | Signature | Parameters | What the game does there |
|---|---|---|---|
| `rac1.omni.damage.build_hit` | `void(f32,u32,u32,u32,u32)` | damage, desc, attacker, mask, dir | Builds the hit description (damage, attacker, direction) for an attack that hurts an enemy. Call the original with another damage to change what is dealt. |
| `rac1.omni.damage.hit_victim` | `void(u32,u32)` | victim, desc | Posts a hit on a victim moby (victim, description). Runs for every hit on an enemy. |
| `rac1.omni.wrench.swing` | `u32(u32,u32,u32,u32,u32,u32,i32)` | prev_a, prev_b, cur_a, cur_b, owner, attack, steps | The Omniwrench swing sweep: finds what the swing hits. Returns 1 when something was hit. |
| `rac1.omni.wrench.throw` | `void()` |  | Comet Strike: the wrench is aimed and thrown. |
| `rac1.omni.wrench.update` | `void(u32)` | moby | Per-tick update of the held Omniwrench (combo state, swing damage). |
| `rac1.omni.weapon.suck_cannon.update` | `void(u32)` | moby | Per-tick update of the held Suck Cannon. |
| `rac1.omni.weapon.bomb_glove.update` | `void(u32)` | moby | Per-tick update of the held Bomb Glove. |
| `rac1.omni.weapon.devastator.update` | `void(u32)` | moby | Per-tick update of the held Devastator. |
| `rac1.omni.weapon.visibomb.update` | `void(u32)` | moby | Per-tick update of the held Visibomb Gun. |
| `rac1.omni.weapon.taunter.update` | `void(u32)` | moby | Per-tick update of the held Taunter. |
| `rac1.omni.weapon.blaster.update` | `void(u32)` | moby | Per-tick update of the held Blaster. |
| `rac1.omni.weapon.pyrocitor.update` | `void(u32)` | moby | Per-tick update of the held Pyrocitor. |
| `rac1.omni.weapon.mine_glove.update` | `void(u32)` | moby | Per-tick update of the held Mine Glove. |
| `rac1.omni.weapon.walloper.update` | `void(u32)` | moby | Per-tick update of the held Walloper. |
| `rac1.omni.weapon.tesla_claw.update` | `void(u32)` | moby | Per-tick update of the held Tesla Claw. |
| `rac1.omni.weapon.glove_of_doom.update` | `void(u32)` | moby | Per-tick update of the held Glove of Doom. |
| `rac1.omni.weapon.morph_o_ray.update` | `void(u32)` | moby | Per-tick update of the held Morph-o-Ray. |
| `rac1.omni.weapon.ryno.update` | `void(u32)` | moby | Per-tick update of the held R.Y.N.O. |
| `rac1.omni.weapon.decoy.update` | `void(u32)` | moby | Per-tick update of the held Decoy Glove. |
| `rac1.omni.ammo.pickup` | `void(u32)` | moby | An ammo crate is picked up (called with the pickup moby). Its amount and ammo item are in its variables. |
| `rac1.omni.ammo.add` | `i32(i32,i32)` | item, amount | Adds ammo to an item and caps it at the item maximum; returns the overflow. |
| `rac1.omni.weapon.select` | `void(u32)` | slot | Equips the requested item into hero slot `slot` (Square = wrench, Circle = last weapon, quick-select commit). |
| `rac1.omni.weapon.quick_select` | `void(u32)` | state | Per-tick update of the quick-select wheel. |
| `rac1.omni.vendor.open` | `void(u32)` | vendor | Opens the vendor (Gadgetron PDA or ammo shop). |
| `rac1.omni.vendor.buy` | `void(u32)` |  | The vendor purchase state machine, once per tick while a shop is open. |
| `rac1.omni.menu.weapons_page.update` | `void(u32)` | page | Pause menu, Weapons page: per-tick update of the item model it shows. |
| `rac1.omni.menu.weapons_page.draw` | `void(u32)` | page | Pause menu, Weapons page: draws the page. |
| `rac1.omni.damage.resolve` | `u32(u32,u32,u32,u32,u32,u32,u32,u32)` | victim, entry, component, arg3, status_out, strength_out, extra_out, mode | Resolves a hit on a target. Returns the reaction class, not the strength. |
| `rac1.omni.weapon.flame.create` | `u32(f32,u32,u32,u32)` | range, pos, vel, flags | Creates a Pyrocitor flame (the held weapon calls it several times a tick). |
| `rac1.omni.weapon.bomb_glove.fire` | `void(u32,u32,u32)` | pos_ptr, bomb, pvar | The Bomb Glove fires a bomb. |
| `rac1.omni.weapon.mine_glove.fire` | `void(u32,u32,u32)` | pos_ptr, mine, pvar | The Mine Glove fires a mine. |
| `rac1.omni.weapon.glove_of_doom.fire` | `void(u32,u32,u32)` | pos_ptr, capsule, pvar | The Glove of Doom fires an agent. |
| `rac1.omni.weapon.devastator.fire` | `u32(u32,u32,u32,f32,f32,f32,f32,f32)` | pvar_arg, pos, owner, f0, f1, f2, f3, f4 | The Devastator fires a rocket. |
| `rac1.omni.weapon.visibomb.fire` | `u32(f32,f32,u32,u32)` | f0, f1, owner, dir | The Visibomb Gun fires a missile. |
| `rac1.omni.weapon.blaster.fire` | `u32(f32,f32,f32,f32,u32,u32,u32)` | f0, f1, f2, f3, iarg, vec, target | The Blaster fires a shot. |
| `rac1.omni.moby.create` | `u32(u32)` | class | Creates a moby of a class. Returns it, or 0 when the pool is full. |
| `rac1.omni.moby.delete` | `void(u32)` | moby | Removes a moby (for example a projectile dying). |
| `rac1.omni.projectile.bomb.update` | `void(u32)` | moby | Per-tick update of a Bomb Glove bomb. |
| `rac1.omni.projectile.mine.update` | `void(u32)` | moby | Per-tick update of a Mine Glove mine. |
| `rac1.omni.projectile.blaster_shot.update` | `void(u32)` | moby | Per-tick update of a Blaster shot: it steers to its aim point and moves. |
| `rac1.omni.projectile.ryno_rocket.update` | `void(u32)` | moby | Per-tick update of an R.Y.N.O. rocket. |
| `rac1.omni.projectile.doom_agent.update` | `void(u32)` | moby | Per-tick update of a Glove of Doom agent. |
| `rac1.omni.projectile.doom_capsule.update` | `void(u32)` | moby | Per-tick update of a Glove of Doom capsule. |
| `rac1.omni.projectile.visibomb.update` | `void(u32)` | moby | Per-tick update of a Visibomb missile. |
| `rac1.omni.projectile.devastator_rocket.update` | `void(u32)` | moby | Per-tick update of a Devastator rocket: it steers to its target. |
| `rac1.omni.collide.segment` | `u32(u32,u32,u32,u32,u32)` | start, end, mask, self, attack | The game's line-against-world test (used by shots, the camera and the hero). It runs very often: keep a hook on it tiny. |
| `rac1.omni.collide.sphere_mobys` | `u32(f32,u32,u32,u32,u32)` | radius, pos, mask, self, filter | Finds the mobys inside a sphere. Blasts call it right before blast.apply. |
| `rac1.omni.blast.apply` | `void(f32,f32,f32,u32,u32,u32,u32,u32,u32,u32,u32)` | strength, hscale, vimpulse, attacker, pos, list, count, exclude, mask, cat, sub | Applies a blast: posts one hit on every moby in the list. |
| `rac1.omni.fx.explode` | `void(f32,f32,f32,f32,f32,f32,u32,u32,u32)` | radius, size, f2, f3, f4, f5, dmg_bits, self, pos | Starts an explosion effect. |
| `rac1.omni.weapon.devastator.retarget` | `void(u32,u32,u32)` | rocket, pvar, ignore | A Devastator rocket picks a new target. |
| `rac1.omni.weapon.blaster.aim_search` | `u32(u32,u32,u32,u32)` | a0, a1, a2, a3 | The Blaster's lock-on search. |
| `rac1.omni.weapon.ryno.fire` | `u32(f32,f32,f32,u32,u32,u32)` | height, yaw, pitch, owner, pos, target | R.Y.N.O. fires a rocket (returns the rocket moby). |
| `rac1.omni.projectile.hit_face_id` | `u32()` |  | The surface id (0 to 31) of the last line-test hit. |
| `rac1.omni.goldcase.update` | `void(u32)` | moby | Per-tick update of a Gold Weapon Case (the seller of gold weapons). |
| `rac1.omni.hero.items_attach` | `void()` |  | Attaches every item the hero holds (weapons) to his hands. |
| `rac1.omni.ammo.collect` | `u32(u32)` | moby | Ammo is collected from a pickup. A mod that changes pickup ammo wraps this and ammo.pickup the same way. |
| `rac1.omni.hud.health_draw` | `u32(u32)` | element | Draws the nanotech meter. A mod that draws its own health skips the original and returns the element's `result` (read the element with `omni.struct("hud_health", element)`). |

## Moby update hook points

`rac1.moby.<class>.update` runs for one kind of object each tick (experimental: names and behaviour may change). Signature `void(u32)`, parameter `moby`, unless noted. The classes:

`record_death`, `amoeboid.update`, `big_axebot.update`, `birdbot.update`, `blarg_bomb_thrower.update`, `blarg_commander.update`, `blarg_gun.update`, `blarg_space_commando.update`, `blarg_trooper.update`, `blarg_turret.update`, `bomber_tank.update`, `circular_saw.update`, `edwina.update`, `extermibot.update`, `extermitank.update`, `fleet_turret.update`, `floating_enemy.update`, `gem_lock_turret.update`, `grazing_critter.update`, `guide.update`, `helicopter_gunship.update`, `horny_toad.update`, `infobot.update`, `linked_turret.update`, `machinegun_turret_1126.update`, `machinegun_turret_30.update`, `minibomber.update`, `moby1048.update`, `moby1068.update`, `moby1196.update`, `moby1246.update`, `moby1267.update`, `moby1290.update`, `moby1382.update`, `moby211.update`, `moby294.update`, `moby326.update`, `moby52.update`, `moby573.update`, `moby90.update`, `mountable_turret.update`, `nanotech_ball.update`, `paratrooper.update`, `path_follow_moby.update`, `peckbot.update`, `plasmabot.update`, `psytcopus.update`, `quest_giver.update`, `robosquawker.update`, `sand_shark.update`, `scoutbot.update`, `sentrybot.update`, `sharkigator_1059.update`, `sharkigator_35.update`, `spiderbot.update`, `toxic_crab.update`, `trooper.update`, `wandering_helper.update`, `wb_moby.update`, `airship_chain_link.update`, `airship_part.update`, `alien_queen.update`, `batalia_bolt_thief.update`, `blarg_fighter111.update`, `blarg_interceptor.update`, `blarg_ship.update`, `blarg_space_fighter.update`, `bolt_thief.update`, `child_spawner.update`, `codebot_slot.update`, `commando.update`, `crate_platform.update`, `deserter.update`, `doom_bot.update`, `door_switch.update`, `drone.update`, `fighter_jet_1379.update`, `fighter_jet_69.update`, `final_boss.update`, `floating_mine.update`, `flying_craft.update`, `frog.update`, `fx_group.update`, `gadgebot.update`, `grass.update`, `gunship.update`, `helicopter.update`, `hoven_homing_mine.update`, `jet_fighter.update`, `kerwan_mine.update`, `lobber_enemy.update`, `mine.update`, `mine_l14.update`, `miner.update`, `moby1178.update`, `moby1264.update`, `moby1302.update`, `moby1428.update`, `moby1434.update`, `moby1454.update`, `moby1568.update`, `moby1805.update`, `moby1843.update`, `moby1885.update`, `moby1886.update`, `moby1906.update`, `moby250.update`, `moby302.update`, `moby303.update`, `moby336.update`, `moby568.update`, `moby604.update`, `moby612.update`, `moby755.update`, `moby77.update`, `moby8.update`, `moby83.update`, `moby914.update`, `mouse_critter.update`, `near_hero_sparkle.update`, `path_vehicle.update`, `qwark_starfighter_part.update`, `rocket_door1.update`, `rocket_launcher.update`, `sand_mouse.update`, `sand_shark_nest.update`, `sentrybot_alarm.update`, `shard_debris.update`, `snagglebeast.update`, `spawn_child_and_delete.update`, `starfighter_part.update`, `swooper.update`, `tethered_mine.update`, `timed_shooter.update`, `ultramech.update`, `vendor.update`

Different signature: `rac1.moby.record_death` is `void(u32,i32,i32)`.

## Game functions

Callable from `on_frame`, a hook or an event with `omni.fn(name, signature)` (see MODDING.md, "Calling game functions"). A hook id can also be the target: the call then runs the whole hook chain like a call from the game.

| Name | Signature | What it does |
|---|---|---|
| `hit_desc_build` | `void(f32,u32,u32,u32,u32)` | Fills a hit description: damage, description address, attacker moby, mask, direction vector address. |
| `hit_ring_write` | `void(u32,u32)` | Posts a hit description on a victim moby's hit queue. |
| `flame_create` | `u32(f32,u32,u32,u32)` | Creates a Pyrocitor flame effect. |
| `tesla_chain` | `void(u32)` | Tesla Claw chain helper: picks targets and draws the lightning chain (takes the Tesla moby). |
| `add_ammo` | `i32(i32,i32)` | Adds ammo to an item; returns the overflow above the maximum. |
| `get_ammo` | `i32(i32)` | The ammo of an item. |
| `rand_vec` | `void(u32)` | Writes a random vector (4 floats) at the address given, using the game's own random numbers. |
| `create_moby` | `u32(u32)` | Spawns a moby of a class in the dynamic pool and returns it (0 when the pool is full). |
| `delete_moby` | `void(u32)` | Removes a moby; its slot can be reused after 2 ticks. |
| `target_component` | `u32(u32)` | Lock-on helper (no further description yet). |
| `blaster_aim_search` | `u32(u32,u32,u32,u32)` | The Blaster's own lock-on: looks for a target from an origin and a heading. |
| `spark_spawn` | `u32(u32,u32,u32,u32,u32)` | Spawns a spark effect (no further description yet). |
| `glow_flare_spawn` | `u32(f32,u32,u32,u32,u32,u32,u32,u32,u32)` | Spawns a glow sphere (the flash of a bomb blast). |
| `moby_cue` | `i32(i32,u32,u32)` | Plays a sound cue of the moby's class on it. |
| `moby_attach_to_joint` | `void(u32,u32,u32)` | Writes the world matrix of an attach point of a moby's model to an address you give. |

## Fields

`omni.fields.<path>` reads and writes game state by name. A dotted name is a path: `omni.fields.omni.weapon.blaster.ammo`. Most fields need a level to be loaded: a read or write fails on the title screen and between levels, so use `pcall` there.

| Field | Type | Notes |
|---|---|---|
| `hero.position.x` | f32 | Where the hero is. |
| `hero.position.y` | f32 |  |
| `hero.position.z` | f32 |  |
| `hero.facing` | f32 | The way he faces, in radians. |
| `hero.move_yaw` | f32 | Where the stick moves him: the stick's angle plus the camera's yaw. |
| `hero.state` | i32 | His state: 0 standing, 2 running, 7 jumping, ... |
| `bolts` | i32 | The player's bolts. |
| `omni.weapon.equipped` | i32 | The item id in the hero's first item slot. |
| `omni.weapon.quick_select` | i32 | Quick-select wheel entry 0: an item id (0 = empty). |
| `omni.weapon.quick_select.1` | i32 | Quick-select wheel entry 1: an item id (0 = empty). |
| `omni.weapon.quick_select.2` | i32 | Quick-select wheel entry 2: an item id (0 = empty). |
| `omni.weapon.quick_select.3` | i32 | Quick-select wheel entry 3: an item id (0 = empty). |
| `omni.weapon.quick_select.4` | i32 | Quick-select wheel entry 4: an item id (0 = empty). |
| `omni.weapon.quick_select.5` | i32 | Quick-select wheel entry 5: an item id (0 = empty). |
| `omni.weapon.quick_select.6` | i32 | Quick-select wheel entry 6: an item id (0 = empty). |
| `omni.weapon.quick_select.7` | i32 | Quick-select wheel entry 7: an item id (0 = empty). |
| `omni.weapon.bomb_glove.max_ammo` | u16 | Bomb Glove (item 10): the most ammo it can hold. |
| `omni.weapon.devastator.max_ammo` | u16 | Devastator (item 11): the most ammo it can hold. |
| `omni.weapon.visibomb.max_ammo` | u16 | Visibomb Gun (item 13): the most ammo it can hold. |
| `omni.weapon.blaster.max_ammo` | u16 | Blaster (item 15): the most ammo it can hold. |
| `omni.weapon.pyrocitor.max_ammo` | u16 | Pyrocitor (item 16): the most ammo it can hold. |
| `omni.weapon.mine_glove.max_ammo` | u16 | Mine Glove (item 17): the most ammo it can hold. |
| `omni.weapon.tesla_claw.max_ammo` | u16 | Tesla Claw (item 19): the most ammo it can hold. |
| `omni.weapon.glove_of_doom.max_ammo` | u16 | Glove of Doom (item 20): the most ammo it can hold. |
| `omni.weapon.ryno.max_ammo` | u16 | R.Y.N.O. (item 23): the most ammo it can hold. |
| `omni.weapon.drone_device.max_ammo` | u16 | Drone Device (item 24): the most ammo it can hold. |
| `omni.weapon.decoy.max_ammo` | u16 | Decoy Glove (item 25): the most ammo it can hold. |
| `omni.menu.weapons_page.highlight_class` | u32 | Pause menu, Weapons page: the moby class of the 3D model of the highlighted item. |
| `omni.hud.ammo_box.max` | u32 | The HUD ammo box: the maximum it shows right now, the N of the printed "count/N". |
| `omni.hud.ammo_box.def_max` | u32 | The HUD ammo box: the maximum the game compares when it re-requests the box every tick. |
| `omni.hud.ammo_box.id` | u32 | Read-only for mods; check it before writing max / def_max. |
| `omni.weapon.suck_cannon.gold_price` | u16 | Suck Cannon (item 9): the price of its gold version in bolts. |
| `omni.weapon.bomb_glove.gold_price` | u16 | Bomb Glove (item 10): the price of its gold version in bolts. |
| `omni.weapon.devastator.gold_price` | u16 | Devastator (item 11): the price of its gold version in bolts. |
| `omni.weapon.blaster.gold_price` | u16 | Blaster (item 15): the price of its gold version in bolts. |
| `omni.weapon.pyrocitor.gold_price` | u16 | Pyrocitor (item 16): the price of its gold version in bolts. |
| `omni.weapon.mine_glove.gold_price` | u16 | Mine Glove (item 17): the price of its gold version in bolts. |
| `omni.weapon.tesla_claw.gold_price` | u16 | Tesla Claw (item 19): the price of its gold version in bolts. |
| `omni.weapon.glove_of_doom.gold_price` | u16 | Glove of Doom (item 20): the price of its gold version in bolts. |
| `omni.weapon.morph_o_ray.gold_price` | u16 | Morph-o-Ray (item 21): the price of its gold version in bolts. |
| `omni.weapon.decoy.gold_price` | u16 | Decoy Glove (item 25): the price of its gold version in bolts. |
| `omni.challenge` | i32 | The New Game+ (challenge mode) counter: 0 on the first playthrough, +1 each time New Game+ is started (kept in the save). |
| `omni.hero.held_moby` | u32 | The moby of the weapon in Ratchet's hand: word 0 of hero item slot 0. 0 when nothing is held. |
| `omni.hero.moby` | u32 | Ratchet's own moby (a pointer cell; the owner argument of the weapon creators; class 0). |
| `omni.hero.health` | i32 | Ratchet's health: the nanotech orbs the HUD shows (1 to omni.hero.max_health; 0 = dead). |
| `omni.hero.max_health` | i32 | Ratchet's maximum health in nanotech orbs: 4, 5 with Premium Nanotech, 8 with Ultra Nanotech (the game sets it from the upgrades; its HUD draws at most 8). |
| `omni.hero.right_hand.x` | f32 | World x (metres) of the hero's right hand attach point. |
| `omni.hero.right_hand.y` | f32 | World y (metres) of the hero's right hand attach point. |
| `omni.hero.right_hand.z` | f32 | World z (metres) of the hero's right hand attach point. |

Weapon fields: `omni.weapon.<name>.have` (owned, 0 or 1), `.ammo` and `.gold` (the gold-weapon flag). Not every weapon has all three:

| Weapon name | Fields (type) |
|---|---|
| `wrench` | `.have` (u8) |
| `suck_cannon` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `bomb_glove` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `devastator` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `visibomb` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `taunter` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `blaster` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `pyrocitor` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `mine_glove` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `walloper` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `tesla_claw` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `glove_of_doom` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `morph_o_ray` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `ryno` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `drone_device` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |
| `decoy` | `.have` (u8), `.ammo` (i32), `.gold` (u8) |

Notes on the HUD ammo box fields: the game copies a weapon's ammo cap into the box only when the box is created. A mod that changes the cap of the weapon in hand checks `omni.hud.ammo_box.id`, then writes the new cap to both `omni.hud.ammo_box.max` and `omni.hud.ammo_box.def_max` in the same tick. Writing only `max` makes the game fade the box out and in again.

## Records for omni.struct

`omni.struct(name, address)` reads one record of game memory into a read-only table of numbers; `omni.struct_set(name, address, field, value)` writes one field (not the ones marked read-only). Both need `omni.has("omni.struct")`. The record types and their fields:

- **`moby`**: `x` f32, `y` f32, `z` f32, `state` u8 (read-only), `flags` u16 (read-only), `pvar` u32 (read-only), `hit_slot` u8, `class` i16 (read-only), `bolts` i16, `rot_x` f32, `rot_y` f32, `yaw` f32, `scale` f32, `class_ptr` u32 (read-only), `next` u32 (read-only), `update` u32, `index` u32 (read-only), `mission` u8, `hit_flag` u8, `tint` u32, `visible` u8, `light0` u32, `light1` u32, `clock0` u32, `clock1` u32, `clock2` u32, `clock3` u32, `row0_x` f32, `row0_y` f32, `row0_z` f32, `row0_w` f32, `row1_x` f32, `row1_y` f32, `row1_z` f32, `row1_w` f32, `row2_x` f32, `row2_y` f32, `row2_z` f32, `row2_w` f32, `flags_w` u16, `sphere_x` f32, `sphere_y` f32, `sphere_z` f32, `sphere_r` f32, `update_dist` u8, `draw_dist` i16
- **`enemy`**: `health` f32
- **`hit`**: `attacker` u32, `strength` f32
- **`hit_desc`**: `attacker` u32, `mask` u32, `damage` f32, `one` u8, `type` u8, `strength_class` u8, `attacker_class` u16
- **`pvar0`**: `component` u32
- **`component`**: `health` f32, `reaction` u8, `radius8` u8, `aim_height` f32, `alert` u32 (read-only)
- **`blaster_shot`**: `aim_x` f32, `aim_y` f32, `aim_z` f32, `yaw_vel` f32, `pitch_vel` f32, `fallback_yaw` f32, `fallback_pitch` f32, `owner` u32, `life` i16, `life_max` i16, `bounces` i32
- **`ryno_rocket`**: `aim_x` f32, `aim_y` f32, `aim_z` f32, `owner` u32, `age` i32, `target` u32, `aim_height` f32, `speed` f32, `turn_yaw` f32, `turn_pitch` f32, `wobble` f32, `target_class` i32
- **`devastator_rocket`**: `last_hit` u32, `life` i32, `target` u32, `aim_height` f32, `bounces` i32, `rearm` i32, `max_dist` f32
- **`bomb`**: `vx` f32, `vy` f32, `vz` f32, `owner` u32, `fuse` i16, `water` i16, `tint` i16
- **`mine`**: `vx` f32, `vy` f32, `vz` f32, `owner` u32, `stuck_to` u32, `landed` i16, `target` u32 (read-only), `age` u32, `chase_wide` u32
- **`doom_agent`**: `goal_x` f32, `goal_y` f32, `goal_z` f32, `vx` f32, `vy` f32, `vz` f32, `age` i32, `target` u32, `speed` f32, `owner` u32
- **`visibomb`**: `life` i32, `speed` f32
- **`gold_case`**: `index` i32 (read-only)
- **`matrix4`**: `m00` f32, `m01` f32, `m02` f32, `m03` f32, `m10` f32, `m11` f32, `m12` f32, `m13` f32, `m20` f32, `m21` f32, `m22` f32, `m23` f32, `m30` f32, `m31` f32, `m32` f32, `m33` f32
- **`decoy`**: `timer` i16, `health` f32
- **`drone`**: `attacks_left` i16
- **`held_taunter`**: `range` f32
- **`held_morph`**: `target` u32 (read-only), `max` f32, `time_left` f32, `lock` i32
- **`tesla_claw`**: `target1` u32 (read-only), `target2` u32 (read-only), `marker` i16
- **`flame`**: `life` u16, `start_life` u8
- **`vec4`**: `x` f32, `y` f32, `z` f32, `w` f32
- **`hud_health`**: `fade_bar` u8 (read-only), `fade_orbs` u8 (read-only), `result` u32 (read-only)

## Skeleton joints for pose overrides

`omni.pose.*` (picture-only re-posing, see MODDING.md) uses joint names of the hero's skeleton (111 joints). Names, with their joint number:

`root`, `pelvis`, `spine_1`, `spine_2`, `spine_3`, `spine_4`, `chest`, `neck`, `head`, `jaw`, `right_leg_hip`, `right_knee`, `right_ankle`, `right_foot`, `right_toe`, `left_leg_hip`, `left_knee`, `left_ankle`, `left_foot`, `left_toe`, `right_clavicle`, `right_shoulder`, `right_elbow`, `right_wrist`, `right_hand`, `left_clavicle`, `left_shoulder`, `left_elbow`, `left_wrist`, `left_hand`, `right_thumb`, `left_thumb`, `right_finger_1`, `right_finger_2`, `right_finger_3`, `right_finger_4`, `left_finger_1`, `left_finger_2`, `left_finger_3`, `left_finger_4`, `right_ear`, `left_ear`, `tail_1`, `tail_2`, `tail_3`, `tail_4`, `tail_5`, `tail_6`, `tail_7`, `right_eye`, `left_eye`, `right_eye_socket`, `left_eye_socket`

Chains (a chain name moves all its joints together):

- `right_arm`: `52`, `72`
- `left_arm`: `72`, `92`
- `right_hand`: `56`, `71`
- `left_hand`: `76`, `91`
- `right_leg`: `92`, `98`
- `left_leg`: `98`, `104`
- `tail`: `104`, `111`
- `spine`: `1`, `7`
- `ears_right`: `44`, `47`
- `ears_left`: `41`, `44`

## Events, modes, pause pages, icons

**Events** for `omni.on(name, fn)` (main chunk only):

| Event | Arguments | When |
|---|---|---|
| `level_loaded` | `level` | a level finished loading |
| `level_unloading` | `level` | a level is about to go away |
| `mode` | `new, old` | the game mode changed |
| `pause_page` | `new, old` (nil = no page) | a pause-menu page opened or closed |
| `new_game` | `new_game_plus` | a new game started (true for New Game+) |
| `save_loaded` | `slot` | a save was loaded; the per-save store already holds that slot's data |
| `saving` | `slot` | the game is writing a slot; change the store now |
| `setting` | `key, value` | the player changed one of your settings |

**Levels**: `omni.level()` is 0 to 18, or -1 before a level is loaded.

**Modes** (`omni.mode()` and the `mode` event): `title`, `gameplay`, `pause`, `cutscene`, `movie`, `loading`, `vendor`, `ship`, `dialog`, `screen` (a full-screen picture), `unknown`.

**Pause pages** (`omni.pause_page()`, `pause_page` event): `root`, `weapons`, `gadgets`, `quick_select`, `items`, `help`, `options`, `goodies`, `map`, `quit`, `goals`, `moves`, `save`, `load`, `skill_points`. Any other page is `page_<hex id>`.

**Icons** for `omni.draw.icon`: `bolt`, `nanotech`, `weapon.<name>` (for example `weapon.blaster`), `weapon.<name>.hud` (the ammo HUD's own picture), `gadget.<name>`, and `item.<id>` for items without a known name. `omni.draw.icons()` returns the full list (about 50 names); call it once, not every tick.

**Weapon item ids** used by `rac1.weapon.take_ammo` and `omni.weapon.equipped`: the item id of the weapon (for example 15 is the Blaster, 8 the Omniwrench, 10 the Bomb Glove). `-1` means "the item in hand".

