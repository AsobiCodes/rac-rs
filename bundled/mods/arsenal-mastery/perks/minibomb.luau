--!nonstrict
-- Mini bombs: the shared spawner behind the Bomb Glove cluster (V6) and the Visibomb split (V6).
-- This file is not a perk. The perk files fetch the one shared spawner with require("perks/minibomb").get(fw).
-- A mini bomb is a small Bomb Glove bomb thrown out of a blast with the game's own throw function, so it flies, bounces
-- and explodes like a real bomb (smaller blast, half damage). Mini bombs never cluster or split again.
local M = {}

local DATA = {
	bomb_class = 121,
	count = 4, -- mini bombs per cluster / split
	speed_min = 0.05, -- sideways speed, metres per tick
	speed_max = 0.10,
	lift_min = 0.08, -- upward speed, metres per tick (gravity pulls it back in about 45 ticks)
	lift_max = 0.12,
	fuse_min = 35, -- ticks to the mini bomb's explosion
	fuse_max = 45,
	size = 0.6, -- model size relative to a full bomb
	start_lift = 0.3, -- metres above the blast point
	toward_hero = 0.4, -- metres the start point moves toward Ratchet (a blast at a wall must not throw the bombs into it)
	fan = 1.9, -- radians each side of the direction to Ratchet in which the bombs fly
	budget_per_tick = 8, -- spawns per tick
	max_alive = 16, -- mini bombs alive at once
	radius_mult = 0.6, -- blast radius of a mini bomb (relative to the bomb's)
	strength_mult = 0.5, -- blast damage of a mini bomb
	forget_after = 600, -- ticks until a mini bomb is dropped from the table
	full_colour = true, -- also set moby update_dist / draw_dist to 255 as the game's own throw does (without them a mini bomb is never updated)
}

-- The one shared spawner (cached on fw, so it is the same table in every perk file).
function M.get(fw)
	if fw.minibomb ~= nil then return fw.minibomb end
	local omni = omni
	local S = { DATA = DATA }
	local children = {} -- mini bomb moby -> tick it was made
	local nchild = 0
	local create, launch = nil, nil
	local budget_tick, budget = -1, 0
	local dead = false
	local once_tick, once_seen = -1, {}
	local TWO_PI = 6.283185307

	-- A hook point is only live in the game when some hook sits on it, and omni.fn("<hook id>") runs the game's function
	-- through that point: take it with a pass-through (this runs in the main chunk, where hooks may be taken).
	-- The throw hook is optional in the manifest: a port without it only loses the mini bombs.
	if omni.has("rac1.omni.weapon.bomb_glove.fire") then
		local ok_hook, err_hook = pcall(function()
			omni.hook("rac1.omni.weapon.bomb_glove.fire", function(original, ...) return original(...) end)
		end)
		if not ok_hook then fw.log("perk: mini bombs: cannot take the throw hook (" .. string.sub(tostring(err_hook), 1, 100) .. ")") end
	else
		dead = true
		fw.log("perk: mini bombs: this port has no throw hook point, mini bombs are off")
	end

	local function fn_of(...)
		local ok, f = pcall(omni.fn, ...)
		if ok then return f end
		return nil
	end

	function S.is_child(addr) return children[addr] ~= nil end
	function S.count() return nchild end

	function S.forget(addr)
		if children[addr] ~= nil then
			children[addr] = nil
			nchild -= 1
		end
	end

	function S.prune(t)
		for a, made in pairs(children) do
			if t - made > DATA.forget_after then
				children[a] = nil
				nchild -= 1
			end
		end
	end

	-- True the first time per tick for (kind, who): two perks that both scale a mini bomb's blast scale it only once.
	function S.once(kind, who)
		local t = fw.tick()
		if t ~= once_tick then once_tick, once_seen = t, {} end
		local k = kind .. ":" .. tostring(who)
		if once_seen[k] then return false end
		once_seen[k] = true
		return true
	end

	-- Hook handlers that make the mini bombs' blast small. Both bomb perks take them (S.once keeps it to one scaling).
	function S.child_hooks()
		return {
			["collide.sphere_mobys"] = function(nxt, radius, pos, mask, self, filter)
				if self ~= 0 and children[self] ~= nil then
					local m = omni.struct("moby", self)
					if m ~= nil and m.state == 2 and S.once("r", self) then
						fw.note("minibomb/blast", string.format("mini bomb blast radius %.2f -> %.2f", radius, radius * DATA.radius_mult))
						return nxt(radius * DATA.radius_mult, pos, mask, self, filter)
					end
				end
				return nxt(radius, pos, mask, self, filter)
			end,
			["blast.apply"] = function(nxt, strength, hscale, vimpulse, attacker, pos, list, count, exclude, mask, cat, sub)
				if attacker ~= 0 and children[attacker] ~= nil and S.once("s", attacker) then
					return nxt(strength * DATA.strength_mult, hscale, vimpulse, attacker, pos, list, count, exclude, mask, cat, sub)
				end
				return nxt(strength, hscale, vimpulse, attacker, pos, list, count, exclude, mask, cat, sub)
			end,
			["moby.delete"] = function(nxt, m)
				S.forget(m)
				return nxt(m)
			end,
		}
	end

	-- Throws up to n mini bombs out of the point (x, y, z). owner = the glove moby that gets the credit. Returns how many.
	-- Runs between two game ticks only (from S.flush): the game's throw function cannot be started from inside another hook.
	local function do_spawn(x, y, z, owner, n)
		if dead then return 0 end
		local t = fw.tick()
		if t ~= budget_tick then budget_tick, budget = t, DATA.budget_per_tick end
		if create == nil then
			create = fn_of("create_moby")
			launch = fn_of("rac1.omni.weapon.bomb_glove.fire", "void(u32,u32,u32)")
		end
		if create == nil or launch == nil then
			dead = true
			fw.log("perk: mini bombs: the game's create/throw function is not available here, mini bombs are off")
			return 0
		end
		local hero = omni.struct("moby", fw.hero())
		if hero == nil then return 0 end
		local made = 0
		for _ = 1, n do
			if budget <= 0 or nchild >= DATA.max_alive then break end
			local m = create(DATA.bomb_class)
			if m == nil or m == 0 then break end
			budget -= 1
			local ok, err = pcall(function()
				local dxh, dyh = hero.x - x, hero.y - y
				local dh = math.sqrt(dxh * dxh + dyh * dyh)
				local base = (dh > 0.01) and math.atan2(dyh, dxh) or 0
				local sx, sy = x, y
				if dh > 0.01 then
					local k = math.min(DATA.toward_hero, dh * 0.5) / dh
					sx, sy = x + dxh * k, y + dyh * k
				end
				local ang = base + (fw.rnd() * 2 - 1) * DATA.fan
				local sp = DATA.speed_min + fw.rnd() * (DATA.speed_max - DATA.speed_min)
				local vx, vy = math.cos(ang) * sp, math.sin(ang) * sp
				local vz = DATA.lift_min + fw.rnd() * (DATA.lift_max - DATA.lift_min)
				local fuse = math.floor(DATA.fuse_min + fw.rnd() * (DATA.fuse_max - DATA.fuse_min) + 0.5)
				local mo = omni.struct("moby", m)
				omni.struct_set("moby", m, "visible", 1)
				omni.struct_set("moby", m, "rot_x", (fw.rnd() * 2 - 1) * 3.1415927)
				omni.struct_set("moby", m, "rot_y", (fw.rnd() * 2 - 1) * 3.1415927)
				omni.struct_set("moby", m, "yaw", (fw.rnd() * 2 - 1) * 3.1415927)
				-- "scale" is the model-to-metre factor the game gave this class (about 0.0043 for a bomb), NOT a plain 1.0
				-- size: shrink it by DATA.size and keep it tiny. (An earlier version treated anything under 0.01 as
				-- "unset" and wrote 0.6, a model about 140 times too big: the huge opaque red/blue shape.)
				local sc = mo.scale
				if sc ~= nil and sc == sc and sc > 0 and sc < 1.0 then
					omni.struct_set("moby", m, "scale", sc * DATA.size)
				end
				omni.struct_set("bomb", mo.pvar, "owner", owner or 0)
				-- The game's throw: position and velocity vectors in game memory. The throw position is the hero's own, so
				-- its wall test has zero length and cannot blow the mini bomb up on the spot; the blast point is set after.
				omni.with_stack(32, function(a)
					local v = a + 16
					omni.struct_set("vec4", a, "x", hero.x)
					omni.struct_set("vec4", a, "y", hero.y)
					omni.struct_set("vec4", a, "z", hero.z)
					omni.struct_set("vec4", a, "w", 1.0)
					omni.struct_set("vec4", v, "x", vx)
					omni.struct_set("vec4", v, "y", vy)
					omni.struct_set("vec4", v, "z", vz)
					omni.struct_set("vec4", v, "w", 0.0)
					launch(a, m, a + 16)
				end)
				local thrown = omni.struct("moby", m)
				if thrown == nil or thrown.state ~= 1 then
					error("the game's throw did not start the mini bomb (state " .. tostring(thrown and thrown.state) .. ")")
				end
				omni.struct_set("moby", m, "x", sx)
				omni.struct_set("moby", m, "y", sy)
				omni.struct_set("moby", m, "z", z + DATA.start_lift)
				omni.struct_set("bomb", mo.pvar, "fuse", fuse)
				omni.struct_set("moby", m, "hit_flag", 0)
				if DATA.full_colour then
					omni.struct_set("moby", m, "update_dist", 255)
					omni.struct_set("moby", m, "draw_dist", 255)
				end
				children[m] = t
				nchild += 1
				made += 1
				if made == 1 then
					local after = omni.struct("moby", m)
					local bp = omni.struct("bomb", mo.pvar)
					fw.note("minibomb/spawn", string.format("mini bomb %d state %d at %.1f,%.1f,%.1f vel %.3f,%.3f,%.3f fuse %d scale %.5f",
						m, after.state, after.x, after.y, after.z, bp.vx, bp.vy, bp.vz, bp.fuse, after.scale))
				end
			end)
			if not ok then
				local del = fn_of("delete_moby")
				if del ~= nil then pcall(del, m) end
				dead = true
				fw.log("perk: mini bombs failed (" .. string.sub(tostring(err), 1, 140) .. "), off")
				break
			end
		end
		return made
	end

	-- Hooks call S.spawn; the throws are queued and made by S.flush, which the perks call from on_tick (between two ticks).
	local queue = {}
	function S.spawn(x, y, z, owner, n)
		if dead then return 0 end
		table.insert(queue, { x = x, y = y, z = z, owner = owner, n = n })
		return n
	end

	function S.flush()
		if #queue == 0 then return end
		local q = queue
		queue = {}
		for _, e in ipairs(q) do
			local made = do_spawn(e.x, e.y, e.z, e.owner, e.n)
			fw.note("minibomb/thrown", string.format("%d of %d mini bombs thrown at %.1f,%.1f,%.1f (%d alive)", made, e.n, e.x, e.y, e.z, nchild))
		end
	end

	fw.minibomb = S
	return S
end

return M
