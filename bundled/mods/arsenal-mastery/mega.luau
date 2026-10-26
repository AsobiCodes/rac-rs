--!nonstrict
-- Arsenal Mastery: V6 Mega is BOUGHT in New Game+ at the game's own Gold Weapon Cases (Novalis and Gemlik Base).
--
-- Retail has no shop screen for gold weapons: ten display cases each sell one weapon by a talk prompt, for bolts
-- plus 4 spare gold bolts, and a case whose weapon is already gold goes dead. To sell Mega, this module wraps the
-- case's update (hook `rac1.omni.goldcase.update`). For a weapon at V5 that is not Mega yet it clears the weapon's
-- gold flag and writes the Mega price into the game's price table for the duration of the game's own update, so the
-- case offers the purchase as if the weapon were not gold yet; right after the update the flag and the retail price
-- are put back. The game writes the flag when the player accepts: that is how a purchase is recognised.
--
-- Mega price: the retail gold price of the weapon times DATA.price_mult (1.0 = 10,000 to 60,000 bolts), always under
-- the table's 65,535 limit with the default. If a tuned price is higher, the table gets 65,535 and the mod takes the
-- rest from the bolts itself after the purchase (the offer is hidden until the player has the full price).
-- Gold bolts: DATA.gold_bolts (4) per Mega, counted in the spare gold bolt count through gold.luau (the game counts,
-- it never spends them). Megas bought are remembered per save slot (key "g2_mega"); a gold version earned by XP
-- (gold.luau) costs no gold bolts, so all 40 gold bolts of a save can go to Mega.
--
-- The 5 weapons with no case (Taunter, Walloper, Visibomb, Drone Device, R.Y.N.O.) are not handled here.

local DATA = {
	-- case number -> weapon (the same table the game uses; even numbers exist only in New Game+)
	cases = {
		[0] = "tesla_claw", "bomb_glove", "devastator", "pyrocitor", "mine_glove",
		"blaster", "morph_o_ray", "glove_of_doom", "decoy", "suck_cannon",
	},
	level = 5, -- the level a weapon must have to be offered Mega
	cache_ticks = 10, -- how long the answer "may this weapon be offered Mega" is kept (a level change clears it)
	case_class = 304, -- the Gold Weapon Case moby (its display moby has its own class)
	gold_bolts = 4, -- gold bolts one Mega costs
	price_mult = 1.0, -- Mega price = retail gold price x this
	price_cap = 65535, -- the price table holds a u16
	store_key = "g2_mega",
}

local M = {}
local api = nil
local gold = nil
local loaded = false
local paid = {} -- name -> true: Megas bought at a case (each cost DATA.gold_bolts gold bolts)
local retail_price = {} -- name -> the table's price, read after each level load
local active = nil -- { name, full, retail_flag } while the game runs an eligible case
local struct_ok = false
local seen = {} -- name -> true: the offer was logged since the last level load
local infos = {} -- moby address -> case_info (cleared with each level)
local elig = {} -- name -> { tick, eligible }
local now = 0

local function log(msg)
	if api ~= nil then
		api.log(msg)
	end
end

local nodes = {}
local function node_of(name)
	local n = nodes[name]
	if n ~= nil then
		return n
	end
	local ok, v = pcall(function()
		return omni.fields.omni.weapon[name]
	end)
	if ok and v ~= nil then
		nodes[name] = v
		return v
	end
	return nil
end

local function load_paid()
	paid = {}
	local v = omni.save.get(DATA.store_key)
	if type(v) == "table" then
		for k, x in pairs(v) do
			if x then
				paid[k] = true
			end
		end
	end
	loaded = true
end

local function count_paid()
	local n = 0
	for _ in pairs(paid) do
		n += 1
	end
	return n
end

-- The mod's effects are on at all (not the "just XP" preset).
local function effects_on()
	return omni.setting("gold_at_v5") == true or omni.setting("level_power") == true or omni.setting("perks") == true
end

-- Reads (and keeps) the retail gold price of every case weapon; the table reloads with each level.
local function read_prices()
	retail_price = {}
	for _, name in pairs(DATA.cases) do
		local node = node_of(name)
		if node ~= nil then
			local ok, p = pcall(function()
				return node.gold_price
			end)
			if ok and p ~= nil and p > 0 then
				retail_price[name] = p
			end
		end
	end
end

local function eligible_now(name)
	if not effects_on() or not api.levels.ng_plus() then
		return false
	end
	-- (levels.cap stays 5 until Mega is bought, so it cannot be asked here)
	local lv = api.levels
	if lv.is_mega(name) or lv.level(name) < DATA.level then
		return false
	end
	local node = node_of(name)
	return node ~= nil and node.have == 1
end

-- The case update runs for up to 20 mobys every tick: the answer is kept for the tick.
local function eligible(name)
	local c = elig[name]
	if c ~= nil and now - c[1] < DATA.cache_ticks then
		return c[2]
	end
	local v = eligible_now(name)
	elig[name] = { now, v }
	return v
end

-- What a moby is, asked once: { name = the case's weapon, case = true for the case itself (class 304) }; false = not ours.
local function case_info(moby)
	local c = infos[moby]
	if c ~= nil then
		return c
	end
	c = false
	local m = omni.struct("moby", moby)
	if m ~= nil and m.pvar ~= 0 then
		local gc = omni.struct("gold_case", m.pvar)
		local name = gc ~= nil and DATA.cases[gc.index] or nil
		if name ~= nil then
			c = { name = name, index = gc.index, case = (m.class == DATA.case_class) }
		end
	end
	infos[moby] = c
	return c
end

-- True when the game's update did not finish inside `original` (loader ABI 1.5: omni.unwound()).
-- The case update waits for frames while the fanfare plays: the port lets the call go, the REST of the update then runs
-- without this hook, and the code after `original` below runs early. That is what we want here: the game has already
-- written the gold flag and taken the bolts when it first waits, and the flag/price must be put back before the rest runs.
-- (`original` gives nil for this void function on every call, finished or not, so nil alone says nothing: unwound() does.)
local unwound_logged = false
local function cut_short()
	return omni.has("omni.unwound") and omni.unwound() == true
end

local function wrapper(original, moby)
	if not loaded then
		load_paid()
	end
	local c = case_info(moby)
	if c == false or not eligible(c.name) then
		return original(moby)
	end
	local name = c.name
	local node = node_of(name)
	if not c.case then
		-- the display moby destroys itself when it sees the gold flag: hide the flag for its update too
		if node == nil then
			return original(moby)
		end
		local keep = node.gold
		node.gold = 0
		local okd, rd = pcall(original, moby)
		node.gold = keep
		if not okd then
			error(rd, 0)
		end
		return rd
	end
	if retail_price[name] == nil then
		read_prices()
	end
	local retail = retail_price[name]
	if node == nil or retail == nil then
		return original(moby)
	end
	local full = math.floor(retail * DATA.price_mult)
	local table_price = math.min(full, DATA.price_cap)
	local prior = node.gold
	if not seen[name] then
		seen[name] = true
		log(string.format("mega: case %d (%s) offers Mega: price %d bolts (table %d, retail %d) + %d gold bolts; gold flag was %d",
			c.index, name, full, table_price, retail, DATA.gold_bolts, prior))
	end
	active = { name = name, full = full, retail_flag = prior ~= 0 and not gold.is_marked(name) }
	-- (one small table per call: only for eligible cases)
	gold.hold(name)
	node.gold = 0
	if table_price ~= retail then
		node.gold_price = table_price
	end
	local ok, r = pcall(original, moby)
	local after = node.gold
	if table_price ~= retail then
		node.gold_price = retail
	end
	node.gold = prior
	active = nil
	gold.release()
	if not ok then
		error(r, 0)
	end
	local early = cut_short()
	if early and not unwound_logged then
		unwound_logged = true
		log(string.format("mega: the case update was cut short at a frame wait (unwound; original returned %s): the rest of it runs without the hook; flag after the call %d",
			tostring(r), after))
	end
	-- the game writes the flag when the player accepts (and the case moby goes 1 -> 4, the autosave wait)
	if after ~= 0 and not paid[name] then
		paid[name] = true
		if full > table_price then
			omni.fields.bolts = math.max(0, omni.fields.bolts - (full - table_price))
		end
		local state = "-1"
		if omni.moby_valid(moby) then
			local m2 = omni.struct("moby", moby)
			state = tostring(m2 ~= nil and m2.state or -1)
		end
		if early then
			state = state .. " (update cut short at its first frame wait)"
		end
		api.levels.set_mega(name)
		elig = {}
		log(string.format("mega: %s bought at case %d (price %d, table %d, flag written by the game %d, case state now %s, gold bolts paid in all %d)",
			name, c.index, full, table_price, after, state, DATA.gold_bolts * count_paid()))
	end
	return r
end

-- The spare gold bolt count as the cases and the Gold Bolts page see it.
local function adjust(s)
	local n = count_paid()
	if n == 0 and active == nil then
		return s
	end
	s = s - DATA.gold_bolts * n
	if active ~= nil then
		-- the case asks "more than 3 spare?": make that mean "enough for this Mega"
		if active.retail_flag then
			s = s - 4 -- paid for in the game: the flag was cleared for the call, so count it back
		end
		s = s - DATA.gold_bolts + 4
		if omni.fields.bolts < active.full then
			s = 0
		end
	end
	if s < 0 then
		s = 0
	end
	return s
end

function M.setup(a)
	api = a
	local ok, g = pcall(require, "gold")
	if not ok or type(g) ~= "table" or type(g.add_spare_adjust) ~= "function" then
		log("mega: gold module missing: Mega at the cases stays off")
		return
	end
	gold = g
	struct_ok = omni.has("omni.struct") and omni.has("omni.mobys") and omni.has("rac1.omni.goldcase.update")
		and omni.has("rac1.gold_bolt.count_spare")
	if not struct_ok then
		log("mega: this port has no Gold Weapon Case hook: Mega at the cases stays off")
		return
	end
	gold.add_spare_adjust(adjust)
	a.levels.on_level(function()
		elig = {}
	end)
	omni.hook("rac1.omni.goldcase.update", function(original, moby)
		return wrapper(original, moby)
	end)
end

function M.on_frame()
	now += 1
end

function M.on_event(name, a)
	if not struct_ok then
		return
	end
	if name == "setting" then
		elig = {}
	elseif name == "new_game" then
		if a ~= true then
			paid = {}
		end
		loaded = true
		elig = {}
	elseif name == "save_loaded" then
		load_paid()
	elseif name == "saving" then
		if loaded then
			local t = {}
			for k in pairs(paid) do
				t[k] = 1
			end
			omni.save.set(DATA.store_key, t)
		end
	elseif name == "level_loaded" or name == "level_unloading" then
		retail_price = {}
		seen = {}
		infos = {}
		elig = {}
		if name == "level_loaded" then
			read_prices()
		end
	end
end

return M
