> Historical v1 baseline. Production now uses v2. See [the optimisation results and next test](multiplayer-test.md) for the ten-player implementation and measured synthetic rates.

# Networking bandwidth audit — 8 September 2026

Verdict: bandwidth is workable for a two-player prototype on a connection with sufficient spare capacity, but this implementation is not yet optimized or qualified for internet P2P. Snapshot loss, stale queues, and collision latency need attention alongside bandwidth.

## Method

Inspected the game capture/send code, production binary serializer/reassembler, direct adapter, and Steam helper. Ran `cargo run --release --target x86_64-pc-windows-msvc -p skate-net --example bandwidth_audit --locked`. This is a nongame serializer harness. It does not launch a game, initialize Steam, or measure the user's internet connection. Raw results: `logs/multiplayer-build/bandwidth-audit.log`.

The current game captures up to 30 animation anchors, exactly 33 physical bodies, and up to 64 enabled collision primitives. Actual active primitive counts depend on gameplay state and board geometry. No live packet capture or per-session bandwidth counters are available in the existing logs. Therefore the scenarios below are explicit synthetic examples/bounds, not measured University gameplay averages.

## Results per player, with one other player

All kB values are decimal. Upload and download are approximately equal when both players send comparable snapshots at the same rate. Application bytes include the 36-byte header on every fragment, but exclude UDP/IP and Steam's encryption, transport headers, acknowledgements, and connection establishment traffic. The localhost IPC leg of the Steam helper is not an extra internet upload.

| Scenario | Bytes/update | Fragments | Upload at 30 Hz | Download at 30 Hz |
|---|---:|---:|---:|---:|
| 30 anchors, no active volumes; default appearance | 2,796 | 3 | 83.88 kB/s / 0.671 Mbps | 83.88 kB/s / 0.671 Mbps |
| 30 anchors, 32 capsules; illustrative example | 3,920 | 4 | 117.60 kB/s / 0.941 Mbps | 117.60 kB/s / 0.941 Mbps |
| 30 anchors, 64 boxes, maximum appearance string | 5,960 | 6 | 178.80 kB/s / 1.430 Mbps | 178.80 kB/s / 1.430 Mbps |
| Maximum accepted protocol: 32 anchors, 64 boxes | 6,020 | 6 | 180.60 kB/s / 1.445 Mbps | 180.60 kB/s / 1.445 Mbps |

The send gate is 32 ms of wall time, checked after fixed physics updates. Approximately 30 Hz is expected with smoothly scheduled 60 Hz physics, but it is not an exact independent network clock. Its long-run ceiling is 31.25 updates/sec. At that ceiling the accepted protocol maximum is **188.125 kB/s / 1.505 Mbps each direction**. IPv4+UDP alone brings it to 1.547 Mbps; Steam overhead has not been measured. The actual rate may be lower with frame stalls or batched fixed updates.

Frame payload size is `56 + appearance_bytes + 30*anchor_count + 52*33 + volume_bytes`. Each sphere costs 18 bytes, capsule 34, box 46, triangle 42. Each 1,000-byte payload fragment adds a 36-byte header.

## Problems found

1. Every update sends full absolute state, including unchanged state. There are no deltas, sleeping/idle bandwidth reductions, or quantized values. The same motion is described separately through body poses, final animation anchors, and world-space collider geometry.
2. A snapshot is unusable unless every fragment arrives. With independent 1% fragment loss, a four-fragment snapshot fails 3.94% of the time; a six-fragment snapshot fails 5.85%. At 5% loss those become 18.55% and 26.49%. These are mathematical scenarios, not observed Steam loss rates. Burst loss or transport packet aggregation changes the distribution. Steam's [unreliable-message contract](https://partner.steamgames.com/doc/api/ISteamNetworkingMessages) explicitly allows loss, duplicates, and reordering.
3. The Steam helper uses `UNRELIABLE_NO_NAGLE`, not the no-delay/drop-stale option. It discards `send_message_to_user` errors and does not inspect pending bytes, queue delay, RTT, or actual send rate. It cannot currently adapt its application update budget to a congested connection.
4. The bundled Steamworks SDK header documents a default 256K bytes/sec send rate. The computed application budget is below that, but this is not evidence that every internet path can sustain it. The application does not query the effective connection rate or establish real transport headroom.
5. The implementation admits two players only. These numbers are per peer relationship, not a constant budget for a future larger session. Routing and interest management would determine scaling.

## Recommended next work

Derive default collision geometry locally from body states and compact enable flags; exchange static appearance/rig information during admission. Quantize position, rotation, and velocity with explicit error budgets. Separate essential root/body updates from optional pose detail so one lost fragment need not discard everything. Add actual up/down counters, completed-snapshot rate, RTT/loss/queue metrics, send-error reporting, and a stale-snapshot/backpressure policy. Then qualify two-PC Steam play with simulated latency, jitter, and packet loss. Collision consistency remains a separate limitation of locally owned physics.

No gameplay behavior, packet format, or executable was changed by this audit.
