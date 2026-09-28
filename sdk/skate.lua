---@meta
-- SDK 1 language-server declarations. Add this directory as a Lua workspace library.
-- This file is not executed. Runtime reference: docs/lua-modding.md.
---@alias Vec3 number[] Three finite components, 1-based, Y up, metres.
---@alias SettingValue boolean|number|string
---@class GrindSnapshot
---@field active boolean
---@field name string
---@field kind integer
---@field distance number
---@class PlayerSnapshot
---@field position Vec3 Animation root position.
---@field velocity Vec3 Physical board velocity, including when offboard.
---@field heading number Animation-root yaw in radians, zero faces +Z.
---@field on_board boolean
---@field state integer Native state identifier, not a trick name.
---@field category integer
---@field bailing boolean
---@field grind GrindSnapshot
---@class MapSnapshot
---@field name string
---@field generation integer
---@class AnimationInfo
---@field bone_names string[]
---@field slots table<string, {fps:number,frame_count:integer}>
---@class SDKSnapshot
---@field player PlayerSnapshot
---@field map MapSnapshot
---@field animation AnimationInfo
---@field network NetworkInfo
---@field tick integer
---@field keys table<string,boolean>
---@field actions number[] 18 values; Lua index 1 corresponds to native action 64.
---@field paused boolean
---@field replay boolean
---@class ModCallbacks
---@field on_load? fun()
---@field on_unload? fun() Commands discarded; host always cleans up.
---@field on_update? fun(event:{dt:number}) Active render time; max dt 0.25 seconds.
---@field on_fixed_update? fun(event:{dt:number}) After native physics, before next input publication.
---@field on_settings? fun(event:{key:string,value:SettingValue}) sdk.settings already updated.
---@field on_event? fun(event:ModEvent)
---@class ModEvent
---@field name 'world_changed'|'bail_changed'|'player_state_changed'|'grind_changed'
---@field map? MapSnapshot world_changed only
---@field bailing? boolean bail_changed only
---@field previous? integer player_state_changed only
---@field state? integer player_state_changed only
---@field grind? GrindSnapshot grind_changed only
sdk = {
    api_version = 1,
    ---@type table<string,SettingValue>
    settings = {},
    ---@type SDKSnapshot
    snapshot = {},
    player = {}, input = {}, ui = {}, scene = {}, animation = {},
    time = {elapsed = 0},
}
---@param text string At most 2048 UTF-8 bytes; queues labelled INFO output.
function sdk.log(text) end
---@param path string Relative within mod root; max 256 KiB, UTF-8 only; errors on failure.
---@return string
function sdk.read_text(path) end
---@return PlayerSnapshot
function sdk.player.read() end
---@param position Vec3 Components within ±100000 metres; author supplies a valid destination.
---@param heading number Radians around world Y; zero faces +Z.
---@param on_board boolean
function sdk.player.teleport(position, heading, on_board) end
---@param key string Bevy physical KeyCode name, e.g. KeyJ, F6, Space. Unknown returns false.
---@return boolean
function sdk.input.down(key) end
---@param id integer Native published gameplay action 64..81; invalid ID raises an error.
---@return number
function sdk.input.action(id) end
---@param key string Owner-local stable visual key, 1..64 lowercase ASCII letters/digits/._-
---@param text string At most 1024 UTF-8 bytes.
function sdk.ui.text(key, text) end
---@param key string Same namespace as sdk.ui.text; repeated key updates/replaces.
---@param position Vec3
---@param size Vec3 Each component >0 and <=100 metres.
---@param color Vec3 Each sRGB component in 0..1.
function sdk.scene.cube(key, position, size, color) end
---@param key string Remove owner's text or cube; absent is a no-op.
function sdk.scene.remove(key) end
---@param key string Timer key, 1..64 bytes. Existing timer is replaced.
---@param seconds number Finite delay 0..86400 active Update seconds.
---@param callback function One-shot callback, part of the on_update transaction/quota.
function sdk.time.after(key, seconds, callback) end
---@param key string Absent is a no-op.
function sdk.time.cancel(key) end
---@return AnimationInfo
function sdk.animation.info() end
---@param path string Package-relative original body-clip JSON, <=4 MiB; host validates supported slots and exact timing.
function sdk.animation.replace(path) end

---@class TrainerTuning Numeric fields default to 1; multipliers of current stock settings.
---@field pop? number 0.25..4, native ground pop heights.
---@field grind_pop? number 0.25..4, native grind launch height.
---@field push_speed? number 0.25..4, push target and maximum pushable speed.
---@field push_power? number 0.25..4, native push velocity-change limits.
---@field braking? number 0.25..4, foot and tail brake force.
---@field steering? number 0.25..4, general steering scalar.
---@field wobble? number 0..2, speed wobble amplitude.
---@field offboard_jump? number 0.25..4, native biped launch height.
---@field grip? number 0.25..4, wheel static/dynamic and sideways slide friction.
---@field turn_power? number 0.25..4, native heading turn strength.
---@field hold_fakie? boolean Defaults false; suppress automatic fakie stance switching.
---@field manual_drag? number 0.25..4, manual balance linear drag.
---@field endless_flips? boolean Defaults false. Not retail: a held kickflip or heelflip keeps cycling past the authored quad while the pop has air for another rung. Needs real height; a flat pop still stops at a double.
---@field endless_flip_max? integer 1..12, default 12; extra flips allowed past the quad, so 12 reaches a 16x.
---@field endless_air_check? boolean Defaults false: keep flipping while held and airborne, with no height requirement. True only starts a rung it has the air to finish.
sdk.trainer = {}
---@param tuning TrainerTuning Owned, reversible native tuning. Conflicting owner is rejected.
function sdk.trainer.apply(tuning) end

---@class VehicleControls
---@field pitch? number -1..1; airborne nose up to nose down (left stick vertical)
---@field throttle? number -1..1, reverse to forward
---@field steering? number -1..1, right to left
---@field brake? number 0..1
---@field handbrake? boolean
---@field lean? number -1..1 rider weight, positive driver-left; lean grounded, roll airborne
---@field weight? number -1..1 rider weight fore/aft, positive back; preload grounded, pitch airborne
---@field whip? number -1..1 airborne yaw, positive driver-left
---@field trick? integer Selected trick, 0 for none; needs a matching authored rider pose
---@field trick_extend? number 0..1 how far into the selected trick
---@field clutch? boolean
---@class VehicleTuning
---@field lean_max? number 0.05..1.4 radians, maximum commanded lean (bike profile)
---@field lean_rate? number 0..60 1/s how quickly the lean takes up (bike profile)
---@field lean_expo? number 0..1 softens the bars around centre (bike profile)
---@field berm_assist? number 0..2 how much a banked surface adds to lean (bike profile)
---@field steer_rate? number 0..8 rad/s turn rate the bars ask for at speed (bike profile)
---@field grip_assist? number 0..1 how much the turn points the bike rather than sliding (bike profile)
---@field step_assist? number 0..0.6 tallest step in metres the bike climbs rather than jamming on (bike profile)
---@field counter_steer? number 0..2 steering to lean coupling (bike profile)
---@field preload_release? number 0..20000 N.s pop from releasing preload (bike profile)
---@field air_yaw? number 0..20 rad/s^2 airborne yaw authority (bike profile)
---@field lean_yaw? number 0..40 1/s how hard the bike follows the arc its lean dictates (bike profile)
---@field whip_rate? number 0..12 rad/s yaw rate at full bars in the air (bike profile)
---@field flip_rate? number 0..12 rad/s pitch rate on a held stick in the air (bike profile)
---@field engine_volume? number 0..1; requires engine_audio.enabled in definition
---@field engine_force? number 0..100000 N
---@field max_speed? number 1..100 m/s, engine limit
---@field brake_impulse? number 0..10000
---@field steering_angle? number 0.01..1.2 radians
---@field tire_grip? number 0.1..20
---@class VehicleSnapshot
---@field position number[] metres
---@field rotation number[] quaternion xyzw
---@field heading number radians
---@field speed number signed m/s, post-solve longitudinal
---@field velocity number[] world linear velocity, m/s
---@field angular_velocity number[] world angular velocity, rad/s
---@field wheel_contacts integer how many wheels are touching
---@field airborne boolean wheel_contacts == 0
---@field wheel_speed number driven-wheel angular speed, rad/s
---@field phase 'parked'|'entering'|'driving'|'exiting'
---@field occupied boolean
---@field ready boolean
sdk.vehicle = {}
---@param key string Owner-local key
---@param definition string Package-relative vehicle JSON
---@param position number[] World position
---@param heading? number Radians
function sdk.vehicle.spawn(key,definition,position,heading) end
---@param key string
---@return VehicleSnapshot?
function sdk.vehicle.read(key) end
---@param key string
---@param controls VehicleControls Refresh every fixed tick
function sdk.vehicle.control(key,controls) end
---@param key string
---@param tuning VehicleTuning
function sdk.vehicle.tune(key,tuning) end
---@param key string
function sdk.vehicle.enter(key) end
---@param key string
function sdk.vehicle.exit(key) end
---@param key string
---@param position number[]
---@param heading? number
function sdk.vehicle.reset(key,position,heading) end
---@param key string
function sdk.vehicle.remove(key) end
---@return table Normalized keyboard/controller axes plus interact and pad_buttons
function sdk.vehicle.input() end

---@class VehicleBailEvent
---@field name 'vehicle_bailed'
---@field owner string
---@field key string
---@field reason 'crash'|'rider_impact'|'inverted'|'landing'
---@field position number[] World seat position at detection, metres
---@field velocity number[] Carried world velocity plus launch lift, m/s
---@field angular_velocity number[] World angular velocity, rad/s
-- Configure rider_safety in the vehicle definition; see docs/vehicle-sdk.md.

---@class NetworkInfo
---@field active boolean
---@field local_id string Keep 64-bit player IDs as strings. Offline: "0".
---@field is_host boolean
---@field states table<string,table<string,table<string,any>>> Mod ID -> peer ID -> state keys.
---@field status string Matching/limit diagnostics.
sdk.net = {}
---@return NetworkInfo
function sdk.net.info() end
---@param key string Stable owner-local key, 1..64 characters.
---@param value any JSON-compatible value, at most 512 encoded bytes; nil clears.
function sdk.net.publish(key,value) end
---@param peer string Player ID from sdk.net.info(), never converted to a number.
---@param key string
---@return any Latest value for this mod/peer/key, or nil.
function sdk.net.read(peer,key) end
-- State coalesces; this is not an exactly-once event channel.
-- World cubes and owned vehicles replicate automatically for matching enabled mods.
-- See docs/multiplayer-mods.md for ownership, collision and late-join behavior.
