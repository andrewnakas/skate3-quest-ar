-- Native Trainer Showcase: original Lua, no game assets.
local tuning_keys={'pop','grind_pop','push_speed','push_power','braking','steering','wobble','offboard_jump','grip','turn_power','manual_drag','hold_fakie'}
local previous, checkpoint, last_position, last_trail = {}, nil, nil, nil
local trail_index, beacon_index = 0, 0
local stats = {distance=0,peak=0,bails=0,grinds=0,grind_time=0}
local stopwatch, running, elapsed = 0, false, 0
local function copy(p) return {p[1],p[2],p[3]} end
local function distance(a,b) return math.sqrt((a[1]-b[1])^2+(a[2]-b[2])^2+(a[3]-b[3])^2) end
local function color()
    return sdk.settings.marker_color=='orange' and {1,0.45,0.1} or sdk.settings.marker_color=='purple' and {0.7,0.25,1} or {0.1,0.85,0.85}
end
local function notice(text)
    sdk.ui.text('notice',text)
    sdk.time.after('notice',3,function() sdk.scene.remove('notice') end)
end
local function clear_trail()
    for i=1,24 do sdk.scene.remove('trail'..i) end
    last_trail=nil; trail_index=0
end
local function clear_session()
    clear_trail()
    for i=1,8 do sdk.scene.remove('beacon'..i) end
    sdk.scene.remove('checkpoint'); sdk.scene.remove('timer'); sdk.scene.remove('notice')
    sdk.time.cancel('notice')
    checkpoint=nil; beacon_index=0; last_position=nil
    stopwatch=0; running=false; elapsed=0
    stats={distance=0,peak=0,bails=0,grinds=0,grind_time=0}
end
local function apply()
    local tuning={}
    for _,key in ipairs(tuning_keys) do tuning[key]=sdk.settings[key] end
    sdk.trainer.apply(tuning)
    if not sdk.settings.trail then clear_trail() end
    if sdk.settings.hud=='off' then sdk.scene.remove('hud'); sdk.scene.remove('hud_stats'); sdk.scene.remove('hud_help') end
end
local function draw()
    local s=sdk.settings
    if s.hud=='off' then sdk.scene.remove('hud'); sdk.scene.remove('hud_stats'); sdk.scene.remove('hud_help'); return end
    local p=sdk.player.read(); local v=p.velocity
    local speed=math.sqrt(v[1]^2+v[2]^2+v[3]^2)
    local factor=s.units=='km/h' and 3.6 or s.units=='mph' and 2.236936 or 1
    local label=s.hud_label; local cut=utf8.offset(label,25); if cut then label=label:sub(1,cut-1) end
    local text=string.format('%s | %.1f %s | peak %.1f | %.0f m',label,speed*factor,s.units,stats.peak*factor,stats.distance)
    if s.hud=='full' then
        sdk.ui.text('hud_stats',string.format('XYZ %.1f %.1f %.1f | bails %d | grinds %d / %.1fs',p.position[1],p.position[2],p.position[3],stats.bails,stats.grinds,stats.grind_time))
    else sdk.scene.remove('hud_stats') end
    if s.show_help then sdk.ui.text('hud_help',string.format('%s save %s return %s timer %s clear %s beacon',s.save_key,s.return_key,s.timer_key,s.reset_key,s.beacon_key)) else sdk.scene.remove('hud_help') end
    sdk.ui.text('hud',text)
end
local function update(e)
    local p=sdk.player.read(); local position=p.position
    elapsed=elapsed+e.dt
    if last_position then local d=distance(position,last_position); if d<50 then stats.distance=stats.distance+d end end
    last_position=copy(position)
    local v=p.velocity; stats.peak=math.max(stats.peak,math.sqrt(v[1]^2+v[2]^2+v[3]^2))
    if p.grind and p.grind.active then stats.grind_time=stats.grind_time+e.dt end
    local edges={}; local down={}
    for _,setting in ipairs({'save_key','return_key','timer_key','reset_key','beacon_key'}) do
        local key=sdk.settings[setting]; down[key]=sdk.input.down(key); edges[setting]=down[key] and not previous[key]
    end
    previous=down
    if edges.save_key then
        if p.bailing or p.state==702 then notice('Wait until the skater is ready to save.')
        else
            checkpoint={position=copy(position),heading=p.heading or 0,on_board=p.on_board}
            sdk.scene.cube('checkpoint',{position[1],position[2]+0.5,position[3]},{0.15,1,0.15},color())
            notice('Trainer checkpoint saved (separate from the native session marker).')
        end
    end
    if edges.return_key then
        if not checkpoint then notice('Save a trainer checkpoint first.')
        elseif p.bailing or p.state==702 then notice('Wait until recovery finishes.')
        else
            sdk.player.teleport(checkpoint.position,checkpoint.heading,checkpoint.on_board)
            last_position=nil; last_trail=nil
            notice('Returning to trainer checkpoint.')
        end
    end
    if edges.timer_key then
        if running then running=false; notice(string.format('Stopwatch stopped: %.2f seconds',stopwatch))
        else stopwatch=0; running=true; notice('Stopwatch started.') end
    end
    if edges.reset_key then clear_session(); notice('Session stats, checkpoint and visual markers cleared.') end
    if edges.beacon_key then
        beacon_index=beacon_index%8+1
        sdk.scene.cube('beacon'..beacon_index,{position[1],position[2]+1,position[3]},{0.25,2,0.25},color())
        notice('Visual beacon placed; it has no collision.')
    end
    if sdk.settings.trail and (not last_trail or distance(position,last_trail)>=sdk.settings.trail_spacing) then
        trail_index=trail_index%24+1; last_trail=copy(position)
        sdk.scene.cube('trail'..trail_index,{position[1],position[2]+0.1,position[3]},{0.2,0.2,0.2},color())
    end
    if running then stopwatch=stopwatch+e.dt end
    if running or stopwatch>0 then sdk.ui.text('timer',string.format('STOPWATCH: %.2f s%s',stopwatch,running and '' or ' (stopped)')) end
    draw()
end
return {
    on_load=function()
        apply(); draw()
        sdk.log(sdk.read_text('help.txt'))
        notice('Native Trainer Showcase ready. Open Escape for settings.')
    end,
    on_settings=function() apply(); draw() end,
    on_update=update,
    on_event=function(e)
        if e.name=='world_changed' then clear_session(); previous={}; apply(); draw(); notice('New map: trainer session cleared.') end
        if e.name=='bail_changed' and e.bailing then stats.bails=stats.bails+1 end
        if e.name=='grind_changed' and e.grind.active then stats.grinds=stats.grinds+1 end
    end
}
