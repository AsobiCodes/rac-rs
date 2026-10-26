--!nonstrict
-- Blaster perks. V3: lock-on (a shot that flies without a target steers to the nearest enemy in front of it).
-- The V6 Mega perk (Dual Blasters) lives in perks/dual_blasters.luau.
local M = {}

local DATA = {
	range = 25.0, -- metres from the shot
	cone = 0.5, -- cosine of the half angle (60 degrees) around the shot's flight direction
	reaim_every = 4, -- ticks between re-aims at the moving enemy
	forget_after = 40, -- ticks without an update before a shot is forgotten
}

function M.setup(fw)
	local omni = omni
	local track = {} -- shot moby -> { x, y, z, last, target, next }
	fw.perk{
		weapon = "blaster", slot = 1, name = "lock_on",
		hooks = {
			["projectile.blaster_shot.update"] = function(nxt, shot)
				local t = fw.tick()
				local m = omni.struct("moby", shot)
				local e = track[shot]
				if m ~= nil then
					if e == nil then
						track[shot] = { x = m.x, y = m.y, z = m.z, last = t, target = 0, next = 0, n = 1 }
					else
						e.last = t
						if e.n == 1 then
							e.n = 2
							local p = omni.struct("blaster_shot", m.pvar)
							local dx, dy, dz = m.x - e.x, m.y - e.y, m.z - e.z
							local len = math.sqrt(dx * dx + dy * dy + dz * dz)
							if p ~= nil and len > 0.05 and p.owner == fw.hero() and p.aim_x == 0 and p.aim_y == 0 and p.aim_z == 0 then
								local en, d = fw.nearest_enemy(m.x, m.y, m.z, DATA.range,
									{ cone = DATA.cone, fx = dx / len, fy = dy / len, fz = dz / len, los = true })
								if en ~= 0 then e.target = en end
							end
						end
						if e.target ~= 0 and t >= e.next then
							e.next = t + DATA.reaim_every
							if fw.alive(e.target) then
								local ax, ay, az = fw.aim_point(e.target)
								omni.struct_set("blaster_shot", m.pvar, "aim_x", ax)
								omni.struct_set("blaster_shot", m.pvar, "aim_y", ay)
								omni.struct_set("blaster_shot", m.pvar, "aim_z", az)
								local back = omni.struct("blaster_shot", m.pvar)
								if not e.logged then e.logged = true
								fw.note("blaster/lock_on", string.format("shot %d locked on moby %d: pvar aim set to %.1f,%.1f,%.1f (read back %.1f,%.1f,%.1f)",
									shot, e.target, ax, ay, az, back.aim_x, back.aim_y, back.aim_z)) end
							else
								e.target = 0
							end
						end
					end
				end
				return nxt(shot)
			end,
		},
		on_tick = function(t)
			if t % 64 == 0 then
				for shot, e in pairs(track) do
					if e.last < t - DATA.forget_after then track[shot] = nil end
				end
			end
		end,
	}
end

return M
