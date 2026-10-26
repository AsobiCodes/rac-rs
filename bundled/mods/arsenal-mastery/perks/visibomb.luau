--!nonstrict
-- Visibomb perks. V3: bigger blast (the blast sphere of the missile grows 1.35x).
-- V6 Mega: split (when the missile explodes it throws mini bombs, the same small bombs as the Bomb Glove cluster).
local M = {}

local DATA = {
	missile_class = 172,
	radius_mult = 1.35,
	split_count = 4, -- mini bombs per missile
	rearm = 120, -- ticks before the same moby slot may split again
}

function M.setup(fw)
	local omni = omni
	local mb = require("perks/minibomb").get(fw)
	fw.radius_perk{ weapon = "visibomb", slot = 1, name = "big_blast", class = DATA.missile_class, mult = DATA.radius_mult }

	local done = {} -- missile moby -> tick it split
	local hooks = mb.child_hooks()
	local child_delete = hooks["moby.delete"]
	hooks["moby.delete"] = function(nxt, m)
		done[m] = nil
		return child_delete(nxt, m)
	end
	local child_blast = hooks["blast.apply"]
	hooks["blast.apply"] = function(nxt, strength, hscale, vimpulse, attacker, pos, list, count, exclude, mask, cat, sub)
		-- the missile's blast first (with the child scaling when a mini bomb is the attacker), then the split
		child_blast(nxt, strength, hscale, vimpulse, attacker, pos, list, count, exclude, mask, cat, sub)
		if attacker ~= 0 then
			local m = omni.struct("moby", attacker)
			if m ~= nil and m.class == DATA.missile_class then
				local t = fw.tick()
				local last = done[attacker]
				if last == nil or t - last > DATA.rearm then
					done[attacker] = t
					local owner = 0
					local ok, held = pcall(function() return omni.fields.omni.hero.held_moby end)
					if ok and held ~= nil and held ~= 0 then owner = held else owner = fw.hero() end
					local n = mb.spawn(m.x, m.y, m.z, owner, DATA.split_count)
					fw.note("visibomb/split", string.format("missile %d exploded at %.1f,%.1f,%.1f: %d mini bombs queued", attacker, m.x, m.y, m.z, n))
				end
			end
		end
	end
	fw.perk{ weapon = "visibomb", slot = 2, name = "split", hooks = hooks, needs = { "moby.draw_dist", "vec4.w" },
		on_tick = function(t)
			mb.flush()
			if t % 64 == 0 then
				mb.prune(t)
				for a, made in pairs(done) do
					if t - made > 600 then done[a] = nil end
				end
			end
		end }
end

return M
