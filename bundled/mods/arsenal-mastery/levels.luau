--!nonstrict
-- Arsenal Mastery: weapon levels. XP, levels V1..V99, caps, Mega state, New Game+, kills, per-slot save data with migration.
-- Other modules use the API at the bottom (names fixed by the mod contract). Every number is in the DATA table.
-- Only hook ids, fields and binding functions are used: no raw memory.
local M = {}

-- ===================================================================================================
-- DATA
-- ===================================================================================================
local DATA = {
	-- Cumulative XP at which level 2, 3, 4, 5 begins (V1 = 0). Multiplied by the weapon's xp_scale.
	xp_to = { [2] = 750, [3] = 2250, [4] = 5000, [5] = 10000 }, -- x5 since mod 0.3.1 (was 150 / 450 / 1000 / 2000)
	-- XP needed after V5 to reach Mega (V6) by XP alone. Only the weapons in xp_mega can; the others buy Mega.
	xp_mega_cost = 15000,
	-- XP earned AFTER Mega (V6) at which V7 and V8 begin (cumulative since Mega), then the Endless levels:
	-- each further level needs this much more than the one before.
	after_mega = { [7] = 20000, [8] = 40000 },
	endless_growth = 1.15,
	endless_cap = 99,
	-- Weapons bought late in the game need less XP (x0.75 mid-game, x0.5 the last ones) so V5 stays reachable in one run.
	xp_scale = {
		wrench = 1.0, bomb_glove = 1.0, blaster = 1.0, pyrocitor = 1.0, suck_cannon = 1.0, taunter = 1.0,
		devastator = 0.75, visibomb = 0.75, mine_glove = 0.75, walloper = 0.75, tesla_claw = 0.75,
		glove_of_doom = 0.5, morph_o_ray = 0.5, ryno = 0.5, drone_device = 0.5, decoy = 0.5,
	},
	-- The five weapons without a gold case: they reach Mega by XP in New Game+.
	xp_mega = { taunter = true, walloper = true, visibomb = true, drone_device = true, ryno = true },
	-- Damage multiplier of each level over V1. V5 includes the retail gold power; the gold factor of the weapons whose
	-- gold version already hits harder is divided out (gold_power) so the total stays about the same.
	power = { 1.0, 1.2, 1.5, 1.9, 2.5 },
	power_no_gold_v5 = 2.6, -- weapons that have no gold version (design: about x2.6)
	mega_power = 2.0, giga_power = 1.25, omega_power = 1.25, endless_power = 1.03,
	has_gold = {
		suck_cannon = true, bomb_glove = true, devastator = true, blaster = true, pyrocitor = true, mine_glove = true,
		tesla_claw = true, glove_of_doom = true, morph_o_ray = true, decoy = true,
	},
	gold_power = { pyrocitor = 2.0, tesla_claw = 1.5 }, -- the strength the game itself adds with the gold flag
	save_format = 2,
	-- The game's item id of every weapon.
	item_id = {
		wrench = 8, suck_cannon = 9, bomb_glove = 10, devastator = 11, visibomb = 13, taunter = 14, blaster = 15, pyrocitor = 16,
		mine_glove = 17, walloper = 18, tesla_claw = 19, glove_of_doom = 20, morph_o_ray = 21, ryno = 23, drone_device = 24, decoy = 25,
	},
	-- v1 store (format 1): grade g started at these XP (grade 1..5); migrated to the same place inside the v2 levels.
	v1_thresholds = { 0, 100, 300, 700, 1500 },
}
M.DATA = DATA

M.WEAPONS = {
	"wrench", "suck_cannon", "bomb_glove", "devastator", "visibomb", "taunter", "blaster", "pyrocitor",
	"mine_glove", "walloper", "tesla_claw", "glove_of_doom", "morph_o_ray", "ryno", "decoy", "drone_device",
}
M.ITEM_ID = DATA.item_id

-- ===================================================================================================
-- state
-- ===================================================================================================
local api = nil
local xp = {} -- name -> XP (float)
local kills = {} -- name -> kills
local mega = {} -- name -> XP at the moment Mega began (nil = not Mega)
local last = {} -- name -> level reported to the listeners
local ng_event = false -- New Game+ seen through the new_game event
local level_listeners, hit_listeners, kill_listeners = {}, {}, {}
local after_cum = {} -- cumulative XP after Mega at which level n begins (n = 7..99), before the weapon scale
local logged_power = {}

local function log(msg)
	if api then api.log(msg) else omni.log("[am] " .. msg) end
end

local function setting(key)
	if api then return api.setting(key) end
	return omni.setting(key)
end

-- Endless table: after_cum[n] for n = 7..99 (V7, V8 from DATA.after_mega, then each step 15 percent more).
do
	local step = DATA.after_mega[8] - DATA.after_mega[7]
	after_cum[7] = DATA.after_mega[7]
	after_cum[8] = DATA.after_mega[8]
	for n = 9, DATA.endless_cap do
		step = step * DATA.endless_growth
		after_cum[n] = after_cum[n - 1] + step
	end
end

local function reset_all()
	for _, n in ipairs(M.WEAPONS) do
		xp[n], kills[n], mega[n], last[n] = 0, 0, nil, nil
	end
end

-- ---------------------------------------------------------------------------------------- settings
local function start_level()
	local v = setting("start_level")
	if type(v) == "string" then
		local n = tonumber(string.match(v, "^v(%d+)$"))
		if n ~= nil and n >= 2 then return n end
	end
	return 0
end

local function endless() return setting("level_cap") == "endless" end

function M.ng_plus()
	if ng_event then return true end
	local ok, v = pcall(function() return omni.fields.omni.challenge end)
	return ok and type(v) == "number" and v > 0
end

local function sandbox_mega() return start_level() >= 6 end

-- ---------------------------------------------------------------------------------------- thresholds
local function th(name, n) -- cumulative XP at which level n (2..5) begins for this weapon
	return DATA.xp_to[n] * (DATA.xp_scale[name] or 1)
end

local function mega_from_xp(name) -- XP at which a no-case weapon reaches Mega
	local s = DATA.xp_scale[name] or 1
	return DATA.xp_to[5] * s + DATA.xp_mega_cost * s
end

local function mega_allowed_by_xp(name) -- the XP route to Mega is open for this weapon right now
	if not DATA.xp_mega[name] then return false end
	return M.ng_plus() or endless() or sandbox_mega()
end

-- the highest level reachable now
function M.cap(name)
	local mg = mega[name] ~= nil
	if endless() then return mg and DATA.endless_cap or 5 end
	if mg and (M.ng_plus() or sandbox_mega()) then return 8 end
	return 5
end

-- the level the XP is worth (uncapped by listeners, capped by cap())
local function compute_level(name)
	local a = xp[name] or 0
	local l = 1
	for n = 2, 5 do
		if a >= th(name, n) then l = n else break end
	end
	if l == 5 and mega[name] ~= nil then
		l = 6
		local rel = a - mega[name]
		local s = DATA.xp_scale[name] or 1
		for n = 7, DATA.endless_cap do
			if rel >= after_cum[n] * s then l = n else break end
		end
	end
	local c = M.cap(name)
	if l > c then l = c end
	return l
end

-- ---------------------------------------------------------------------------------------- listeners
local function fire_level(name, level, old)
	for _, fn in ipairs(level_listeners) do
		local ok, e = pcall(fn, name, level, old)
		if not ok then log("on_level listener error: " .. tostring(e)) end
	end
end

-- Report a level change (or the level once, old = nil, when force is true).
local function sync(name, force)
	local l = compute_level(name)
	local old = last[name]
	if force then
		last[name] = l
		fire_level(name, l, nil)
	elseif old ~= l then
		last[name] = l
		if old ~= nil then
			log(string.format("level %s V%d -> V%d (xp %.1f)", name, old, l, xp[name]))
		end
		fire_level(name, l, old)
	end
end

local function sync_all(force)
	for _, n in ipairs(M.WEAPONS) do sync(n, force) end
end

-- XP route to Mega, and the sandbox start level.
local function apply_mega_rules(name)
	if mega[name] == nil and mega_allowed_by_xp(name) and (xp[name] or 0) >= mega_from_xp(name) then
		mega[name] = mega_from_xp(name)
		log("mega by xp: " .. name)
	end
end

local function apply_start(name)
	local sl = start_level()
	if sl < 2 then return end
	local s = DATA.xp_scale[name] or 1
	local need
	if sl <= 5 then
		need = th(name, sl)
	else
		if mega[name] == nil then mega[name] = mega_from_xp(name) end
		need = mega[name] + (sl >= 7 and (after_cum[math.min(sl, DATA.endless_cap)] * s) or 0)
	end
	if (xp[name] or 0) < need then xp[name] = need end
end

local function refresh(force)
	for _, n in ipairs(M.WEAPONS) do
		apply_start(n)
		apply_mega_rules(n)
	end
	sync_all(force)
end

-- ---------------------------------------------------------------------------------------- API
function M.level(name) return last[name] or compute_level(name) end
function M.xp(name) return xp[name] or 0 end
function M.kills(name) return kills[name] or 0 end

function M.progress(name)
	local l = M.level(name)
	local a = xp[name] or 0
	local s = DATA.xp_scale[name] or 1
	if l >= M.cap(name) then
		-- the XP route to Mega is shown on a V5 no-case weapon
		if l == 5 and mega[name] == nil and mega_allowed_by_xp(name) then
			local lo, hi = th(name, 5), mega_from_xp(name)
			return math.max(0, math.min(1, (a - lo) / (hi - lo)))
		end
		return 1
	end
	if l < 5 then
		local lo, hi = 0, th(name, l + 1)
		if l > 1 then lo = th(name, l) end
		return math.max(0, math.min(1, (a - lo) / (hi - lo)))
	end
	if l == 5 then return 1 end -- waiting for Mega to be bought
	local rel = a - (mega[name] or a)
	local lo = (l == 6) and 0 or after_cum[l] * s
	local hi = after_cum[l + 1] * s
	return math.max(0, math.min(1, (rel - lo) / (hi - lo)))
end

function M.add_xp(name, amount)
	if xp[name] == nil or amount == nil or amount <= 0 then return end
	local rate = setting("xp_rate")
	if type(rate) ~= "number" then rate = 1 end
	xp[name] += amount * rate
	apply_mega_rules(name)
	sync(name, false)
end

function M.is_mega(name) return mega[name] ~= nil end

function M.set_mega(name)
	if xp[name] == nil or mega[name] ~= nil then return end
	local floor5 = th(name, 5)
	if xp[name] < floor5 then xp[name] = floor5 end
	mega[name] = xp[name]
	log("mega bought: " .. name)
	sync(name, false)
end

-- Damage multiplier of the level (1.0 with the setting off).
function M.power(name)
	if setting("level_power") ~= true then return 1.0 end
	local l = M.level(name)
	local p
	if l <= 4 then
		p = DATA.power[l] or 1.0
	else
		local gold = DATA.has_gold[name] and setting("gold_at_v5") == true
		p = gold and DATA.power[5] or DATA.power_no_gold_v5
		-- the game itself already hits harder with the gold flag (Pyrocitor, Tesla Claw): take that out of the level factor
		if gold and DATA.gold_power[name] then p = p / DATA.gold_power[name] end
		if l >= 6 then p = p * DATA.mega_power end
		if l >= 7 then p = p * DATA.giga_power end
		if l >= 8 then p = p * DATA.omega_power end
		if l >= 9 then p = p * DATA.endless_power ^ (l - 8) end
	end
	return p
end

function M.on_level(fn) table.insert(level_listeners, fn) end
function M.on_hit(fn) table.insert(hit_listeners, fn) end
function M.on_kill(fn) table.insert(kill_listeners, fn) end

-- Called by power.luau after each scored hit (fn(weapon, victim_moby, damage_taken, killed)); a kill also fires on_kill.
function M.report_hit(name, victim, damage, killed)
	for _, fn in ipairs(hit_listeners) do
		local ok, e = pcall(fn, name, victim, damage, killed)
		if not ok then log("on_hit listener error: " .. tostring(e)) end
	end
	if killed then
		kills[name] = (kills[name] or 0) + 1
		for _, fn in ipairs(kill_listeners) do
			local ok, e = pcall(fn, name, victim)
			if not ok then log("on_kill listener error: " .. tostring(e)) end
		end
	end
end

-- ---------------------------------------------------------------------------------------- save / load
-- MIGRATE[n](data) turns save data of format n into format n + 1. Format 1 = the v1 store {xp = {weapon = XP}} with grades 1..5
-- at 0 / 100 / 300 / 700 / 1500 XP: the XP moves to the same place inside the v2 levels.
local MIGRATE = {
	[0] = function(d)
		local t = {}
		for _, n in ipairs(M.WEAPONS) do
			if type(d[n]) == "number" then t[n] = d[n] end
		end
		return { format = 1, xp = t }
	end,
	[1] = function(d)
		local out = {}
		local v1 = DATA.v1_thresholds
		for _, n in ipairs(M.WEAPONS) do
			local a = type(d.xp) == "table" and d.xp[n] or nil
			if type(a) == "number" and a > 0 then
				local g = 1
				for i = 2, #v1 do
					if a >= v1[i] then g = i else break end
				end
				if g >= 5 then
					out[n] = th(n, 5)
				else
					local f = (a - v1[g]) / (v1[g + 1] - v1[g])
					local lo = (g == 1) and 0 or th(n, g)
					out[n] = lo + f * (th(n, g + 1) - lo)
				end
			end
		end
		return { format = 2, xp = out, mega = {}, kills = {}, ng = false }
	end,
}
M.MIGRATE = MIGRATE

local function write_store()
	local x, mg, k = {}, {}, {}
	for _, n in ipairs(M.WEAPONS) do
		local a = xp[n]
		if a and a > 0 then x[n] = math.floor(a * 10 + 0.5) / 10 end
		if mega[n] ~= nil then mg[n] = math.floor(mega[n] * 10 + 0.5) / 10 end
		if (kills[n] or 0) > 0 then k[n] = kills[n] end
	end
	local save = omni.save
	save.set("format", DATA.save_format)
	save.set("xp", x)
	save.set("mega", mg)
	save.set("kills", k)
	save.set("ng", M.ng_plus())
end

local function read_store()
	reset_all()
	local save = omni.save
	local d = { format = save.get("format"), xp = save.get("xp"), mega = save.get("mega"), kills = save.get("kills"), ng = save.get("ng") }
	if d.xp == nil and d.format == nil then
		local flat, any = {}, false
		for _, n in ipairs(M.WEAPONS) do
			local v = save.get(n)
			if type(v) == "number" then flat[n] = v; any = true end
		end
		if not any then return false end
		d = flat
	end
	local fmt = type(d.format) == "number" and d.format or 0
	if fmt > DATA.save_format then
		log("save data format " .. fmt .. " is newer than this mod's (" .. DATA.save_format .. "): reading what it knows")
	end
	while fmt < DATA.save_format do
		local step = MIGRATE[fmt]
		if step == nil then log("no migration from save format " .. fmt); return false end
		d = step(d)
		fmt = d.format
		log("save data migrated to format " .. fmt)
	end
	if type(d.xp) ~= "table" then return false end
	for _, n in ipairs(M.WEAPONS) do
		local a = d.xp[n]
		if type(a) == "number" and a >= 0 then xp[n] = a end
		local m = type(d.mega) == "table" and d.mega[n] or nil
		if type(m) == "number" then mega[n] = m end
		local kk = type(d.kills) == "table" and d.kills[n] or nil
		if type(kk) == "number" then kills[n] = kk end
	end
	ng_event = d.ng == true
	return true
end

-- One line: every weapon's level (for the log).
function M.describe()
	local t = {}
	for _, n in ipairs(M.WEAPONS) do table.insert(t, n .. "=V" .. M.level(n) .. (mega[n] ~= nil and "M" or "")) end
	return table.concat(t, " ") .. (M.ng_plus() and " (New Game+)" or "")
end

-- ---------------------------------------------------------------------------------------- module
function M.setup(a)
	api = a
	reset_all()
end

function M.on_event(name, a, b)
	if name == "new_game" then
		if not a then
			reset_all()
			ng_event = false
		else
			ng_event = true
		end
		log("new_game" .. (a and " (plus)" or "") .. ": xp " .. (a and "kept" or "reset"))
		refresh(true)
	elseif name == "save_loaded" then
		local found = read_store()
		log(string.format("save_loaded slot %s: %s (written by mod %s)", tostring(a), found and "levels read" or "no data, levels reset", tostring(omni.save.version())))
		refresh(true)
		log("levels now: " .. M.describe())
	elseif name == "saving" then
		write_store()
		log("saving slot " .. tostring(a) .. ": format " .. DATA.save_format .. "; levels: " .. M.describe())
	elseif name == "setting" then
		if a == "level_cap" or a == "start_level" or a == "level_power" or a == "gold_at_v5" then refresh(false) end
	elseif name == "level_loaded" then
		refresh(false)
	end
end

return M
