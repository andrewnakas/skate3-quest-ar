-- Freestyle MX. Left stick is the bike, right stick is the rider.
-- The host owns the physics; this file owns the control scheme, the trick
-- roster, the scoring and the recovery, so all four can be edited without a
-- rebuild.

-- Each trick is one authored rider pose, blended in by how far the stick is
-- pushed. `base` is what it is worth held to full extension.
local TRICKS = {
 [1] = {name = 'Superman',      base = 300, pose = 'superman'},
 [2] = {name = 'Lazy Boy',      base = 280, pose = 'lazyboy'},
 [3] = {name = 'Nac-Nac',       base = 220, pose = 'nacnac'},
 [4] = {name = 'Can-Can',       base = 180, pose = 'cancan'},
 [5] = {name = 'Heel Clicker',  base = 320, pose = 'heelclicker'},
 [6] = {name = 'Kiss of Death', base = 420, pose = 'kissofdeath'},
 [7] = {name = 'Cordova',       base = 400, pose = 'cordova'},
 [8] = {name = 'No-Hander',     base = 150, pose = 'nohander'},
}
-- Extension must be under this at touchdown or the landing is cased.
local TUCKED = 0.35
-- A trick has to be out this long to count, so a flicked stick scores nothing.
local MIN_HELD = 0.15
-- Yaw away from the line of travel that counts as a whip, and how square the
-- bike has to be by touchdown for it to have been brought back.
local WHIP_ANGLE = 0.55
local WHIP_HOME = 0.4
-- Throttle held this long against a bike that will not move means stuck.
local STUCK_SECONDS = 1.5
-- How long after a bail before the rider is put back on, and how long to keep
-- trying: the native skater has to finish its ragdoll before it can remount.
local REMOUNT_DELAY = 1.6
local REMOUNT_WINDOW = 6.0

local previous = {}
local interact_was_down, reset_was_down = false, false
local trick, extend = 0, 0
local air = nil
local total, combo, best = 0, 0, 0
local banner, banner_time = '', 0
local stuck_for, clock = 0, 0
local remount_until, remount_at = 0, nil
local safe_spot, safe_heading = nil, 0

local function pressed(key)
 local down = sdk.input.down(key)
 local edge = down and not previous[key]
 previous[key] = down
 return edge
end

local function tune()
 local s = sdk.settings
 sdk.vehicle.tune('mx', {
  engine_force = s.engine, max_speed = s.speed, tire_grip = s.tire_friction,
  lean_max = s.lean_max, lean_rate = s.lean_rate, lean_expo = s.lean_expo,
  counter_steer = s.counter_steer,
  lean_yaw = s.lean_yaw, berm_assist = s.berm_assist,
  steer_rate = s.steer_rate, grip_assist = s.grip_assist,
  step_assist = s.step_assist,
  preload_release = s.preload_release,
  air_yaw = s.air_yaw, whip_rate = s.whip_rate, flip_rate = s.flip_rate,
  engine_volume = s.engine_volume,
 })
end

local function say(text)
 banner, banner_time = text, 2.5
end

local function nearby()
 local p = sdk.player.read()
 local h = p.heading or 0
 return {p.position[1] + math.sin(h) * 3, p.position[2] + 1, p.position[3] + math.cos(h) * 3}, h
end

local function spawn()
 local bike = sdk.vehicle.read('mx')
 if bike and bike.occupied then
  sdk.vehicle.reset('mx', {bike.position[1], bike.position[2] + 1, bike.position[3]}, bike.heading)
  return
 end
 if bike then sdk.vehicle.remove('mx') end
 local p, h = nearby()
 sdk.vehicle.spawn('mx', 'vehicle.json', p, h)
 tune()
end

-- Put the bike back on its wheels where it is, and forget the run. Used by the
-- reset button, by the stuck detector and by the remount after a bail.
local function recover(position, heading)
 sdk.vehicle.reset('mx', position, heading)
 trick, extend, air, combo, stuck_for = 0, 0, nil, 0, 0
end

-- Modifier plus a right-stick octant picks the trick. Holding no modifier, or
-- leaving the stick centred, means the rider is riding rather than tricking.
local function selected(i)
 if not (i.trick_a or i.trick_b) then return 0 end
 local x, y = i.lean or 0, i.rider_y or 0
 if math.abs(x) < 0.4 and math.abs(y) < 0.4 then return 0 end
 local dir
 if math.abs(y) > math.abs(x) then dir = (y > 0) and 1 or 2 else dir = (x > 0) and 3 or 4 end
 return (i.trick_b and 4 or 0) + dir
end

-- Signed angle from a to b, wrapped to -pi..pi. Heading crosses +/-pi in the
-- middle of an ordinary corner, and an unwrapped difference reads that as most
-- of a rotation.
local function angle_between(a, b)
 local d = (b - a) % (2 * math.pi)
 if d > math.pi then d = d - 2 * math.pi end
 return d
end

local function land(bike)
 if not air then return end
 if air.seconds < 0.3 then air = nil return end
 if extend > TUCKED then
  -- Still hanging off the bike at touchdown. This is the rule that makes the
  -- trick a decision rather than a free bonus.
  combo = 0
  say('CASED - tuck in before you land')
  air = nil
  return
 end
 local score, label = 0, nil
 for id, seconds in pairs(air.tricks) do
  if seconds >= MIN_HELD then
   score = score + TRICKS[id].base * math.min(seconds, 1.5) / 1.5
  end
 end
 -- Each quarter turn of yaw, and each full flip.
 local spins = math.floor(math.abs(air.spin) / (math.pi / 2))
 local flips = math.floor(air.flip / (math.pi * 2))
 score = score + spins * 100 + flips * 500
 -- A whip is thrown sideways and brought back square. Sent and not brought
 -- back is not a whip, it is a case, and the host has already said so by
 -- bailing the rider before this ever runs.
 local squared = math.abs(angle_between(bike.heading, air.course)) < WHIP_HOME
 if air.whip > WHIP_ANGLE and squared and flips == 0 then
  score = score + 200 + math.floor((air.whip - WHIP_ANGLE) * 260)
  label = air.whip > 1.2 and 'Big Whip' or 'Whip'
 end
 score = score * (1 + math.min(air.seconds, 4) / 4)
 if score >= 1 then
  combo = combo + 1
  local banked = math.floor(score * combo)
  total = total + banked
  if banked > best then best = banked end
  label = (air.best_trick and TRICKS[air.best_trick].name) or label or 'Air'
  if flips > 0 then label = label .. ' + ' .. flips .. ' flip' end
  if spins > 0 then label = label .. ' + ' .. (spins * 90) .. '\194\176' end
  say(string.format('%s  %d  x%d', label, banked, combo))
 end
 air = nil
end

local function score_air(bike, dt)
 if not sdk.settings.scoring then air = nil return end
 if bike.airborne then
  if not air then
   local v = bike.velocity or {0, 0, 0}
   local flat = math.sqrt(v[1] * v[1] + v[3] * v[3])
   air = {
    seconds = 0, spin = 0, flip = 0, whip = 0, tricks = {}, best_trick = nil, best_held = 0,
    -- The line the bike left the lip on. A whip is measured against where it
    -- is going, not against where it was pointing when it took off.
    course = flat > 1 and math.atan(v[1], v[3]) or bike.heading,
   }
  end
  air.seconds = air.seconds + dt
  local w = bike.angular_velocity or {0, 0, 0}
  air.spin = air.spin + w[2] * dt
  air.flip = air.flip + math.sqrt(w[1] * w[1] + w[3] * w[3]) * dt
  air.whip = math.max(air.whip, math.abs(angle_between(bike.heading, air.course)))
  if trick ~= 0 and extend > 0.5 then
   local held = (air.tricks[trick] or 0) + dt
   air.tricks[trick] = held
   if held > air.best_held then air.best_held, air.best_trick = held, trick end
  end
 elseif air then
  land(bike)
 end
end

-- A bike wedged against geometry, or one that has fallen out of the world, is
-- the one failure the rider cannot ride out of. Both put it back on its wheels
-- at the last place it was actually moving.
local function unstick(bike, i, dt)
 if bike.position[2] < -50 then
  local p = safe_spot or select(1, nearby())
  recover({p[1], p[2] + 1, p[3]}, safe_heading)
  say('OUT OF THE WORLD - put back')
  return true
 end
 local trying = (i.trigger_r or 0) > 0.5 or (i.trigger_l or 0) > 0.5
 if trying and math.abs(bike.speed) < 0.5 and not bike.airborne then
  stuck_for = stuck_for + dt
 else
  stuck_for = 0
 end
 if stuck_for > STUCK_SECONDS then
  recover({bike.position[1], bike.position[2] + 0.6, bike.position[3]}, bike.heading)
  say('UNSTUCK')
  return true
 end
 if not bike.airborne and math.abs(bike.speed) > 2 then
  safe_spot, safe_heading = bike.position, bike.heading
 end
 return false
end

-- Two lines and a banner. The control list lives in the mod window, where it
-- can be read once rather than sat on screen for the whole session.
local function hud(bike)
 local lines
 if not bike then
  lines = {'FREESTYLE MX | F9 spawns the bike'}
 else
  local name = (trick ~= 0 and TRICKS[trick].name .. string.format(' %.0f%%', extend * 100)) or nil
  lines = {
   string.format('%.0f km/h%s', math.abs(bike.speed) * 3.6, bike.airborne and '  AIR' or ''),
   string.format('SCORE %d   x%d   best %d%s', total, combo, best,
    name and ('   ' .. name) or ''),
  }
  if not bike.ready then lines[1] = 'loading the bike...' end
  if banner ~= '' then lines[#lines + 1] = banner end
 end
 for i = 1, 7 do
  local key = 'mx-hud-' .. i
  if sdk.settings.hud and lines[i] then sdk.ui.text(key, lines[i]) else sdk.scene.remove(key) end
 end
end

return {
 on_load = function() hud() end,
 on_fixed_update = function(event)
  local dt = event.dt
  clock = clock + dt
  if pressed('F9') then spawn() end
  local bike = sdk.vehicle.read('mx')
  if not bike then hud() return end

  -- Getting back on after a crash. The native skater has to finish its
  -- ragdoll before the host will let it mount, and the host silently refuses
  -- an enter it cannot honour, so this keeps asking for a few seconds rather
  -- than assuming the first attempt took.
  if remount_at and clock >= remount_at then
   local p = safe_spot or bike.position
   recover({bike.position[1], bike.position[2] + 0.6, bike.position[3]}, safe_heading)
   sdk.player.teleport({p[1] + math.cos(safe_heading) * 1.2, p[2] + 0.2,
    p[3] - math.sin(safe_heading) * 1.2}, safe_heading, false)
   remount_at, remount_until = nil, clock + REMOUNT_WINDOW
  end
  if remount_until > clock then
   if bike.occupied then
    remount_until = 0
   elseif sdk.settings.remount then
    sdk.vehicle.enter('mx')
   end
  end

  local i = sdk.vehicle.input()
  local interact = i.interact
  if interact and not interact_was_down then
   if bike.occupied then sdk.vehicle.exit('mx') else sdk.vehicle.enter('mx') end
   remount_until = 0
  end
  interact_was_down = interact
  local reset_down = bike.occupied and ((i.pad_buttons or 0) & 0x80) ~= 0
  if pressed('KeyR') or (reset_down and not reset_was_down) then
   local p, h = nearby()
   if bike.occupied then p = {bike.position[1], bike.position[2] + 1, bike.position[3]}; h = bike.heading end
   recover(p, h)
  end
  reset_was_down = reset_down

  if bike.phase == 'driving' then
   if not unstick(bike, i, dt) then
    local want = selected(i)
    if want ~= 0 then trick = want end
    -- Extension eases out and snaps back, so a half-thrown trick reads as a
    -- half-thrown trick rather than an on/off pose swap.
    if want ~= 0 then
     extend = math.min(1, extend + dt * 4)
    else
     extend = math.max(0, extend - dt * 5)
     if extend == 0 then trick = 0 end
    end
    -- Throwing the rider off the bike means you are not steering it. The stick
    -- does one job at a time.
    local tricking = extend > 0.05
    -- The left trigger brakes, and once you are stopped it backs you up.
    -- The host reads reverse as negative throttle, which nothing was ever
    -- sending, so the bike could brake but never reverse out of anything.
    local lt = i.trigger_l or 0
    local reversing = math.abs(bike.speed) < 1.2 and lt > 0.25
    sdk.vehicle.control('mx', {
     throttle = reversing and -lt or (i.trigger_r or 0),
     brake = reversing and 0 or lt,
     steering = i.steering,
     weight = i.weight,
     whip = i.whip,
     lean = tricking and 0 or (i.lean or 0),
     handbrake = i.handbrake,
     clutch = i.clutch,
     trick = trick,
     trick_extend = extend,
    })
    score_air(bike, dt)
   end
  end
  if banner_time > 0 then
   banner_time = banner_time - dt
   if banner_time <= 0 then banner = '' end
  end
  hud(bike)
 end,
 on_settings = function()
  if sdk.vehicle.read('mx') then tune() end
  hud(sdk.vehicle.read('mx'))
 end,
 on_event = function(e)
  if e.name == 'world_changed' then
   previous, interact_was_down, reset_was_down = {}, false, false
   trick, extend, air, combo = 0, 0, nil, 0
   remount_at, remount_until, safe_spot = nil, 0, nil
   hud()
  elseif e.name == 'vehicle_bailed' and e.key == 'mx' then
   air, trick, extend, combo = nil, 0, 0, 0
   local reason = e.reason
   say(reason == 'landing' and 'CASED IT' or (reason == 'inverted' and 'TIPPED OVER' or 'CRASH'))
   if sdk.settings.remount then
    remount_at = clock + REMOUNT_DELAY
   end
  end
 end,
}
