--!nonstrict
-- Walloper perks. Every swing (hero state "walloper swing") also sends a shockwave around Ratchet through the game's own hit path.
-- V3: radius 4 m, damage 2.5 (weaker than the swing's 3.0, so the enemy in front keeps the swing's hit).
-- V6 Mega: radius 6.5 m, damage 4.0 (stronger than the swing: it wins on the enemy in front too). V6 replaces V3 (never both).
-- The wave fires on the first tick of the swing's hit window; the hits count for the Walloper (power.luau: hero + held weapon).
local M = {}

local DATA = {
	held_class = 180, -- the held Walloper moby
	swing_state = 32, -- hero.state while the Walloper swings
	delay = { pal = 9, ntsc = 11 }, -- ticks from the swing's start to the first tick of its hit window
	v3 = { radius = 4.0, damage = 2.5 },
	v6 = { radius = 6.5, damage = 4.0 },
	mask = 0x30000, -- the Walloper swing's own hit mask (a mask value, not an address)
	kind0 = 5, kind1 = 1, -- hit descriptor kind bytes of a non-wrench attack
	max_victims = 24,
}

function M.setup(fw)
	local omni = omni
	local H = require("perks/hitpath").get(fw)
	local delay = DATA.delay[fw.api.disc] or DATA.delay.pal

	-- One swing tracker for both perks: true on the tick the shockwave is due (once per swing).
	local prev, start, fired = -1, nil, false
	local cache_tick, cache_due = -1, false
	local function due(t)
		if t == cache_tick then return cache_due end
		local st = omni.fields.hero.state
		if cache_tick ~= t - 1 then prev, start = st, nil end -- the perk was off a moment ago: do not trust the old edge
		cache_tick, cache_due = t, false
		if st == DATA.swing_state then
			if prev ~= DATA.swing_state then start, fired = t, false end
			if start ~= nil and not fired and t - start >= delay then
				fired = true
				cache_due = true
			end
		else
			start = nil
		end
		prev = st
		return cache_due
	end

	local function held_walloper()
		local ok, h = pcall(function() return omni.fields.omni.hero.held_moby end)
		if not ok or not h or h == 0 then return false end
		local m = omni.struct("moby", h)
		return m ~= nil and m.class == DATA.held_class
	end

	local function wave(key, p)
		if not held_walloper() then return end
		local hero = fw.hero()
		local hx, hy, hz = fw.hero_pos()
		local list = omni.mobys{ enemy = true, near = { hx, hy, hz }, radius = p.radius, max = DATA.max_victims }
		local n = 0
		for _, a in ipairs(list) do
			if fw.alive(a) then
				local m = omni.struct("moby", a)
				local dx, dy = m.x - hx, m.y - hy
				local l = math.sqrt(dx * dx + dy * dy)
				if l < 0.01 then dx, dy, l = 0, 1, 1 end
				if H.hit(a, p.damage, hero, DATA.mask, dx / l, dy / l, 0, DATA.kind0, DATA.kind1, DATA.held_class) then n += 1 end
			end
		end
		fw.note(key, string.format("shockwave hit %d enemies at %.1f,%.1f,%.1f (radius %.1f damage %.1f)", n, hx, hy, hz, p.radius, p.damage))
	end

	local e2 = nil
	fw.perk{
		weapon = "walloper", slot = 1, name = "shockwave",
		on_tick = function(t)
			H.flush()
			if e2 ~= nil and e2.on then return end
			if due(t) then wave("walloper/shockwave", DATA.v3) end
		end,
	}
	e2 = fw.perk{
		weapon = "walloper", slot = 2, name = "big_shockwave",
		on_tick = function(t)
			H.flush()
			if due(t) then wave("walloper/big_shockwave", DATA.v6) end
		end,
	}
end

return M
