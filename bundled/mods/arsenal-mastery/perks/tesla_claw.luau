--!nonstrict
-- Tesla Claw perks. The game's own chain zaps one target (two at gold) within 12 m (18 gold); the reach is a game global that a
-- mod cannot scale, so both perks are SUBSTITUTES that zap through the game's hit path (perks/hitpath.luau):
--   V3 long_reach: while the claw fires and the game found no target, the nearest enemy in a tight cone 12-17 m away is zapped,
--                  with a dotted line from Ratchet's hand to him.
--   V6 extra_jump: one more jump of the chain: the nearest other enemy within 7 m of the last target in the chain is zapped,
--                  with a line from the last target to him.
local M = {}

local DATA = {
	claw_class = 177,
	damage = 2.0, -- per zap (the game: 2 + gold); power.luau scales it by the weapon's level
	mask = 0x210000, -- the claw's own hit mask
	kind0 = 5, kind1 = 1,
	every = 2, -- ticks between zaps
	reach_min = 12.0, -- the game's reach (metres); the band starts here
	reach_max = 17.0,
	cone = 0.9, -- cosine of the angle to Ratchet's facing
	jump_radius = 7.0, -- metres around the last target
	firing_min = 19, -- marker >= this after the update = the chain ran this tick (the game sets 20)
	line_step = 6, -- pixels between the dots of the dotted line
	line_color = 0x80E0FFFF,
	line_size = 2,
	line_ticks = 2, -- a line stays this many ticks after the zap
	-- The claw's variables come from the binding's struct "tesla_claw": target1 (first target of the chain, 0 = none),
	-- target2 (second target, gold only), marker (set to 20 on every tick the chain runs).
}

function M.setup(fw)
	local omni = omni
	local H = require("perks/hitpath").get(fw)

	local lines = {} -- { x1,y1,z1, x2,y2,z2, last tick it is drawn }

	local function claw_state(claw)
		local m = omni.struct("moby", claw)
		if m == nil or m.pvar == 0 or m.class ~= DATA.claw_class then return nil end
		local c = omni.struct("tesla_claw", m.pvar)
		if c == nil then return nil end
		return { marker = c.marker, t1 = c.target1, t2 = c.target2 }
	end

	local function zap(victim, claw, x1, y1, z1)
		local ex, ey, ez = fw.aim_point(victim)
		if not H.hit(victim, DATA.damage, claw, DATA.mask, 0, 0, 0, DATA.kind0, DATA.kind1, DATA.claw_class) then return end
		table.insert(lines, { x1, y1, z1, ex, ey, ez, fw.tick() + DATA.line_ticks })
	end

	local drawn_tick = -1
	local function draw_lines(t)
		if drawn_tick == t then return end -- both perks call this: draw once per tick
		drawn_tick = t
		local d = omni.draw
		local keep = {}
		for _, l in ipairs(lines) do
			if t <= l[7] then
				table.insert(keep, l)
				local ax, ay, af = d.world_to_screen(l[1], l[2], l[3])
				local bx, by, bf = d.world_to_screen(l[4], l[5], l[6])
				if ax ~= nil and bx ~= nil and af and bf then
					local dx, dy = bx - ax, by - ay
					local len = math.sqrt(dx * dx + dy * dy)
					local n = math.max(1, math.floor(len / DATA.line_step))
					for i = 0, n do
						local f = i / n
						d.rect(ax + dx * f - 1, ay + dy * f - 1, DATA.line_size, DATA.line_size, DATA.line_color)
					end
				end
			end
		end
		lines = keep
	end

	local function band_target(hx, hy, hz, claw)
		local fx, fy = fw.forward()
		local best, bd = 0, 1e9
		for _, e in ipairs(omni.mobys{ near = { hx, hy, hz }, radius = DATA.reach_max, max = 12, enemy = true }) do
			if fw.alive(e) then
				local ex, ey, ez = fw.aim_point(e)
				local dx, dy, dz = ex - hx, ey - hy, ez - hz
				local d = math.sqrt(dx * dx + dy * dy + dz * dz)
				if d > DATA.reach_min and d <= DATA.reach_max and d < bd and (dx * fx + dy * fy) / d >= DATA.cone then
					local clear = true
					if omni.raycast ~= nil then
						local hit, qx, qy, qz = omni.raycast(hx, hy, hz, ex, ey, ez)
						if hit then clear = math.sqrt((qx - hx) ^ 2 + (qy - hy) ^ 2 + (qz - hz) ^ 2) >= d - 1.5 end
					end
					if clear then best, bd = e, d end
				end
			end
		end
		return best, bd
	end

	local zaps_a, zaps_b = 0, 0
	local seen_marker = 0
	fw.perk{
		weapon = "tesla_claw", slot = 1, name = "long_reach", needs = { "tesla_claw.marker", "tesla_claw.target1" },
		hooks = {
			["weapon.tesla_claw.update"] = function(nxt, claw)
				nxt(claw)
				local s = claw_state(claw)
				if s == nil then return end
				if seen_marker < 3 and s.marker >= DATA.firing_min then
					seen_marker += 1
					fw.log(string.format("perk: tesla_claw firing marker %d, target1 %d, target2 %d", s.marker, s.t1, s.t2))
				end
				if s.marker >= DATA.firing_min and s.t1 == 0 and fw.tick() % DATA.every == 0 then
					local hx, hy, hz = omni.fields.omni.hero.right_hand.x, omni.fields.omni.hero.right_hand.y, omni.fields.omni.hero.right_hand.z
					local e, d = band_target(hx, hy, hz, claw)
					if e ~= 0 then
						zap(e, claw, hx, hy, hz)
						zaps_a += 1
						fw.note("tesla_claw/long_reach", string.format("no game target: zapped enemy %d at %.1f m (band %.0f-%.0f m), zap %d", e, d, DATA.reach_min, DATA.reach_max, zaps_a))
					end
				end
			end,
		},
		on_tick = function(t)
			H.flush()
			if #lines > 0 then draw_lines(t) end
		end,
	}

	fw.perk{
		weapon = "tesla_claw", slot = 2, name = "extra_jump", needs = { "tesla_claw.marker", "tesla_claw.target1", "tesla_claw.target2" },
		hooks = {
			["weapon.tesla_claw.update"] = function(nxt, claw)
				nxt(claw)
				local s = claw_state(claw)
				if s == nil or s.marker < DATA.firing_min or s.t1 == 0 or fw.tick() % DATA.every ~= 0 then return end
				if not fw.alive(s.t1) then return end
				local last = (s.t2 ~= 0 and fw.alive(s.t2)) and s.t2 or s.t1
				local lx, ly, lz = fw.aim_point(last)
				local best, bd = 0, 1e9
				for _, e in ipairs(omni.mobys{ near = { lx, ly, lz }, radius = DATA.jump_radius, max = 8, enemy = true }) do
					if e ~= s.t1 and e ~= s.t2 and e ~= last and fw.alive(e) then
						local ex, ey, ez = fw.aim_point(e)
						local d = math.sqrt((ex - lx) ^ 2 + (ey - ly) ^ 2 + (ez - lz) ^ 2)
						if d < bd then best, bd = e, d end
					end
				end
				if best ~= 0 then
					zap(best, claw, lx, ly, lz)
					zaps_b += 1
					fw.note("tesla_claw/extra_jump", string.format("chain %d -> %d: one more jump to enemy %d (%.1f m), zap %d", s.t1, last, best, bd, zaps_b))
				end
			end,
		},
		on_tick = function(t)
			H.flush()
			if #lines > 0 then draw_lines(t) end
		end,
	}
end

return M
