--!nonstrict
-- Glove of Doom perks. V3: more agents (every agent the glove releases brings a twin).
-- V6 Mega: the agents explode bigger (blast sphere 1.6x while an agent explodes).
local M = {}

local DATA = {
	agent_class = 186,
	explode_state = 0xb,
	radius_mult = 1.6,
	extra_per_agent = 1, -- twins per agent
	max_agents = 14, -- agents alive at once (twins included)
	offset = 0.4, -- metres a twin starts away from its original
	turn = 0.35, -- radians a twin's heading differs by, at most
	forget_after = 1500, -- ticks until an agent is dropped from the seen table
	full_colour = false, -- fallback: also set moby update_dist / draw_dist to 255 as the game's own creator does
}

function M.setup(fw)
	local omni = omni
	local seen = {} -- agent moby -> tick it was first seen (originals and twins: a twin never gets a twin)
	local create = nil
	local dead = false

	local function twin_of(agent)
		local p = omni.struct("moby", agent)
		if p == nil or p.pvar == 0 then return 0 end
		local a = omni.struct("doom_agent", p.pvar)
		if a == nil then return 0 end
		if create == nil then
			local ok, f = pcall(omni.fn, "create_moby")
			if not ok then
				dead = true
				fw.log("perk: glove_of_doom more_agents: the game's create function is not available here, off")
				return 0
			end
			create = f
		end
		local c = create(DATA.agent_class)
		if c == nil or c == 0 then return 0 end
		seen[c] = fw.tick()
		local jx, jy = (fw.rnd() * 2 - 1) * DATA.offset, (fw.rnd() * 2 - 1) * DATA.offset
		local ang = (fw.rnd() * 2 - 1) * DATA.turn
		local co, si = math.cos(ang), math.sin(ang)
		local cm = omni.struct("moby", c)
		omni.struct_set("moby", c, "visible", 1)
		omni.struct_set("moby", c, "x", p.x + jx)
		omni.struct_set("moby", c, "y", p.y + jy)
		omni.struct_set("moby", c, "z", p.z)
		omni.struct_set("moby", c, "scale", p.scale)
		omni.struct_set("moby", c, "light0", p.light0)
		omni.struct_set("moby", c, "light1", p.light1)
		omni.struct_set("moby", c, "rot_x", p.rot_x)
		omni.struct_set("moby", c, "rot_y", p.rot_y)
		omni.struct_set("moby", c, "yaw", p.yaw)
		omni.struct_set("doom_agent", cm.pvar, "vx", a.vx * co - a.vy * si)
		omni.struct_set("doom_agent", cm.pvar, "vy", a.vx * si + a.vy * co)
		omni.struct_set("doom_agent", cm.pvar, "vz", a.vz)
		omni.struct_set("doom_agent", cm.pvar, "goal_x", a.goal_x)
		omni.struct_set("doom_agent", cm.pvar, "goal_y", a.goal_y)
		omni.struct_set("doom_agent", cm.pvar, "goal_z", a.goal_z)
		omni.struct_set("doom_agent", cm.pvar, "target", a.target)
		omni.struct_set("doom_agent", cm.pvar, "speed", a.speed)
		omni.struct_set("doom_agent", cm.pvar, "owner", a.owner)
		if DATA.full_colour then
			omni.struct_set("moby", c, "update_dist", 255)
			omni.struct_set("moby", c, "draw_dist", 255)
		end
		return c
	end

	fw.perk{
		weapon = "glove_of_doom", slot = 1, name = "more_agents",
		hooks = {
			["projectile.doom_agent.update"] = function(nxt, agent)
				if not dead and seen[agent] == nil then
					seen[agent] = fw.tick()
					local alive = omni.mobys{ class = DATA.agent_class, max = 64 }
					for _ = 1, DATA.extra_per_agent do
						if #alive >= DATA.max_agents then break end
						local c = twin_of(agent)
						if c ~= 0 then
							alive[#alive + 1] = c
							fw.note("glove_of_doom/more_agents", string.format("agent %d got a twin %d (%d agents alive)", agent, c, #alive))
						end
					end
				end
				return nxt(agent)
			end,
			["moby.delete"] = function(nxt, m)
				seen[m] = nil
				return nxt(m)
			end,
		},
		on_tick = function(t)
			if t % 64 == 0 then
				for a, first in pairs(seen) do
					if t - first > DATA.forget_after then seen[a] = nil end
				end
			end
		end,
	}
	fw.radius_perk{
		weapon = "glove_of_doom", slot = 2, name = "big_agents", class = DATA.agent_class, mult = DATA.radius_mult,
		when = function(m) return m.state == DATA.explode_state end,
	}
end

return M
