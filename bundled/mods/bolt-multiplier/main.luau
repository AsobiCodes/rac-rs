--!nonstrict
-- Bolt Multiplier -- Fun experimental RAC-RS / OmniLoader mod
-- Hook ids and fields are from the public RAC-RS MODDING_REFERENCE.md

if not (omni.has and omni.has("omni.draw") and omni.has("omni.events")
        and omni.has("omni.is_enemy") and omni.has("omni.mobys")) then
    omni.warn("bolt-multiplier: OmniLoader enemy tracking, drawing, or events unavailable")
    return
end

local ticks = 0
local multiplier = 1
local streak = 0
local last_bolts = nil
local previous_health = nil
local known_enemies = {}   -- address -> last tick observed ALIVE
local killed = {}          -- address -> last tick credited
local flash_until = 0
local initialized = false
local hud_visible_until = 0

local function debug_log(s)
    if false then omni.log("bolt-multiplier: " .. s) end
end

local function reset(reason)
    multiplier = 1
    streak = 0
    flash_until = 0
    hud_visible_until = 0
    debug_log("reset: " .. reason)
end

local function clear_level()
    reset("level/save transition")
    known_enemies = {}
    killed = {}
    last_bolts = nil
    previous_health = nil
    initialized = false
end

omni.on("level_loaded", function() clear_level() end)
omni.on("level_unloading", function() clear_level() end)
omni.on("new_game", function() clear_level() end)
omni.on("save_loaded", function() clear_level() end)

-- Called at the point the game records a moby's death. By the time this runs,
-- omni.is_enemy may be false because the enemy no longer has positive health.
-- We therefore recognize either a still-live enemy OR a recently seen enemy.
omni.before("rac1.moby.record_death", function(moby, flags, index)
    if type(moby) ~= "number" or moby == 0 then return end
    if not initialized then return end
    if omni.level() < 0 then return end
    local last = known_enemies[moby]
    local recognized = omni.is_enemy(moby) or (last ~= nil and ticks - last <= 120)
    if not recognized then
        debug_log("ignored unrecognized death: moby=" .. tostring(moby))
        return
    end
    -- Multiple death notifications can occur for one moby. Never count twice
    -- while the moby is dead; stale addresses are removed after a time window.
    if killed[moby] ~= nil and ticks - killed[moby] <= 120 then return end
    killed[moby] = ticks
    known_enemies[moby] = nil

    -- Linear kill-step progression: 5 kills -> x2, 10 -> x3, etc.
    streak = streak + 1
    local threshold = math.max(1, omni.setting("kill_threshold"))
    local maximum = omni.setting("max_multiplier")
    local next_multiplier = math.max(1, math.min(maximum,
        1 + math.floor(streak / threshold)))
    if next_multiplier ~= multiplier then
        multiplier = next_multiplier
        flash_until = ticks + 35
        debug_log("multiplier now x" .. multiplier)
    end
    debug_log("enemy kill; x" .. multiplier .. " consecutive kills " .. streak)
end)

-- Don't reset on invulnerability / zero-damage calls. Compare actual health
-- after the original function has executed (rather than the requested amount).
omni.hook("rac1.player.take_damage", function(original, amount)
    local ok_before, before = pcall(function() return omni.fields.omni.hero.health end)
    local result = original(amount)
    local ok_after, after = pcall(function() return omni.fields.omni.hero.health end)
    if ok_before and ok_after and type(before) == "number"
        and type(after) == "number" and after < before then
        reset("health fell from " .. tostring(before) .. " to " .. tostring(after))
    end
    if ok_after then previous_health = after end
    return result
end)

omni.on_frame(function()
    ticks = ticks + 1
    if omni.level() < 0 then
        last_bolts = nil
        initialized = false
        return
    end

    local ok, current_bolts, hp = pcall(function()
        return omni.fields.bolts, omni.fields.omni.hero.health
    end)
    if not ok or type(current_bolts) ~= "number" or type(hp) ~= "number" then
        last_bolts = nil
        initialized = false
        return
    end

    if not initialized then
        last_bolts = current_bolts
        previous_health = hp
        initialized = true
    else
        -- Also covers damage not routed through Hero_TakeDamage (if any).
        if previous_health ~= nil and hp < previous_health then
            reset("health decreased")
        end
        previous_health = hp

        -- Delta-based bonus: no bolt-pickup hook currently exists. Capped to
        -- avoid multiplying huge scripted awards, reloads and save edits.
        local delta = current_bolts - last_bolts
        -- The public API exposes no dedicated bolt-HUD visibility state.
        -- Approximate it by showing the multiplier after a positive bolt gain.
        -- Trigger on the original delta, before writing any bonus bolts.
        if delta > 0 then
            hud_visible_until = ticks + omni.setting("hud_duration_ticks")
        end
        if delta > 0 and delta <= omni.setting("max_award")
            and multiplier > 1 and omni.setting("multiply_bolts")
            and (not true or omni.mode() == "gameplay") then
            local bonus = delta * (multiplier - 1)
            local adjusted = current_bolts + bonus
            -- Protect signed 32-bit bolt field from overflowing.
            if adjusted <= 2147483647 then
                local wrote = pcall(function() omni.fields.bolts = adjusted end)
                if wrote then
                    current_bolts = adjusted
                    debug_log("base +" .. delta .. ", bonus +" .. bonus .. " (x" .. multiplier .. ")")
                end
            end
        end
        last_bolts = current_bolts
    end

    -- Snapshot the living enemies; death detection consults this snapshot.
    -- The query is cheap on the game side but limit to 256 entries/tick.
    if ticks % 2 == 0 then
        local list = omni.mobys{ enemy = true, max = 256 }
        for _, addr in ipairs(list) do
            if killed[addr] == nil or ticks - killed[addr] > 120 then
                known_enemies[addr] = ticks
            end
        end
    end
    if ticks % 120 == 0 then
        for addr, seen_at in pairs(known_enemies) do
            if ticks - seen_at > 120 then known_enemies[addr] = nil end
        end
        for addr, seen_at in pairs(killed) do
            if ticks - seen_at > 120 then killed[addr] = nil end
        end
    end

    -- No HUD at x1; and don't render over menus, cutscenes or the ship.
    -- Coordinates use the native 512x448 game canvas.
    local display_window = (not true)
        or ticks < hud_visible_until
    if omni.setting("show_hud") and multiplier > 1
        and omni.mode() == "gameplay" and display_window then
        local accent = 0xFFD75CFF
        if ticks < flash_until then accent = 0xFFFFFFFF end

        -- hud_x denotes the RIGHT edge of the multiplier text, not its start.
        -- RAC-RS provides right-aligned text but no text-width measurement.
        -- Estimate glyph widths for placing the icon and background to its left.
        local right = 487
        local y = 66
        local scale = 107 / 100
        local label = "x" .. multiplier
        local digits = #tostring(multiplier)
        local text_width = (7.5 + digits * 9.5) * scale
        local text_left = right - text_width
        local icon_size = 32
        local icon_gap = 4
        local with_icon = true
        local icon_x = text_left - (with_icon and (icon_size + icon_gap) or 0)
        -- Previous offset was -half the icon height. Move down by 5 units total (2 below v0.3.4).
        local icon_y = y - math.floor(icon_size / 2) + 5

        if true then
            -- Soft rounded-corner approximation: 5 stacked translucent
            -- rectangular bands. There is no rounded-rect drawing primitive.
            local pad = 4
            local vertical_pad = 1
            -- Tighten the oversized optical inset on the icon side.
            -- The source bolt texture has transparent margins of its own.
            local left = (with_icon and icon_x or text_left) - pad + (with_icon and 5 or 0)
            -- Trim one unit from the top only; keep the icon and text fixed.
            local top = math.min(with_icon and icon_y or y, y - 3 * scale) - vertical_pad + 1
            local bottom = math.max(with_icon and (icon_y + icon_size) or (y + 14 * scale),
                y + 14 * scale) + vertical_pad
            local width = math.max(1, right + pad - left)
            local height = math.max(8, bottom - top)
            local radius = math.min(5, math.floor(height / 4), math.floor(width / 4))
            local bg = 0x01010170  -- Near-black, 44% opacity (56% transparent); match the native bolt HUD
            -- Banded corners: same tint throughout, no overlapping alpha.
            omni.draw.rect(left + radius, top, width - radius * 2, 1, bg)
            omni.draw.rect(left + 2, top + 1, width - 4, 2, bg)
            omni.draw.rect(left + 1, top + 3, width - 2, 2, bg)
            omni.draw.rect(left, top + 5, width, height - 10, bg)
            omni.draw.rect(left + 1, top + height - 5, width - 2, 2, bg)
            omni.draw.rect(left + 2, top + height - 3, width - 4, 2, bg)
            omni.draw.rect(left + radius, top + height - 1, width - radius * 2, 1, bg)
        end

        if with_icon then
            omni.draw.icon(icon_x, icon_y, icon_size, icon_size,
                "bolt", { tint = 0xFFFFFFFF })
        end
        omni.draw.text(right, y, label,
            { align = "right", color = accent, size = scale,
              shadow = 0x000000FF })
    end
end)

omni.log("bolt-multiplier mod ready")
