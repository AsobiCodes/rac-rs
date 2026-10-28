--!nonstrict
-- Arsenal Mastery: loads the modules, calls their setup, and dispatches on_frame / events / draw to each of them.
-- A module that is missing or fails is skipped with one log line.
local omni = omni

-- Log lines: written at once while the mod loads; once it runs they are queued and written one per tick (two when many wait).
-- One log call costs about a millisecond of script time (the host writes and flushes a file), and the loader stops a mod that spends
-- more than its budget per tick on half of the ticks: a burst of lines (hits, level-ups, perks) must not cost a whole tick each.
local running = false
local queue, qhead, qtail = {}, 1, 0
local QUEUE_MAX = 400
local dropped_lines = 0

local function log(msg)
	if not running then
		omni.log("[am] " .. msg)
	elseif qtail - qhead < QUEUE_MAX then
		qtail += 1
		queue[qtail] = msg
	else
		dropped_lines += 1
	end
end

local function flush_log()
	if qhead > qtail then return end
	local n = (qtail - qhead >= 20) and 2 or 1
	for _ = 1, n do
		if qhead > qtail then break end
		local msg = queue[qhead]
		queue[qhead] = nil
		qhead += 1
		omni.log("[am] " .. msg)
	end
	if qhead > qtail then qhead, qtail = 1, 0 end
end

-- Without the omni extension (another loader or port) the mod does nothing.
local has_omni = omni ~= nil and type(omni.has) == "function" and omni.has("omni.save") and omni.has("omni.events")
if not has_omni then
	local api = omni or openrac -- compat: a loader of the same format names its table `openrac`
	if api ~= nil then
		api.log("[am] this port has no omni extension (events + per-save storage): Arsenal Mastery does nothing here")
	end
	return
end

local disc = "pal"
do
	local ok, v = pcall(function() return omni.version end)
	if ok and type(v) == "string" then disc = string.lower(v) end
end

local levels = require("levels")
local api = {
	levels = levels,
	setting = function(key) return omni.setting(key) end,
	log = log,
	disc = disc,
}

-- Order matters: levels first (events reach it before the others), then the effects, then the bars.
-- gold, mega, ammo and perks belong to other stages: when one is missing it is skipped with one log line.
local mcache_ok, mcache = pcall(require, "mcache")
local files = { "levels", "power", "nanotech", "assist", "wrench", "gold", "mega", "ammo", "perks", "bars", "hudbar" }
local modules = {}
if mcache_ok and type(mcache) == "table" then
	mcache.setup()
	table.insert(modules, { name = "mcache", mod = mcache, hidden = true, fl = {} })
end
for _, name in ipairs(files) do
	local m = nil
	if name == "levels" then
		m = levels
	else
		local ok, res = pcall(require, name)
		if ok then
			m = res
		else
			log(name .. ": not loaded (" .. string.sub(tostring(res), 1, 100) .. ")")
		end
	end
	if m ~= nil then
		local ok, err = pcall(m.setup, api)
		if ok then
			table.insert(modules, { name = name, mod = m, fl = {} })
		else
			log(name .. ": setup failed (" .. string.sub(tostring(err), 1, 100) .. "), skipped")
		end
	end
end

-- A call that errors is logged (the first three times) and the module goes on; a module whose same call errors MAX_FAILS times in a
-- row is dropped with one log line. (A single transient error used to drop the module for good.)
local MAX_FAILS = 8

local function failed_call(i, e, what, err)
	local n = (e.fl[what] or 0) + 1
	e.fl[what] = n
	if n <= 3 then log(e.name .. "." .. what .. " failed (" .. string.sub(tostring(err), 1, 100) .. ")") end
	if n >= MAX_FAILS then
		log(e.name .. "." .. what .. " failed " .. n .. " times in a row, module dropped")
		table.remove(modules, i)
		return true
	end
	return false
end

-- Calls mod[method](a, b) for every module that has it.
local function dispatch(method, a, b)
	local i = 1
	while i <= #modules do
		local e = modules[i]
		local f = e.mod[method]
		local dropped = false
		if f ~= nil then
			local ok, err = pcall(f, a, b)
			if not ok then
				dropped = failed_call(i, e, method, err)
			elseif e.fl[method] ~= nil and e.fl[method] ~= 0 then
				e.fl[method] = 0
			end
		end
		if not dropped then i += 1 end
	end
end

local function dispatch_event(name, a, b)
	local i = 1
	while i <= #modules do
		local e = modules[i]
		local f = e.mod.on_event
		local dropped = false
		if f ~= nil then
			local ok, err = pcall(f, name, a, b)
			if not ok then
				dropped = failed_call(i, e, "on_event(" .. name .. ")", err)
			elseif e.fl["on_event(" .. name .. ")"] ~= nil then
				e.fl["on_event(" .. name .. ")"] = 0
			end
		end
		if not dropped then i += 1 end
	end
end

for _, name in ipairs({ "level_loaded", "level_unloading", "mode", "pause_page", "new_game", "save_loaded", "saving", "setting" }) do
	omni.on(name, function(a, b) dispatch_event(name, a, b) end)
end

local tick = 0
omni.on_frame(function()
	tick += 1
	dispatch("on_frame", tick)
	dispatch("draw")
	flush_log()
end)

local names = {}
for _, e in ipairs(modules) do
	if not e.hidden then table.insert(names, e.name) end
end
log("ready: " .. table.concat(names, " "))
running = true
