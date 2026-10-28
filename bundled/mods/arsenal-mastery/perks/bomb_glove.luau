--!nonstrict
-- Bomb Glove perks. V3: bigger blast (the blast sphere of a bomb grows 1.35x).
-- V6 Mega: cluster (when a bomb explodes it throws mini bombs; they explode a moment later with a smaller blast).
local M = {}

local DATA = {
	bomb_class = 121,
	radius_mult = 1.35,
	explode_state = 2, -- the bomb asks for its blast sphere in this state (state 1 = in flight: not scaled)
	flight_state = 1,
	cluster_count = 4, -- mini bombs per blast
}

function M.setup(fw)
	local omni = omni
	local mb = require("perks/minibomb").get(fw)
	fw.radius_perk{ weapon = "bomb_glove", slot = 1, name = "big_blast", class = DATA.bomb_class, mult = DATA.radius_mult,
		when = function(m) return m.state == DATA.explode_state end }

	local hooks = mb.child_hooks()
	hooks["projectile.bomb.update"] = function(nxt, bomb)
		-- a mini bomb of a cluster is never split again: no record reads for it
		if mb.is_child(bomb) then return nxt(bomb) end
		local before = omni.struct("moby", bomb)
		local s0 = before and before.state or -1
		nxt(bomb)
		if s0 == DATA.flight_state and not mb.is_child(bomb) then
			local after = omni.struct("moby", bomb)
			if after ~= nil and after.state == DATA.explode_state then
				local bp = omni.struct("bomb", after.pvar)
				local n = mb.spawn(after.x, after.y, after.z, bp and bp.owner or 0, DATA.cluster_count)
				fw.note("bomb_glove/cluster", string.format("bomb %d exploded at %.1f,%.1f,%.1f: %d mini bombs queued", bomb, after.x, after.y, after.z, n))
			end
		end
	end
	fw.perk{ weapon = "bomb_glove", slot = 2, name = "cluster", hooks = hooks, needs = { "moby.draw_dist", "vec4.w" },
		on_tick = function(t) mb.flush() if t % 64 == 0 then mb.prune(t) end end }
end

return M
