--!nonstrict
-- The shared hit helper: hurts an enemy through the game's own hit path (the same two functions every attack uses:
-- build a hit descriptor, then post it on the victim). Because it is the game's path, power.luau sees the hit
-- (attacker -> weapon, level power, XP, kills) exactly like a hit from the weapon itself.
-- This file is not a perk. The perk files fetch the one shared helper with require("perks/hitpath").get(fw).
--
--   local H = require("perks/hitpath").get(fw)
--   H.hit(victim, damage, attacker, mask, dx, dy, dz, kind0, kind1, kind2)   -- posts (or queues) one hit
--   H.flush()                                                                -- call from on_tick
--
-- Mode: the hit is posted at once. If the first posts do not show on the victim (hit_slot stays 255) the helper switches to
-- queue mode (hits are collected and posted by H.flush, between two ticks) and says so in the log, like the mini bombs.
local M = {}

local DATA = {
	size = 96, -- bytes of stack: descriptor at +0, direction vector at +dir
	dir = 64, -- direction vec4 (4 f32)
	-- The descriptor's kind bytes are the binding's hit_desc fields: k0 = type (wrench 0, other attacks 5), k1 = strength_class
	-- (1), k2 = attacker_class (wrench 71, others: the class of the weapon moby).
	not_posted = 255, -- victim hit_slot value when no hit is waiting
	switch_after = 2, -- posts without any readback success before queue mode
	max_queue = 64,
}

function M.get(fw)
	if fw.hitpath ~= nil then return fw.hitpath end
	local omni = omni
	local H = { DATA = DATA, posted = 0, queued = 0, seen_ok = 0, seen_bad = 0, mode = "now" }
	local build, post = nil, nil
	local dead = false
	local queue = {}

	local function fn_of(name)
		local ok, f = pcall(omni.fn, name)
		if ok then return f end
		return nil
	end

	local function post_now(h)
		if build == nil then
			build = fn_of("hit_desc_build")
			post = fn_of("hit_ring_write")
		end
		if build ~= nil and not (fw.has_field("hit_desc", "attacker_class") and fw.has_field("vec4", "w")) then
			build = nil
			fw.log("perk: hit path: this port's binding has no hit_desc type/strength_class/attacker_class or vec4 fields")
		end
		if build == nil or post == nil then
			dead = true
			fw.log("perk: hit path: the game's hit_desc_build / hit_ring_write are not available here, extra hits are off")
			return false
		end
		local before = omni.struct("moby", h.victim)
		local slot0 = before and before.hit_slot or DATA.not_posted
		omni.with_stack(DATA.size, function(a)
			local d = a + DATA.dir
			omni.struct_set("vec4", d, "x", h.dx)
			omni.struct_set("vec4", d, "y", h.dy)
			omni.struct_set("vec4", d, "z", h.dz)
			omni.struct_set("vec4", d, "w", 0.0)
			build(h.damage, a, h.attacker, h.mask, d)
			omni.struct_set("hit_desc", a, "type", h.k0)
			omni.struct_set("hit_desc", a, "strength_class", h.k1)
			omni.struct_set("hit_desc", a, "attacker_class", h.k2)
			post(h.victim, a)
		end)
		H.posted += 1
		local after = omni.struct("moby", h.victim)
		local slot1 = after and after.hit_slot or DATA.not_posted
		if slot1 ~= DATA.not_posted then
			H.seen_ok += 1
		else
			H.seen_bad += 1
		end
		fw.note("hitpath/post", string.format("victim %d damage %.2f attacker %d mask %d kind %d,%d,%d: hit_slot %d -> %d (%s)",
			h.victim, h.damage, h.attacker, h.mask, h.k0, h.k1, h.k2, slot0, slot1, H.mode))
		if H.mode == "now" and H.seen_ok == 0 and H.seen_bad >= DATA.switch_after then
			H.mode = "queue"
			fw.log("perk: hit path: a hit posted inside a hook did not show on the victim, switching to queued hits (posted between ticks)")
		end
		return true
	end

	-- Hurts `victim`. damage = base damage (power.luau scales it by the weapon's level). (dx, dy, dz) = hit direction.
	function H.hit(victim, damage, attacker, mask, dx, dy, dz, k0, k1, k2)
		if dead or victim == nil or victim == 0 then return false end
		local h = { victim = victim, damage = damage, attacker = attacker, mask = mask, dx = dx or 0, dy = dy or 0, dz = dz or 0,
			k0 = k0 or 5, k1 = k1 or 1, k2 = k2 or 0 }
		if H.mode == "queue" then
			if #queue < DATA.max_queue then
				table.insert(queue, h)
				H.queued += 1
			end
			return true
		end
		return post_now(h)
	end

	-- Posts the queued hits (queue mode). Call from on_tick.
	function H.flush()
		if #queue == 0 then return end
		local q = queue
		queue = {}
		for _, h in ipairs(q) do
			if dead then break end
			if fw.alive(h.victim) then post_now(h) end
		end
	end

	fw.hitpath = H
	return H
end

return M
