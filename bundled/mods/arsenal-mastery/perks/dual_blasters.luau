--!nonstrict
-- Blaster V6 Mega perk: Dual Blasters. Ratchet holds a second Blaster in his left hand (mirrored), his left arm copies
-- the right arm's hold, and every Blaster shot also fires one from the left gun.
--
-- THE FOUR PARTS
--   Arm   (picture only): pose.mirror(hero, "right_arm", "left_arm") while the Blaster is held; cleared with a blend when
--         it is put away, the perk turns off or V6 is lost. The pose store forgets it on level change, death, cutscene,
--         new game and load, so it is applied again after those events (and every DATA.arm_reapply ticks as a safety net).
--   Look  (picture only): on a port with pose.follow (omni.has("omni.pose.follow")) the renderer draws the left gun, EVERY displayed
--         frame, exactly where the right gun sits relative to the right hand, mirrored onto the left hand
--         (pose.follow(clone, right gun, "right_hand", "left_hand", { mirror = true })): in the left hand whatever the arm pose, the
--         strafe twist or the smoothing do. The game state below (placement, shots) is NOT changed by it; a port without it draws
--         the clone where the game state puts it, as before. The host forgets the record on level change, death, cutscene, new game
--         and load, so it is renewed after those events and every DATA.follow_reapply ticks.
--   Gun   (game state): a real second Blaster moby (class of the held one), made once while the Blaster is held and
--         deleted on put-away / perk off. Every tick, from the hook that runs right after the game glued the right gun to
--         the hand (hero.items_attach), it is set to the right gun's position and basis rows reflected across Ratchet's
--         body plane (the plane through his origin whose normal is his own left axis; nothing else enters). The game never updates it (flag "not scheduled", "no transform" kept by us), it is not in the target
--         registry and not hittable (those two flag bits are cleared), so it cannot be shot, hit, or counted as an enemy.
--   Shot  (game state): the fire hook runs the game's creator for the right gun as usual and queues the left shot; on_tick
--         makes it with the game's creator (omni.fn through the same hook id, guarded so it does not queue again), from
--         the right muzzle reflected across the same plane as the gun of that tick, same yaw, pitch and target. The creator consumes no ammo (the
--         held update spends it once before the call), so one trigger pull costs one ammo. The left shot is a normal class
--         305 shot, so power.luau credits Blaster XP and level power exactly as for the right one. It spawns one tick
--         later than the right shot on purpose: two hits on one enemy in the SAME tick count as one in the game's hit
--         queue, one tick apart both count.
--
-- DETERMINISM (game state must not depend on render timing, so replays and saves stay exact)
--   Everything that enters game state (gun placement, left muzzle) uses only: the held Blaster moby (= the stored right-hand
--   attach matrix, exact and current) and the hero moby (position and left row, current tick). The plane is a point on the hero
--   origin (plus the fixed DATA.plane_offset along the normal) and the hero's left row as its ONE normal: it never switches.
--   No joint of any pose (stored or rendered) places the gun or the shot. The looks-only diagnostic (DATA.diag) reads joints
--   of the rendered pose and only writes log lines.
local M = {}

local DATA = {
	held_class = 168, -- class of the held Blaster; the clone is made of the same class
	arm_blend = 0.15, -- seconds the left arm takes to fade in and out
	arm_reapply = 250, -- ticks between safety re-applications of the arm pose
	follow_reapply = 25, -- ticks between safety renewals of the follow pose (the host forgets it on death)
	mirror = "flag", -- "flag": the clone is drawn mirrored (mirror flag + left-handed rows). "axis": one axis negated, a plain rotation
	plane_offset = 0.0, -- metres: the mirror plane is moved this far along Ratchet's left axis (0 = through his origin, which lies on his body's centre line)
	max_pending = 4, -- left shots that may wait at once
	max_age = 8, -- ticks a queued left shot waits before it is dropped
	follow = true, -- put the left muzzle where the right gun's muzzle is one tick after the pull, reflected across that tick's plane
	create_retry = 25, -- ticks between tries when the moby pool is full
	max_errors = 5, -- script errors before the perk gives up
	diag = true, -- log, a few times, how far the mirrored grip is from the real left hand (rendered pose, looks only, never used for placement)
	trace = false, -- test rig: log every shot and the clone state
}

-- Moby flag bits (see the binding's flags_w).
local FLAG_HIDDEN = 1
local FLAG_NOT_SCHEDULED = 2
local FLAG_NO_TRANSFORM = 4
local FLAG_REGISTRY = 0x1000
local FLAG_HITTABLE = 0x4000
local FLAG_MIRROR = 0x8000

function M.setup(fw)
	local omni = omni
	local pose = omni.pose
	local have_pose = omni.has("omni.pose")
	local have_read = have_pose and omni.has("omni.pose.read")
	local have_follow = have_pose and omni.has("omni.pose.follow") and DATA.mirror ~= "axis" -- the draw needs the clone's mirror flag
	local KEY = "blaster/dual"

	local clone = 0 -- the second Blaster moby (0 = none)
	local next_create = 0
	local create_fn, delete_fn, fire_fn = nil, nil, nil
	local arm_on, arm_dirty, arm_t = false, true, -1000
	local follow_item, follow_src, follow_t = 0, 0, -1000 -- the clone and the right gun the follow pose was last asked for
	local pending = {} -- queued left shots
	local in_second = false -- true while the left shot is made, so the fire hook does not queue it again
	local errors, dead = 0, false
	local stat = { right = 0, left = 0, queued = 0, dropped = 0 }
	local trace_lines = 0

	local function trace(text)
		if DATA.trace and trace_lines < 600 then
			trace_lines += 1
			fw.log("dual: " .. text)
		end
	end

	local function fn_of(name)
		local ok, f = pcall(omni.fn, name)
		if ok then return f end
		return nil
	end

	local function S(key, value) omni.struct_set("moby", clone, key, value) end

	-- Two moby fields the game's draw pass needs on the clone (proven by the G3e2 runs: without them it is never drawn): the draw
	-- sphere (sphere_x/y/z in 1/1024 m, sphere_r), the held gun's moved by the offset between the guns, and the low byte of the
	-- held gun's draw_dist (0x20 on the Blaster).
	local sync_failed = false
	local function sphere_sync(h, dx, dy, dz)
		if sync_failed then return end
		local ok, err = pcall(function()
			local k = 1024
			local g, c = omni.struct("moby", h), omni.struct("moby", clone)
			S("sphere_x", g.sphere_x + dx * k)
			S("sphere_y", g.sphere_y + dy * k)
			S("sphere_z", g.sphere_z + dz * k)
			S("sphere_r", g.sphere_r)
			local mine, theirs = c.draw_dist % 65536, g.draw_dist % 65536
			local want = bit32.bor(bit32.band(mine, 0xff00), bit32.band(theirs, 0x00ff))
			if want ~= mine then S("draw_dist", want >= 32768 and want - 65536 or want) end
		end)
		if not ok then
			sync_failed = true
			fw.log("perk: blaster dual sphere / draw distance write failed, the left gun will not be drawn (" .. string.sub(tostring(err), 1, 120) .. ")")
		end
	end

	-- The Blaster Ratchet holds now: address and record, or 0.
	local function held()
		local ok, h = pcall(function() return omni.fields.omni.hero.held_moby end)
		if not ok or h == nil or h == 0 then return 0, nil end
		local m = omni.struct("moby", h)
		if m == nil or m.class ~= DATA.held_class then return 0, nil end
		return h, m
	end

	-- The mirror plane: point (cx, cy, cz) and unit normal (nx, ny, nz). Hero moby only (see DETERMINISM): the normal is always his
	-- left axis (row 1), the point his origin moved DATA.plane_offset along it.
	local function plane()
		local hero = fw.hero()
		local hm = hero ~= 0 and omni.struct("moby", hero) or nil
		if hm == nil then return nil end
		local nx, ny, nz = hm.row1_x, hm.row1_y, hm.row1_z
		local len = math.sqrt(nx * nx + ny * ny + nz * nz)
		if not (len > 0.5) then return nil end
		nx, ny, nz = nx / len, ny / len, nz / len
		local o = DATA.plane_offset
		return { nx = nx, ny = ny, nz = nz, cx = hm.x + nx * o, cy = hm.y + ny * o, cz = hm.z + nz * o }
	end

	local function reflect_point(P, x, y, z)
		local d = (x - P.cx) * P.nx + (y - P.cy) * P.ny + (z - P.cz) * P.nz
		return x - 2 * d * P.nx, y - 2 * d * P.ny, z - 2 * d * P.nz
	end

	local function reflect_dir(P, x, y, z)
		local d = x * P.nx + y * P.ny + z * P.nz
		return x - 2 * d * P.nx, y - 2 * d * P.ny, z - 2 * d * P.nz
	end

	-- ------------------------------------------------------------------------------------------ the gun
	-- A valid clone: still a Blaster-class moby that nobody but us made "not scheduled", and not the held gun itself.
	local function clone_record()
		if clone == 0 then return nil end
		local c = omni.struct("moby", clone)
		if c == nil or c.class ~= DATA.held_class or bit32.band(c.flags, FLAG_NOT_SCHEDULED) == 0 then return nil end
		return c
	end

	-- Looks-only diagnostic: how far the mirrored grip is from the rendered left hand (rendered pose, a frame behind: never used for placement).
	local diag_n = 0
	local function diagnose(P, x, y, z, hm)
		local every, most = 50, 6
		if DATA.trace then every, most = 10, 60 end
		if not (DATA.diag and have_read) or diag_n >= most or fw.tick() % every ~= 0 then return end
		local hero = fw.hero()
		local ok, lx, ly, lz = pcall(pose.joint_position, hero, "left_hand")
		local ok2, rx, ry, rz = pcall(pose.joint_position, hero, "right_hand")
		if ok and ok2 and type(lx) == "number" and type(rx) == "number" then
			diag_n += 1
			fw.log(string.format("perk: blaster dual_diag t%d: mirrored grip %.3f m from the rendered left hand (%.2f,%.2f,%.2f), right gun %.3f m from the right hand",
				fw.tick(), math.sqrt((x - lx) ^ 2 + (y - ly) ^ 2 + (z - lz) ^ 2), lx, ly, lz,
				math.sqrt((hm.x - rx) ^ 2 + (hm.y - ry) ^ 2 + (hm.z - rz) ^ 2)))
		end
	end

	-- Runs right after the game glued the right gun to the hand (hook hero.items_attach): put the clone on the mirrored grip.
	local function place()
		if clone == 0 then return end
		local c = clone_record()
		if c == nil then
			fw.note(KEY .. "_gone", "clone moby " .. clone .. " is gone, will be made again")
			clone = 0
			return
		end
		local h, hm = held()
		local P = (h ~= 0 and h ~= clone) and plane() or nil
		if P == nil then
			if c.visible ~= 0 then S("visible", 0) end
			return
		end
		local x, y, z = reflect_point(P, hm.x, hm.y, hm.z)
		local ax, ay, az = reflect_dir(P, hm.row0_x, hm.row0_y, hm.row0_z)
		local bx, by, bz = reflect_dir(P, hm.row1_x, hm.row1_y, hm.row1_z)
		local cx, cy, cz = reflect_dir(P, hm.row2_x, hm.row2_y, hm.row2_z)
		local mirrored = DATA.mirror ~= "axis"
		if not mirrored then bx, by, bz = -bx, -by, -bz end -- reflected rows have determinant -1: negating one makes a plain rotation
		S("x", x) S("y", y) S("z", z)
		S("row0_x", ax) S("row0_y", ay) S("row0_z", az)
		S("row1_x", bx) S("row1_y", by) S("row1_z", bz)
		S("row2_x", cx) S("row2_y", cy) S("row2_z", cz)
		if c.visible ~= hm.visible then S("visible", hm.visible) end
		if c.scale ~= hm.scale then S("scale", hm.scale) end
		if c.light0 ~= hm.light0 then S("light0", hm.light0) end
		if c.light1 ~= hm.light1 then S("light1", hm.light1) end
		if c.clock0 ~= hm.clock0 then S("clock0", hm.clock0) end
		if c.clock1 ~= hm.clock1 then S("clock1", hm.clock1) end
		if c.clock2 ~= hm.clock2 then S("clock2", hm.clock2) end
		if c.clock3 ~= hm.clock3 then S("clock3", hm.clock3) end
		local keep = bit32.band(c.flags, bit32.bnot(FLAG_HIDDEN + FLAG_REGISTRY + FLAG_HITTABLE + FLAG_MIRROR))
		local want = bit32.bor(keep, FLAG_NOT_SCHEDULED, FLAG_NO_TRANSFORM, mirrored and FLAG_MIRROR or 0, bit32.band(hm.flags, FLAG_HIDDEN))
		if c.flags ~= want then S("flags_w", want) end
		sphere_sync(h, x - hm.x, y - hm.y, z - hm.z)
		if fw.tick() % 25 == 0 then trace(string.format("t%d clone %d at %.2f,%.2f,%.2f (right gun %.2f,%.2f,%.2f) flags %x visible %s plane at %.2f,%.2f,%.2f normal %.3f,%.3f,%.3f", fw.tick(), clone, x, y, z, hm.x, hm.y, hm.z, want, tostring(hm.visible), P.cx, P.cy, P.cz, P.nx, P.ny, P.nz)) end
		diagnose(P, x, y, z, hm)
	end

	local function make_clone(t)
		if create_fn == nil then create_fn = fn_of("create_moby") end
		if create_fn == nil then
			dead = true
			fw.log("perk: blaster dual: the game's create function is not available here, off")
			return
		end
		local c = create_fn(DATA.held_class)
		if c == nil or c == 0 then
			next_create = t + DATA.create_retry
			fw.note(KEY .. "_pool", "moby pool full, will try again")
			return
		end
		clone = c
		local m = omni.struct("moby", c)
		local flags = bit32.bor(bit32.band(m.flags, bit32.bnot(FLAG_HIDDEN + FLAG_REGISTRY + FLAG_HITTABLE)), FLAG_NOT_SCHEDULED, FLAG_NO_TRANSFORM, DATA.mirror ~= "axis" and FLAG_MIRROR or 0)
		S("flags_w", flags)
		S("update", 0)
		S("visible", 0)
		local back = omni.struct("moby", c)
		fw.note(KEY, string.format("on: left gun is moby %d (class %d, flags %x, update %d); mirror mode %s; arm pose %s", c, back.class, back.flags, back.update,
			DATA.mirror, have_pose and "on" or "not supported on this port"))
	end

	-- Picture only: draw the clone in the left hand where the right gun is in the right hand (see "Look" above). Idempotent: asking again
	-- for the same item replaces the host's record.
	local function follow_apply(t, h)
		if not have_follow or clone == 0 or h == 0 then return end
		if clone == follow_item and h == follow_src and t - follow_t < DATA.follow_reapply then return end
		follow_item, follow_src, follow_t = clone, h, t
		pcall(pose.follow, clone, h, "right_hand", "left_hand", { mirror = true })
	end

	local function follow_clear()
		if follow_item ~= 0 then pcall(pose.unfollow, follow_item) end
		follow_item, follow_src, follow_t = 0, 0, -1000
	end

	local function delete_clone(why)
		follow_clear()
		if clone == 0 then return end
		local c = clone_record()
		local h = held()
		local was = clone
		if c ~= nil and clone ~= h then
			if delete_fn == nil then delete_fn = fn_of("delete_moby") end
			if delete_fn ~= nil then pcall(delete_fn, clone) end -- the moby.delete hook below clears `clone` as well
		end
		fw.note(KEY .. "_off", "left gun " .. was .. " removed (" .. why .. ")")
		clone = 0
	end

	-- ------------------------------------------------------------------------------------------ the arm
	local function arm_apply(t, hero)
		local ok, n = pcall(pose.mirror, hero, "right_arm", "left_arm", { blend = DATA.arm_blend })
		arm_t, arm_dirty = t, false
		if ok then
			if not arm_on then fw.note(KEY .. "_arm", "left arm mirrors the right arm (" .. tostring(n) .. " joints)") end
			arm_on = true
		else
			arm_on = false
			fw.log("perk: blaster dual arm pose failed (" .. string.sub(tostring(n), 1, 120) .. ")")
		end
	end

	local function arm_clear(why)
		if not arm_on then return end
		arm_on = false
		local hero = fw.hero()
		if hero ~= 0 and have_pose then pcall(pose.clear, hero, "left_arm", { blend = DATA.arm_blend }) end
		fw.note(KEY .. "_arm_off", "left arm released (" .. why .. ")")
	end

	-- ------------------------------------------------------------------------------------------ the second shot
	local function flush(t)
		if #pending == 0 then return end
		local q = pending
		pending = {}
		local h, hm = held()
		if h == 0 or clone == 0 then
			stat.dropped += #q
			return
		end
		if fire_fn == nil then fire_fn = fn_of("rac1.omni.weapon.blaster.fire") end
		if fire_fn == nil then
			dead = true
			fw.log("perk: blaster dual: the game's Blaster shot function is not available here, off")
			return
		end
		for _, e in ipairs(q) do
			if t - e.t > DATA.max_age then
				stat.dropped += 1
			else
				-- The left muzzle = the right muzzle (moved with the right gun since the pull) reflected across the plane of THIS tick,
				-- the same reflection that places the clone now.
				local x, y, z = e.x, e.y, e.z
				if DATA.follow then
					local P = plane()
					if P ~= nil then
						x, y, z = reflect_point(P, e.mx + (hm.x - e.hx), e.my + (hm.y - e.hy), e.mz + (hm.z - e.hz))
					end
				end
				local tgt = e.target
				if tgt ~= 0 and not fw.alive(tgt) then tgt = 0 end
				local shot = 0
				omni.with_stack(16, function(a)
					omni.struct_set("vec4", a, "x", x)
					omni.struct_set("vec4", a, "y", y)
					omni.struct_set("vec4", a, "z", z)
					omni.struct_set("vec4", a, "w", e.w)
					in_second = true
					local ok, r = pcall(fire_fn, e.f0, e.f1, e.f2, e.f3, e.owner, a, tgt)
					in_second = false
					if ok and type(r) == "number" then shot = r end
				end)
				if shot ~= 0 then
					stat.left += 1
					fw.note(KEY .. "_shot", string.format("left shot moby %d from %.2f,%.2f,%.2f (right shot %d, %d tick(s) earlier), target %d", shot, x, y, z, e.right, t - e.t, tgt))
					trace(string.format("t%d left shot %d from %.2f,%.2f,%.2f queued t%d right %d target %d (left shots so far %d)", t, shot, x, y, z, e.t, e.right, tgt, stat.left))
				else
					stat.dropped += 1
					trace(string.format("t%d left shot not made (creator returned 0: wall or pool)", t))
				end
			end
		end
	end

	-- The game fires the right gun; queue the left one.
	local function on_fire(nxt, f0, f1, f2, f3, owner, vec, target)
		local r = nxt(f0, f1, f2, f3, owner, vec, target)
		if in_second or dead or clone == 0 or r == nil or r == 0 or #pending >= DATA.max_pending then return r end
		local ok, err = pcall(function()
			local h, hm = held()
			local P = h ~= 0 and plane() or nil
			if P == nil then return end
			local v = omni.struct("vec4", vec)
			local mx, my, mz, mw = v.x, v.y, v.z, v.w
			local x, y, z = reflect_point(P, mx, my, mz)
			table.insert(pending, { f0 = f0, f1 = f1, f2 = f2, f3 = f3, owner = owner, target = target, x = x, y = y, z = z, mx = mx, my = my, mz = mz, w = mw,
				hx = hm.x, hy = hm.y, hz = hm.z, t = fw.tick(), right = r })
			stat.right += 1
			stat.queued += 1
			trace(string.format("t%d right shot %d, muzzle %.2f,%.2f,%.2f -> left muzzle %.2f,%.2f,%.2f (plane at %.2f,%.2f,%.2f)", fw.tick(), r, mx, my, mz, x, y, z, P.cx, P.cy, P.cz))
		end)
		if not ok then
			errors += 1
			fw.log("perk: blaster dual fire hook error (" .. string.sub(tostring(err), 1, 120) .. ")")
			if errors >= DATA.max_errors then dead = true end
		end
		return r
	end

	-- ------------------------------------------------------------------------------------------ per tick
	local function work(t)
		local h = held()
		local hero = fw.hero()
		if h ~= 0 and clone == 0 and t >= next_create then make_clone(t) end
		follow_apply(t, h)
		if h == 0 and clone ~= 0 then delete_clone("Blaster put away") end
		if have_pose and hero ~= 0 then
			if h ~= 0 then
				if not arm_on or arm_dirty or t - arm_t >= DATA.arm_reapply then arm_apply(t, hero) end
			elseif arm_on then
				arm_clear("Blaster put away")
			end
		end
		if h == 0 then pending = {} else flush(t) end
		if DATA.trace and t % 50 == 0 then
			trace(string.format("t%d held %d clone %d arm %s right %d left %d dropped %d", t, h, clone, tostring(arm_on), stat.right, stat.left, stat.dropped))
		end
	end

	fw.perk{
		weapon = "blaster", slot = 2, name = "dual_blasters", needs = { "moby.sphere_r", "moby.draw_dist", "vec4.w" },
		hooks = {
			["hero.items_attach"] = function(nxt)
				nxt()
				if not dead and clone ~= 0 then
					local ok, err = pcall(place)
					if not ok then
						errors += 1
						fw.log("perk: blaster dual placement error (" .. string.sub(tostring(err), 1, 120) .. ")")
						if errors >= DATA.max_errors then dead = true end
					end
				end
			end,
			["weapon.blaster.fire"] = on_fire,
			["moby.delete"] = function(nxt, m)
				if m == clone then clone = 0 follow_item, follow_src = 0, 0 end
				return nxt(m)
			end,
		},
		on_tick = function(t)
			if dead then return end
			local ok, err = pcall(work, t)
			if not ok then
				errors += 1
				fw.log("perk: blaster dual tick error (" .. string.sub(tostring(err), 1, 140) .. ")")
				if errors >= DATA.max_errors then
					dead = true
					fw.log("perk: blaster dual gives up after " .. errors .. " errors")
					pcall(delete_clone, "errors")
					pcall(arm_clear, "errors")
				end
			end
		end,
		-- The perk turned off (setting, V6 lost) or failed: leave no gun and no pose behind.
		on_disable = function()
			pending = {}
			delete_clone("perk off")
			arm_clear("perk off")
			next_create = 0
		end,
		-- The pose store and the moby pool are reset by these; start over.
		on_event = function(name)
			if name == "level_unloading" or name == "level_loaded" or name == "new_game" or name == "save_loaded" then
				clone, pending, arm_on, arm_dirty = 0, {}, false, true
				follow_item, follow_src, follow_t = 0, 0, -1000
			elseif name == "mode" then
				arm_dirty = true
			end
		end,
	}
end

return M
