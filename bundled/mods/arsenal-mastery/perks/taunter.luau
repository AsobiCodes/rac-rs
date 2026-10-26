--!nonstrict
-- Taunter perks.
-- V3: the taunt reaches further (alert range 30 -> 45 m, written into the held Taunter's variables before its update).
-- V6 Mega: a taunted enemy takes 1.5x damage from the player's hits (assist.luau tells which enemies are taunted; the hit's
--   descriptor is scaled in its pre_hit step, before the game posts the hit; power.luau then applies the level power as usual).
local M = {}

local DATA = {
	base_range = 30.0, -- retail alert range the update sets when it is 0
	range = 45.0,
	damage_mult = 1.5,
}

-- The alert range comes from the binding's struct "held_taunter" (range).

function M.setup(fw)
	local omni = omni
	local assist = require("assist")

	fw.perk{
		weapon = "taunter", slot = 1, name = "far_taunt", needs = { "held_taunter.range" },
		hooks = {
			["weapon.taunter.update"] = function(nxt, moby)
				local m = omni.struct("moby", moby)
				local p = (m ~= nil and m.pvar ~= 0) and omni.struct("held_taunter", m.pvar) or nil
				if p ~= nil then
					local r = p.range
					if math.abs(r - DATA.base_range) < 0.01 then
						omni.struct_set("held_taunter", m.pvar, "range", DATA.range)
						fw.note("taunter/far_taunt", string.format("range %.0f -> %.0f", r, DATA.range))
					end
				end
				return nxt(moby)
			end,
		},
	}

	local e2 = fw.perk{ weapon = "taunter", slot = 2, name = "focus" }
	assist.pre_hit(function(victim, desc, info)
		if not e2.on or not info.player or not assist.is_taunted(victim) then return end
		local h = omni.struct("hit_desc", desc)
		if h == nil or not (h.damage > 0) then return end
		local d2 = h.damage * DATA.damage_mult
		omni.struct_set("hit_desc", desc, "damage", d2)
		fw.note("taunter/focus", string.format("taunted enemy %d: hit damage %.2f -> %.2f", victim, h.damage, d2))
	end)
end

return M
