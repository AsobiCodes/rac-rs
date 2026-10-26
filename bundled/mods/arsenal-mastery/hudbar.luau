--!nonstrict
-- Arsenal Mastery: "V<n>" and an XP bar for the HELD weapon, drawn under the HUD's ammo box (top left, weapon icon + "40/350").
-- It follows the box: same left edge, same fade-in / fade-out (the game fades the box, it does not slide it), nothing when the
-- box is hidden. Switched by the same setting as the Weapons-page bars (xp_display). Look = bars.luau (colours, font).
-- Drawn in the game's own 2D space (512 x 448), which the host maps like the game's HUD at any window size and aspect.
local M = {}

-- ===================================================================================================
-- DATA: layout (game 2D pixels) and the box's fade, copied from how the game draws its ammo box.
-- ===================================================================================================
local DATA = {
	-- The ammo box: left edge, top (PAL / NTSC), height of its icon row, width of its bar art (small / big weapons).
	box_left = 12,
	box_top = { pal = 10, ntsc = 18 },
	box_h = 32,
	box_bw = 70, box_bw_big = 95, big_from = 100, -- a weapon with a maximum of 100 or more has the longer box
	-- This module's bar: gap below the box, bar height, where the bar starts after the "V<n>" text, its length = box bw + extra.
	gap = 1, bar_h = 4, bar_dx = 24, bar_extra = 20,
	text_size = 0.6, text_dy = -3,
	back = 0x1C2438C8, -- a little lighter than the page's black: the HUD sits on a dark picture
	-- The box fades in/out in two steps per tick: icon + bar (f1) 0..8, then the text (f2) 0..8. We follow f1.
	fade_max = 8,
	-- Weapons that have an ammo box (the others, wrench, Suck Cannon, Taunter, Walloper, Morph-o-Ray, have none): item id -> name.
	items = {
		[10] = "bomb_glove", [11] = "devastator", [13] = "visibomb", [15] = "blaster", [16] = "pyrocitor",
		[17] = "mine_glove", [19] = "tesla_claw", [20] = "glove_of_doom", [23] = "ryno", [24] = "drone_device", [25] = "decoy",
	},
	-- Modes with the box up: gameplay, message boxes ("dialog") and the Gadgetron vendor screen ("vendor": the game keeps the box of
	-- the held weapon in its top left corner there, seen in pictures of the vendor screen), so the bar follows it in those modes
	-- (the box is gone in menus, ship, cutscenes: nothing is drawn there).
	show_modes = { gameplay = true, dialog = true, vendor = true },
	keep_modes = { gameplay = true, dialog = true, vendor = true, pause = true }, -- the fade state survives these; any other mode resets it
}
M.DATA = DATA

local api = nil
local bars = nil -- bars.luau (colours)
local have = nil -- omni.fields.omni.weapon
local mode = "unknown"
local cur = nil -- the weapon the box shows (name)
local f1, f2 = 0, 0

function M.setup(a)
	api = a
	if not omni.has("omni.draw") then return end
	local okb, b = pcall(require, "bars")
	if not okb or type(b) ~= "table" or b.color_of == nil then
		api.log("hudbar: bars.luau not available, HUD bar off")
		return
	end
	bars = b
	local ok, w = pcall(function() return omni.fields.omni.weapon end)
	if ok and w ~= nil then have = w end
	local okm, m = pcall(omni.mode)
	if okm and type(m) == "string" then mode = m end
end

-- The weapon whose box the game shows now, or nil. (Vehicle modes, where the game replaces the box, hold an item that is not in
-- DATA.items, so they give nil here without a special cell.)
local function wanted()
	if not DATA.show_modes[mode] then return nil end
	local ok, id = pcall(function() return have.equipped end)
	if not ok or id == nil then return nil end
	return DATA.items[id]
end

-- One game tick of the box's fade: shown = f1 up to 8 then f2 up to 8; hidden = f2 down to 0 then f1 down to 0;
-- a switch of weapon always passes through hidden (old box fades out, then the new one fades in).
function M.on_frame()
	if have == nil or bars == nil then return end
	if not DATA.keep_modes[mode] then
		cur, f1, f2 = nil, 0, 0
		return
	end
	if not DATA.show_modes[mode] then return end -- paused: the state waits
	local want = wanted()
	if cur ~= want then
		if f2 > 0 then f2 -= 1
		elseif f1 > 0 then f1 -= 1
		else cur = want end
	elseif cur ~= nil then
		if f1 < DATA.fade_max then f1 += 1
		elseif f2 < DATA.fade_max then f2 += 1 end
	end
end

function M.on_event(name, new)
	if name == "mode" and type(new) == "string" then mode = new end
end

function M.draw()
	if have == nil or bars == nil or cur == nil or f1 <= 0 or not DATA.show_modes[mode] then return end
	if not api.setting("xp_display") then return end
	local lv = api.levels
	local d = omni.draw
	local ok, max = pcall(function() return have[cur].max_ammo end)
	local big = ok and type(max) == "number" and max >= DATA.big_from
	local bw = big and DATA.box_bw_big or DATA.box_bw
	local top = DATA.box_top[api.disc] or DATA.box_top.pal
	local y = top + DATA.box_h + DATA.gap
	local alpha = 255 * f1 / DATA.fade_max
	local l = lv.level(cur)
	local col = bars.with_alpha(bars.color_of(l), alpha)
	local shadow = bars.with_alpha(0x000000FF, alpha)
	d.text(DATA.box_left, y + DATA.text_dy, "V" .. l, { size = DATA.text_size, color = col, shadow = shadow, layer = "hud" })
	d.bar(DATA.box_left + DATA.bar_dx, y, bw + DATA.bar_extra, DATA.bar_h, lv.progress(cur), col, bars.with_alpha(DATA.back, (DATA.back % 256) * f1 / DATA.fade_max), { layer = "hud" })
end

return M
