# Writing script mods for RAC-RS (OmniLoader, Luau)

This guide is for hobby modders who know a little scripting. You do not need to know how the game works inside. It covers script mods for Ratchet & Clank 1 in RAC-RS: small folders of text files that change or add to the game while it runs, without touching the game files.

Companion files: [MODDING_REFERENCE.md](MODDING_REFERENCE.md) (every hook id, game function and field) and [TEXTURE_PACKS.md](TEXTURE_PACKS.md) (replacing pictures).

Everything here works in **RAC-RS v0.4.0**, the first version with mods, unless a line says "(since v0.4.1)".

Contents: [1 What a mod is](#1-what-a-mod-is) - [2 Your first mod](#2-your-first-mod-in-5-minutes) - [3 Where mods live and how to switch them on](#3-where-mods-live-and-how-to-switch-them-on) - [4 The omni table](#4-the-omni-table) - [5 Hooks](#5-hooks) - [6 Fields](#6-fields-game-state-by-name) - [7 The omni extension](#7-the-omni-extension) - [8 Settings and presets](#8-settings-and-presets) - [9 The time budget](#9-the-time-budget) - [10 Other ports and RAC-RS features](#10-other-ports-and-rac-rs-features) - [11 Debugging](#11-debugging) - [12 Publishing](#12-publishing-checklist) - [13 FAQ](#13-faq-and-pitfalls) - [14 A bigger example](#14-a-bigger-example-arsenal-mastery) - [Versions](#versions-what-needs-what)

Words used in this guide, each defined once:

- **Tick**: one step of the game's logic. 50 a second on the PAL (European) disc, 60 on the NTSC (American) disc. A tick is not a drawn picture.
- **Moby**: the game's word for any object that lives in the world and updates itself every tick: an enemy, a crate, a bolt, a shot, your held weapon.
- **Hook**: a place in the game's own code where your script is allowed to run before, after or instead of the game's function.
- **JSON**: a plain text format for data, with `{ }` for records and `[ ]` for lists. The manifest is JSON.
- **Port**: a PC version of the game. RAC-RS is one; OpenRAC is another.

## 1. What a mod is

A script mod is a folder with two kinds of files:

1. `omni-mod.json`, the **manifest**: the mod's name, version, which game and discs it is for, which script files to run, and the options players can change.
2. One or more `.luau` files. [Luau](https://luau.org) is a small scripting language, close to Lua. It runs inside a **sandbox**: your script sees the game only through a table called `omni`, cannot touch the player's files or the internet, and each mod has its own private variables, so two mods cannot break each other.

The format follows the MIT-licensed mod SDK of [OpenRAC](https://github.com/OpenRAC/OpenRAC) (manifest "format 2", `"type": "script"`, interface 1.3). OmniLoader is this project's own code, written from that SDK. A mod folder that has OpenRAC's `openrac-mod.json` instead of `omni-mod.json` is read too, and OpenRAC's `openrac` table is available under that name. On top of the OpenRAC base, OmniLoader adds an **extension** for bigger gameplay mods: calling game functions, drawing on screen, saving data per save slot, game events. The extension is only in the `omni` table of RAC-RS, so a mod checks `omni.has` before using it ([section 7](#7-the-omni-extension)).

Your main script runs **once, when the game starts**. In that first run you register what you want to happen later (hooks, per-tick functions, events). After that the game calls your functions. Think of the first run as "setting up", not "doing".

With no mod switched on, nothing is loaded and the game is exactly the original. A mod that fails switches itself off and the game carries on.

## 2. Your first mod in 5 minutes

It shows the current level number on screen, writes a line in the log when a level loads, and has one on/off setting. Make a folder called `hello-level` with these two files.

`omni-mod.json`:

```json
{
  "format": 2,
  "id": "hello-level",
  "name": "Hello level",
  "version": "1.0.0",
  "author": "Your name",
  "description": "Shows the level number in the top left corner.",
  "license": "MIT",
  "type": "script",
  "api": "1.3",
  "scripts": ["main.luau"],
  "games": { "rac1": ["pal", "ntsc"] },
  "settings": [
    { "key": "show_level", "name": "Show the level number", "type": "bool", "default": true }
  ]
}
```

`main.luau`:

```lua
--!nonstrict
-- hello-level: needs the omni extension (events and drawing).
if not (omni.has and omni.has("omni.events") and omni.has("omni.draw")) then
    omni.log("hello-level: this port cannot do events or drawing here; doing nothing")
    return
end

local level = omni.level()   -- 0..18, or -1 before a level is loaded

omni.on("level_loaded", function(n)
    level = n
    omni.log("level loaded: " .. n)
end)

omni.on_frame(function()
    if omni.setting("show_level") and level >= 0 then
        omni.draw.text(16, 24, "Level " .. level, { color = 0xFFFF00FF, shadow = 0x000000FF })
    end
end)

omni.log("hello-level ready")
```

What each part does:

- `id`: lower case letters, digits and single hyphens, at most 64 characters. It never changes between versions.
- `api`: the interface version you wrote for. `1.3` is the newest. A game with an older interface refuses the mod and says why in the log.
- `games`: the game and discs. `"rac1"` is Ratchet & Clank 1; `"pal"` and `"ntsc"` are the European and American discs.
- `omni.has(name)` asks "does this port have it?" and answers `true` or `false`. It takes the extension parts (`"omni.events"`, `"omni.draw"`, `"omni.save"`, `"omni.call"`, `"omni.raycast"`, `"omni.struct"`, `"omni.struct_set"`, `"omni.mobys"`, `"omni.pose"`, `"omni.pose.read"`, `"omni.pose.debug"`, `"omni.pose.follow"`, `"omni.unwound"`, `"omni.ticks_per_second"`, `"omni.is_enemy"`, `"omni.out_f32"`), a hook id or a field name. Use it before anything that is not in the base table.
- `omni.on_frame(fn)`: `fn` runs after every game tick. The name says "frame" but it is a tick.
- `omni.draw.text(x, y, text, options)` draws for **this tick only**; call it every tick to keep the text on screen. Coordinates are in the game's own 2D space, 512 wide and 448 high. Colours are `0xRRGGBBAA` (red, green, blue, alpha): `0xFFFF00FF` is opaque yellow.
- `omni.setting("show_level")` returns the player's current value (here true or false).

Try it: copy the folder into the game's `mods` folder, switch it on ([section 3](#3-where-mods-live-and-how-to-switch-them-on)), start the game and load a level. You should see "Level N" in yellow in the top left and a line `level loaded: N` in the log ([section 11](#11-debugging)).

## 3. Where mods live and how to switch them on

Everything mod-related sits in **one folder**, the `mods` folder in your RAC-RS folder (the folder that holds `rac-rs.exe`). Texture packs live there too ([TEXTURE_PACKS.md](TEXTURE_PACKS.md)).

```
<your RAC-RS folder>\mods\
    active.txt          which mods and texture packs are on, in priority order
    settings.json       the players' values for every mod's settings
    status.json         what the loader did with each mod at the last start (the game writes it)
    hello-level\
        omni-mod.json
        main.luau
```

`omniloader.dll` must be next to `rac-rs.exe`. Setup puts it there and also copies the preinstalled mod (Arsenal Mastery) into `mods`. Without the DLL the game starts normally with mods off and writes one line to the log. Setting the environment variable `RAC_MODS_DIR` makes the game use another folder as the `mods` folder.

**A mod is off until somebody switches it on.** A folder you drop into `mods` never starts by itself. Ways to switch it on:

- **The launcher** (RAC-RS Launcher, Mods page): script mods are listed under "Script mods" once their folder is in `mods` (the page has an "Open mods folder" button). Each has an on/off switch, priority arrows and a "Mod settings" button. A master switch "Load mods" holds all script mods back without forgetting the per-mod choices. The launcher takes effect at the next start of the game. Launcher 0.1.1 does not install script mods from a zip or a link: you copy the folder in yourself. (Since v0.4.1: launcher 0.1.2 can update a mod, such as Arsenal Mastery, from its GitHub release.)
- **The title screen**: Options > Mods has the "Load mods" master switch, an On/Off row for each installed script mod and, while it is on, that mod's settings. On or off applies at the next start of the game, because the loader starts once per run.
- **By hand**: add the mod's `id` (the id in `omni-mod.json`, which may differ from the folder name) as a line in `mods\active.txt`. The file is one id per line, the first line is the highest priority, a `#` starts a comment, and the launcher's two master switches show up as comment lines `# Load mods: off` and `# Load texture packs: off`. A script mod that is not listed is off, and with no `active.txt` nothing is on.

The **pause menu** (Pause > Options > Mods) cannot switch a script mod on or off. It shows the basic settings of the mods that are running, and a change there applies at once ([section 8](#8-settings-and-presets)).

The order of the lines in `active.txt` is the load order.

**For testing only**, a file `mods\plugins.json` replaces `active.txt` and `settings.json`. If it exists the loader reads it as it is:

```json
{
  "format": 1,
  "mods": [
    { "id": "hello-level", "enabled": true, "settings": { "show_level": true } }
  ]
}
```

Delete it again afterwards, or the launcher's and the game's switches will seem to do nothing.

Two folders with the same manifest id: the first one in folder-name order is the mod, the other is ignored.

## 4. The omni table

Everything a mod can do starts from the global `omni`. Its base is OpenRAC's (OpenRAC calls the same table `openrac`; this loader offers it under that name too, so OpenRAC mods run unchanged).

| Member | What it does |
|---|---|
| `omni.game`, `.version`, `.platform`, `.api` | `"rac1"`, `"pal"` or `"ntsc"`, the system, the interface version |
| `omni.log(...)`, `omni.warn(...)`, `print(...)` | a line in the game's log, tagged with your mod's id (`print` = `log`) |
| `omni.setting(key)` | one of your settings; `nil` for a key the manifest does not declare |
| `omni.hook`, `.before`, `.after` | take a hook point ([section 5](#5-hooks)) |
| `omni.on_frame(fn)` | `fn()` after each tick |
| `omni.on_pad(fn)` | `fn(port, report)` for each controller report, before the game reads it; `report` is an 18-byte `buffer` you may change |
| `omni.fields` | game state by name ([section 6](#6-fields-game-state-by-name)) |
| `omni.read(type, address)`, `omni.write(type, address, value)` | game memory; `type` is `"u8" "i8" "u16" "i16" "u32" "i32" "f32"`; addresses below 32 MB only. Prefer fields. |
| `omni.key(name)`, `omni.pressed(name)`, `omni.mouse()`, `omni.window()`, `omni.pointer()`, `omni.take_input(...)`, `omni.capture_mouse(on)` | keyboard and mouse (interface 1.2 and 1.3) |
| `omni.pvars(moby)` | a moby's per-class variables by name (experimental). Not available yet: raises "not supported on this port". |
| `require("name")` | another `.luau` file from your own mod folder |
| `omni.has(name)` | feature detection. It is `nil` on OpenRAC, so a mod for both writes `omni.has and omni.has(...)`. |
| `omni.fn`, `omni.draw`, `omni.on`, `omni.save`, `omni.struct`, ... | the extension, [section 7](#7-the-omni-extension) |

What the sandbox gives you: `math`, `string`, `table`, `buffer`, `bit32`, `utf8`, `vector`, `coroutine`, `os.clock`, `debug.traceback`, `print`. Not available: `io`, the rest of `os`, `load`, `loadstring`, `dofile`, `getfenv`, `setfenv`, and files outside your folder. Library tables and `omni` are read-only.

An unsupported call does not crash the game: it raises a Luau error with the text "not supported on this port". An error that nothing catches switches your mod off ([section 9](#9-the-time-budget)); catch it with `pcall(fn, ...)` where you expect one.

For editor autocomplete, point a Luau language server (luau-lsp) at the OpenRAC SDK's `types/rac1.d.luau`.

## 5. Hooks

A **hook point** is a game function that mods may take over. The game's own code runs exactly as before when no mod is on it. There are three calls and four uses (wrap, replace, before, after):

| Call | Your function | Use it to |
|---|---|---|
| `omni.before(id, fn)` | `fn(...)` runs before the game's function; the game's code always runs | count things, read arguments |
| `omni.after(id, fn)` | `fn(...)` runs after (same arguments) | react to what happened |
| `omni.hook(id, fn)` | `fn(original, ...)` wraps it; call `original(...)` to continue, or do not call it to **replace** the function | change an argument or a result, or write the behaviour anew |

Rules:

- Take hooks **in the main chunk** (the top level of your script), never later.
- `original(...)` goes on down the chain and returns the result. `original()` with no arguments passes the call's own arguments. If your function returns nothing, the original's result is kept.
- Several mods on one hook point form a chain in load order. The mod loaded **later** runs **first**; its `original` is the mod before it, down to the game. A mod that never calls `original` stops the chain there, so use before/after where you can.
- Ids look like `rac1.player.take_damage`. Write ids in new mods; the older names (`Hero_TakeDamage`) are aliases and still work.
- List every id you take in the manifest: in `"hooks": [...]` when the mod cannot work without it, in `"optional_hooks": [...]` when it can ([Optional hooks](#optional-hooks)).
- If the original produced **no result** (see [When the original cannot finish](#when-the-original-cannot-finish)), `original(...)` returns `nil` instead of a number.

### Examples

React to damage, change nothing:

```lua
local hits = 0
omni.after("rac1.player.take_damage", function(amount)
    hits = hits + 1
    omni.log("hit for " .. amount .. ", hits so far: " .. hits)
end)
```

Replace: the Blaster never uses ammo. `take_ammo(item, amount)` returns 1 when the ammo is taken; item `-1` means "the item in hand" (15 is the Blaster):

```lua
omni.hook("rac1.weapon.take_ammo", function(original, item, amount)
    if item == 15 then
        return 1                        -- do not call original: no ammo is used
    end
    return original(item, amount)       -- every other weapon: the game's own rule
end)
```

Change an argument, then continue:

```lua
omni.hook("rac1.player.take_damage", function(original, amount)
    if amount > 1 then
        amount = amount - 1             -- armour: one point less per hit
    end
    original(amount)
end)
```

A mod with all three examples lists `"hooks": ["rac1.player.take_damage", "rac1.weapon.take_ammo"]` in its manifest (each id once, even if you take it twice).

### Finding hook points

- The ids and signatures are in [MODDING_REFERENCE.md](MODDING_REFERENCE.md): the 17 base hook points (the same ids as on OpenRAC), the `rac1.omni.*` ones added for gameplay mods (wrench, weapons, shots, ammo, vendor, pause Weapons page, damage, HUD), and the experimental moby update classes.
- A **signature** such as `void(f32,u32)` reads "returns nothing, takes a decimal number and a whole number". `u32` and `i32` are whole numbers (an address is a `u32`), `f32` is a decimal. Your hook function gets the arguments in that order.
- **PAL and NTSC**: you write one id. RAC-RS finds the game function by its code, not by a fixed address, so a hook works on both discs wherever the function exists. Level code (weapons, enemies) lives at different places in each level; RAC-RS installs the hook again on every level load and skips it where that level does not have the function. A hook can therefore fire only in some levels. `omni.has("<id>")` asks whether the port can hook it on this disc.
- RAC-RS can hold up to 256 hook points at once across all mods, more than the 210 that exist.
- A mod whose `"hooks"` list names a hook point the port does not have is **not loaded**: `status.json` marks it `refused` with the reason, and no code of the mod runs. That is right for a mod that cannot work without the hook. For everything else use `"optional_hooks"` and a guard.

### Optional hooks

`"optional_hooks"` is a second list next to `"hooks"` (an OmniLoader extension; other loaders ignore the key):

```json
"hooks": ["rac1.player.take_damage"],
"optional_hooks": ["rac1.omni.ammo.pickup", "rac1.omni.wrench.throw"]
```

- A hook point on `"hooks"` that the port lacks stops the whole mod. One on `"optional_hooks"` that the port lacks is **skipped**: the mod loads, `omni.hook`, `omni.before` and `omni.after` for that id do nothing and raise no error, `omni.has(id)` is `false`, and the loader writes one info line to the log (`optional hook point <id> skipped: ...`).
- List the hook points added late as optional and the mod simply does less on a build that lacks them. Test before you take the hook:

```lua
if omni.has("rac1.omni.ammo.pickup") then
    omni.after("rac1.omni.ammo.pickup", function(moby) end)
else
    omni.log("this port cannot tell me about ammo pickups: ammo bonuses are off")
end
```

- An id may not be in both lists: it stays required.

### When the original cannot finish

Some game functions wait for the next frame in the middle (the Gold Weapon Case fanfare is one). A hook cannot sit and wait inside the game's tick, so the port lets the call go: the rest of that game function then runs unhooked, and your hook's code after `original(...)` runs early.

- `original(...)` returns `nil` (not 0) when the original gave **no result**. A real result, including 0, is returned as before. Test `local r = original(...)` with `if r == nil then ... end`.
- `omni.unwound()` is `true` for the rest of that hook call once any `original` call in it was let go, else `false`. Use it when the hook has no result to look at, or to skip work that only makes sense when the original finished.

## 6. Fields: game state by name

`omni.fields` reads and writes game state without knowing addresses. Values are plain numbers.

```lua
local f = omni.fields
omni.on_frame(function()
    if f.bolts < 100 then
        f.bolts = 100                    -- never fewer than 100 bolts
    end
    local p = f.hero.position
    omni.log(p.x .. " " .. p.y .. " " .. p.z)   -- hero.state is read-only
end)
```

Base fields (OpenRAC's): `hero.position.x/y/z`, `hero.facing`, `hero.move_yaw`, `hero.state` (read-only), `bolts`. RAC-RS adds `omni.*` fields for gameplay mods: for each weapon `omni.weapon.<name>.have` (owned, 0 or 1), `.ammo` and `.gold` (the gold-weapon flag), `omni.weapon.equipped` (the item id in hand), the quick-select wheel, `omni.hero.health`, `omni.hero.max_health`, `omni.hero.moby` and the HUD ammo box. The full list with types is in [MODDING_REFERENCE.md](MODDING_REFERENCE.md#fields); `omni.has("omni.weapon.blaster.ammo")` tells you if a build has one.

A read or write of a level field fails on the title screen and between levels: use `pcall` there. Prefer fields over `omni.read(type, address)`: addresses differ between versions of the game, field names do not. Fields are marked experimental in the format and may grow.

**The HUD ammo box.** The game copies a weapon's ammo cap into the box only when the box is created. A mod that changes the cap of the weapon in hand writes the new cap to both `omni.hud.ammo_box.max` and `omni.hud.ammo_box.def_max` in the same tick, after checking `omni.hud.ammo_box.id`, and the box shows it at once. Writing only `max` makes the game fade the box out and in again.

## 7. The omni extension

The extension is OmniLoader's addition (`omni.loader_version == 1`), in the same `omni` table. On OpenRAC and other loaders it does not exist. Check what the port really has:

| `omni.has(...)` | Gives you |
|---|---|
| `"omni"` | the extension exists |
| `"omni.call"` | `fn`, `with_stack` (calling game functions) |
| `"omni.raycast"` | `raycast` |
| `"omni.events"` | events (`on`) |
| `"omni.save"` | per-save data |
| `"omni.draw"` | drawing |
| `"omni.struct"` | `struct` (read a record by name) |
| `"omni.struct_set"` | `struct_set` (write one field of a record) |
| `"omni.mobys"` | `mobys`, `moby_valid` (find the game's live objects) |
| `"omni.pose"` | `pose.mirror`, `rotate`, `set`, `hold`, `clear`, ... (re-pose a character, section 7.6) |
| `"omni.pose.read"` | `pose.joint_position`, `joint_rotation`, `mirrored` |
| `"omni.pose.debug"` | `pose.debug` (joint names on screen) |
| `"omni.pose.follow"` | `pose.follow`, `pose.unfollow` (draw a prop on a joint) |
| `"omni.unwound"` | `unwound` ([section 5](#when-the-original-cannot-finish)) |
| `"omni.ticks_per_second"` | `ticks_per_second` |
| `"omni.is_enemy"` | `is_enemy` |
| `"omni.out_f32"` | `out_f32` |

If a part is missing, its functions raise "not supported on this port" (events and save quietly do nothing). All of the extension is in RAC-RS v0.4.0.

### 7.0 Small helpers

- `omni.ticks_per_second()` returns how many logic ticks the game runs per second on this disc: `50` on the PAL disc, `60` on the NTSC disc, `nil` for a disc the loader does not know. Use it to turn seconds into ticks (`on_frame` runs once per tick).
- `omni.unwound()`: see [section 5](#when-the-original-cannot-finish).
- `omni.is_enemy(moby)` returns `true` when `moby` (an address) is a living enemy: a live object in the targetable list, of the enemy class, with health above zero. It is `false` for `nil`, 0 or an object that is not live. It gives the same answer as `omni.mobys{ enemy = true }` without building a list, so use it in a damage hook.
- `omni.out_f32(ptr)` reads the decimal number a hook's `float*` argument points to (`nil` if it cannot be read); `omni.out_f32(ptr, value)` writes it and returns `true` or `false`. Use it for the "out" arguments of a hook (a value the game function hands back through a pointer) instead of raw memory reads.

On a port without a helper the name is `nil`, so write `if omni.is_enemy then ... end` or test `omni.has` first.

### 7.1 Calling game functions

`omni.fn(target, signature)` returns a function you can call.

```lua
local get_ammo = omni.fn("get_ammo", "i32(i32)")   -- a name from the game functions list

omni.on_frame(function()
    local blaster_ammo = get_ammo(15)
    omni.draw.text(16, 60, "Blaster: " .. blaster_ammo)
end)
```

- `target` is a hook id (the call then runs the whole chain, every mod's hook, then the game's function, exactly like a call from the game), a name from the game functions list ([MODDING_REFERENCE.md](MODDING_REFERENCE.md#game-functions)), or an address number. Give the `signature` (`"u32(u32,f32)"`, `"void(ptr)"`) unless the target is a hook id.
- Call the function from `on_frame`, a hook or an event. **Not** while the main chunk runs: that raises "game functions can only be called while the game runs". Creating the function in the main chunk is fine.
- A function that is not in the code loaded now (a level's own code) raises "game function X is not in the code loaded now". It is a text error: catch it with `pcall`.
- Time spent inside game functions does not count against your time budget.

Some functions need room for a vector or a record. `omni.with_stack(size, fn)` gives you `size` zeroed bytes of game stack as an address, freed when `fn` returns:

```lua
local rand_vec = omni.fn("rand_vec", "void(u32)")   -- writes the game's random vector to float[4]
local tick = 0
omni.on_frame(function()
    tick = tick + 1
    if tick % 250 == 0 then
        omni.with_stack(16, function(addr)
            rand_vec(addr)
            local x = omni.read("f32", addr)
            local y = omni.read("f32", addr + 4)
            omni.log("random vector: " .. x .. ", " .. y)
        end)
    end
end)
```

Reading a record by name: `omni.struct(name, addr)` reads one record of game memory into a read-only table of numbers (`nil` for address 0 or memory that cannot be read; an unknown name is an error). The record types are listed in [MODDING_REFERENCE.md](MODDING_REFERENCE.md#records-for-omnistruct). Test `omni.has("omni.struct")` first.

```lua
omni.before("rac1.omni.damage.hit_victim", function(victim)
    local m = omni.struct("moby", victim)
    if m then omni.log("hit a moby of class " .. m.class) end
end)
```

### 7.2 Events

`omni.on(name, fn)`, in the main chunk only. Callbacks run in load order. Their time counts like any other script time. The events (`level_loaded`, `level_unloading`, `mode`, `pause_page`, `new_game`, `save_loaded`, `saving`, `setting`), the mode names and the pause page names are in [MODDING_REFERENCE.md](MODDING_REFERENCE.md#events-modes-pause-pages-icons).

You can also ask at any time: `omni.level()` (0 to 18, or -1), `omni.mode()` and `omni.pause_page()` (the open page or `nil`).

```lua
omni.on("mode", function(new, old)
    omni.log("mode " .. old .. " -> " .. new)
end)
omni.on("pause_page", function(new, old)
    if new == "weapons" then omni.log("the Weapons page opened") end
end)
```

### 7.3 Per-save data

`omni.save` keeps your mod's data per save slot: a new game starts empty, loading slot N loads slot N's data. A mod's progress follows the save, and switching a mod off never loses it.

```lua
local save = omni.save
local kills = 0

omni.on("new_game",   function() kills = 0 end)
omni.on("save_loaded", function() kills = save.get("kills") or 0 end)
omni.on("saving",     function() save.set("kills", kills) end)

omni.after("rac1.moby.record_death", function(moby, flags, index)
    kills = kills + 1           -- keep the count in a local; write it only when saving
end)
```

- Functions: `get(key)`, `set(key, value)`, `keys()`, `clear()`, `version()` (the mod version that wrote the loaded data, `nil` when fresh).
- Values: nil, boolean, number, string, or tables of those (string keys or a list 1..n; no functions, no cycles). At most 64 KB per mod; a bigger value makes `set` raise an error.
- Set data at `saving`, not on every change: each `set` costs script time.
- The files are in the `omni` folder inside the game's `player` folder, one per save slot, written by the loader.

### 7.4 Drawing

Drawing is **immediate mode**: draw what you want shown on every tick, from `on_frame` or an event. The list is cleared after each tick; if nothing is drawn, no drawing work happens at all.

```lua
local d = omni.draw
omni.on_frame(function()
    d.text(20, 30, "Hello", { size = 1, color = 0xFFFF00FF, align = "left", shadow = 0x000000FF, layer = "hud" })
    d.rect(20, 50, 100, 8, 0x000000AA)
    d.bar(20, 50, 100, 8, 0.5, 0x00FF00FF, 0x333333FF)    -- fraction, fill colour, back colour
    d.icon(20, 70, 16, 16, "bolt", { tint = 0xFFFFFFFF })
    local w, h = d.size()                                  -- 512, 448
    local sx, sy, front = d.world_to_screen(0, 0, 0)       -- nil when there is no 3D view
end)
```

- **Space**: the game's own 2D space, 512 x 448, where the game draws its HUD and menus. It is mapped to the window exactly like the game's own 2D graphics, so your bar lines up with a game menu at any window size.
- **Colours**: `0xRRGGBBAA`. The last byte is opacity: `FF` solid, `80` half.
- **Options**: `size` (text only; 1 = the game's normal text height), `color` (text), `align` (`"left"`, `"center"`, `"right"`), `shadow` (a colour, or `false`; text has no shadow unless you give one), `layer` (`"hud"`, the default, or `"menu"`), `tint` (icons).
- **Layers**: both draw on top of the game's picture. `"menu"` also sits above the game's menus.
- **Icons** by name: `bolt`, `nanotech`, `weapon.<name>`, `weapon.<name>.hud`, `gadget.<name>`, `item.<id>`. `omni.draw.icons()` returns the full list (about 50 names): call it once, not every tick. An unknown name raises an error that lists the valid names. An icon the current level's art does not hold draws nothing.
- **Font**: the game's own. It shows `[` and `]` as `?`, and `%` as a button picture. Avoid those characters in text.
- `world_to_screen(x, y, z)` turns a point in the world (for example `omni.fields.hero.position`) into a screen spot; `front` is false when the point is behind the camera.

### 7.5 Finding mobys, writing records, ray test

A moby is one of the game's live objects. These functions need the port's binding (the `structs` part) and a build with moby queries; test `omni.has` first.

- `omni.struct_set(name, addr, field, value)` writes one field of the record `name` at `addr` and returns `true`, or `false` when the memory could not be written. It raises an error for an unknown struct or field, for a field marked read-only (a moby's class, state, ...: things the game owns), and for a value that is not a number or does not fit the type. Whole numbers are cut toward zero (`2.9` becomes `2`) and must then fit the field; floats must be finite. (Since v0.4.1: a mod can no longer write into the game's own program code by mistake.)
- `omni.mobys{ class = 305, near = {x, y, z}, radius = 20, max = 8, enemy = true, registry = false }` returns an array of moby addresses, nearest first. Every key is optional. `class` is one class id or a list of at most 16; `near` and `radius` (metres) limit by distance (`radius` needs `near`); `max` defaults to 32 and is never more than 256; `enemy = true` keeps enemies only; `registry = true` walks the game's list of targetable mobys instead of the whole pool. The array is empty when no level is loaded. Asked again in the same tick, the same query gives the same answer.
- `omni.moby_valid(addr)` is `true` when `addr` is a live moby right now.
- **An address is good only until the end of the tick.** The game reuses moby slots, so call `moby_valid` before you write to one a hook handed you or one you kept.
- Like `fn` and `raycast`, the time the game spends answering a query is not counted against your budget; the Luau that builds and uses the array is.

Find the nearest enemy within 20 metres of the hero:

```lua
omni.on_frame(function()
    local p = omni.fields.hero.position
    local list = omni.mobys{ near = { p.x, p.y, p.z }, radius = 20, max = 1, enemy = true }
    if list[1] and omni.moby_valid(list[1]) then
        local e = omni.struct("moby", list[1])
        omni.log(string.format("nearest enemy: class %d at %.1f, %.1f, %.1f", e.class, e.x, e.y, e.z))
    end
end)
```

**Ray test.** `local hit, x, y, z, nx, ny, nz = omni.raycast(x1, y1, z1, x2, y2, z2)` is the game's own line test. It returns `false` for a miss (then nothing else), `nil` when the port cannot test right now (in menus, with no level), otherwise the hit point and the surface normal. Z points up. Time in `raycast` is the game's and is not counted against your budget.

```lua
omni.on_frame(function()
    local p = omni.fields.hero.position
    local hit, x, y, z = omni.raycast(p.x, p.y, p.z + 1, p.x, p.y, p.z - 10)
    if hit then
        omni.draw.text(16, 90, "ground " .. string.format("%.1f", p.z - z) .. " below")
    end
end)
```

### 7.6 Re-posing characters (pose overrides)

A character is drawn from a skeleton: a tree of joints (shoulder, elbow, wrist, ...). A **pose override** turns some joints your own way, on top of (or instead of) the animation. It changes **only the picture**: where the character is, what it can hit and what hits it do not change, so saves are not affected. You need no 3D maths: joints have names (listed in [MODDING_REFERENCE.md](MODDING_REFERENCE.md#skeleton-joints-for-pose-overrides)) and angles are in degrees, seen from the character himself (positive `yaw` turns him to his left).

Test the parts first: `omni.has("omni.pose")` (write calls), `"omni.pose.read"`, `"omni.pose.debug"`, `"omni.pose.follow"`. Without them every `pose.*` call raises "not supported on this port". `local pose = omni.pose` is the usual first line. The first argument of every call is the **moby** (the character's address). For the player it is `omni.fields.omni.hero.moby`; read it again each time (it changes with the level), not once at the top of your script. Pose calls cannot run while your script first loads, only from `on_frame` and events.

What the calls look like:

```lua
local pose = omni.pose
local function hero() return omni.fields.omni.hero.moby end

-- raise the left arm into the right arm's pose, blending over 0.15 s
pose.mirror(hero(), "right_arm", "left_arm", { blend = 0.15 })
-- give a joint back to the animation
pose.clear(hero(), "left_arm", { blend = 0.15 })
-- turn the head 30 degrees to his left
pose.rotate(hero(), "head", { yaw = 30 }, { blend = 0.2 })

-- read where a joint is (the game's pose, not your drawn one)
local x, y, z = pose.joint_position(hero(), "left_hand")
local m = pose.mirrored(hero(), "right_hand")   -- the right hand reflected to the left side; m.x, m.y, m.z
```

- Reads (`joint_position`, `joint_rotation`, `mirrored`) return `nil` when there is no body to read (no level, ship mode). They give the **game's** pose, on purpose: a mod that reacted to its own drawn pose would make the game depend on the frame rate.
- `pose.debug(moby, true)` draws a dot and the name next to every joint (green for the joints you overrode); `false` removes it. It costs nothing when off. Use it to find the joint you want.
- `pose.follow(item, source, from_joint, to_joint [, { mirror = true }])` draws a prop (`item`, a moby your mod owns) in another joint's place, copying `source` (for example the gun the hero holds, attached at `from_joint`): a second gun in the other hand, a shield, a hat. `pose.unfollow(item)` stops it. It changes only the picture, returns `true` when the port took it and `false` (no error) when there is no level, the hero cannot be posed or a moby is not alive. At most 8 follow records at once. The port drops a record itself when the level changes, a cutscene starts, the hero dies or a moby disappears, so call `pose.follow` again when the moby is back. A bad joint name raises an error with your file and line.
- Other calls on `pose` (`set`, `hold`, `apply`, `snapshot`, `names`, `chain`, `partner`) are listed by `omni.has("omni.pose")`; a joint name can be a single joint or a chain, and the port raises a clear error text for a wrong argument.

## 8. Settings and presets

Settings are options for players, declared in the manifest. The launcher and the game's Options > Mods pages draw the controls from them. Six types:

```json
"settings": [
  { "key": "mode", "name": "Mode", "type": "choice", "default": "normal",
    "choices": [ { "value": "normal", "name": "Normal" }, { "value": "hard", "name": "Hard" } ],
    "description": "How hard the mod is." },
  { "key": "trace", "name": "Trace", "type": "bool", "default": false },
  { "key": "every", "name": "Log every", "type": "int", "default": 10, "min": 1, "max": 600 },
  { "key": "scale", "name": "Speed", "type": "float", "default": 1.0, "min": 0.5, "max": 3.0, "step": 0.1 },
  { "key": "greeting", "name": "Greeting", "type": "string", "default": "hi", "max_length": 40 },
  { "key": "boost", "name": "Boost key", "type": "keybind", "default": "KeyF,MouseMiddle", "group": "play" }
]
```

`key` is lower case letters, digits and `_`. A `keybind` needs `"api": "1.2"` or higher and takes up to two inputs named like the web's key codes (`KeyW`, `Space`, `MouseLeft`, ...). A value that is missing, of the wrong type or out of range is replaced by the default, so `omni.setting(key)` always gives a valid value for a declared key.

Presets set several settings at once (interface 1.2 and later); the launcher shows them as buttons:

```json
"presets": [
  { "id": "easy", "name": "Easy",    "values": { "mode": "normal", "scale": 0.5 } },
  { "id": "wild", "name": "Wild",    "values": { "mode": "hard",   "scale": 2.0 } }
]
```

Each value must be one its setting accepts. A preset with the id `default` replaces the launcher's own Default button.

The players' values are kept in `mods\settings.json`, keyed by the mod id.

**When a change applies.** In the game's own Options > Mods pages, a change to a running mod applies at once: `omni.setting(key)` returns the new value on the next call, and the `"setting"` event runs for your mod. In the launcher, a change applies at the next start. So read settings in your tick or on the event instead of once at start, if you want live changes:

```lua
local speed = omni.setting("scale")
omni.on("setting", function(key, value)
    if key == "scale" then speed = value end
end)
```

## 9. The time budget

Scripts must be quick, because they run inside the game's tick.

**v0.4.0:**

- Your script time per tick is measured. The budget is **2 ms**. Over it you get one warning in the log, and the mod is switched off if it is over on **30 of the last 60 ticks**.
- A **hard stop** at 50 ms in one callback: a runaway loop ends the mod at once.
- The first 10 ticks after load and after each `level_loaded` count only toward the hard stop, so a slow first tick (building a table) is forgiven.

**Since v0.4.1** the soft rule is kinder, because a crowded fight can legitimately need 2 to 8 ms for many ticks in a row: a tick over 2 ms gives **one warning per run** (it says it is only a warning), and the mod is stopped only if ticks take **over 20 ms** on 30 of the last 60 ticks. The 10-tick grace and the 50 ms hard stop are unchanged. Log lines are also much cheaper since v0.4.1 (a few thousandths of a millisecond instead of about 1 ms each), so a chatty mod no longer steals real time.

In all versions:

- **Game time does not count**: time inside `original`, `omni.fn` calls, `raycast` and `require` is the game's, not yours.
- When a mod fails: its error and traceback go to the log with your mod's id, `status.json` marks it `failed` with the reason, its hooks pass straight through to the original game and its callbacks stop. The game goes on.
- OpenRAC's own rule is stricter (2 ms per call). If you want your mod portable, keep every callback under 2 ms.

How to stay inside it: do heavy setup once, not per tick; keep counters in locals and write the per-save store only at `"saving"`; build strings only when a value changes; spread long work over several ticks; avoid creating big tables in `on_frame`.

## 10. Other ports and RAC-RS features

**One mod, several ports.** The format is OpenRAC's, so one mod can run on RAC-RS and on OpenRAC: name the manifest `openrac-mod.json` there and begin the script with `local omni = omni or openrac`. The base table works on both; the extension (`omni.has`, fn, draw, events, save, ...) and the `rac1.omni.*` hook points exist only in RAC-RS, so guard them with `omni.has` and list them under `optional_hooks`. Test on every port you claim.

**Conflicts with RAC-RS's own PC features.** RAC-RS has built-in features that change the same things a gameplay mod might: `rac-rs.strafe-aim` (strafe-aim with the right mouse button), `rac-rs.modern-camera`, `rac-rs.lockon` (wider lock-on) and `rac-rs.kbm` (the keyboard and mouse table). The rule is: **a mod wins**. RAC-RS switches its feature off while your mod is on, so the two never fight over the same game values. It decides that automatically when your mod takes a hook point the feature uses, writes game words the feature writes, or takes the keyboard with `omni.take_input("keyboard")`; and it always does when your manifest names the feature:

```json
"conflicts": [ { "id": "rac-rs.strafe-aim", "reason": "my mod does its own strafing" } ]
```

The run log then says `omni: <mod> replaces rac-rs ...` and `status.json` lists `"replaces"`. Naming the feature in `conflicts` is the safe, explicit way when you know your mod overlaps; raw memory writes are the case the automatic rule can miss.

**Mods that need other mods.** `"requires"` and `"conflicts"` also name other mods by id (OpenRAC manifest rules); a missing or conflicting mod makes the loader refuse yours with a reason in `status.json`.

## 11. Debugging

- **Log lines**: `omni.log("text")`, `omni.warn("text")` and `print` write to the game's log, `player\log.txt` in your RAC-RS folder (the run before it is `log.prev.txt`). Your lines are tagged with your mod's id. Errors and tracebacks of a failing mod are there too.
- **At start** the log has `omni: N mods loaded (ids)`, then one line per mod that did not load: `omni: mod <id> failed: <reason>`. With mods in the folder but none switched on it says `none switched on (mods/active.txt): mods are off`.
- **`mods\status.json`**: for each mod `state` is `loaded` (and the `hooks` it took), `refused` (a check failed before it opened; the `reason` says which: another disc, a newer interface, a missing hook point, a missing or conflicting mod) or `failed` (an error or the time budget). `hook_points` lists every hook point the port has.
- **Common reasons**: "needs interface 1.3; this port has ..." (lower your `api` to what you really use); "not for this disc" (add the disc to `games` if it works there); "no hook point X" (check the id against the reference); "signature differs".
- **Test switches** (set before starting the game; PowerShell: `$env:NAME = "value"`):
  - `RAC_OMNI=0`: all mods off (to find out whether a bug is yours).
  - `RAC_OMNI_DRAW_DEMO=1`: with any mod loaded, the game draws a fixed test picture each tick (text, bars, an outline of the 512 x 448 space, a marker on the hero). Use it to see if drawing works at all.
  - `RAC_MODS_DIR=<folder>`: use another folder as the mods folder (a clean test folder).
  - `RAC_OMNI_DLL=<path>`: use another `omniloader.dll`.
  - `RAC_OMNI_DISC=pal|ntsc`: which disc the mods are told. A debug switch.
- **Draw a debug line**: `omni.draw.text(16, 40, tostring(value))` every tick shows a value in-game.
- **Catch your own errors** while developing: `local ok, err = pcall(fn); if not ok then omni.warn(err) end`, so one bad call does not switch the whole mod off.
- Start with one feature at a time, and keep the main chunk short.

## 12. Publishing checklist

1. **What a mod may contain** (common modding practice, not legal advice):
   - **Only your own work**: the manifest, your scripts, a README, a `LICENSE`, and pictures, sounds or text you made yourself.
   - **Nothing from the game**: no files from the disc or from an install (textures, models, sounds, music, movies, the game's text, code, memory dumps, savestates, memory cards), not even edited, converted or upscaled.
   - **Use the game's things at run time instead**: `omni.draw.icon` and `draw.text` use the game's own icons and font from the player's disc; hooks and fields reach the game's code and values by name.
   - **No reverse-engineering notes** in the published folder: no disassembly, decompiler output or address lists. Script comments name things (hooks, fields), not memory addresses.
   - **Names**: you may say which game the mod is for ("a mod for Ratchet & Clank on RAC-RS"). Use no logos or box art, do not call the mod official, and say it is not affiliated with Sony Interactive Entertainment or Insomniac Games.
   - **Free**: a mod that needs the player's own disc, is made only of your own work and is free to download is the safest way to share.
2. **Licence**: choose one, put its name in the manifest (`"license": "MIT"`) and a `LICENSE` file in the folder. Do not copy code from mods or ports whose licence you cannot reuse.
3. **Version**: raise `version` for every release (`1.0.0`, `1.1.0`, `2.0.0-beta`). Keep `id` the same forever.
4. **`api`**: the lowest interface version you really use (1.1 for base scripts, 1.2 for keybinds, presets and keyboard, 1.3 for `omni.pointer`).
5. **`games`**: list only the discs you tested.
6. **`hooks`** lists every hook point you take (`optional_hooks` the ones you can live without); **`requires`** / **`conflicts`** name other mods and RAC-RS features as needed.
7. **Test**: start from a new game, load a save, switch the mod off and on again, change each setting, open the pause menu. Check the log for warnings, and for the time-budget warning.
8. **Package**: a script mod is its folder zipped as `<id>-<version>.zip`, with the manifest at the top. Everything in the folder ships, so keep design notes and test files in a folder of their own, outside the mod folder. OpenRAC's SDK has a `tools/package_mod.py --check` that checks a manifest the way a launcher does (give it a copy named `openrac-mod.json`, the name it knows).
9. **A GitHub release**: push a public repository, tag the version, and attach the zip. Players unzip the mod into their `mods` folder.
10. Write a README for players: what the mod does, what the settings mean, what it replaces, what you tested.

## 13. FAQ and pitfalls

**My mod does nothing.** Look in `mods\status.json` and the log ([section 11](#11-debugging)). Is the mod switched on (launcher Mods page, title screen Options > Mods, or a line in `active.txt`) and is "Load mods" on? It may be `refused` (disc, interface, hook), or switched off after an error. Try `RAC_OMNI_DRAW_DEMO=1` to see whether drawing works.

**I called `omni.fn(...)` and got "game functions can only be called while the game runs".** You called it in the main chunk. Create the function there (that is allowed), call it from `on_frame`, a hook or an event.

**`omni.on` errors.** It only works in the main chunk, and the name must be one of the events in [the reference](MODDING_REFERENCE.md#events-modes-pause-pages-icons).

**Text shows `?` instead of `[`, `]`, or a button picture where I wrote `%`.** That is the game's font. Use other characters.

**My text or bar disappears.** Drawing is immediate: call the draw functions every tick. Later draws are on top, and `layer = "menu"` is needed to appear above game menus.

**An icon does not show.** An icon the current level's art does not hold draws nothing. Check the name against `omni.draw.icons()`.

**My second gun still floats where the game puts it.** `pose.follow` only works where `omni.has("omni.pose.follow")` is true; call it again after a level change (the port forgets records) and check both mobys are alive.

**My hook never fires.** The function may not exist in the level you are in (level code differs); check with `omni.has("<id>")` and in other levels. Check the hook id is in `"hooks"` (or `"optional_hooks"`) and spelled exactly, and look in the log for `optional hook point ... skipped`.

**A hook breaks other mods.** You replaced a function without calling `original`. Call it unless you are really replacing the behaviour.

**It works on RAC-RS but not on OpenRAC (or the other way round).** Use `omni.has` for everything outside the base table; see [section 10](#10-other-ports-and-rac-rs-features).

**It works on the PAL disc but not on the NTSC disc.** Prefer fields to raw addresses and test on both. RAC-RS tells script mods the disc is `"pal"` or `"ntsc"`; the game data layout is the same on both.

**My save data is gone.** The store is per mod id and per save slot. Changing `id` loses it. A new game starts empty. Data from a mod that is switched off is kept, not deleted.

**My mod fails the time budget at first load.** Do less in the main chunk and in the first tick. Call `draw.icons()` once and keep the list.

**A RAC-RS feature (strafe, camera, keyboard) stopped working when my mod is on.** By design: your mod replaced it ([section 10](#10-other-ports-and-rac-rs-features)). If you did not mean to, stop taking those hook points or writing those values.

**Can I write a native (compiled) mod?** OmniLoader can also load OpenRAC native mods, but they run with no sandbox and no time budget and a crash ends the game. This guide does not cover them; Luau needs no compiler and cannot crash the game.

**What if a hook point I need does not exist?** Ask for it in an issue at github.com/AsobiCodes/rac-rs. Adding a hook point changes nothing for players without mods.

## 14. A bigger example: Arsenal Mastery

Arsenal Mastery comes with the game (preinstalled, switched off) in `bundled\mods\arsenal-mastery` of the release. It is a complete gameplay mod that shows most of the API working together; read it with this guide beside you. Its files, linked from this repository:

- [`omni-mod.json`](../bundled/mods/arsenal-mastery/omni-mod.json): the manifest. Note the empty `hooks` list and the long `optional_hooks` list (the mod runs, with fewer features, on a build that lacks a hook), the 13 settings and the 4 presets.
- [`main.luau`](../bundled/mods/arsenal-mastery/main.luau): a thin entry file. It checks `omni.has("omni.save")` and `omni.has("omni.events")`, logs one line and stops if the extension is missing, then loads the modules with `require` and forwards each tick and event to them. One place owns the listeners, which makes the time budget easy to follow.
- One module per feature: [`levels.luau`](../bundled/mods/arsenal-mastery/levels.luau) (XP and grades, kept per save slot), [`power.luau`](../bundled/mods/arsenal-mastery/power.luau), [`nanotech.luau`](../bundled/mods/arsenal-mastery/nanotech.luau), [`assist.luau`](../bundled/mods/arsenal-mastery/assist.luau), [`wrench.luau`](../bundled/mods/arsenal-mastery/wrench.luau), [`gold.luau`](../bundled/mods/arsenal-mastery/gold.luau), [`mega.luau`](../bundled/mods/arsenal-mastery/mega.luau), [`ammo.luau`](../bundled/mods/arsenal-mastery/ammo.luau), [`perks.luau`](../bundled/mods/arsenal-mastery/perks.luau) (with one file per weapon in the [`perks`](../bundled/mods/arsenal-mastery/perks) folder), [`bars.luau`](../bundled/mods/arsenal-mastery/bars.luau) and [`hudbar.luau`](../bundled/mods/arsenal-mastery/hudbar.luau) (the XP bars and the HUD bar, drawn with `omni.draw`).
- [`README.md`](../bundled/mods/arsenal-mastery/README.md): what it does for players.

Copy its layout for your own big mod: a thin `main.luau`, one module per feature, settings in the manifest, XP-like state in the per-save store. (v0.4.0 ships Arsenal Mastery 0.3.2; since v0.4.1 it is 0.3.3 and has its own page, github.com/AsobiCodes/arsenal-mastery.)

## Versions: what needs what

| What | Needs |
|---|---|
| Script mods, OmniLoader with the extension (`omni.loader_version == 1`), mod interface 1.3 (OpenRAC SDK), loader C interface 1.6 | RAC-RS v0.4.0 and later |
| `omni.has` names, events, drawing, per-save data, `fn`, `raycast`, `struct`, `struct_set`, `mobys`, `pose.*` including `follow`, `unwound`, `ticks_per_second`, `is_enemy`, `out_f32` | v0.4.0 |
| All hook ids and fields in MODDING_REFERENCE.md (the binding is the same in v0.4.0 and v0.4.1) | v0.4.0 |
| Pause menu Options > Mods: settings of running mods; title screen Options > Mods: on/off and settings | v0.4.0 |
| Time budget: warn once at 2 ms, stop only above 20 ms on 30 of 60 ticks | v0.4.1 (v0.4.0: stop if over 2 ms on 30 of 60 ticks) |
| Cheap log lines (a background writer) | v0.4.1 |
| A mod cannot write into the game's own program code by mistake | v0.4.1 |
| Launcher mod update from GitHub, Arsenal Mastery 0.3.3 | v0.4.1 (launcher 0.1.2) |

The mod interface version (`api` in the manifest) and the loader's own C interface version are separate numbers: you only ever write the first one (`1.3`).

## Credits

The mod format and the base of the `omni` table are OpenRAC's, from its MIT-licensed SDK, so mods work across projects; OmniLoader itself is this project's own code, written from that SDK (no code of OpenRAC's game port is used). The SDK licence is in `NOTICE.md` of the release. OmniLoader is MIT OR Apache-2.0.
