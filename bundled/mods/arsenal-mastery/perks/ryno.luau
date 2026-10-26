--!nonstrict
-- R.Y.N.O. perks. V3: more rockets per salvo (every shot fires two rockets, yaw +-0.08 apart).
-- V6 Mega: homing rockets (a rocket fired with no target gets the nearest enemy, and the game's own rocket steering does the rest).
local M = {}

local DATA = {
	spread = 0.08, -- radians each side
	range = 40.0, -- metres
	cone = 0.5, -- cosine of the half angle around Ratchet's heading (60 degrees: enemies beside or behind him are not picked)
}

function M.setup(fw)
	local omni = omni
	fw.perk{
		weapon = "ryno", slot = 1, name = "salvo",
		hooks = {
			["weapon.ryno.fire"] = function(nxt, height, yaw, pitch, owner, pos, target)
				local r1 = nxt(height, yaw - DATA.spread, pitch, owner, pos, target)
				local r2 = nxt(height, yaw + DATA.spread, pitch, owner, pos, target)
				fw.note("ryno/salvo", string.format("fire called twice: rockets %s and %s (yaw %.3f -> %.3f / %.3f)",
					tostring(r1), tostring(r2), yaw, yaw - DATA.spread, yaw + DATA.spread))
				if r1 == nil or r1 == 0 then return r2 end
				return r1
			end,
		},
	}
	fw.perk{
		weapon = "ryno", slot = 2, name = "homing",
		hooks = {
			["weapon.ryno.fire"] = function(nxt, height, yaw, pitch, owner, pos, target)
				local given = 0
				if target == 0 then
					local hx, hy, hz = fw.hero_pos()
					local fx, fy = fw.forward()
					local e, d = fw.nearest_enemy(hx, hy, hz + 1.0, DATA.range, { cone = DATA.cone, fx = fx, fy = fy, los = true })
					if e == 0 then fw.note("ryno/homing_none", "target arg 0, no enemy in range/cone/line of sight") end
					if e ~= 0 then
						given = e
						target = e
						fw.note("ryno/homing", string.format("target arg 0 -> moby %d (%.1f m)", e, d))
					end
				end
				if target ~= 0 and given == 0 then fw.note("ryno/homing_kept", "the game already gave target " .. tostring(target)) end
				local r = nxt(height, yaw, pitch, owner, pos, target)
				if given ~= 0 and r ~= nil and r ~= 0 then
					local m = omni.struct("moby", r)
					local rp = m and omni.struct("ryno_rocket", m.pvar)
					fw.note("ryno/homing_check", "rocket " .. tostring(r) .. " pvar target " .. tostring(rp and rp.target)
						.. " class " .. tostring(rp and rp.target_class))
				end
				return r
			end,
		},
	}
end

return M
