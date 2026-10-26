--!nonstrict
-- Devastator perks. V3: faster lock-on (a rocket fired with no target gets the nearest enemy in front of Ratchet).
-- V6 Mega: twin missiles (every shot launches two rockets, yaw +-0.08 apart).
local M = {}

local DATA = {
	lock_range = 30.0, -- metres
	lock_cone = 0.5, -- cosine of the half angle (60 degrees)
	twin_yaw = 0.08, -- radians each side
}

function M.setup(fw)
	local omni = omni
	fw.perk{
		weapon = "devastator", slot = 1, name = "fast_lock",
		hooks = {
			["weapon.devastator.fire"] = function(nxt, a0, pos, target, f0, f1, f2, f3, f4)
				local given = 0
				if target == 0 then
					local hx, hy, hz = fw.hero_pos()
					local fx, fy = fw.forward()
					local e, d = fw.nearest_enemy(hx, hy, hz + 1.0, DATA.lock_range, { cone = DATA.lock_cone, fx = fx, fy = fy, los = true })
					if e ~= 0 then
						given = e
						target = e
						fw.note("devastator/fast_lock", string.format("target arg 0 -> moby %d (%.1f m)", e, d))
					end
				end
				local r = nxt(a0, pos, target, f0, f1, f2, f3, f4)
				if given ~= 0 and r ~= nil and r ~= 0 then
					local m = omni.struct("moby", r)
					local rp = m and omni.struct("devastator_rocket", m.pvar)
					fw.note("devastator/fast_lock_check", "rocket " .. tostring(r) .. " pvar target " .. tostring(rp and rp.target))
				end
				return r
			end,
		},
	}
	fw.perk{
		weapon = "devastator", slot = 2, name = "twin_missiles",
		hooks = {
			["weapon.devastator.fire"] = function(nxt, a0, pos, target, f0, f1, f2, f3, f4)
				local r1 = nxt(a0, pos, target, f0, f1, f2, f3 - DATA.twin_yaw, f4)
				local r2 = nxt(a0, pos, target, f0, f1, f2, f3 + DATA.twin_yaw, f4)
				fw.note("devastator/twin_missiles", string.format("fire called twice: rockets %s and %s (yaw %.3f -> %.3f / %.3f)",
					tostring(r1), tostring(r2), f3, f3 - DATA.twin_yaw, f3 + DATA.twin_yaw))
				if r1 == nil or r1 == 0 then return r2 end
				return r1
			end,
		},
	}
end

return M
