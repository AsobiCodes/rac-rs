--!nonstrict
-- Pyrocitor perks. V3: the flames fly 1.35x as far (the flame lives 1.35x as many ticks; the "range" the creator takes only shapes the
-- slowing-down and does not set how far a flame flies: measured, a range of 12 and of 16.2 gave the same reach of 8.9 m). V6 Mega: two extra flames at +-0.30 rad beside every lead flame (a wider fan),
-- plus hit-path damage for the extras (the game's flame damage follows its own particles only).
-- The flame creator is shared with other callers: a perk acts only while the held weapon moby is the Pyrocitor (class 176).
local M = {}

local DATA = {
	held_class = 176,
	range_mult = 1.35,
	-- The flame particle's life and start life are the binding's struct "flame" (the hit sphere grows with life / start life).
	spread = 0.30, -- radians each side
	lead_flag = 1, -- flags bit 0 = lead flame (the held update calls the creator 2 lead + 2 trailing times per tick)
	damage_every = 5, -- ticks between two rounds of extra-flame damage
	damage = 0.5,
	damage_radius = 5.0, -- metres
	cone_min = 0.82, -- cosine of the angle to Ratchet's facing: the band the extra flames cover
	cone_max = 0.97, -- (the middle flame is the game's own)
	mask = 0x10000, -- flame hit mask
	kind0 = 5, kind1 = 1,
	summary_every = 100, -- ticks
	summary_lines = 6,
}

function M.setup(fw)
	local omni = omni
	local H = require("perks/hitpath").get(fw)

	local held_tick, held_ok = -1, false
	local function pyro_held()
		local t = fw.tick()
		if t ~= held_tick then
			held_tick = t
			held_ok = false
			local ok, h = pcall(function() return omni.fields.omni.hero.held_moby end)
			if ok and h and h ~= 0 then
				local m = omni.struct("moby", h)
				held_ok = m ~= nil and m.class == DATA.held_class
			end
		end
		return held_ok
	end
	local function held_addr()
		local ok, h = pcall(function() return omni.fields.omni.hero.held_moby end)
		if ok and h then return h end
		return 0
	end

	fw.perk{
		weapon = "pyrocitor", slot = 1, name = "range", needs = { "flame.life", "flame.start_life" },
		hooks = {
			["weapon.flame.create"] = function(nxt, range, pos, vel, flags)
				if pyro_held() then
					fw.note("pyrocitor/range", string.format("flame range %.2f -> %.2f, life x%.2f (flags %d)", range, range * DATA.range_mult, DATA.range_mult, flags))
					local r = nxt(range * DATA.range_mult, pos, vel, flags)
					if r ~= nil and r ~= 0 then
						local f = omni.struct("flame", r)
						if f ~= nil then
							local nl = math.min(255, math.floor(f.life * DATA.range_mult + 0.5))
							omni.struct_set("flame", r, "life", nl)
							omni.struct_set("flame", r, "start_life", nl)
						end
					end
					return r
				end
				return nxt(range, pos, vel, flags)
			end,
		},
	}

	local extras, zeros, rounds = 0, 0, 0
	local last_extra_tick = -1000
	local lines = 0
	fw.perk{
		weapon = "pyrocitor", slot = 2, name = "fan", needs = { "vec4.w" },
		hooks = {
			["weapon.flame.create"] = function(nxt, range, pos, vel, flags)
				local r = nxt(range, pos, vel, flags)
				if pyro_held() and bit32.band(flags, DATA.lead_flag) == DATA.lead_flag then
					local v = omni.struct("vec4", vel)
					local vx, vy, vz, vw = v.x, v.y, v.z, v.w
					for _, sign in ipairs({ 1, -1 }) do
						local a = sign * DATA.spread
						local co, si = math.cos(a), math.sin(a)
						local e = 0
						omni.with_stack(16, function(s)
							omni.struct_set("vec4", s, "x", vx * co - vy * si)
							omni.struct_set("vec4", s, "y", vx * si + vy * co)
							omni.struct_set("vec4", s, "z", vz)
							omni.struct_set("vec4", s, "w", vw)
							e = nxt(range, pos, s, flags)
						end)
						if e == nil or e == 0 then zeros += 1 else extras += 1 end
					end
					last_extra_tick = fw.tick()
					fw.note("pyrocitor/fan", string.format("extra flames at +-%.2f rad: made %d, pool full %d", DATA.spread, extras, zeros))
				end
				return r
			end,
		},
		on_tick = function(t)
			H.flush()
			if t % DATA.summary_every == 0 and extras + zeros > 0 and lines < DATA.summary_lines then
				lines += 1
				fw.log(string.format("perk: pyrocitor fan summary: %d extra flames made, %d refused (pool full), %d damage rounds", extras, zeros, rounds))
			end
			if t % DATA.damage_every == 0 and t - last_extra_tick <= 2 then
				local held = held_addr()
				if held == 0 then return end
				local hx, hy, hz = fw.hero_pos()
				local fx, fy = fw.forward()
				local list = omni.mobys{ near = { hx, hy, hz + 0.8 }, radius = DATA.damage_radius, max = 8, enemy = true }
				local n = 0
				for _, e in ipairs(list) do
					if fw.alive(e) then
						local m = omni.struct("moby", e)
						local dx, dy = m.x - hx, m.y - hy
						local dl = math.sqrt(dx * dx + dy * dy)
						if dl > 0.01 then
							local c = (dx * fx + dy * fy) / dl
							if c >= DATA.cone_min and c <= DATA.cone_max then
								if H.hit(e, DATA.damage, held, DATA.mask, dx / dl, dy / dl, 0, DATA.kind0, DATA.kind1, DATA.held_class) then n += 1 end
							end
						end
					end
				end
				rounds += 1
				if n > 0 then fw.note("pyrocitor/fan_damage", string.format("%d enemies in the extra-flame band hit for %.2f", n, DATA.damage)) end
			end
		end,
	}
end

return M
