--!nonstrict
-- Arsenal Mastery: loads the modules, calls their setup, and dispatches on_frame / events / draw to each of them.
-- A module that is missing or fails is skipped with one log line.
local omni = omni

local function log(msg)
	omni.log("[am] " .. msg)
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
local files = { "levels", "power", "nanotech", "assist", "wrench", "gold", "mega", "ammo", "perks", "bars", "hudbar" }
local modules = {}
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
			table.insert(modules, { name = name, mod = m })
		else
			log(name .. ": setup failed (" .. string.sub(tostring(err), 1, 100) .. "), skipped")
		end
	end
end

-- Calls mod[method](a, b) for every module that has it; a module whose call errors is dropped with one log line.
local function dispatch(method, a, b)
	local i = 1
	while i <= #modules do
		local e = modules[i]
		local f = e.mod[method]
		local dropped = false
		if f ~= nil then
			local ok, err = pcall(f, a, b)
			if not ok then
				log(e.name .. "." .. method .. " failed (" .. string.sub(tostring(err), 1, 100) .. "), module dropped")
				table.remove(modules, i)
				dropped = true
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
				log(e.name .. ".on_event(" .. name .. ") failed (" .. string.sub(tostring(err), 1, 100) .. "), module dropped")
				table.remove(modules, i)
				dropped = true
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
end)

local names = {}
for _, e in ipairs(modules) do table.insert(names, e.name) end
log("ready: " .. table.concat(names, " "))
