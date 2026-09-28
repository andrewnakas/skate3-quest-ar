# Multiplayer mod SDK (protocol 5)

Use the same game build and map on every PC. Install and enable the same mod ZIP
on each PC for that mod's shared objects. Launch `PLAY.bat`, then use
**Escape > Multiplayer** to host or join. The build includes the optional Steam
relay; Steam must be running for Steam lobbies. Direct local testing remains
available. `Build.ps1` builds and stages both executables without launching them.

## What synchronizes automatically

| SDK feature | Multiplayer behavior |
| --- | --- |
| `scene.cube` / `scene.remove` | Owner-scoped shared visual objects; updates, removal and late joins supported. Cubes remain non-colliding. |
| `vehicle.spawn/remove/reset/tune/control` | Matching package supplies the model and definition. Owner sends chassis pose, velocities, wheel state, controls, tuning and occupancy. Other peers render and collide with replicas. |
| Vehicle entry/exit/steering | The final displayed native skeleton, including stance blending and arm steering, is replicated. Seated riders follow the same car render sample. |
| Vehicle engine audio | Each occupied car has a smoothed engine voice; other players' cars attenuate with distance. |
| Vehicle crash/inversion | The owning player decides the ejection, preserving pre-impact momentum. Their native wipeout/ragdoll then uses existing player replication. |
| Trainer / animation replacements / teleport | Run on the owning player's game. Their resulting movement, pose and ragdoll replicate through native player networking. Remote tuning never overwrites your local controls. |
| UI, mod settings windows, input, timers and logs | Local to each player. No duplicate remote input execution. |
| Custom Lua scores, checkpoints or game rules | Use `sdk.net.publish/read`; Lua variables alone are not shared. |

This applies to the SDK, not a special Mario Kart network script. Existing trainer
beacons/trails and vehicle mods use these paths without additional networking code.
The imported multiplayer renderer still displays remote players using the stock
skater; custom character appearance transfer is a separate feature.

## Matching and ownership

A world/state record carries its mod ID and a portable fingerprint of package-relative
file names and file contents, including scripts and assets. Absolute installation
paths and ZIP timestamps do not participate. Different assets or scripts count as
a different package even if the version text is unchanged. The receiver must have
the matching package enabled and running. Mismatches appear in the multiplayer HUD.
No ZIP download, remote Lua execution, or arbitrary remote asset path is allowed.

Object identity is `(player ID, mod ID, key)`. Both players may spawn `kart` without
replacing each other's car. The spawning player remains the simulation owner and
driver; transferring a vehicle to another player or sharing a passenger seat is not
implemented. Disconnecting, disabling a mod or removing an object removes its
replicas. Resetting a car updates its existing replica. Joining later receives the
current state rather than replaying callbacks. Stable keys should be reused.

Settings are personal. Shared vehicle tuning follows its owner's settings. For a
single shared course or scoreboard, have one designated peer (usually the lobby
host) own it; do not run the same authoritative rule independently on every peer.
Check `local_id`, `active` and `is_host` each update because connections and hosts
can change. Objects owned by a departed peer disappear; a replacement host must
recreate host-owned game rules/objects if your mod needs that behavior.

## Shared-state API

```lua
local previous_id
return {
  on_update = function(e)
    local net = sdk.net.info()
    -- Keep player IDs as strings; they are 64-bit and can exceed Lua's exact range.
    if previous_id ~= net.local_id then
      previous_id = net.local_id
      sdk.net.publish("ready", {ready=true})
    end
    local mine = sdk.net.read(net.local_id, "ready")
    -- net.states[sdk.mod_id] maps peer ID strings to that peer's state keys.
    for peer, values in pairs((net.states or {})[sdk.mod_id] or {}) do
      local ready = sdk.net.read(peer, "ready")
      -- Display results locally; only mutate your own authoritative state.
    end
  end
}
```

- `sdk.net.info()` returns `{active, local_id, is_host, states, status}`. Offline ID
  is `"0"` and `is_host=true`. State is still readable locally while offline.
- `sdk.net.publish(key, value)` stores this mod's latest value under the local
  player. Values can be JSON-compatible scalars/tables, at most 512 encoded bytes.
  Keys follow normal SDK key rules (1..64 characters). `nil` clears a value.
- `sdk.net.read(peer_id, key)` reads only this mod's value for that peer. Missing
  peers/keys return `nil`. Poll this from an update callback; no remote callback
  runs and there is no exactly-once event API.
- Publish durable state, such as a counter or `{sequence=12, checkpoint=3}`, rather
  than a one-frame pulse. Fast updates may coalesce into the newest value.
- A Lua callback's commands still commit only after it succeeds. Failing callbacks
  do not send their queued state changes.

## Collision and authority contract

Native players already use owner simulation. Cars follow the same model: your
Rapier world owns your cars, and remote cars use kinematic collision proxies.
Chassis collisions affect your local simulation and can trigger a local rider bail;
the resulting motion is sent back. Two locally owned cars use ordinary dynamic
Rapier contacts. Remote cars include the occupied rider capsule.

Native skating/board collision queries include nearby car chassis. Rapier also
receives nearby native player/board proxies (small spheres around active physical
parts). This bridges the two physics engines; it is an approximation of the native
articulated hitboxes. A seated driver's native standing hitboxes are disabled in
network snapshots so they cannot collide at their old skating position.

Remote positions use bounded extrapolation (150 ms), limited correction velocity,
and fixed-step render interpolation. Rider attachment uses the car's displayed
transform. Large resets snap instead of drawing a car through the map. This is
not a host-authoritative shared Rapier world: delayed impacts can differ across
peers and are corrected from each owner's next state. It does not promise exactly
conserved cross-client momentum or competitive anti-cheat.

## Bounds and failure behavior

Application datagrams share the native lobby's byte budget and MTU (1200 bytes).
Native body/pose traffic has priority. Latest records are acknowledged, resent
with varied retry intervals after loss, and relayed only by authenticated owners.
Records contain at most 1024 value bytes and 128 key bytes; 256 wire keys per player
per session includes deletion tombstones. SDK data has a 128-live-record limit;
vehicle state is additional. Reuse keys rather than generating unlimited unique
IDs. A full network record includes metadata, so oversized world descriptors are
reported in the multiplayer HUD instead of being truncated. Typical kart and
trainer records fit. Heavy object loads lower effective update frequency.

The existing limits remain 8 vehicles per mod and 32 local vehicles. Up to 288
remote vehicle replicas are admitted across the other nine players. No peer may
write another peer's state; stale revisions and oversized packets are rejected.
Leaving a lobby clears remote replicas. Map changes leave the lobby and reset the
native collision schema.

## Verification

Run headless checks (these do not start the game):

```powershell
cargo test --locked -p skate-net -p skate-mods -p skate-vehicles
cargo check --locked -p skate-game --bin skate3rust --no-default-features
```

Coverage includes loss/reordering, late joins, tombstones/disconnects, ten-player
record replication, spoofed owners and bounds, portable asset fingerprints, Lua
state transaction rollback, remote chassis collisions and crash ejection. Manual
follow-up: two PCs with matching enabled ZIPs; spawn separate cars, drive into one
another, hit a standing player, bail, reset, join late, disable/re-enable a mod, and
check trainer markers and mod mismatch messages. No game was launched as part of
these automated checks.
