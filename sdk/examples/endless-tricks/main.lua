-- Endless Tricks: the flip ladder past retail's quad.
--
-- The mod itself is thin on purpose. Lua cannot reach the MotionGraph or the scorer, so the loop
-- lives in the host and this only owns the switch, through the same trainer tuning the Native
-- Trainer uses. Only one mod may own that tuning at a time.
--
-- Settings are read on load and whenever they change, not every tick: the tuning is owned and
-- reversible, so re-applying an unchanged table each frame would be pure waste.

local HUD_KEY = 'status'

local function tuning()
    local enabled = sdk.settings.enabled
    if enabled == nil then
        enabled = true
    end
    -- The host counts rungs *past* the authored quad, so a 16-flip limit is 12 extra rungs.
    local flips = tonumber(sdk.settings.max_flips) or 16
    local extra = math.floor(flips) - 4
    if extra < 1 then
        extra = 1
    elseif extra > 12 then
        extra = 12
    end
    return {
        endless_flips = enabled and true or false,
        endless_flip_max = extra,
        endless_air_check = sdk.settings.require_air == true,
    }
end

local function refresh()
    local t = tuning()
    sdk.trainer.apply(t)
    if sdk.settings.hud == false then
        sdk.scene.remove(HUD_KEY)
    elseif t.endless_flips then
        sdk.ui.text(HUD_KEY, string.format('ENDLESS TRICKS  up to %dx  (needs air)', t.endless_flip_max + 4))
    else
        sdk.ui.text(HUD_KEY, 'ENDLESS TRICKS  off')
    end
end

return {
    on_load = function()
        refresh()
        sdk.log('Endless Tricks ready. Hold a kickflip, heelflip, 360 flip or laserflip off something tall.')
    end,
    on_settings = refresh,
    on_event = function(event)
        -- A map change drops owned visuals, so the status line is placed again.
        if event.name == 'world_changed' then
            refresh()
        end
    end,
    on_unload = function()
        sdk.scene.remove(HUD_KEY)
    end,
}
