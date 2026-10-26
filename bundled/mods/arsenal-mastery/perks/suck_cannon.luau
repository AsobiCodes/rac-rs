--!nonstrict
-- Suck Cannon perks.
-- V3 (SUBSTITUTE for "carry more"): the Suck Cannon has no ammo to raise; what it holds is a code constant, 5 enemies, 10 with the
--   gold flag. While the held Suck Cannon updates, the gold byte is set (capacity 10) and put back right after, so the saved
--   game never sees it.
-- V6 Mega: an enemy that was flung and hits another enemy (or blows up on impact) bursts (radius 3 m, damage 3.0 to the enemies
--   around it, through the game's own hit path, once per flung enemy). No visual yet (binding request).
local M = {}

local DATA = {
	held_class = 849, -- the held Suck Cannon moby
	fling_mask = 0x430000, -- hit mask of a flung enemy (a mask value, not an address)
	burst_radius = 3.0, burst_damage = 3.0,
	burst_mask = 0x10000,
	kind0 = 5, kind1 = 1,
	cooldown = 60, -- ticks before the same flung enemy can burst again
	max_victims = 12,
	max_pending = 8,
}

function M.setup(fw)
	local omni = omni
	local assist = require("assist")
	local H = require("perks/hitpath").get(fw)

	fw.perk{
		weapon = "suck_cannon", slot = 1, name = "capacity",
		hooks = {
			["weapon.suck_cannon.update"] = function(nxt, moby)
				local node = omni.fields.omni.weapon.suck_cannon
				local gold = node.gold
				if gold ~= 0 then return nxt(moby) end
				node.gold = 1
				local ok, err = pcall(nxt, moby)
				node.gold = 0
				fw.note("suck_cannon/capacity", "gold-byte toggle: capacity 5 -> 10 during the held update, byte restored to " .. tostring(node.gold))
				if not ok then error(err, 0) end
			end,
		},
	}

	local pending, last_burst = {}, {}
	local e2 = fw.perk{
		weapon = "suck_cannon", slot = 2, name = "fling_burst",
		on_tick = function(t)
			H.flush()
			if #pending == 0 then return end
			local q = pending
			pending = {}
			local hero = fw.hero()
			for _, b in ipairs(q) do
				local list = omni.mobys{ enemy = true, near = { b.x, b.y, b.z }, radius = DATA.burst_radius, max = DATA.max_victims }
				local n = 0
				for _, a in ipairs(list) do
					if a ~= b.attacker and a ~= b.victim and fw.alive(a) then
						local m = omni.struct("moby", a)
						local dx, dy = m.x - b.x, m.y - b.y
						local l = math.sqrt(dx * dx + dy * dy)
						if l < 0.01 then dx, dy, l = 0, 1, 1 end
						if H.hit(a, DATA.burst_damage, hero, DATA.burst_mask, dx / l, dy / l, 0, DATA.kind0, DATA.kind1, DATA.held_class) then n += 1 end
					end
				end
				fw.note("suck_cannon/fling_burst", string.format("burst of flung enemy %d at %.1f,%.1f,%.1f hit %d enemies", b.attacker, b.x, b.y, b.z, n))
			end
		end,
	}
	assist.pre_hit(function(victim, desc, info)
		if not e2.on or info.attacker == 0 or not assist.is_fling(info.mask, info.aclass) then return end
		local t = fw.tick()
		local last = last_burst[info.attacker]
		if last ~= nil and t - last < DATA.cooldown then return end
		if #pending >= DATA.max_pending then return end
		local m = omni.struct("moby", info.attacker)
		if m == nil then return end
		last_burst[info.attacker] = t
		table.insert(pending, { x = m.x, y = m.y, z = m.z, attacker = info.attacker, victim = victim })
	end)
end

return M
