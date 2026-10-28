--!nonstrict
-- Arsenal Mastery: owns the game's hit resolver (rac1.omni.damage.resolve). Every scored hit on an enemy is
--   1. attributed to a weapon (the attacker moby's class, or the held weapon / the wrench when Ratchet himself hits),
--   2. paid out as XP: health actually removed x10 plus a last-hit bonus,
--   3. scaled: the strength the enemy takes is multiplied by (level power of that weapon) / T (the enemy rebalance).
-- Also owns rac1.player.take_damage (Hard: every second hit on Ratchet costs one more orb).
local M = {}

-- ===================================================================================================
-- DATA
-- ===================================================================================================
local DATA = {
	xp_per_health = 10, -- XP per point of health taken off an enemy
	last_hit_bonus = 0.5, -- of the enemy's starting health x xp_per_health
	-- Enemy rebalance T by level number (planet order 0..18: Veldin Novalis Aridia Kerwan Eudora Rilgar Blarg Umbris Batalia
	-- Gaspar Orxon Pokitaru Hoven Gemlik Oltanis Quartu Kalebo Veldin-orbit Veldin-finale). An enemy takes strength / T.
	T = { [0] = 1.0, 1.0, 1.1, 1.2, 1.3, 1.45, 1.6, 1.75, 1.9, 2.0, 2.1, 2.2, 2.3, 2.35, 2.4, 2.45, 2.5, 2.5, 2.5 },
	hard_factor = 1.3, -- Hard: T x this
	ng_factor = 2.5, -- New Game+ (Standard and Hard): T x this more, so Mega weapons are about 2x the original game early and 1.25x at V8 on the last planets
	hard_extra_orb_every = 2, -- Hard: every Nth hit on Ratchet costs one more orb
	-- Moby class of whatever lands a hit -> weapon. Projectiles, held weapon objects and effects.
	attacker = {
		[121] = "bomb_glove", [192] = "bomb_glove",
		[74] = "mine_glove", [190] = "mine_glove",
		[305] = "blaster", [168] = "blaster",
		[457] = "ryno", [454] = "ryno",
		[186] = "glove_of_doom", [230] = "glove_of_doom", [229] = "glove_of_doom",
		[172] = "visibomb", [163] = "visibomb",
		[153] = "devastator", [157] = "devastator",
		[176] = "pyrocitor", [177] = "tesla_claw", [180] = "walloper",
		[175] = "taunter", [185] = "morph_o_ray", [562] = "decoy", [849] = "suck_cannon", [483] = "drone_device", [479] = "drone_device", [203] = "decoy", [1900] = "decoy",
		[71] = "wrench",
	},
	-- The held weapon's moby class when Ratchet himself is the attacker (his own wrench/glove hits).
	held = {
		[71] = "wrench", [849] = "suck_cannon", [192] = "bomb_glove", [157] = "devastator", [163] = "visibomb",
		[175] = "taunter", [168] = "blaster", [176] = "pyrocitor", [190] = "mine_glove", [180] = "walloper",
		[177] = "tesla_claw", [229] = "glove_of_doom", [185] = "morph_o_ray", [454] = "ryno", [483] = "drone_device", [562] = "decoy",
	},
	-- Enemy classes (health read from the game code). Anything else is asked from the game once per class (mobys{enemy}).
	enemies = {
		8, 28, 30, 31, 36, 44, 52, 63, 101, 111, 170, 217, 231, 233, 238, 252, 253, 294, 326, 333, 340, 347, 388, 427, 455, 459,
		491, 541, 563, 572, 573, 574, 577, 578, 580, 612, 623, 625, 631, 638, 643, 668, 749, 827, 865, 866, 871, 933, 1023, 1041,
		1048, 1051, 1059, 1068, 1106, 1110, 1112, 1126, 1172, 1196, 1202, 1219, 1229, 1231, 1246, 1261, 1262, 1269, 1356, 1382,
		1422, 1440, 1454, 1805, 1885, 1906,
	},
	ring = 64, -- victims whose starting health is remembered
	log_hits_per_weapon = 6, -- hit lines logged per weapon (then only level-ups and summaries)
	summary_every = 1500, -- ticks between summary lines
}
M.DATA = DATA

local api = nil
local levels = nil
local wrench = nil -- the wrench module (its active() says a swing is running)
local nanotech = nil -- the nanotech module (its enabled() says the health pool is on)
local omni = omni
local mcache = nil -- mcache.luau (class of a moby, cached)
local enemy_class = {} -- class -> true/false (known)
local unseen = {} -- attacker classes already logged as unattributed
local stats = {} -- weapon -> { hits, xp, kills }
local logged = {}
local failed = false
local errors = 0 -- scored hits that raised an error (one odd hit must not switch XP and scaling off for good)
local slow_logged = 0
local hero_hits = 0
local extra_orbs = 0
local owed = 0 -- enemy_damage: the part of an orb carried to the next hit

local function ratio(key) -- a multiplier setting (float, 1.0 = as designed)
	local v = omni.setting(key)
	return (type(v) == "number" and v > 0) and v or 1
end
local tick = 0
local have_struct, have_mobys = false, false
local have_out, have_isenemy = false, false -- omni.out_f32 / is_enemy (loader ABI 1.5); older loaders use the raw paths

-- Starting health of the victims being fought (ring, preallocated).
local N = DATA.ring
local r_victim, r_class, r_hp = {}, {}, {}
local r_next = 1
for i = 1, N do r_victim[i], r_class[i], r_hp[i] = 0, 0, 0 end

local function log(msg) api.log(msg) end

-- ---------------------------------------------------------------------------------------- T
local function rebalance_T()
	local mode = omni.setting("rebalance")
	if mode == "off" then return 1.0 end
	local lvl = omni.level()
	local t = DATA.T[lvl] or 1.0
	if mode == "hard" then t = t * DATA.hard_factor end
	if levels.ng_plus() then t = t * DATA.ng_factor end
	return t
end
M.T = rebalance_T

-- ---------------------------------------------------------------------------------------- attribution
local function read_class(addr)
	if addr == nil or addr == 0 then return nil end
	if mcache ~= nil then return mcache.class(addr) end
	local ok, m = pcall(omni.struct, "moby", addr)
	if ok and m then return m.class, m end
	return nil
end

local hero_cache, hero_cache_tick = 0, -1
local function hero_moby()
	if hero_cache_tick == tick and hero_cache ~= 0 then return hero_cache end
	local ok, v = pcall(function() return omni.fields.omni.hero.moby end)
	if ok then
		hero_cache, hero_cache_tick = v, tick
		return v
	end
	return 0
end

local function held_weapon()
	local ok, h = pcall(function() return omni.fields.omni.hero.held_moby end)
	if ok and h and h ~= 0 then
		local c = read_class(h)
		if c and DATA.held[c] then return DATA.held[c] end
	end
	return nil
end

-- Which weapon made this hit? Returns weapon name (nil = unknown) and a short reason for the log.
local function attribute(attacker)
	if attacker == 0 then return nil, "no attacker" end
	local cls = read_class(attacker)
	if attacker == hero_moby() or cls == 0 then
		if wrench ~= nil and wrench.active() then return "wrench", "hero/wrench" end
		local w = held_weapon()
		if w then return w, "hero/held" end
		return nil, "hero (nothing held)", 0
	end
	-- A mini bomb (class 121) thrown by a Visibomb split has the Visibomb gun (class 163) as its owner: credit the Visibomb.
	if cls == 121 then
		-- the owner of a mini bomb never changes: looked up once per bomb (kept in its mcache record)
		local rec = mcache ~= nil and mcache.rec(attacker) or nil
		local oc = rec and rec.owner_class or nil
		if oc == nil then
			local ok, v = pcall(function()
				local m = omni.struct("moby", attacker)
				local b = omni.struct("bomb", m.pvar)
				if b == nil or b.owner == 0 then return 0 end
				return mcache ~= nil and mcache.class(b.owner) or omni.struct("moby", b.owner).class
			end)
			oc = ok and v or false
			if rec ~= nil and ok and oc ~= 0 then rec.owner_class = oc end
		end
		if oc == 163 then
			if not unseen.split then
				unseen.split = true
				log("mini bomb of a Visibomb split (owner class 163): kills and XP credited to visibomb")
			end
			return "visibomb", "class 121 (owner visibomb)", cls
		end
	end
	local w = DATA.attacker[cls]
	if w then return w, "class " .. tostring(cls), cls end
	return nil, "class " .. tostring(cls), cls
end

-- The strength the game's hit resolver hands back through its float* argument.
local function read_out(ptr)
	if have_out then return omni.out_f32(ptr) or 0 end
	return omni.read("f32", ptr)
end

local function write_out(ptr, v)
	if have_out then omni.out_f32(ptr, v) else omni.write("f32", ptr, v) end
end

local function is_enemy(victim, vclass, vm)
	local known = enemy_class[vclass]
	if known ~= nil then return known end
	local res = false
	if have_isenemy then
		res = omni.is_enemy(victim) == true
	elseif have_mobys and vm then
		local ok, list = pcall(omni.mobys, { class = vclass, near = { vm.x, vm.y, vm.z }, radius = 1.0, max = 4, enemy = true })
		if ok and list then
			for _, a in ipairs(list) do
				if a == victim then res = true break end
			end
		end
	end
	enemy_class[vclass] = res
	log(string.format("class %d is %s", vclass, res and "an enemy" or "not an enemy (crate/other): no XP, no scaling"))
	return res
end

-- ---------------------------------------------------------------------------------------- victim ring
local function start_health(victim, vclass, hp)
	for i = 1, N do
		if r_victim[i] == victim then
			if r_class[i] ~= vclass then r_class[i], r_hp[i] = vclass, hp end
			return i, r_hp[i]
		end
	end
	local i = r_next
	r_next = r_next % N + 1
	r_victim[i], r_class[i], r_hp[i] = victim, vclass, hp
	return i, hp
end

local function stat(w)
	local s = stats[w]
	if s == nil then s = { hits = 0, xp = 0, kills = 0 }; stats[w] = s end
	return s
end

local function summary(why)
	local parts = {}
	for _, w in ipairs(levels.WEAPONS) do
		local s = stats[w]
		if s ~= nil then
			table.insert(parts, string.format("%s hits=%d kills=%d xp=%.0f V%d", w, s.hits, s.kills, s.xp, levels.level(w)))
		end
	end
	if #parts > 0 then log("summary (" .. why .. ") T=" .. string.format("%.2f", rebalance_T()) .. ": " .. table.concat(parts, "; ")) end
end

-- ---------------------------------------------------------------------------------------- the resolver
local function score(victim, entry, comp, sptr)
	local s = read_out(sptr)
	if s <= 0 then return end
	if comp == 0 then return end
	local c = omni.struct("component", comp)
	if c == nil then return end
	local hp = c.health
	if hp <= 0 then return end -- already dead
	local vclass, vm = read_class(victim)
	if vclass == nil then return end
	if enemy_class[vclass] == nil and vm == nil then
		local okm, m = pcall(omni.struct, "moby", victim)
		vm = okm and m or nil
	end
	if not is_enemy(victim, vclass, vm) then return end
	local hit = omni.struct("hit", entry)
	local attacker = hit and hit.attacker or 0
	local weapon, why, acls = attribute(attacker)
	if weapon == nil and acls ~= nil and acls ~= 0 and not unseen[acls] then
		unseen[acls] = true
		log(string.format("unattributed attacker: %s (victim class %d): no XP", why, vclass))
	elseif weapon == nil and (acls == nil or acls == 0) and not unseen[why] then
		unseen[why] = true
		log("unattributed attacker: " .. why .. " (victim class " .. vclass .. "): no XP")
	end
	local power = weapon and levels.power(weapon) or 1.0
	local T = rebalance_T()
	local k = power / T
	if weapon ~= nil then k = k * ratio("weapon_damage") end
	local s2 = s * k
	if k ~= 1.0 then
		write_out(sptr, s2)
		omni.struct_set("hit", entry, "strength", s2)
	end
	local removed = math.min(s2, hp)
	local killed = s2 >= hp
	local _, hp0 = start_health(victim, vclass, hp)
	if weapon ~= nil then
		local gain = removed * DATA.xp_per_health
		if killed then gain = gain + hp0 * DATA.xp_per_health * DATA.last_hit_bonus end
		local st = stat(weapon)
		st.hits += 1
		st.xp += gain * (omni.setting("xp_rate") or 1)
		if killed then st.kills += 1 end
		local lg = logged[weapon] or 0
		if lg < DATA.log_hits_per_weapon then
			logged[weapon] = lg + 1
			log(string.format("hit w=%s atk=%s vic=%d s %.3f -> %.3f (power %.2f / T %.2f) hp %.2f removed %.2f%s xp+%.1f",
				weapon, why, vclass, s, s2, power, T, hp, removed, killed and " KILL" or "", gain * (omni.setting("xp_rate") or 1)))
		end
		levels.add_xp(weapon, gain)
		levels.report_hit(weapon, victim, removed, killed)
	elseif k ~= 1.0 then
		local lg = logged["?"] or 0
		if lg < DATA.log_hits_per_weapon then
			logged["?"] = lg + 1
			log(string.format("hit w=? atk=%s vic=%d s %.3f -> %.3f (T %.2f)", why, vclass, s, s2, T))
		end
	end
	if killed then
		for i = 1, N do
			if r_victim[i] == victim then r_victim[i] = 0 break end
		end
	end
end

local function on_resolve(original, victim, entry, comp, a3, status, sptr, out2, mode)
	if entry == 0 or sptr == 0 or failed then return original() end
	local ret = original()
	local t0 = os.clock()
	local ok, err = pcall(score, victim, entry, comp, sptr)
	local ms = (os.clock() - t0) * 1000
	if ms > 1.0 and slow_logged < 5 then
		slow_logged += 1
		log(string.format("power: one scored hit took %.2f ms of script time", ms))
	end
	if not ok then
		errors += 1
		if errors <= 3 then log("power: scoring one hit failed (" .. string.sub(tostring(err), 1, 160) .. ")") end
		if errors >= 20 then
			failed = true
			log("power: scoring failed 20 times, XP and scaling off (" .. string.sub(tostring(err), 1, 160) .. ")")
		end
	end
	return ret
end

local function on_take_damage(original, amount)
	-- Nanotech levels on: the hit takes points off the mod's health pool (nanotech.luau; Hard scales the points there).
	if nanotech ~= nil and nanotech.enabled() then return nanotech.take_damage(original, amount) end
	-- Damage you take (enemy_damage) in orbs: the fraction carries over, so x0.5 skips every second hit and x1.5 costs 1, 2, 1, 2.
	local n = 1
	local m = ratio("enemy_damage")
	if m ~= 1 and amount ~= nil and amount >= 1 then
		owed += m
		n = math.floor(owed)
		owed -= n
	end
	for _ = 1, n do original() end
	if n >= 1 and amount ~= nil and amount >= 1 and omni.setting("rebalance") == "hard" then
		hero_hits += 1
		if hero_hits % DATA.hard_extra_orb_every == 0 then
			extra_orbs += 1
			if extra_orbs <= 5 then log("hard: hit #" .. hero_hits .. " on Ratchet costs one more orb (" .. extra_orbs .. ")") end
			original()
		end
	end
end

-- ---------------------------------------------------------------------------------------- module
function M.setup(a)
	api = a
	levels = a.levels
	local okc, mc = pcall(require, "mcache")
	if okc and type(mc) == "table" then
		mcache = mc
		mc.setup()
	end
	have_struct = omni.has("omni.struct") and omni.has("omni.struct_set")
	have_mobys = omni.has("omni.mobys")
	have_out = omni.has("omni.out_f32")
	have_isenemy = omni.has("omni.is_enemy")
	for _, c in ipairs(DATA.enemies) do enemy_class[c] = true end
	local okw, w = pcall(require, "wrench")
	if okw then wrench = w end
	local okn, n = pcall(require, "nanotech")
	if okn then nanotech = n end
	if not have_struct then
		log("power: this port has no struct access: XP and level power are off")
		return
	end
	if not have_mobys then log("power: no mobys query: unknown classes are treated as not enemies") end
	if omni.has("rac1.omni.damage.resolve") then
		omni.hook("rac1.omni.damage.resolve", on_resolve)
	else
		log("power: no rac1.omni.damage.resolve on this disc: XP and level power are off")
	end
	if omni.has("rac1.player.take_damage") then omni.hook("rac1.player.take_damage", on_take_damage) end
end

function M.on_frame(t)
	tick = t
	if t % DATA.summary_every == 0 then summary("tick " .. t) end
end

function M.on_event(name, a, b)
	if name == "level_loaded" or name == "level_unloading" then
		for i = 1, N do r_victim[i] = 0 end
		if name == "level_unloading" then summary("level " .. tostring(a)) end
	elseif name == "saving" then
		summary("saving")
	end
end

return M
