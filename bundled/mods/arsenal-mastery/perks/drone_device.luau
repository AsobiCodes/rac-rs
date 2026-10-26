--!nonstrict
-- Drone Device perks (both SUBSTITUTES: the drone count, 6, is a table size in the game code, so there is no "one more drone").
-- A drone is gone after 100 attacks (its attack counter). V3: each new drone gets x1.5 attacks (150). V6 Mega: x2.5 (250).
-- V6 replaces V3 (never both). Drones are found by polling (class 479); the counter is raised once, the first tick a drone is seen.
local M = {}

local DATA = {
	drone_class = 479,
	radius = 80.0, max = 12,
	mult_v3 = 1.5,
	mult_v6 = 2.5,
	forget = 120, -- ticks until a drone that is no longer listed is forgotten
}

-- The drone's attack counter comes from the binding's struct "drone" (attacks_left).

function M.setup(fw)
	local omni = omni

	local function boost(key, seen, mult, t)
		local x, y, z = fw.hero_pos()
		local list = omni.mobys{ class = DATA.drone_class, near = { x, y, z }, radius = DATA.radius, max = DATA.max }
		for _, a in ipairs(list) do
			if seen[a] == nil then
				local m = omni.struct("moby", a)
				if m ~= nil and m.pvar ~= 0 then
					local d = omni.struct("drone", m.pvar)
					local c = d and d.attacks_left or 0
					if c > 0 then
						local v = math.min(32000, math.floor(c * mult))
						omni.struct_set("drone", m.pvar, "attacks_left", v)
						fw.note(key, string.format("drone %d counter %d -> %d", a, c, v))
						seen[a] = t
					end
				end
			else
				seen[a] = t
			end
		end
		for a, last in pairs(seen) do
			if t - last > DATA.forget then seen[a] = nil end
		end
	end

	local seen1, seen2 = {}, {}
	local e2 = nil
	fw.perk{
		weapon = "drone_device", slot = 1, name = "long_lived", needs = { "drone.attacks_left" },
		on_tick = function(t)
			if e2 ~= nil and e2.on then return end
			boost("drone_device/long_lived", seen1, DATA.mult_v3, t)
		end,
	}
	e2 = fw.perk{
		weapon = "drone_device", slot = 2, name = "enduring", needs = { "drone.attacks_left" },
		on_tick = function(t) boost("drone_device/enduring", seen2, DATA.mult_v6, t) end,
	}
end

return M
