--!nonstrict
-- Decoy Glove perks.
-- V3: the decoy stands longer (its timer x1.5, once per decoy).
-- V6 Mega: when enemies shoot the decoy down it blows up (radius 5 m, damage 4.0 to the enemies around it, through the game's own
--   hit path, once per decoy). The hits are credited to the decoy (assist.luau maps its classes to the Decoy Glove).
local M = {}

local DATA = {
	classes = { 203, 1900 }, -- the Decoy actor, the gold Decoy actor
	radius = 60.0, max = 8,
	active_states = { first = 3, last = 4 }, -- landed, counting down
	time_mult = 1.5,
	blast_radius = 5.0, blast_damage = 4.0,
	blast_mask = 0x10000, -- hit mask of an ordinary attack on enemies (a mask value, not an address)
	kind0 = 5, kind1 = 1,
	max_victims = 16,
	forget = 30, -- ticks until a decoy is dropped from the tables
}

-- The decoy's variables come from the binding's struct "decoy" (timer, health).

function M.setup(fw)
	local omni = omni
	local H = require("perks/hitpath").get(fw)

	local function decoys()
		local x, y, z = fw.hero_pos()
		return omni.mobys{ class = DATA.classes, near = { x, y, z }, radius = DATA.radius, max = DATA.max }
	end

	local boosted = {} -- decoy -> tick
	fw.perk{
		weapon = "decoy", slot = 1, name = "long_stay", needs = { "decoy.timer" },
		on_tick = function(t)
			for _, a in ipairs(decoys()) do
				local m = omni.struct("moby", a)
				if m ~= nil and m.pvar ~= 0 and m.state >= DATA.active_states.first and m.state <= DATA.active_states.last then
					if boosted[a] == nil then
						local d = omni.struct("decoy", m.pvar)
						local timer = d and d.timer or 0
						if timer > 0 then
							local v = math.min(32000, math.floor(timer * DATA.time_mult))
							omni.struct_set("decoy", m.pvar, "timer", v)
							fw.note("decoy/long_stay", string.format("timer %d -> %d (state %d)", timer, v, m.state))
							boosted[a] = t
						end
					else
						boosted[a] = t
					end
				end
			end
			for a, seen in pairs(boosted) do
				if t - seen > DATA.forget then boosted[a] = nil end
			end
		end,
	}

	local blasted = {} -- decoy -> tick
	fw.perk{
		weapon = "decoy", slot = 2, name = "last_stand", needs = { "decoy.health" },
		on_tick = function(t)
			H.flush()
			for _, a in ipairs(decoys()) do
				local m = omni.struct("moby", a)
				if blasted[a] ~= nil then
					blasted[a] = t
				elseif m ~= nil and m.pvar ~= 0 and m.state >= DATA.active_states.first then
					local d = omni.struct("decoy", m.pvar)
					local hp = d and d.health or 0
					if hp < 0 then
						blasted[a] = t
						local list = omni.mobys{ enemy = true, near = { m.x, m.y, m.z }, radius = DATA.blast_radius, max = DATA.max_victims }
						local n = 0
						for _, v in ipairs(list) do
							if fw.alive(v) then
								local vm = omni.struct("moby", v)
								local dx, dy = vm.x - m.x, vm.y - m.y
								local l = math.sqrt(dx * dx + dy * dy)
								if l < 0.01 then dx, dy, l = 0, 1, 1 end
								if H.hit(v, DATA.blast_damage, a, DATA.blast_mask, dx / l, dy / l, 0, DATA.kind0, DATA.kind1, m.class) then n += 1 end
							end
						end
						fw.note("decoy/last_stand", string.format("decoy %d destroyed (health %.2f): blast hit %d enemies at %.1f,%.1f,%.1f", a, hp, n, m.x, m.y, m.z))
					end
				end
			end
			for a, seen in pairs(blasted) do
				if t - seen > DATA.forget then blasted[a] = nil end
			end
		end,
	}
end

return M
