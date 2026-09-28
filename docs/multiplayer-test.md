# Multiplayer development


## Online characters

Current builds replicate the equipped outfit, including clothing/material choices,
face/body morphs, colours and tattoos, and all prepared pro/special/DLC character
identities. Each receiving installation resolves retail identities from its own
prepared character library. The same setup update that supplies the local
customiser supplies these online assets.

Imported characters transfer their converted, self-contained GLB automatically;
other players do not need to import the model. There is no installed-library scan
or local-file bypass. Direct and Steam use the same authenticated bulk stream.
The host forwards available chunks while downloading. A sliding selective-ACK
window replaces the old 32-chunk request/wait cycle. Each peer receives the current
model once; completed content is cached for later swaps, and a late join receives
the current selection. New selections cancel obsolete transfers.
Movement/collision packets are serviced first. Bulk traffic has no fixed
bytes-per-second cap: its acknowledgement window grows with successful delivery
and backs off on packet loss. Each service call is bounded to 512 KB per peer
and 2 MB overall to protect frame time and local IPC queues. Missing chunks retry.
Steam's configured maximum is the highest representable rate; its minimum is
unchanged, so this does not force traffic beyond Steam's connection control.
The relay no longer discards packets based on a once-per-second queue snapshot.
Movement retains Steam's NoDelay flag. Datagram queues hold slow render frames' traffic on both direct sockets
and Steam IPC. The Steam send-rate ceiling is raised from its small-message
default. The HUD shows character transfer progress.
Each remote keeps its visible character until the new scenes, textures and every
clothing rig are ready. A replacement is posed before visibility is published;
paused local swaps use the current skeleton and remote swaps use the latest
received pose (or the idle pose while waiting for a sample).
Imported textures retain their own materials; retail
characters use their authored clothing lighting. Remote colours/tattoos never
modify the local player or another remote's materials.

All players need this build to see appearances; older builds continue to display
their stock fallback. Missing retail content or rejected imports also retain a
fallback. Online imports are limited to 64 MiB, standard embedded PNG/JPEG glTF
content, 8192 pixels per texture dimension and 64 million total texture pixels.
The network cache is bounded to twelve maximum-sized blobs (768 MiB), with inactive
content evicted when needed; active actor content is retained for forwarding. Temporary imports
are separate from the personal character library and cleaned up on leaving/exiting.

Manual regression: use two PCs/accounts or local clients with separate settings.
Choose different clothes, skin colours, tattoos and body/face morphs. Verify both
views, then switch to a pro, a special/DLC skater, an imported model the other
player does not own, and back to the customised stock skater. Join a third client
after the selections; check it receives the latest appearances. Test a change
while downloading, leave/rejoin, and a host departure with three Steam clients.
Skate, bail, retrieve the board and enter vehicles; every remote clothing rig and
board should follow the existing network pose. Nongame tests simulate ten players
with loss, duplication, reordering, a late join and changes during transfer.

Build the game and Steam relay with `./scripts/build-multiplayer-test.ps1`.
The output is staged in `bin/multiplayer`.

Open the menu on a prepared installation:

```powershell
./scripts/launch-multiplayer-test.ps1 -AssetRoot <prepared-assets-directory> -MenuOnly
```

Use `-Players 2` through `-Players 10` instead of `-MenuOnly` for local clients,
`-TwoControllers` for XInput slots 0 and 1, and `-MapPath <map.skate>` to select
a map. The default map is University beside the supplied assets directory.
`SKATE3_ASSETS` can supply the asset directory instead of `-AssetRoot`.

## Steam lobby test (current build)

Use the menu launcher on each PC with matching assets/build and separate Steam accounts. Choose Host via Steam / Spacewar on one PC. On the other, choose Browse public Steam lobbies, select the matching University row, then Resume. Refresh updates membership counts. You can also type the numeric lobby code and use Join via Steam. Steam is initialized only by Steam multiplayer actions; offline play remains available when Steam is closed. After opening Steam following an error, refresh or retry.

Public discovery is filtered to this game's protocol namespace within AppID 480. Rows show map, occupancy and compatibility; incompatible maps/physics cannot join, and maps are not downloaded or switched automatically. Capacity is ten including the host. There is no friends list or invite UI.

Steam lobby-owner reassignment triggers automatic transport rerouting and a fresh game handshake among the remaining members. Each survivor keeps their own simulated skater and local capture history; remote presentation rebuilds after reconnection. The numeric lobby code stays the same. Test host departure with three players, then verify the two survivors resume and a new player can join. Graceful Leave and an abrupt host exit should both be tested; abrupt failure waits for Steam's membership detection, so migration is not instantaneous. The helper also exits after 15 seconds without its parent heartbeat. Direct UDP tests and legacy SteamID-session codes have no automatic election.

Twenty nongame tests passed, covering bounded discovery messages, Unicode labels, repeated ten-player session migration, local-state preservation, stale departed-host packets, production loopback packets, interpolation and physical contact. Release game/helper builds are staged. No game or Steam process was launched for this verification. Real public discovery, Steam relay connectivity and provider-driven migration still need the multi-PC test above.

> New visual interpolation test: [multiplayer-interpolation.md](multiplayer-interpolation.md). It describes timestamped protocol v3. This page retains the original optimisation results.

# Multiplayer optimisation / next test

Run the launch script with `-Players 2`. It launches two connected games on **University** using the same ten-player session implementation as online play. Player A hosts; B uses an unavailable outfit identifier and spawns two metres away. Click a window to control that player with raw XInput. `-TwoControllers` assigns slots 0 and 1. Steam is only initialized when choosing Steam multiplayer from the menu.

Use `-Players 10` to launch ten local windows. This is substantially heavier than one game with nine online players.

The HUD shows player count, application upload/download in decimal kB/s, worst current direct-link ping, stale packets, missing delta baselines and send errors. Steam sessions additionally show SDK transfer rates, worst link quality estimate, queue time and adapter drops/errors. Host ping is to guests; guest ping is to the host, not the full guest-to-guest path. Sequence gaps are not reported as packet loss because distance throttling intentionally skips source updates.

Test skating into each other, board impacts, tricks, bails, walking, board retrieval and switching window focus. Check the University scenery and remote pose/fallback. Logs are in `logs/multiplayer/<launch-id>/player-*.err.log`, including one `MULTIPLAYER_STATS` sample per second. Opening the menu keeps the online simulation running; replay remains disabled during multiplayer.

## Online setup

On one PC choose **Esc > Multiplayer > Host via Steam / Spacewar**. On each other PC/account enter the same host code in the multiplayer menu and choose Join. Up to nine guests can join. Map, physics and version fingerprints do not block joining. Different wire protocols may still fail to communicate; players on different maps can appear inside scenery or see different collisions. Visual outfit/rig differences use the default skater. Unknown identifiers never become asset paths. An incompatible visual hierarchy uses the default idle pose instead of foreign bone indices.

The host is an ordinary player, not a dedicated server. All guests connect to that host; it forwards the newest state to the other players. Transport, player IDs, codec, membership, budgets and rendering are independent of Steam. Spacewar/AppID 480 is the optional connection/relay provider. The game has no Steam DLL dependency and neither executable invokes Steam Input. Two processes under one Steam account still require the local launcher for testing.

Guests can reconnect; timed-out actors are removed and slots reused. The current Steam lobby browser and migration flow are described above. Legacy direct sessions require manually starting/joining another host after departure.

## Optimisations

- Version 2 separates physical state and visual anchors into independent datagrams. Full 33-body updates are 825 bytes normally or 1,023 bytes with every position outside the compact range. Thirty compact anchors use 426 bytes. There is no fragment reassembly or lost-fragment amplification.
- Millimetre root-relative positions, four-byte quaternions and half-float velocities. World roots and large relative offsets retain floats. Quantisation adds approximately 0.5 mm per axis plus float precision; quaternion error is bounded by the regression test at 0.006 radians for its samples. This is lossy replication, not deterministic simulation state.
- Per-recipient acknowledged deltas, 64-revision bounded histories and periodic full refreshes. The next delta does not depend on an unacknowledged lost packet.
- Static stock collider geometry stays local. Physical state carries an enabled-part mask and ragdoll flag. Nearby remote bodies join the existing solver with unique reaction indices; distant assemblies are excluded and primitive bounding spheres reject separated pairs before narrow phase. Proxy buffers and resolved remote animation samples are reused/cached.
- Guest capture targets 20 Hz physics and 10 Hz anchors. Host forwarding targets 20/10 Hz within 30 metres, 10/4 Hz within 100 metres and 2/1 Hz beyond. Detached boards are included in distance checks. Unchanged state slows further except nearby physical state. Guest uploads always retain the full rate so two guests near each other do not depend on proximity to the host.
- Physical updates take priority under upload pressure. A latest-state scheduler rotates actors and recipients; stale states are replaced, not queued. State budgets are 180 kB/s per link and 1 MB/s aggregate host; control traffic is additional. Crowded lobbies therefore reduce delivered rates to stay within the host budget.
- Steam state sends use unreliable/no-delay delivery, explicit drop/error counters and queue-pressure feedback. Handshake/control uses unreliable/no-Nagle so Steam can establish the connection. Controls repeat periodically. No reliable snapshot backlog.

Physical collisions still use local ownership with temporary remote proxies, not shared authority or rollback. Up to 50 ms translation prediction, 100 ms visual interpolation and stale-state expiry limit extrapolation; delayed impacts can remain asymmetric. Climbing retains its existing special movement path. This pass does not claim retail Skate 3 online synchronisation or competitive anti-cheat.

## Verification

Release game and relay built and staged using `scripts/build-multiplayer-test.ps1`, which launches neither. Ten nongame tests passed: existing wire validation, real UDP, quantisation/deltas, truncation, full/mismatched admission, spoof rejection, disconnect/rejoin and a real core-solver contact using the last of nine 33-body remote blocks. The launcher was parsed without execution.

`cargo run --release --target x86_64-pc-windows-msvc -p skate-net --example lobby_audit --locked` runs the production codec/session with ten simulated peers for 30 seconds per scenario, measuring the final 25 seconds. It introduces independent 30–90 ms delay per link, reorder and optional 5% loss. The near scenario changes every body and anchor continuously. It checks all ten memberships, bounded packet sizes, replication freshness, and zero invalid packets/missing acknowledged baselines. It does not run a game, the renderer, Steam or a real WAN.

| Synthetic ten-player scenario | Host up/down kB/s | Guest average up/down kB/s |
|---|---:|---:|
| Moving together, no loss | 1,025.51 / 205.81 | 22.87 / 113.95 |
| Moving together, 5% loss | 1,025.48 / 195.23 | 22.87 / 108.09 |
| Moving 150 m apart, 5% loss | 193.65 / 195.70 | 22.87 / 20.44 |
| Idle together, 5% loss | 189.56 / 36.75 | 4.30 / 20.01 |

These are application datagrams including controls, before UDP/IP/Steam overhead. Crowded host upload is approximately **8.20 Mbps**, guest upload **0.18 Mbps**, guest download **0.86 Mbps** with loss. The host upload is the limiting connection. Choose a host with headroom above those figures. Measured crowded maximum gaps between received body/pose updates were 350/590 ms under 5% loss; this is not a bound for arbitrary internet conditions.

The previous representative serializer used 117.6 kB/s per direction for one peer. The new busy guest upload is about 81% smaller despite the ten-player control overhead. Real game rates depend on motion, distance and relay conditions. In-game feel, ten real clients' performance and Steam connectivity still need the user-run test; no game or Steam process was launched during this pass.


## Integration with the main branch

The pause menu includes both Character customiser and Multiplayer. Offline map
changes use the in-process loader. Leave multiplayer before loading another map;
the loader cancels pending discovery/join requests and refreshes the map fingerprint
and physics schema before another session starts. Matching maps are recommended but are not required to join.

The build helper links into its own staging directory while reusing the dependency
cache. Supply your prepared asset view through `-AssetRoot`. If using a separate
session-marker overlay, set `SKATE3_SESSION_MARKER_OVERLAY` before launching.

The merged game and Steam relay were compiled without launching either. Actual
Steam connectivity still needs two PCs with separate Steam accounts. Current
appearance replication is described in the Online characters section above.

### Network stream regression (September 9)

The previous page exchange took approximately two minutes for the reported 32 MB
model. The installed-file shortcut has been removed. Production UDP sockets now
transfer 32,067,793 bytes through client -> host -> client in approximately 8.4 s
at a deliberately limited 15 FPS, with movement continuing and no model chunks
sent after acknowledgement. This is measured transfer time, not a claim of
instantaneous delivery or a Steam/WAN measurement.

Nine headless character tests pass, including the real owned import transferred
through UDP before Bevy loads it from the receiver's private source, binds the
rig and applies successive poses. Other checks cover male/female clothing rigs,
material isolation, swapped rigs, invalid GLBs, and ten peers with delay/loss,
reordering, late joins and changed selections. The transport adds explicit
origin-authentication, stale-generation and corrupt-content regression tests.
No game or Steam client was launched. Manual tests should check both screens,
including skating/bailing and switching back to stock. Logs distinguish download
completion (`ONLINE_CHARACTER_READY`) from a posed scene becoming visible
(`ONLINE_CHARACTER_VISIBLE`).
