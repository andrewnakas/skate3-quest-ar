-- The original Lua/JSON example uses the user's separately supplied kart model.
local previous={}
local interact_was_down=false
local reset_was_down=false
local function hud(car)
 local lines={
  car and string.format('MARIO KART | %.0f km/h | %s',math.abs(car.speed)*3.6,car.ready and car.phase or 'loading') or 'MARIO KART | F10: spawn kart',
  'KEYBOARD | F10 spawn | E enter / exit | W / S accelerate / reverse | A / D steer',
  'KEYBOARD | Space brake | Left Shift handbrake | R reset',
  'CONTROLLER | Y enter / exit | RT / LT accelerate / reverse | Left stick steer',
  'CONTROLLER | A brake | B handbrake | '..(sdk.settings.reset_button=='Left stick' and 'L3' or 'R3')..' (stick click) reset',
  'Slow below 3 m/s to exit | Hard crashes / rollovers eject the rider | Escape: settings'
 }
 for i,line in ipairs(lines) do
  local key=i==1 and 'kart-hud' or 'kart-hud-'..i
  if sdk.settings.hud then sdk.ui.text(key,line) else sdk.scene.remove(key) end
 end
end
local function pressed(key)
 local down=sdk.input.down(key);local edge=down and not previous[key];previous[key]=down;return edge
end
local function tune()
 local s=sdk.settings
 sdk.vehicle.tune('kart',{engine_volume=s.engine_volume,engine_force=s.engine,max_speed=s.speed,brake_impulse=s.brake,steering_angle=s.steering,tire_grip=s.tire_friction})
end
local function nearby()
 local p=sdk.player.read();local h=p.heading or 0
 return {p.position[1]+math.sin(h)*3,p.position[2]+1,p.position[3]+math.cos(h)*3},h
end
local function spawn()
 local car=sdk.vehicle.read('kart')
 if car and car.occupied then sdk.vehicle.reset('kart',{car.position[1],car.position[2]+1,car.position[3]},car.heading);return end
 if car then sdk.vehicle.remove('kart') end
 local p,h=nearby();sdk.vehicle.spawn('kart','vehicle.json',p,h);tune()
end
return {
 on_load=function() hud() end,
 on_fixed_update=function()
  if pressed('F10') then spawn() end
  local car=sdk.vehicle.read('kart')
  if not car then hud();return end
  local interact=sdk.vehicle.input().interact
  local interact_pressed=interact and not interact_was_down;interact_was_down=interact
  if interact_pressed then if car.occupied then sdk.vehicle.exit('kart') else sdk.vehicle.enter('kart') end end
  local reset_mask=sdk.settings.reset_button=='Left stick' and 0x40 or 0x80
  local reset_down=car.occupied and ((sdk.vehicle.input().pad_buttons or 0) & reset_mask)~=0
  local reset_pressed=reset_down and not reset_was_down;reset_was_down=reset_down
  if pressed('KeyR') or reset_pressed then
   local p,h=nearby()
   if car.occupied then p={car.position[1],car.position[2]+1,car.position[3]};h=car.heading end
   sdk.vehicle.reset('kart',p,h)
  end
  if car.phase=='driving' then sdk.vehicle.control('kart',sdk.vehicle.input() and {
   throttle=sdk.vehicle.input().throttle,steering=sdk.vehicle.input().steering,
   pitch=sdk.vehicle.input().pitch or 0,
   brake=sdk.vehicle.input().brake,handbrake=sdk.vehicle.input().handbrake}) end
  hud(car)
 end,
 on_settings=function() local car=sdk.vehicle.read('kart');if car then tune() end;hud(car) end,
 on_event=function(e) if e.name=='world_changed' then previous={};interact_was_down=false;reset_was_down=false;hud() end end,
}
