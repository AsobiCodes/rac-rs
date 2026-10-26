--!nonstrict
-- Morph-o-Ray perks (both SUBSTITUTES: the beam's width is a game constant that no field reaches yet).
-- V3: a faster morph. V6 Mega: faster still (2.2x in all, with V3 on). "Faster" is applied in two places of the held Morph-o-Ray:
--   (a) the morph progress of the locked target (time left, drained 1.5x / 2.2x as fast). Only enemies that need more than one
--       tick of beam show it: an enemy whose health block says 1 (every enemy placed on the first planets) morphs on the tick the
--       beam locks on, so there is nothing left to speed up.
--   (b) the lock cooldown (the game waits 25 ticks after each lock before the beam may take a new target; counted down 1.5x / 2.2x
--       as fast), so a crowd is turned into sheep faster even when every single morph is instant.
-- After each held update, the time the update took off the progress / the cooldown is taken off once more (x0.5 / x0.47).
local M = {}

local DATA = {
	extra_v3 = 0.5, -- extra drain, relative to what the game drained (total x1.5)
	total_v6 = 2.2, -- total drain factor with both perks on
	extra_v6_alone = 1.2, -- if V3 is somehow off while V6 is on
	log_cycles = 12, -- lock cycles logged
}

-- The held Morph-o-Ray's variables come from the binding's struct "held_morph": time_left (f32, starts at the target's
-- health, drained each tick, <= 0 = sheep; 0 at once for a target whose max is 1) and lock (i32 cooldown, 25 ticks after
-- each lock, a new target needs 0).

function M.setup(fw)
	local omni = omni
	local acc = {} -- perk key -> fractional extra ticks of the lock cooldown carried to the next tick
	local seen_tick, prev_lock, lock_t0, cycles = -1, 0, 0, 0

	-- One log line per finished lock cooldown, counted from the value the game had when the tick began (the same for both perks).
	local function track(lock)
		local t = fw.tick()
		if t == seen_tick then return end
		seen_tick = t
		-- a cycle ends when the lock is seen at 0, or already reloaded (a new target is taken on the tick the old lock runs out)
		local ended = (lock == 0 and prev_lock > 0) or (lock > prev_lock and prev_lock > 0)
		if ended and cycles < DATA.log_cycles then
			cycles += 1
			fw.log(string.format("perk: morph_o_ray retarget lock lasted %d ticks (the game alone: 25), cycle %d", t - lock_t0, cycles))
		end
		if lock > prev_lock then lock_t0 = t end
		prev_lock = lock
	end

	-- Runs the game's update and takes `extra` x its drain off the progress and the lock cooldown once more. Returns what the update returns.
	local function faster(key, extra, nxt, moby)
		local m = omni.struct("moby", moby)
		local p0 = (m ~= nil and m.pvar ~= 0) and omni.struct("held_morph", m.pvar) or nil
		if p0 == nil then return nxt(moby) end
		local before = p0.time_left
		local lock_before = p0.lock
		track(lock_before)
		local r = nxt(moby)
		local p1 = omni.struct("held_morph", m.pvar)
		if p1 == nil then return r end
		local after = p1.time_left
		if before > 0 and after > 0 and after < before then
			local v = math.max(0, after - (before - after) * extra)
			omni.struct_set("held_morph", m.pvar, "time_left", v)
			fw.note(key, string.format("time left %.3f -> %.3f (game drained %.3f, extra x%.2f)", after, v, before - after, extra))
		end
		local lock_after = p1.lock
		if lock_before > 0 and lock_after > 0 and lock_after < lock_before then
			local carry = (acc[key] or 0) + (lock_before - lock_after) * extra
			local n = math.floor(carry)
			acc[key] = carry - n
			if n > 0 then
				local v = math.max(0, lock_after - n)
				omni.struct_set("held_morph", m.pvar, "lock", v)
				fw.note(key .. "/lock", string.format("retarget lock %d -> %d (game counted down %d, extra x%.2f)", lock_after, v, lock_before - lock_after, extra))
			end
		end
		return r
	end

	local e1 = nil
	e1 = fw.perk{
		weapon = "morph_o_ray", slot = 1, name = "quick_morph", needs = { "held_morph.time_left", "held_morph.lock" },
		hooks = { ["weapon.morph_o_ray.update"] = function(nxt, moby) return faster("morph_o_ray/quick_morph", DATA.extra_v3, nxt, moby) end },
	}
	fw.perk{
		weapon = "morph_o_ray", slot = 2, name = "swift_morph", needs = { "held_morph.time_left", "held_morph.lock" },
		hooks = {
			["weapon.morph_o_ray.update"] = function(nxt, moby)
				-- slot 1's handler wraps this one: it adds (1 + extra_v3) on top, so this one adds the rest of the total
				local extra = DATA.extra_v6_alone
				if e1.on then extra = DATA.total_v6 / (1 + DATA.extra_v3) - 1 end
				return faster("morph_o_ray/swift_morph", extra, nxt, moby)
			end,
		},
	}
end

return M
