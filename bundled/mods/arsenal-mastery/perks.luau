--!nonstrict
-- Arsenal Mastery: perks. Every weapon gets its first perk at V3 and its second at V6 Mega (setting "perks").
--
-- HOW G3b ADDS A PERK WITHOUT TOUCHING THIS FILE'S HOOKS
--   This module is the ONLY one that takes the shared perk moments (hook points) and dispatches them. A perk lives in
--   perks/<weapon>.luau, which returns a module with setup(fw). Add the file name to FILES below (one word), then in
--   setup(fw) call fw.perk{...} once per perk:
--
--     local M = {}
--     function M.setup(fw)
--       fw.perk{
--         weapon = "ryno", slot = 1, name = "big_rocket",           -- slot 1 = V3, slot 2 = V6 Mega
--         hooks = {                                                  -- key = hook id without the "rac1.omni." prefix
--           ["weapon.ryno.fire"] = function(nxt, a, b, c)            -- nxt = the next perk / the game's function
--             local r = nxt(a, b, c)                                 -- call it (zero, one or more times) and return
--             fw.note("ryno/big_rocket", "rocket " .. tostring(r))   -- log line "[am] perk: ryno big_rocket rocket 123"
--             return r
--           end,
--         },
--         on_tick = function(tick) end,                              -- optional, every tick while the perk is on
--       }
--       fw.radius_perk{ weapon = "mine_glove", slot = 1, name = "big_blast", class = 74, mult = 1.3 }  -- sugar
--     end
--     return M
--
--   A handler runs ONLY while its perk is on: setting "perks" on, and the weapon at V3 (slot 1) or Mega (slot 2).
--   With perks off, or preset "Just the XP", every handler is skipped and nothing is written. The hook for a key is
--   taken once, here in setup (the main chunk), only if some perk registered a handler for it, and only if the port has
--   it. Handlers run in pcall: one that errors is switched off with one log line and the game's own call goes on.
--   Handlers get the same arguments as the game's function after nxt; return what the game's function returns.
--   Helpers on fw: fw.hero(), fw.hero_pos(), fw.forward(), fw.nearest_enemy(x, y, z, radius, opts), fw.aim_point(moby),
--   fw.alive(moby), fw.rnd(), fw.tick(), fw.note(key, text), fw.log(text), fw.struct (omni.struct), fw.has_field(struct, field),
--   fw.api. Game records are read and written only through binding fields (omni.struct / omni.struct_set), never by raw offset;
--   list them in the perk's `needs = { "struct.field", ... }` so a port without them keeps the perk off.
--   Randomness: only fw.rnd() (the mod's own generator, never the game's).
local M = {}

-- Perk files (perks/<name>.luau). A missing or broken file is skipped with one log line.
local FILES = { "devastator", "blaster", "bomb_glove", "visibomb", "glove_of_doom", "ryno", "wrench", "pyrocitor", "mine_glove", "tesla_claw", "walloper", "suck_cannon", "decoy", "taunter", "morph_o_ray", "drone_device", "dual_blasters" }

local HOOK_PREFIX = "rac1.omni."
local NOTE_LIMIT = 3 -- log lines per perk key

local api, levels = nil, nil
local omni = omni
local mcache = nil -- mcache.luau
local dirty = true -- a level, setting or save change: refresh() on the next tick
local perks = {} -- every registered perk
local by_key = {} -- hook key -> list of perks (slot order) that have a handler for it
local installed = {} -- hook key -> true
local any_on = false
local tick = 0
local seed = 2463534242
local notes = {}

local function log(msg) api.log(msg) end

local function wanted(e)
	local enabled = api.setting("perks")
	if enabled == false then return false end
	if e.slot == 1 then return levels.level(e.weapon) >= 3 end
	return levels.is_mega(e.weapon)
end

local active = {} -- hook key -> list of the perks that are on and have a handler for it (slot order); rebuilt by refresh()

local function refresh()
	dirty = false
	local on = false
	for _, e in ipairs(perks) do
		local w = (not e.failed) and e.usable and wanted(e)
		e.on = w and true or false
		if e.on then on = true end
	end
	any_on = on
	for key, list in pairs(by_key) do
		local act = {}
		for _, e in ipairs(list) do
			if e.on then act[#act + 1] = e end
		end
		active[key] = act
	end
end

-- ---------------------------------------------------------------------------------------- dispatch
local function run(key, list, i, original, ...)
	while i <= #list and not list[i].on do i += 1 end
	if i > #list then return original(...) end
	local e = list[i]
	local called = false
	local function nxt(...)
		called = true
		return run(key, list, i + 1, original, ...)
	end
	local ok, r = pcall(e.hooks[key], nxt, ...)
	if ok then return r end
	e.failed, e.on = true, false
	log("perk: " .. e.weapon .. " " .. e.name .. " failed (" .. string.sub(tostring(r), 1, 140) .. "), switched off")
	if called then return nil end
	return run(key, list, i + 1, original, ...)
end

local function make_dispatcher(key)
	local list = by_key[key]
	return function(original, ...)
		-- nothing on for this key (the usual case below V3): straight to the game, no closure, no pcall
		local act = active[key]
		if act == nil or act[1] == nil then return original() end
		return run(key, list, 1, original, ...)
	end
end

-- ---------------------------------------------------------------------------------------- helpers for the perk files
local fw = {}

function fw.rnd()
	seed = (seed * 1664525 + 1013904223) % 4294967296
	return seed / 4294967296
end

function fw.tick() return tick end

function fw.log(text) log(text) end

function fw.note(key, text)
	local n = (notes[key] or 0) + 1
	notes[key] = n
	if n > NOTE_LIMIT then return end
	local weapon, perk = string.match(key, "^([^/]+)/(.+)$")
	log("perk: " .. (weapon or key) .. " " .. (perk or "") .. " " .. text)
end

function fw.hero()
	local ok, v = pcall(function() return omni.fields.omni.hero.moby end)
	if ok and v then return v end
	return 0
end

function fw.hero_pos()
	local p = omni.fields.hero.position
	return p.x, p.y, p.z
end

-- Unit forward vector of Ratchet in the ground plane. hero.facing is the heading angle measured like atan2(dy, dx): the flames of the
-- Pyrocitor and the gun in his hand point along (cos f, sin f) (measured 2026-10-23; the matrix row the game calls row1 is his side axis,
-- row1 angle = facing + pi/2, and an earlier version of this helper returned that side axis).
function fw.forward()
	local f = omni.fields.hero.facing
	return math.cos(f), math.sin(f)
end

fw.struct = function(name, addr) return omni.struct(name, addr) end

-- True when the binding names `struct_name.field` ("read-only" counts: the field is there, only struct_set refuses it).
-- struct_set with a nil address checks the struct and the field and writes nothing.
local field_known = {}
function fw.has_field(struct_name, field)
	local key = struct_name .. "." .. field
	if field_known[key] == nil then
		local ok, err = pcall(omni.struct_set, struct_name, nil, field, 0)
		field_known[key] = ok or string.find(tostring(err), "read-only", 1, true) ~= nil
	end
	return field_known[key]
end

-- The enemy's health block, or nil.
local function component_of(addr)
	local m = omni.struct("moby", addr)
	if m == nil or m.pvar == 0 then return nil end
	local p0 = omni.struct("pvar0", m.pvar)
	if p0 == nil or p0.component == 0 then return nil end
	return omni.struct("component", p0.component)
end

function fw.alive(addr)
	local ok, c = pcall(component_of, addr)
	if not ok or c == nil then return true end
	return c.health > 0
end

-- The point to aim at on an enemy (its origin plus the game's own aim height).
function fw.aim_point(addr)
	local m = omni.struct("moby", addr)
	local h = 0.5
	local ok, c = pcall(component_of, addr)
	if ok and c ~= nil and c.aim_height ~= nil and c.aim_height == c.aim_height then h = c.aim_height end
	return m.x, m.y, m.z + h
end

-- Nearest living enemy within `radius` of (x, y, z).
-- opts.cone = minimum cosine of the angle to the forward vector (opts.fx, opts.fy, opts.fz; default the ground
-- plane); opts.los = true: the game's line test from (x, y, z) to the enemy must be free; opts.max = candidates (8).
-- Returns the moby address and its distance, or 0.
function fw.nearest_enemy(x, y, z, radius, opts)
	opts = opts or {}
	local list = omni.mobys{ near = { x, y, z }, radius = radius, max = opts.max or 8, enemy = true }
	for _, a in ipairs(list) do
		if fw.alive(a) then
			local ex, ey, ez = fw.aim_point(a)
			local dx, dy, dz = ex - x, ey - y, ez - z
			local d = math.sqrt(dx * dx + dy * dy + dz * dz)
			local ok = d > 0.01
			if ok and opts.cone then
				local fx, fy, fz = opts.fx or 0, opts.fy or 0, opts.fz or 0
				ok = (dx * fx + dy * fy + dz * fz) / d >= opts.cone
			end
			if ok and opts.los and omni.raycast ~= nil then
				local hit, hx, hy, hz = omni.raycast(x, y, z, ex, ey, ez)
				if hit then
					local hd = math.sqrt((hx - x) ^ 2 + (hy - y) ^ 2 + (hz - z) ^ 2)
					ok = hd >= d - 1.5
				end
			end
			if ok then return a, d end
		end
	end
	return 0, 0
end

function fw.perk(def)
	local e = {
		weapon = def.weapon, slot = def.slot, name = def.name, hooks = def.hooks or {}, on_tick = def.on_tick,
		on_disable = def.on_disable, on_event = def.on_event, -- optional: called once when the perk turns off (from on_frame) / on every event, on or off
		on = false, failed = false, usable = true, was_on = false,
	}
	table.insert(perks, e)
	-- def.needs = { "struct.field", ... }: the binding fields the perk reads or writes; a port without one keeps the perk off.
	for _, need in ipairs(def.needs or {}) do
		local st, field = string.match(need, "^([%w_]+)%.([%w_]+)$")
		if st == nil or not fw.has_field(st, field) then
			log("perks: no binding field " .. need .. " on this port: " .. e.weapon .. " " .. e.name .. " is off")
			e.usable = false
			break
		end
	end
	for key, _ in pairs(e.hooks) do
		by_key[key] = by_key[key] or {}
		table.insert(by_key[key], e)
		table.sort(by_key[key], function(a, b) return a.slot < b.slot end)
	end
	return e
end

-- Sugar: while the perk is on, the radius of collide.sphere_mobys calls made for a moby of class `class` is multiplied by `mult`.
-- def.when(moby_struct) -> bool optionally limits it to a state (e.g. only while exploding).
function fw.radius_perk(def)
	local key = def.weapon .. "/" .. def.name
	local cls, mult, when = def.class, def.mult, def.when
	return fw.perk{
		weapon = def.weapon, slot = def.slot, name = def.name,
		hooks = {
			["collide.sphere_mobys"] = function(nxt, radius, pos, mask, self, filter)
				local c = (self ~= 0) and (mcache ~= nil and mcache.class(self) or nil) or nil
				if self ~= 0 and (c == cls or (mcache == nil and c == nil)) then
					local m = omni.struct("moby", self)
					if m ~= nil and m.class == cls and (when == nil or when(m)) then
						fw.note(key, string.format("radius %.2f -> %.2f (state %d)", radius, radius * mult, m.state))
						return nxt(radius * mult, pos, mask, self, filter)
					end
				end
				return nxt(radius, pos, mask, self, filter)
			end,
		},
	}
end

-- ---------------------------------------------------------------------------------------- module
function M.setup(a)
	api = a
	levels = a.levels
	fw.api = a
	local okc, mc = pcall(require, "mcache")
	if okc and type(mc) == "table" then
		mcache = mc
		mc.setup()
	end
	local need = { "omni.struct", "omni.struct_set", "omni.mobys" }
	for _, n in ipairs(need) do
		if not omni.has(n) then
			log("perks: this port has no " .. n .. ": perks are off")
			return
		end
	end
	for _, name in ipairs(FILES) do
		local ok, mod = pcall(require, "perks/" .. name)
		if ok and type(mod) == "table" and mod.setup then
			local ok2, err = pcall(mod.setup, fw)
			if not ok2 then log("perks/" .. name .. ": setup failed (" .. string.sub(tostring(err), 1, 120) .. "), skipped") end
		else
			log("perks/" .. name .. ": not loaded (" .. string.sub(tostring(mod), 1, 120) .. ")")
		end
	end
	-- One hook per key, only for keys some perk uses; perks whose hook the port lacks stay off.
	for key, list in pairs(by_key) do
		local id = HOOK_PREFIX .. key
		if omni.has(id) then
			omni.hook(id, make_dispatcher(key))
			installed[key] = true
		else
			log("perks: no " .. id .. " on this disc: its perks are off")
			for _, e in ipairs(list) do e.usable = false end
		end
	end
	if levels.on_level ~= nil then levels.on_level(function() dirty = true end) end
	refresh()
end

function M.on_frame(t)
	tick = t
	-- the answer only changes with a level, a setting or a save; the 30-tick check covers anything else (Mega bought at the cap)
	if dirty or t % 30 == 0 then refresh() end
	for _, e in ipairs(perks) do
		if e.was_on and not e.on and e.on_disable ~= nil then
			local ok, err = pcall(e.on_disable)
			if not ok then log("perk: " .. e.weapon .. " " .. e.name .. " on_disable failed (" .. string.sub(tostring(err), 1, 140) .. ")") end
		end
		e.was_on = e.on
	end
	if any_on then
		for _, e in ipairs(perks) do
			if e.on and e.on_tick ~= nil and not e.failed then
				local ok, err = pcall(e.on_tick, t)
				if not ok then
					e.failed, e.on = true, false
					log("perk: " .. e.weapon .. " " .. e.name .. " tick failed (" .. string.sub(tostring(err), 1, 140) .. "), switched off")
				end
			end
		end
	end
end

function M.on_event(name, a, b)
	if name == "setting" or name == "save_loaded" or name == "new_game" or name == "level_loaded" then dirty = true end
	for _, e in ipairs(perks) do
		if e.on_event ~= nil and not e.failed then pcall(e.on_event, name, a, b) end
	end
end

return M
