--!nonstrict
-- Wrench perks. V3 reach: the swing sweep reaches 1.5x further (the wrench's far end of every segment is pushed outward).
-- V6 Mega Hyper-Strike: a landed swing makes a shockwave around Ratchet (an area hit, 80% damage, with a ring drawn on screen).
-- Both go through the game's own hit path (perks/hitpath.luau), so power, XP and kill credit follow the wrench.
local M = {}

local DATA = {
	reach_mult = 1.5, -- far end of each swing segment: near + (far - near) * this
	strike_radius = 5.0, -- metres around Ratchet
	strike_max = 8, -- enemies per shockwave
	strike_damage = 0.8, -- of the swing's damage
	strike_cooldown = 40, -- ticks between two shockwaves
	strike_lift = 0.5, -- metres above Ratchet's feet for the area search
	ring_ticks = 16, -- how long the ring grows
	ring_points = 24,
	ring_size = 3, -- pixels
	ring_color = 0xFFE060FF,
	ring_height = 0.3,
	-- The swing's own descriptor gives the shockwave its kind bytes (hit_desc type, strength_class, attacker_class).
}

function M.setup(fw)
	local omni = omni
	local H = require("perks/hitpath").get(fw)

	-- ----- V3: reach
	-- The four swing points are vec4s the hook passes by address.
	local function read3(p)
		local v = omni.struct("vec4", p)
		return { v.x, v.y, v.z }
	end
	local function write3(p, v)
		omni.struct_set("vec4", p, "x", v[1])
		omni.struct_set("vec4", p, "y", v[2])
		omni.struct_set("vec4", p, "z", v[3])
	end
	local function dist(v, x, y, z)
		return math.sqrt((v[1] - x) ^ 2 + (v[2] - y) ^ 2 + (v[3] - z) ^ 2)
	end

	-- Pushes the end of the pair that is farther from Ratchet outward; returns (saved, far pointer, metres moved).
	local function stretch(pa, pb, hx, hy, hz)
		local a, b = read3(pa), read3(pb)
		local da, db = dist(a, hx, hy, hz), dist(b, hx, hy, hz)
		local near, far, pfar, dfar, dnear = a, b, pb, db, da
		if da > db then near, far, pfar, dfar, dnear = b, a, pa, da, db end
		local moved = { near[1] + (far[1] - near[1]) * DATA.reach_mult, near[2] + (far[2] - near[2]) * DATA.reach_mult,
			near[3] + (far[3] - near[3]) * DATA.reach_mult }
		write3(pfar, moved)
		return { p = pfar, v = far }, (dfar - dnear) * (DATA.reach_mult - 1)
	end

	local swings = 0
	fw.perk{
		weapon = "wrench", slot = 1, name = "reach", needs = { "vec4.z" },
		hooks = {
			["wrench.swing"] = function(nxt, prev_a, prev_b, cur_a, cur_b, owner, attack, steps)
				local hx, hy, hz = fw.hero_pos()
				hz += 0.8
				local s1, m1 = stretch(prev_a, prev_b, hx, hy, hz)
				local s2, m2 = stretch(cur_a, cur_b, hx, hy, hz)
				local ok, r = pcall(nxt, prev_a, prev_b, cur_a, cur_b, owner, attack, steps)
				write3(s1.p, s1.v)
				write3(s2.p, s2.v)
				if not ok then error(r) end
				swings += 1
				if r ~= nil and r ~= 0 then
					fw.note("wrench/reach", string.format("swing %d hit (returned %s) with the far ends pushed out by %.2f and %.2f m", swings, tostring(r), m1, m2))
				elseif swings == 1 then
					fw.note("wrench/reach", string.format("swing sweep stretched: far ends pushed out by %.2f and %.2f m (no hit this tick)", m1, m2))
				end
				return r
			end,
		},
	}

	-- ----- V6: Hyper-Strike
	local pending = {} -- shockwaves waiting for on_tick
	local rings = {} -- { tick, x, y, z }
	local last_strike = -1000
	local struck = 0

	local function ring_draw(t)
		local d = omni.draw
		local keep = {}
		for _, rg in ipairs(rings) do
			local age = t - rg.t
			if age >= 0 and age < DATA.ring_ticks then
				table.insert(keep, rg)
				local radius = DATA.strike_radius * (age + 1) / DATA.ring_ticks
				local alpha = 255 - math.floor(160 * age / DATA.ring_ticks)
				local col = DATA.ring_color - (255 - alpha)
				for i = 0, DATA.ring_points - 1 do
					local ang = i * 6.2831853 / DATA.ring_points
					local sx, sy, front = d.world_to_screen(rg.x + math.cos(ang) * radius, rg.y + math.sin(ang) * radius, rg.z)
					if sx ~= nil and front then
						d.rect(sx - 1, sy - 1, DATA.ring_size, DATA.ring_size, col)
					end
				end
			end
		end
		rings = keep
	end

	fw.perk{
		weapon = "wrench", slot = 2, name = "hyper_strike", needs = { "hit_desc.attacker_class" },
		hooks = {
			["wrench.swing"] = function(nxt, prev_a, prev_b, cur_a, cur_b, owner, attack, steps)
				local r = nxt(prev_a, prev_b, cur_a, cur_b, owner, attack, steps)
				local t = fw.tick()
				if r ~= nil and r ~= 0 and t - last_strike >= DATA.strike_cooldown and attack ~= 0 then
					last_strike = t
					local desc = omni.struct("hit_desc", attack)
					if desc ~= nil then
						table.insert(pending, {
							attacker = desc.attacker, mask = desc.mask, damage = desc.damage * DATA.strike_damage,
							k0 = desc.type, k1 = desc.strength_class, k2 = desc.attacker_class,
						})
					end
				end
				return r
			end,
		},
		on_tick = function(t)
			H.flush()
			if #pending > 0 then
				local q = pending
				pending = {}
				local hx, hy, hz = fw.hero_pos()
				for _, p in ipairs(q) do
					local list = omni.mobys{ near = { hx, hy, hz + DATA.strike_lift }, radius = DATA.strike_radius, max = DATA.strike_max, enemy = true }
					local n = 0
					for _, e in ipairs(list) do
						if fw.alive(e) then
							local m = omni.struct("moby", e)
							local dx, dy = m.x - hx, m.y - hy
							local dl = math.sqrt(dx * dx + dy * dy)
							if dl > 0.01 then dx, dy = dx / dl, dy / dl end
							if H.hit(e, p.damage, p.attacker, p.mask, dx, dy, 0, p.k0, p.k1, p.k2) then n += 1 end
						end
					end
					struck += 1
					table.insert(rings, { t = t, x = hx, y = hy, z = hz + DATA.ring_height })
					fw.note("wrench/hyper_strike", string.format("shockwave %d at %.1f,%.1f,%.1f: %d of %d enemies in %.0f m hit for %.2f", struck, hx, hy, hz,
						n, #list, DATA.strike_radius, p.damage))
				end
			end
			if #rings > 0 then ring_draw(t) end
		end,
	}
end

return M
