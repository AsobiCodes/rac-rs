--!nonstrict
-- Arsenal Mastery: XP bars and V numbers on the pause menu's Weapons page, and a level-up pop-up (weapon icon + V number)
-- on the HUD. No words: icons and numbers only, so it reads the same in every language.
-- Drawn in the game's own 2D space (512 x 448).
local M = {}

-- ===================================================================================================
-- DATA: layout (game 2D pixels) and colours.
-- ===================================================================================================
local DATA = {
	page = "weapons", -- the pause_page name of the Weapons page
	-- The page's 5 x 3 icon grid, row by row: the weapon in each cell (the game's own page layout).
	grid = {
		{ "bomb_glove", "pyrocitor", "blaster", "glove_of_doom", "mine_glove" },
		{ "taunter", "suck_cannon", "devastator", "walloper", "visibomb" },
		{ "decoy", "drone_device", "tesla_claw", "morph_o_ray", "ryno" },
	},
	x0 = 44.8, y0 = 110.8, step = 42.5, -- centre of cell (1,1) and the distance between cell centres
	bar_w = 28, bar_h = 3, bar_dy = 19.5, bar_fill_w = 15, -- bar size and its offset below the cell centre
	back = 0x000000B0,
	-- Colour per level: 1..5 as in the game's colours (V5 gold), 6 Mega, 7 Giga, 8 Omega, 9+ Endless.
	colors = { 0x8FA0D8FF, 0x5CD66CFF, 0x4CC3F0FF, 0xC070F0FF, 0xFFC830FF, 0xFF5CD8FF, 0x40E8FFFF, 0xFFFFFFFF },
	endless_color = 0xFF9030FF,
	wrench = { x = 174, y = 43, w = 56, h = 4 }, -- the wrench (not in the grid): a V number and a bar in the header panel
	-- Level-up pop-up (HUD layer): icon size, position of the first one, vertical distance, seconds shown.
	popup = { x = 256, y = 64, icon = 28, dy = 34, seconds = 2.0, fade = 0.4, max = 3 },
	ticks_per_second = { pal = 50, ntsc = 60 },
}
M.DATA = DATA

local api = nil
local have = nil -- omni.fields.omni.weapon
local page = nil
local tick = 0
local tps = 50
local icon_ok = {} -- icon name -> true when the game knows it
local popups = {} -- { name, level, start }

local function color_of(level)
	return DATA.colors[level] or DATA.endless_color
end

local function with_alpha(c, a)
	return bit32.bor(bit32.band(c, 0xFFFFFF00), math.floor(math.max(0, math.min(255, a)) + 0.5))
end

-- shared with hudbar.luau (the bar under the ammo box)
M.color_of = color_of
M.with_alpha = with_alpha

function M.setup(a)
	api = a
	if not omni.has("omni.draw") then
		api.log("bars: this host cannot draw (no omni.draw), XP bars off")
		return
	end
	local ok, w = pcall(function() return omni.fields.omni.weapon end)
	if ok and w ~= nil then have = w else api.log("bars: no omni.weapon fields in the binding, XP bars off") end
	tps = DATA.ticks_per_second[a.disc] or 50
	local okf, f = pcall(function() return omni.ticks_per_second end)
	if okf and type(f) == "function" then
		local ok2, v = pcall(f)
		if ok2 and type(v) == "number" and v > 0 then tps = v end
	end
	local oki, names = pcall(omni.draw.icons)
	if oki and type(names) == "table" then
		for _, n in ipairs(names) do icon_ok[n] = true end
	end
	a.levels.on_level(function(name, level, old)
		if old == nil or level <= old or not api.setting("xp_display") then return end
		table.insert(popups, { name = name, level = level, start = tick })
		if #popups > 8 then table.remove(popups, 1) end
	end)
end

function M.on_frame(t) tick = t end

function M.on_event(name, new)
	if name == "pause_page" then page = new end
end

local function draw_popups(d)
	local P = DATA.popup
	local life = P.seconds * tps
	local fade = P.fade * tps
	local i = 1
	local row = 0
	while i <= #popups do
		local p = popups[i]
		local age = tick - p.start
		if age >= life then
			table.remove(popups, i)
		else
			if row < P.max then
				local alpha = 255
				if age < fade then alpha = 255 * age / fade elseif age > life - fade then alpha = 255 * (life - age) / fade end
				local c = color_of(p.level)
				local y = P.y + P.dy * row
				local iname = "weapon." .. p.name
				local half = P.icon / 2
				d.rect(P.x - 44, y - half - 3, 88, P.icon + 6, with_alpha(0x000000FF, alpha * 0.6), { layer = "hud" })
				if icon_ok[iname] then
					pcall(d.icon, P.x - 40, y - half, P.icon, P.icon, iname, { tint = with_alpha(0xFFFFFFFF, alpha) })
				end
				d.text(P.x + 4, y - 7, "V" .. p.level, { size = 1.0, color = with_alpha(c, alpha), shadow = with_alpha(0x000000FF, alpha), layer = "hud" })
				row += 1
			end
			i += 1
		end
	end
end

local function draw_page(d)
	local lv = api.levels
	local g = DATA.grid
	for r = 1, #g do
		local row = g[r]
		for c = 1, #row do
			local name = row[c]
			if have[name].have ~= 0 then
				local cx = DATA.x0 + DATA.step * (c - 1)
				local cy = DATA.y0 + DATA.step * (r - 1)
				local l = lv.level(name)
				local col = color_of(l)
				d.bar(cx - DATA.bar_w / 2, cy + DATA.bar_dy, DATA.bar_fill_w, DATA.bar_h, lv.progress(name), col, DATA.back, { layer = "menu" })
				d.text(cx + 16, cy + DATA.bar_dy - 3, "V" .. l, { size = 0.5, color = col, align = "right", shadow = 0x000000FF, layer = "menu" })
			end
		end
	end
	local w = DATA.wrench
	local l = lv.level("wrench")
	local col = color_of(l)
	d.text(w.x, w.y - 12, "V" .. l, { size = 0.5, color = col, shadow = 0x000000FF, layer = "menu" })
	d.bar(w.x, w.y, w.w, w.h, lv.progress("wrench"), col, DATA.back, { layer = "menu" })
end

function M.draw()
	if have == nil or not api.setting("xp_display") then return end
	local d = omni.draw
	if page == DATA.page then draw_page(d) end
	if #popups > 0 then draw_popups(d) end
end

return M
