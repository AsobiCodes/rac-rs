--!nonstrict
-- Arsenal Mastery: ammo.
--   1. Capacity grows with the weapon's level (field omni.weapon.<name>.max_ammo): retail cap x 1.25 at V3,
--      x 1.5 at V5, x 1.75 at V6 and above. The game reloads the table with every level, so the retail cap is read
--      again after each level load and the new cap is written over it. Switched by the setting `level_power`
--      (the levels' effects); with it off nothing is read or written.
--   2. Survival (setting `ammo_mode` = "survival"):
--      - ammo crates give half of their ammo (at least 1),
--      - every kill gives 1 shot back to the weapon that made it (the game caps it at the weapon's capacity; not the Glove of Doom, whose agents kill on their own),
--      - vendors are not touched: ammo bought there is the full amount.
--      With ammo_mode "normal" the crate hooks only pass through.

-- ALL numbers of this module (the final values are set by a later stage).
local DATA = {
	-- capacity factor from this level on (the highest row that fits wins)
	capacity = { { 1, 1.0 }, { 3, 1.25 }, { 5, 1.5 }, { 6, 1.75 } },
	box_id_base = 60000, -- the ammo box's element id is this + the weapon's item id
	poll_ticks = 30, -- how often the capacity is checked (plus at once after a level change or an event)
	pickup_ratio = 0.5, -- share of a crate's ammo that is kept
	pickup_min = 1, -- a crate always gives at least this much
	kill_refund = 1, -- shots given back per kill
	-- Weapons whose kills come from a swarm released by one shot (Glove of Doom: the agents, twins included, kill on their own and
	-- would pay a shot each, refilling the glove): no refund.
	no_refund = { glove_of_doom = true },
	-- weapons with a capacity field -> item id (Suck Cannon, Taunter, Walloper and Morph-o-Ray have no ammo)
	items = {
		bomb_glove = 10, devastator = 11, visibomb = 13, blaster = 15, pyrocitor = 16, mine_glove = 17,
		tesla_claw = 19, glove_of_doom = 20, ryno = 23, drone_device = 24, decoy = 25,
	},
}

local M = {}
local api = nil
local ticks = 0
local dirty = true
local in_pickup = false
local add_ammo = nil
local retail = {} -- name -> the cap the game's table holds (read, not written by us)
local written = {} -- name -> the cap we wrote last
local logged = {} -- name -> last "level -> cap" line logged

local function log(msg)
	if api ~= nil then
		api.log(msg)
	end
end

local function survival()
	return omni.setting("ammo_mode") == "survival"
end

local function capacity_on()
	return omni.setting("level_power") == true
end

local function factor(level)
	local f = 1.0
	for _, row in ipairs(DATA.capacity) do
		if level >= row[1] then
			f = row[2]
		end
	end
	return f
end

-- Half of a crate's ammo, at least pickup_min, never more than the crate holds.
local function share(amount)
	if amount <= 0 then
		return amount
	end
	local v = math.floor(amount * DATA.pickup_ratio + 1e-6)
	if v < DATA.pickup_min then
		v = DATA.pickup_min
	end
	if v > amount then
		v = amount
	end
	return v
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

-- Modes in which the level's capacity table is loaded and can be read. "ship" is the arrival: the level is loaded, the ship
-- is still landing and gameplay has not begun. The new cap must be written there: the game's ammo box takes the weapon's cap
-- once, when it is created in the first gameplay tick, and keeps it until the weapon is switched, so a cap written after that
-- shows the old value (the retail one) in the box. "loading" and "title" are left out: the table is not there (reads give junk).
local READABLE = { gameplay = true, pause = true, vendor = true, dialog = true, cutscene = true, ship = true }

-- The HUD ammo box takes its maximum ("count/N") from the weapon once, when the box is created, so a cap that changes while the weapon
-- stays in hand (a level-up) would keep showing the old N until the weapon is switched. When the port has the box fields, the new cap is
-- written to the box itself right after the cap changes: `max` is what the box prints, `def_max` is the value the game compares every
-- tick (a difference there makes the game fade the box out and in again), so both get the same number in the same frame. The box is only
-- touched when its element id says it belongs to this weapon (60000 + the item id); a pending box of another weapon is left alone.
-- A port without the box fields has no `omni.hud.ammo_box.*` path: the read below then errors or gives nil. `omni.has` is asked first; some
-- loader builds answer false for every field under `omni.`, so a readable numeric id also counts. Only a positive answer is remembered (a read
-- fails between levels and on the title screen).
local box_known = false
local function box_available()
	if box_known then
		return true
	end
	if type(omni.has) == "function" and omni.has("omni.hud.ammo_box.max") and omni.has("omni.hud.ammo_box.def_max")
		and omni.has("omni.hud.ammo_box.id") then
		box_known = true
		return true
	end
	local ok, id = pcall(function()
		return omni.fields.omni.hud.ammo_box.id
	end)
	if ok and type(id) == "number" then
		box_known = true
	end
	return box_known
end

local function refresh_box(name, cap)
	if not box_available() or DATA.items[name] == nil then
		return
	end
	local ok, err = pcall(function()
		local box = omni.fields.omni.hud.ammo_box
		if box.id ~= DATA.box_id_base + DATA.items[name] then
			return
		end
		if box.max ~= cap or box.def_max ~= cap then
			box.max = cap
			box.def_max = cap
			log(string.format("ammo: box for %s now shows /%d", name, cap))
		end
	end)
	if not ok then
		log("ammo: box refresh failed: " .. tostring(err))
	end
end

-- One pass over the weapons. The capacity table is overlay data of the level now loaded: a read fails between levels.
local function sync_capacity()
	if not READABLE[omni.mode()] then
		return
	end
	local on = capacity_on()
	for name in pairs(DATA.items) do
		local node = node_of(name)
		if node ~= nil then
			local ok, cur = pcall(function()
				return node.max_ammo
			end)
			if ok and cur ~= nil then
				-- a value other than the one we wrote is the game's own (a level load rewrote the table)
				if written[name] == nil or cur ~= written[name] then
					retail[name] = cur
					written[name] = nil
				end
				local base = retail[name]
				local lv = api.levels.level(name)
				local want = base
				if on then
					want = math.min(65535, math.floor(base * factor(lv) + 1e-6))
				end
				if want ~= cur then
					node.max_ammo = want
					written[name] = want
					refresh_box(name, want)
				end
				if on then
					local key = lv .. ":" .. want
					if logged[name] ~= key then
						logged[name] = key
						log(string.format("ammo: %s V%d max ammo %d (retail %d)", name, lv, want, base))
					end
				end
			end
		end
	end
end

local function refund(weapon)
	local item = DATA.items[weapon]
	if item == nil or DATA.kill_refund <= 0 or DATA.no_refund[weapon] then
		return
	end
	if add_ammo == nil then
		add_ammo = omni.fn("add_ammo")
	end
	local node = node_of(weapon)
	local before = node ~= nil and node.ammo or 0
	add_ammo(item, DATA.kill_refund)
	if node ~= nil then
		log(string.format("ammo: kill with %s: ammo %d -> %d", weapon, before, node.ammo))
	end
end

function M.setup(a)
	api = a
	local noted_cut = false
	if omni.has("rac1.omni.ammo.pickup") and omni.has("rac1.omni.ammo.add") then
		-- Ammo crates: the pickup calls AddAmmo; halve the amount only inside that call.
		omni.hook("rac1.omni.ammo.pickup", function(original, moby)
			if not survival() then
				return original(moby)
			end
			in_pickup = true
			local ok, r = pcall(original, moby)
			in_pickup = false
			if not ok then
				error(r, 0)
			end
			-- The pickup update waits for a frame after it added the ammo: the port lets it go (unwound), the rest runs
			-- unhooked. Nothing is lost here (the amount was halved inside the add hook before the wait), so only note it.
			-- (nil alone says nothing: this is a void function, `original` gives nil on every call.)
			if not noted_cut and omni.has("omni.unwound") and omni.unwound() == true then
				noted_cut = true
				log("ammo: the pickup update was cut short at a frame wait (unwound): the rest of it runs without the hook")
			end
			return r
		end)
		-- The second collect path (the small collect helper the hero/blast scans call); older ports have no such hook.
		if omni.has("rac1.omni.ammo.collect") then
			omni.hook("rac1.omni.ammo.collect", function(original, moby)
				if not survival() then
					return original(moby)
				end
				in_pickup = true
				local ok, r = pcall(original, moby)
				in_pickup = false
				if not ok then
					error(r, 0)
				end
				return r
			end)
		end
		omni.hook("rac1.omni.ammo.add", function(original, item, amount)
			if in_pickup then
				local n = share(amount)
				log(string.format("ammo: crate item %d: %d -> %d", item, amount, n))
				return original(item, n)
			end
			return original(item, amount)
		end)
	else
		log("ammo: this port has no ammo hooks: survival ammo stays off")
	end
	a.levels.on_kill(function(weapon)
		if survival() then
			pcall(refund, weapon)
		end
	end)
	a.levels.on_level(function()
		dirty = true
	end)
end

function M.on_frame()
	ticks += 1
	if dirty or ticks % DATA.poll_ticks == 0 then
		dirty = false
		sync_capacity()
	end
end

function M.on_event(name)
	if name == "level_loaded" or name == "setting" or name == "save_loaded" or name == "new_game" or name == "mode" then
		dirty = true
	end
end

return M
