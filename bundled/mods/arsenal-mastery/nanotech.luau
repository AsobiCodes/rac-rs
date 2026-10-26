--!nonstrict
-- Arsenal Mastery: Nanotech levels, Ratchet & Clank 3 style. Ratchet has a pool of health points (10 at the start, +1 a level, up to
-- 100 in the first playthrough and 200 in New Game+) that grows with the XP of every hit he scores, and enemy hits take more points
-- the further he is in the game.
--
-- HOW IT SITS ON THE GAME
--   The game keeps its own health in nanotech orbs (omni.hero.health, 1 .. omni.hero.max_health = 4, 5 or 8 with the upgrades; its
--   HUD draws at most 8), so the pool is the mod's and the game's value follows it: orbs = the share of the pool that is left
--   (all orbs only when the pool is full, at least one while it is above 0). The orb meter is hidden and the points are drawn
--   instead (see HUD below). The game's own rules stay in charge of everything else:
--   - a hit (rac1.player.take_damage, owned by power.luau, which hands it here): the pool loses the planet's damage, then the game
--     takes its one orb from a value set one above the new share, so its HUD and death path run as for any hit;
--   - an orb the game GIVES (a nanotech pickup, a level start, the refill after a death, an upgrade): the pool gains that share;
--   - an orb the game TAKES without a hit (instant kills set health 0): the pool follows it down.
--   The Premium / Ultra Nanotech upgrades still count: the pool's maximum is the level's points x (game maximum / 4).
-- The XP is a third of a weapon's (10/3 per point of health taken off an enemy, x the Nanotech XP rate), whatever weapon scored it.
local M = {}

-- ===================================================================================================
-- DATA
-- ===================================================================================================
local DATA = {
	start_hp = 10, -- level 1 = 10 points, every level +1 (as Ratchet & Clank 3)
	cap_hp = 100, cap_hp_ng = 200, -- the most points: first playthrough, New Game+
	-- Cumulative XP at which the pool reaches these points (index = points); the levels between are spread evenly. Past 100 points
	-- (New Game+) each point costs ng_step more.
	xp_curve = { { 10, 0 }, { 20, 2000 }, { 30, 5000 }, { 40, 9000 }, { 50, 14500 }, { 60, 21500 }, { 70, 30000 }, { 80, 41000 }, { 90, 55000 }, { 100, 72500 } }, -- x5 since mod 0.3.1
	ng_step = 1100,
	xp_per_health = 10 / 3, -- nanotech XP per point of health taken off an enemy: a third of a weapon's (power.luau) since mod 0.3.2
	-- Points an enemy hit (one game orb) takes, by level number (planet order as power.luau's T: Veldin Novalis Aridia Kerwan Eudora
	-- Rilgar Blarg Umbris Batalia Gaspar Orxon Pokitaru Hoven Gemlik Oltanis Quartu Kalebo Veldin-orbit Veldin-finale). Veldin: 4 hits
	-- from 10 points (as the game's 4 orbs); later about 6 to 8 hits at the expected level.
	damage = { [0] = 3, 4, 6, 8, 10, 11, 12, 13, 14, 15, 16, 17, 18, 18, 19, 20, 20, 20, 20 },
	ng_damage = 2.0, -- New Game+: enemy hits x this
	hard_damage = 1.3, -- Hard rebalance: x this (instead of the extra orb of every second hit)
	base_orbs = 4, -- the game's maximum without upgrades
	-- HUD (game 2D space 512 x 448), where the game's orb meter was: its centre x 256, orb row y 24 on PAL, 32 on NTSC (hud.md).
	hud_x = 256, hud_base = { pal = 24, ntsc = 32 },
	-- The Ratchet & Clank 3 health box in the game's own HUD colours: a dark panel with a blue frame and a cut lower right corner, the points large ("96"),
	-- the maximum small ("/100"), and under them the nanotech XP to the next level as a thin bar marked EXP. Offsets from the
	-- meter's centre and orb row.
	hud = { left = -60, top = -10, w = 120, h = 38, cut = 10,
		num_dx = 8, num_dy = 2, num_size = 1.5, max_dy = 11, max_size = 0.85,
		exp_dx = 8, exp_dy = 29, exp_w = 76, exp_h = 3, exp_label_dx = 88, exp_label_dy = 26, exp_size = 0.45 },
	fade_max = 8, -- the meter's fade steps (binding struct "hud_health": fade_bar, fade_orbs) run 0..8
	show_ticks = 100, start_ticks = 180, fade_ticks = 16, -- the game's meter: up 100 ticks after a change (180 at a level start)
	-- Colours of the game's own HUD (the ammo box's "40/350" blue, its dark backing; the XP bar's colours as hudbar.luau).
	color = 0x8898E8FF, max_color = 0x6A7AC8FF, low_color = 0xFF5040FF, shadow = 0x000000C0, low_share = 0.25,
	panel = 0x141A2890, frame = 0x4A5A9CFF, exp_fill = 0x8FA0D8FF, exp_back = 0x1C2438C8, exp_color = 0x6A7AC8FF,
	levelup_ticks = 90,
	-- Hero states of a death (hero_movement.md, death_respawn.md): 0x3d hit to death, 0x7c lava, 0x77 pit tumble, 0x6a swim death.
	-- Leaving them alive is the respawn, where the game refills its orbs: the pool refills too (also after a fall that took no orb).
	death_states = { [0x3d] = true, [0x7c] = true, [0x77] = true, [0x6a] = true },
}

local api, levels
local xp = 0 -- nanotech XP of this save
local pool, pmax = nil, 10 -- points left (nil until the first look at the game in gameplay), maximum
local last = nil -- the orbs as the mod last saw or wrote them
local mode = "gameplay"
local shown, popup = 0, 0
local have = false
local dying = false
local ticks = 0

local function log(msg) api.log(msg) end

local function f() return omni.fields.omni.hero end

-- ---------------------------------------------------------------------------------------- level
function M.enabled()
	return have and omni.setting("nanotech") ~= false
end

local function hp_of(n) return DATA.start_hp + n - 1 end -- the points of level n

local function cap() return (levels.ng_plus() and DATA.cap_hp_ng or DATA.cap_hp) - DATA.start_hp + 1 end

local function xp_for(n) -- cumulative XP at which level n begins
	local hp = hp_of(n)
	local c = DATA.xp_curve
	if hp <= c[1][1] then return 0 end
	for i = 2, #c do
		if hp <= c[i][1] then
			local a, b = c[i - 1], c[i]
			return math.floor(a[2] + (b[2] - a[2]) * (hp - a[1]) / (b[1] - a[1]) + 0.5)
		end
	end
	local top = c[#c]
	return top[2] + (hp - top[1]) * DATA.ng_step
end

function M.level()
	local n, c = 1, cap()
	while n < c and xp >= xp_for(n + 1) do n += 1 end
	return n
end

local function rmax()
	local ok, v = pcall(function() return f().max_health end)
	if ok and type(v) == "number" and v >= 1 and v <= 16 then return v end
	return DATA.base_orbs
end

local function pool_max(r) return hp_of(M.level()) * r / DATA.base_orbs end

-- The orbs that show `p` points of `m`: all only when full, 0 only at 0, else 1 .. r - 1 by share.
local function orbs_of(p, m, r)
	if p >= m then return r end
	if p <= 0 then return 0 end
	return math.clamp(math.ceil(p / m * (r - 1)), 1, r - 1)
end

local function write_orbs(n)
	local ok = pcall(function() f().health = n end)
	if ok then last = n end
end

-- ---------------------------------------------------------------------------------------- damage
local function ratio(key) -- a multiplier setting (float, 1.0 = as designed)
	local v = omni.setting(key)
	return (type(v) == "number" and v > 0) and v or 1
end

local function damage_per_hit()
	local lvl = omni.level() or 0
	local d = DATA.damage[lvl] or DATA.damage[#DATA.damage]
	if levels.ng_plus() then d *= DATA.ng_damage end
	if omni.setting("rebalance") == "hard" then d *= DATA.hard_damage end
	d *= ratio("enemy_damage")
	return d
end

-- power.luau's rac1.player.take_damage hands every hit here while nanotech is on.
function M.take_damage(original, amount)
	if amount == nil or amount < 1 or pool == nil then return original() end
	local okh, h = pcall(function() return f().health end)
	if not okh or type(h) ~= "number" or h <= 0 then return original() end
	local r = rmax()
	pmax = pool_max(r)
	local d = damage_per_hit()
	pool = math.max(0, pool - d)
	-- The game takes one orb (its HUD and, at 0, its death path): set one above the share the pool now shows.
	local want = orbs_of(pool, pmax, r)
	write_orbs(math.min(want + 1, r))
	original()
	last = want
	shown = DATA.show_ticks
	if pool <= 0 then log(string.format("nanotech: hit for %.1f, Ratchet down (N%d, %d max)", d, M.level(), pmax)) end
end

-- ---------------------------------------------------------------------------------------- per tick
local function sync()
	local okh, h = pcall(function() return f().health end)
	if not okh or type(h) ~= "number" then return end
	local r = rmax()
	local m = pool_max(r)
	if m ~= pmax and pool ~= nil then pool = math.min(pool, m) end
	pmax = m
	if pool == nil or last == nil then
		pool = (h >= r) and pmax or pmax * h / r
		last = h
	elseif h > last then
		-- the game gave orbs (pickup, level start, refill after a death, upgrade): the pool gains that share
		pool = (h >= r and last == 0) and pmax or math.min(pmax, pool + (h - last) * pmax / r)
		shown = DATA.show_ticks
	elseif h < last then
		-- taken without a hit through the mod (an instant kill sets 0): follow it down
		pool = (h <= 0) and 0 or math.min(pool, pmax * h / r)
		shown = DATA.show_ticks
	end
	local oks, st = pcall(function() return omni.fields.hero.state end)
	if oks and type(st) == "number" then
		if DATA.death_states[st] then
			dying = true
		elseif dying and h > 0 then
			dying = false
			pool = pmax
		end
	end
	local want = orbs_of(pool, pmax, r)
	if want ~= h then write_orbs(want) else last = h end
end

local function on_hit(name, victim, damage, killed)
	if not M.enabled() or damage == nil or damage <= 0 then return end
	local before = M.level()
	local rate = omni.setting("nanotech_xp_rate")
	if type(rate) ~= "number" then rate = 1 end
	xp += damage * DATA.xp_per_health * rate
	local now = M.level()
	if now > before then
		local r = rmax()
		local old = pmax
		pmax = pool_max(r)
		if pool ~= nil then pool = math.min(pmax, pool + (pmax - old)) end -- the new point comes full, as a level-up in RaC 3
		popup = DATA.levelup_ticks -- the box stays up a while: the new maximum shows in it
		shown = DATA.show_ticks
		log(string.format("nanotech: N%d (%d XP): %d points", now, math.floor(xp), pmax))
	end
end

function M.on_frame()
	ticks += 1
	if popup > 0 then popup -= 1 end
	if shown > 0 then shown -= 1 end
	if not M.enabled() or mode ~= "gameplay" then return end
	sync()
end

-- ---------------------------------------------------------------------------------------- HUD
-- The game's orb meter is switched off (rac1.omni.hud.health_draw: the draw is skipped, its result kept) and the points are drawn
-- in its place as Ratchet & Clank 3 shows them: a framed box with "points/maximum" and the nanotech XP bar (EXP) under it. They fade with the game's
-- own meter (its fade steps, read from the meter's element in the hook), and also show for a while after any change of the points.
local game_f1, game_f2, game_seen = 0, 0, -1

local function hide_orbs(original, elem)
	if not M.enabled() or elem == nil or elem == 0 then return original(elem) end
	local ok, e = pcall(omni.struct, "hud_health", elem)
	if not ok or e == nil then return original(elem) end
	game_f1, game_f2, game_seen = e.fade_bar, e.fade_orbs, ticks
	return e.result
end

local function fade()
	local own = math.min(1, shown / DATA.fade_ticks)
	if popup > 0 then own = 1 end
	if game_seen >= ticks - 1 then
		return math.max(own, game_f1 / DATA.fade_max), math.max(own, game_f2 / DATA.fade_max)
	end
	return own, own
end

local function with_alpha(c, a) return c - (c % 256) + math.floor((c % 256) * a) end

-- The nanotech XP of this level towards the next one, 0..1 (1 at the cap).
local function exp_share()
	local n = M.level()
	if n >= cap() then return 1 end
	local a, b = xp_for(n), xp_for(n + 1)
	return (b > a) and math.clamp((xp - a) / (b - a), 0, 1) or 1
end

-- The panel: body with the lower right corner cut in 1-unit steps (the sharp HUD smooths them into a clean diagonal), frame 1 unit wide around it.
local function panel(d, x, y, w, h, cut, body, frame)
	local o = { layer = "hud" }
	d.rect(x, y, w, h - cut, body, o)
	for k = 0, cut - 1 do d.rect(x, y + h - cut + k, w - k - 1, 1, body, o) end
	d.rect(x, y, w, 1, frame, o)
	d.rect(x, y, 1, h, frame, o)
	d.rect(x, y + h - 1, w - cut, 1, frame, o)
	d.rect(x + w - 1, y, 1, h - cut, frame, o)
	for k = 0, cut - 1 do d.rect(x + w - k - 1, y + h - cut + k, 1, 1, frame, o) end
end

function M.draw()
	if not M.enabled() or pool == nil or mode ~= "gameplay" then return end
	local a1, a2 = fade()
	if a1 <= 0 and a2 <= 0 then return end
	local d = omni.draw
	local L = DATA.hud
	local x = DATA.hud_x + L.left
	local y = (DATA.hud_base[api.disc] or DATA.hud_base.pal) + L.top
	local low = pool <= pmax * DATA.low_share
	panel(d, x, y, L.w, L.h, L.cut, with_alpha(DATA.panel, a1), with_alpha(DATA.frame, a1))
	local now, top = tostring(math.ceil(pool)), "/" .. tostring(math.floor(pmax + 0.5))
	local sh = with_alpha(DATA.shadow, a2)
	-- the two numbers meet at the panel's middle: points right-aligned to it, maximum left-aligned
	local mid = x + L.w * 0.55
	d.text(mid, y + L.num_dy, now, { size = L.num_size, color = with_alpha(low and DATA.low_color or DATA.color, a2), shadow = sh, align = "right", layer = "hud" })
	d.text(mid + 1, y + L.max_dy, top, { size = L.max_size, color = with_alpha(DATA.max_color, a2), shadow = sh, align = "left", layer = "hud" })
	d.bar(x + L.exp_dx, y + L.exp_dy, L.exp_w, L.exp_h, exp_share(), with_alpha(DATA.exp_fill, a1), with_alpha(DATA.exp_back, a1), { layer = "hud" })
	d.text(x + L.exp_label_dx, y + L.exp_label_dy, "EXP", { size = L.exp_size, color = with_alpha(DATA.exp_color, a2), shadow = sh, align = "left", layer = "hud" })
end

-- ---------------------------------------------------------------------------------------- save / module
function M.describe()
	return string.format("N%d (%d XP, %s / %d points)", M.level(), math.floor(xp), pool and tostring(math.ceil(pool)) or "?", math.floor(pmax + 0.5))
end

function M.setup(a)
	api = a
	levels = a.levels
	local ok = pcall(function() return f().health end)
	have = ok and omni.save ~= nil
	if not have then
		log("nanotech: this port has no omni.hero.health field: nanotech levels are off")
		return
	end
	levels.on_hit(on_hit)
	if omni.has("rac1.omni.hud.health_draw") and omni.has("omni.struct") then
		omni.hook("rac1.omni.hud.health_draw", hide_orbs)
	else
		log("nanotech: no rac1.omni.hud.health_draw (or omni.struct) on this port: the game's orbs stay under the points")
	end
end

function M.on_event(name, a, b)
	if name == "mode" and type(a) == "string" then
		mode = a
	elseif name == "level_loaded" then
		last, shown, dying = nil, DATA.start_ticks, false
	elseif name == "new_game" then
		if not a then xp = 0 end
		pool, last = nil, nil
		log("nanotech: new game" .. (a and " (plus): XP kept, " or ": ") .. M.describe())
	elseif name == "save_loaded" then
		local v = omni.save.get("nanotech_xp")
		xp = (type(v) == "number" and v >= 0) and v or 0
		pool, last = nil, nil
		log("nanotech: save " .. tostring(a) .. ": " .. M.describe())
	elseif name == "saving" then
		omni.save.set("nanotech_xp", math.floor(xp * 10 + 0.5) / 10)
	end
end

return M
