--!nonstrict
-- Arsenal Mastery: assist XP. The support weapons (Taunter, Decoy Glove, Morph-o-Ray) do no damage themselves, so they earn
-- XP from what happens around them:
--   Taunter     a taunted enemy (the alert scan marks it) takes damage from any other weapon -> Taunter XP.
--   Decoy       the decoy soaks hits (its health drops), and kills made while a decoy stands -> Decoy XP.
--   Morph-o-Ray every new sheep (class 270) made while the Morph-o-Ray is held -> Morph-o-Ray XP.
-- Suck Cannon and Drone Device earn XP through power.luau as usual; this module only teaches power.luau their attacker classes
-- (the drones, and the enemy the Suck Cannon flings, which hits with a special mask).
-- Always on (also in the preset "Just the XP"). It only READS game memory and calls levels.add_xp. The one write is the documented
-- pre_hit scaling that a perk may ask for (Taunter V6 damage), and only while that perk is on.
-- It owns the single rac1.omni.damage.hit_victim hook; other modules use M.pre_hit(fn) (fn(victim, desc, info), called before the
-- game posts the hit; info = { mask, attacker, aclass, player }).
local M = {}

-- ===================================================================================================
-- DATA
-- ===================================================================================================
local DATA = {
	classes = { taunter = 175, morph = 185, sheep = 270, drone = 479 }, -- held Taunter, held Morph-o-Ray, morphed sheep, drone moby
	decoy_classes = { 203, 1900 }, -- Decoy actor, gold Decoy actor
	extra_attacker = { [479] = "drone_device", [203] = "decoy", [1900] = "decoy" }, -- added to power.DATA.attacker at setup
	fling_mask = 0x430000, -- hit mask of an enemy flung by the Suck Cannon (a mask value, not an address)
	blast_mask = 0x830000, -- hit mask of the explosion a flung enemy makes on impact (gold Suck Cannon); the attacker is the flung enemy
	suck_classes = { [572] = true, [577] = true, [865] = true, [866] = true }, -- enemy classes the Suck Cannon can pick up
	fling_weapon = "suck_cannon",
	-- Taunter
	taunt_poll = 5, -- ticks between alert scans (only while the Taunter is held)
	taunt_radius = 50.0, taunt_max = 16,
	taunt_window = 480, -- ticks (8 s at 60 Hz) a taunted enemy keeps counting after the last scan that saw it taunted
	taunt_xp_per_health = 5, taunt_kill_xp = 20,
	-- Decoy
	decoy_poll = 2, decoy_radius = 60.0, decoy_max = 8,
	decoy_states = { first = 3, last = 5 }, -- active states (3 landed, 4 counting down, 5 expiring)
	decoy_xp_per_health = 6, decoy_kill_xp = 15,
	decoy_kill_window = 6, -- ticks since the decoy was last seen standing during which a kill counts
	decoy_forget = 30,
	-- Morph-o-Ray
	morph_poll = 5, morph_radius = 40.0, morph_max = 16, morph_xp = 40,
	morph_grace = 180, -- ticks after the Morph-o-Ray left the hand during which new sheep still count
	morph_forget = 1200,
	note_limit = 3, -- log lines per kind
}
M.DATA = DATA

-- Game records are read through the binding's fields: component.alert (the Taunter moby the alert scan wrote, 0 = not taunted)
-- and decoy.health (< 0 = destroyed).

local api, levels = nil, nil
local omni = omni
local power = nil
local tick = 0
local taunted = {} -- enemy -> tick of the last scan that saw it taunted
local decoys = {} -- decoy -> { hp, seen }
local sheep = {} -- sheep moby -> tick last seen
local morph_until, morph_open = -1, false
local listeners = {} -- pre_hit listeners
local learned = {} -- class -> true (fling classes already logged)
local notes = {}
local failed = {} -- part -> true when switched off after an error

local function log(msg) api.log(msg) end

local function note(key, text)
	local n = (notes[key] or 0) + 1
	notes[key] = n
	if n <= DATA.note_limit then log("assist: " .. text) end
end

local function guard(part, fn, ...)
	if failed[part] then return end
	local ok, err = pcall(fn, ...)
	if not ok then
		failed[part] = true
		log("assist: " .. part .. " failed (" .. string.sub(tostring(err), 1, 140) .. "), switched off")
	end
end

local function hero_moby()
	local ok, v = pcall(function() return omni.fields.omni.hero.moby end)
	if ok and v then return v end
	return 0
end

local function held_class()
	local ok, h = pcall(function() return omni.fields.omni.hero.held_moby end)
	if not ok or not h or h == 0 then return 0 end
	local m = omni.struct("moby", h)
	return m and m.class or 0
end

local function owns(name)
	local ok, v = pcall(function() return omni.fields.omni.weapon[name].have end)
	return ok and v ~= nil and v ~= 0
end

local function hero_pos()
	local p = omni.fields.hero.position
	return { p.x, p.y, p.z }
end

local function component_of(addr)
	local m = omni.struct("moby", addr)
	if m == nil or m.pvar == 0 then return nil end
	local p0 = omni.struct("pvar0", m.pvar)
	if p0 == nil or p0.component == 0 then return nil end
	return omni.struct("component", p0.component)
end

-- True when the binding names struct_name.field (struct_set with a nil address checks it and writes nothing; "read-only" = there).
local function has_field(struct_name, field)
	local ok, err = pcall(omni.struct_set, struct_name, nil, field, 0)
	return ok or string.find(tostring(err), "read-only", 1, true) ~= nil
end

-- ---------------------------------------------------------------------------------------- Taunter
local function poll_taunted()
	local list = omni.mobys{ enemy = true, near = hero_pos(), radius = DATA.taunt_radius, max = DATA.taunt_max }
	for _, a in ipairs(list) do
		local c = component_of(a)
		if c ~= nil and c.alert ~= 0 then
			if taunted[a] == nil then note("taunt_seen", "enemy " .. a .. " is taunted") end
			taunted[a] = tick
		end
	end
	for a, t in pairs(taunted) do
		if tick - t > DATA.taunt_window * 2 then taunted[a] = nil end
	end
end

-- True when the enemy was seen taunted within the last DATA.taunt_window ticks.
function M.is_taunted(victim)
	local t = taunted[victim]
	return t ~= nil and tick - t <= DATA.taunt_window
end

local function taunt_xp(weapon, victim, dmg, killed)
	if weapon == "taunter" or not M.is_taunted(victim) then return end
	local xp = dmg * DATA.taunt_xp_per_health + (killed and DATA.taunt_kill_xp or 0)
	levels.add_xp("taunter", xp)
	note("taunt_xp", string.format("taunter +%.0f xp (enemy %d hit by %s for %.2f%s)", xp, victim, weapon, dmg, killed and ", killed" or ""))
end

-- ---------------------------------------------------------------------------------------- Decoy
local function poll_decoys()
	local list = omni.mobys{ class = DATA.decoy_classes, near = hero_pos(), radius = DATA.decoy_radius, max = DATA.decoy_max }
	for _, a in ipairs(list) do
		local m = omni.struct("moby", a)
		if m ~= nil and m.pvar ~= 0 and m.state >= DATA.decoy_states.first and m.state <= DATA.decoy_states.last then
			local d = omni.struct("decoy", m.pvar)
			local hp = d and d.health or 0
			local prev = decoys[a]
			if prev ~= nil and hp < prev.hp then
				local drop = prev.hp - math.max(hp, 0)
				if drop > 0 then
					local xp = drop * DATA.decoy_xp_per_health
					levels.add_xp("decoy", xp)
					note("decoy_xp", string.format("decoy +%.0f xp (decoy %d health %.2f -> %.2f)", xp, a, prev.hp, hp))
				end
			end
			decoys[a] = { hp = hp, seen = tick }
		end
	end
	for a, d in pairs(decoys) do
		if tick - d.seen > DATA.decoy_forget then decoys[a] = nil end
	end
end

local function decoy_standing()
	for _, d in pairs(decoys) do
		if tick - d.seen <= DATA.decoy_kill_window then return true end
	end
	return false
end

local function decoy_kill_xp(weapon, victim)
	if weapon == "decoy" or not decoy_standing() then return end
	levels.add_xp("decoy", DATA.decoy_kill_xp)
	note("decoy_kill", string.format("decoy +%d xp (kill of %d while a decoy stands)", DATA.decoy_kill_xp, victim))
end

-- ---------------------------------------------------------------------------------------- Morph-o-Ray
local function poll_morph(held)
	if held == DATA.classes.morph then morph_until = tick + DATA.morph_grace end
	if tick > morph_until then
		morph_open = false
		return
	end
	local list = omni.mobys{ class = DATA.classes.sheep, near = hero_pos(), radius = DATA.morph_radius, max = DATA.morph_max }
	if not morph_open then
		-- the window just opened: the sheep that already stand there do not count
		morph_open = true
		for _, a in ipairs(list) do sheep[a] = tick end
		return
	end
	for _, a in ipairs(list) do
		if sheep[a] == nil then
			levels.add_xp("morph_o_ray", DATA.morph_xp)
			note("morph_xp", string.format("morph_o_ray +%d xp (new sheep %d)", DATA.morph_xp, a))
		end
		sheep[a] = tick
	end
	for a, t in pairs(sheep) do
		if tick - t > DATA.morph_forget then sheep[a] = nil end
	end
end

-- ---------------------------------------------------------------------------------------- hit_victim hook
-- fn(victim, desc, info) runs before the game posts the hit on the victim; it may change the descriptor (struct "hit_desc").
function M.pre_hit(fn) table.insert(listeners, { fn = fn, bad = false }) end

-- A hit made by a flung enemy: its direct hit, or the blast it makes on impact (then the attacker must be a suckable class).
function M.is_fling(mask, aclass)
	return mask == DATA.fling_mask or (mask == DATA.blast_mask and DATA.suck_classes[aclass] == true)
end

local function learn_fling(aclass)
	if aclass == nil or aclass == 0 or learned[aclass] then return end
	learned[aclass] = true
	if power ~= nil and power.DATA.attacker[aclass] == nil then
		power.DATA.attacker[aclass] = DATA.fling_weapon
		log("assist: suck_cannon fling class " .. aclass .. " (its hits now count for the Suck Cannon)")
	end
end

local function pre(victim, desc)
	local h = omni.struct("hit_desc", desc)
	if h == nil then return end
	local attacker, mask = h.attacker, h.mask
	local aclass = 0
	if attacker ~= 0 then
		local am = omni.struct("moby", attacker)
		aclass = am and am.class or 0
	end
	local player = attacker ~= 0 and (attacker == hero_moby() or (power ~= nil and power.DATA.attacker[aclass] ~= nil))
	if M.is_fling(mask, aclass) then learn_fling(aclass) end
	if #listeners == 0 then return end
	local info = { mask = mask, attacker = attacker, aclass = aclass, player = player }
	for _, l in ipairs(listeners) do
		if not l.bad then
			local ok, err = pcall(l.fn, victim, desc, info)
			if not ok then
				l.bad = true
				log("assist: a pre_hit listener failed (" .. string.sub(tostring(err), 1, 140) .. "), switched off")
			end
		end
	end
end

local function on_hit_victim(original, victim, desc)
	if desc ~= nil and desc ~= 0 and not failed.hook then guard("hook", pre, victim, desc) end
	return original()
end

-- ---------------------------------------------------------------------------------------- module
function M.setup(a)
	api = a
	levels = a.levels
	local okp, pm = pcall(require, "power")
	if okp and type(pm) == "table" and pm.DATA ~= nil then
		power = pm
		for cls, w in pairs(DATA.extra_attacker) do
			if power.DATA.attacker[cls] == nil then power.DATA.attacker[cls] = w end
		end
	else
		log("assist: power.luau is not available: drone and fling hits cannot be credited")
	end
	for _, n in ipairs({ "omni.struct", "omni.mobys" }) do
		if not omni.has(n) then
			log("assist: this port has no " .. n .. ": assist XP is off")
			failed.all = true
			return
		end
	end
	for part, need in pairs({ ["taunter poll"] = { "component", "alert" }, ["decoy poll"] = { "decoy", "health" } }) do
		if not has_field(need[1], need[2]) then
			log("assist: no binding field " .. need[1] .. "." .. need[2] .. " on this port: " .. part .. " is off")
			failed[part] = true
		end
	end
	levels.on_hit(function(weapon, victim, dmg, killed) guard("taunter xp", taunt_xp, weapon, victim, dmg, killed) end)
	levels.on_kill(function(weapon, victim) guard("decoy xp", decoy_kill_xp, weapon, victim) end)
	if omni.has("rac1.omni.damage.hit_victim") then
		omni.hook("rac1.omni.damage.hit_victim", on_hit_victim)
	else
		log("assist: no rac1.omni.damage.hit_victim on this disc: Suck Cannon hits cannot be learned")
	end
end

function M.on_frame(t)
	tick = t
	if failed.all then return end
	local held = nil
	if t % DATA.taunt_poll == 0 then
		held = held_class()
		if held == DATA.classes.taunter then guard("taunter poll", poll_taunted) end
	end
	if t % DATA.decoy_poll == 0 and owns("decoy") then guard("decoy poll", poll_decoys) end
	if t % DATA.morph_poll == 0 and owns("morph_o_ray") then
		guard("morph poll", poll_morph, held or held_class())
	end
end

function M.on_event(name, a, b)
	if name == "level_loaded" or name == "level_unloading" or name == "new_game" or name == "save_loaded" then
		taunted, decoys, sheep = {}, {}, {}
		morph_open, morph_until = false, -1
	end
end

return M
