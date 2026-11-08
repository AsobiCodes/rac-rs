-- SPDX-License-Identifier: MIT
-- Settings are persisted by the standard game/launcher UI.
-- The host reads them once at the next movie; no per-frame filesystem access.
omni.log("[rtx-video] Enabled: upscale " .. tostring(omni.setting("scale")) .. "x, quality " .. tostring(omni.setting("quality")) .. ". Changes apply to the next movie.")
