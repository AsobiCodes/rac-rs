--!nonstrict
-- Arsenal Mastery: the wrench as a levelled weapon. The wrench's XP and power go through power.luau's hit attribution;
-- this module tells it when a swing is running (every hit inside the wrench's own update is a wrench hit).
-- Hyper-Strike and throws: the thrown wrench is a moby of its own class, attributed by power.luau's class table when known.
local M = {}

local api = nil
local in_wrench = false
local last_throw_tick = -1000
local tick = 0
local swings = 0

-- True while the held wrench's own update runs.
function M.active() return in_wrench end
-- Ticks since the last wrench throw (a big number when none yet).
function M.since_throw() return tick - last_throw_tick end

function M.setup(a)
	api = a
	if omni.has("rac1.omni.wrench.update") then
		omni.hook("rac1.omni.wrench.update", function(original)
			in_wrench = true
			local ok, r = pcall(original)
			in_wrench = false
			if not ok then error(r) end
			return r
		end)
	else
		api.log("wrench: no wrench update hook here: swings are credited to the held weapon")
	end
	if omni.has("rac1.omni.wrench.throw") then
		omni.before("rac1.omni.wrench.throw", function()
			last_throw_tick = tick
			swings += 1
			if swings <= 3 then api.log("wrench: throw at tick " .. tick) end
		end)
	end
end

function M.on_frame(t) tick = t end

return M
