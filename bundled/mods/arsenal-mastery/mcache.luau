--!nonstrict
-- Arsenal Mastery: a cache of what never changes while a moby lives (its class), so the hooks that run many times a tick do not
-- read the whole moby record (omni.struct, about 10 to 15 us each) for every call.
-- An entry is dropped when the game deletes the moby (rac1.omni.moby.delete: the only code that frees a moby), on every level
-- change, and the whole cache is dropped every 600 ticks as a safety net. Other modules keep their own per-moby facts in the same
-- record (rec.owner_class ...) so they die with it.
local M = {}

local omni = omni
local recs = {} -- moby address -> { class = n, ... }
local hooked = false
local count = 0

local function clear()
	recs = {}
	count = 0
end
M.clear = clear

-- The record of a living moby (class + whatever other modules added), or nil when the address is not a moby.
function M.rec(addr)
	if addr == nil or addr == 0 then return nil end
	local r = recs[addr]
	if r ~= nil then return r end
	local ok, m = pcall(omni.struct, "moby", addr)
	if not ok or m == nil then return nil end
	r = { class = m.class }
	if count >= 4096 then clear() end -- never grows without bound
	recs[addr] = r
	count += 1
	return r
end

function M.class(addr)
	local r = M.rec(addr)
	return r and r.class or nil
end

-- Call once from a module's setup (the main chunk: hooks can only be taken then). Safe to call twice.
function M.setup()
	if hooked then return end
	hooked = true
	if omni.has("rac1.omni.moby.delete") then
		omni.hook("rac1.omni.moby.delete", function(original, m)
			if recs[m] ~= nil then
				recs[m] = nil
				count -= 1
			end
			return original()
		end)
	end
end

function M.on_frame(t)
	if t % 600 == 0 then clear() end
end

function M.on_event(name)
	if name == "level_loaded" or name == "level_unloading" or name == "save_loaded" or name == "new_game" then clear() end
end

return M
