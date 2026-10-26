--!nonstrict
-- Mine Glove perks. V3: more mines at once (the game keeps 8 and deletes the oldest at the next throw; this lets 12 stay).
-- V6 Mega: wider chase (a landed mine already rolls to the nearest enemy within 4 m (8 m gold); this makes it three times that).
local M = {}

local DATA = {
	mine_class = 74,
	cap = 12, -- mines alive at once (the game: 8)
	game_cap = 8,
	chase_state_bit = 16, -- moby state bit 0x10 = the mine is chasing
	chase_m = 12, -- chase radius in metres (the game: 4 at the base level)
	game_chase_m = 4,
	-- The mine's variables come from the binding's struct "mine": age ("oldest mine" key; the game sets it to 0xffffffff to make
	-- the mine update delete the mine), target (the enemy a chasing mine rolls to), chase_wide (1 = the landed-mine scan radius is
	-- 3x; the game clears it at the end of every mine update).
	deleted = 4294967295,
}

function M.setup(fw)
	local omni = omni

	local function pvar_of(m)
		local s = omni.struct("moby", m)
		if s == nil then return 0 end
		return s.pvar
	end

	local function mine_field(pv, field)
		local s = omni.struct("mine", pv)
		return s and s[field]
	end

	local throws, restored = 0, 0
	fw.perk{
		weapon = "mine_glove", slot = 1, name = "more_mines", needs = { "mine.age" },
		hooks = {
			["weapon.mine_glove.fire"] = function(nxt, pos_ptr, mine, pvar)
				local snap, n = {}, 0
				for _, m in ipairs(omni.mobys{ class = DATA.mine_class, max = 40 }) do
					if m ~= mine then
						local pv = pvar_of(m)
						if pv ~= 0 then
							snap[m] = { pv = pv, age = mine_field(pv, "age") }
							n += 1
						end
					end
				end
				nxt(pos_ptr, mine, pvar)
				throws += 1
				local back = 0
				if n < DATA.cap then
					for m, e in pairs(snap) do
						if e.age ~= nil and e.age ~= DATA.deleted and mine_field(e.pv, "age") == DATA.deleted then
							omni.struct_set("mine", e.pv, "age", e.age)
							back += 1
						end
					end
				end
				restored += back
				fw.note("mine_glove/more_mines", string.format("throw %d with %d other mines alive: %d kept that the game marked for deletion (cap %d, game %d)",
					throws, n, back, DATA.cap, DATA.game_cap))
			end,
		},
	}

	local chases, far_chases = 0, 0
	fw.perk{
		weapon = "mine_glove", slot = 2, name = "chase", needs = { "mine.chase_wide", "mine.target" },
		hooks = {
			["projectile.mine.update"] = function(nxt, mine)
				local pv = pvar_of(mine)
				if pv ~= 0 then omni.struct_set("mine", pv, "chase_wide", 1) end
				nxt(mine)
				if pv ~= 0 then
					local m = omni.struct("moby", mine)
					if m ~= nil and bit32.band(m.state, DATA.chase_state_bit) ~= 0 then
						local target = mine_field(pv, "target") or 0
						if target ~= 0 then
							local tm = omni.struct("moby", target)
							if tm ~= nil then
								local d = math.sqrt((tm.x - m.x) ^ 2 + (tm.y - m.y) ^ 2 + (tm.z - m.z) ^ 2)
								chases += 1
								if d > DATA.game_chase_m then
									far_chases += 1
									fw.note("mine_glove/chase", string.format("mine %d chases enemy %d at %.1f m (the game's own limit is %d m, twice that for the gold glove)", mine, target, d, DATA.game_chase_m))
								end
							end
						end
					end
				end
			end,
		},
	}
end

return M
