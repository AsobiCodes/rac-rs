--!nonstrict
-- Arsenal Mastery: V5 is the game's own gold version (setting `gold_at_v5`).
--
-- The gold flag of a weapon (field omni.weapon.<name>.gold, saved with the game) is ONE switch in retail: it changes
-- the look (gold icon, gold name, gold glow on the held weapon, gold effect colours) AND the retail gold stats of that
-- weapon together. This module sets it when an owned weapon with a retail gold version reaches V5, and remembers
-- which flags IT set (per save slot, key "g2_gold"): only those are ever cleared again (setting turned off, or the
-- weapon's level fell below 5 after a new game). Flags the player got in the game are never touched.
--
-- Gold bolts: retail counts 4 spare gold bolts away for every gold flag. A gold version earned by XP must cost none,
-- so the hook on `rac1.gold_bolt.count_spare` asks the game's own count with the mod-set flags cleared for the moment.
-- mega.luau adds the gold bolts paid for Mega through `add_spare_adjust`.

local DATA = {
	-- the 10 weapons that have a retail gold version (and a Gold Weapon Case)
	weapons = {
		"suck_cannon", "bomb_glove", "devastator", "blaster", "pyrocitor",
		"mine_glove", "tesla_claw", "glove_of_doom", "morph_o_ray", "decoy",
	},
	level = 5, -- level from which the weapon is gold
	poll_ticks = 30, -- how often the flags are checked (plus at once after a level change or an event)
	store_key = "g2_gold",
	old_store_key = "f2_gold", -- the draft version's store (same layout)
}

local M = {}
local api = nil
local ticks = 0
local dirty = true
local marks = {} -- name -> true: gold flags this mod set
local loaded = false
local held = nil -- name of the weapon whose flag mega.luau has cleared for a moment
local adjusts = {} -- fn(spare) -> spare, see add_spare_adjust
local last_pair = ""
local hook_ok = true

local function log(msg)
	if api ~= nil then
		api.log(msg)
	end
end

local function node_of(name)
	local ok, n = pcall(function()
		return omni.fields.omni.weapon[name]
	end)
	if ok then
		return n
	end
	return nil
end

local function load_marks()
	marks = {}
	local v = omni.save.get(DATA.store_key)
	if v == nil then
		v = omni.save.get(DATA.old_store_key)
	end
	if type(v) == "table" then
		for k, x in pairs(v) do
			if x then
				marks[k] = true
			end
		end
	end
	loaded = true
end

local function store_marks()
	local t = {}
	for k in pairs(marks) do
		t[k] = 1
	end
	omni.save.set(DATA.store_key, t)
end

local function playing()
	local mode = omni.mode()
	return mode == "gameplay" or mode == "pause" or mode == "vendor"
end

local function enabled()
	return hook_ok and omni.setting("gold_at_v5") == true
end

-- One pass over the weapons; returns how many flags it changed.
local function sync()
	if not playing() then
		return 0
	end
	if not loaded then
		load_marks()
	end
	local on = enabled()
	local changed = 0
	for _, name in ipairs(DATA.weapons) do
		local node = node_of(name)
		if node ~= nil and name ~= held then
			local flag = node.gold
			local want = on and api.levels.level(name) >= DATA.level and node.have == 1
			if want and flag == 0 then
				node.gold = 1
				marks[name] = true
				changed += 1
				log("gold: " .. name .. " set to gold (V" .. api.levels.level(name) .. ")")
			elseif not want and marks[name] then
				if flag ~= 0 then
					node.gold = 0
					changed += 1
					log("gold: " .. name .. " back to normal (the mod had set it)")
				end
				marks[name] = nil
			elseif flag == 0 and marks[name] then
				marks[name] = nil -- cleared by the game (new game): forget it
			end
		end
	end
	return changed
end

-- ---------------------------------------------------------------- for mega.luau

-- True when the mod set this weapon's gold flag (so it costs no gold bolts).
function M.is_marked(name)
	if not loaded then
		load_marks()
	end
	return marks[name] == true
end

-- mega.luau clears the flag of ONE weapon while the game runs a Gold Weapon Case: sync leaves it alone meanwhile.
function M.hold(name)
	held = name
end

function M.release()
	held = nil
end

-- fn(spare) -> spare: applied to the game's count of spare gold bolts (already without the mod-set flags).
function M.add_spare_adjust(fn)
	table.insert(adjusts, fn)
end

-- ---------------------------------------------------------------- the count_spare wrapper

local function spare(original)
	if not loaded then
		load_marks()
	end
	-- the game's own count with the mod-set flags in place (only asked when there are some: it is for the log)
	local retail = nil
	local any = false
	for name in pairs(marks) do
		local node = node_of(name)
		if node ~= nil and node.gold ~= 0 and name ~= held then
			any = true
		end
	end
	if any then
		local ok0, v0 = pcall(original)
		if ok0 then
			retail = v0
		end
	end
	-- clear the mod-set flags for the moment: the game then counts only gold weapons the player paid for
	local cleared = nil
	for name in pairs(marks) do
		local node = node_of(name)
		if node ~= nil and node.gold ~= 0 and name ~= held then
			node.gold = 0
			cleared = cleared or {}
			table.insert(cleared, node)
		end
	end
	local ok, raw = pcall(original)
	if cleared ~= nil then
		for _, node in ipairs(cleared) do
			node.gold = 1
		end
	end
	if not ok then
		error(raw, 0)
	end
	local s = raw
	for _, fn in ipairs(adjusts) do
		s = fn(s)
	end
	if s < 0 then
		s = 0
	end
	if cleared ~= nil or s ~= raw then
		local key = tostring(raw) .. ">" .. tostring(s)
		if key ~= last_pair then
			last_pair = key
			log(string.format("gold bolts: spare count %s with the mod-set gold flags, %d without, %d after the mod's adjustments (mod-set gold flags: %d)",
				tostring(retail), raw, s, cleared and #cleared or 0))
		end
	end
	return s
end

function M.setup(a)
	api = a
	a.levels.on_level(function()
		dirty = true
	end)
	if omni.has("rac1.gold_bolt.count_spare") then
		omni.hook("rac1.gold_bolt.count_spare", function(original)
			return spare(original)
		end)
	else
		hook_ok = false
		log("gold: this port cannot fix the gold bolt count: gold at V5 stays off")
	end
end

function M.on_frame()
	ticks += 1
	if dirty or ticks % DATA.poll_ticks == 0 then
		if playing() then
			dirty = false
			sync()
		end
	end
end

function M.on_event(name, a)
	if name == "new_game" then
		if a ~= true then
			marks = {}
		end
		loaded = true
		dirty = true
	elseif name == "save_loaded" then
		load_marks()
		dirty = true
	elseif name == "saving" then
		if loaded then
			store_marks()
		end
	elseif name == "setting" or name == "mode" or name == "level_loaded" then
		dirty = true
	end
end

return M
