# Retail player-audio drivers: recovered reference (2026-09-18)

Recovered from the lifted retail C++ (`C:\dev\skate3recomp\generated\skate3_recomp.*.cpp`), the
guest image dump (`<assets>\private\stock\audio-runtime-image\g_XXXX.bin`, page = `addr >> 16`,
big-endian), the owner's AttribSys vault (`tools/asset_pipeline/vlt.py` over
`skatercollections.vlt`, also `<assets>\private\stock\skater-collections.json`) and the retail
traces (`…\worktrees\51bd\…\probe\traces\sessions\*.audio-trace.log.gz`).

Packet length is always the constructor's object size minus the 4-byte header. Trace dump lines
print 28 words for posts and 17 for updates regardless of size; the rest is adjacent heap.

Three findings change the architecture:

1. **Every gain/pitch/spatial word is a MixMap output** (EA PathFinder 5.03 mixer, authored
   `data\audio\MixMapSK8.mxb`), read by owner vfuncs 52/56/60/64. See §2.
2. **Retail's continuous rolling sound is a granular player over `grains.big`** owned by
   `SFXObj_SkateBoard`, not `Class_rolling`. `Class_rolling` is a random one-shot scheduler. See §3.
3. **Every packet's gameplay inputs come from the audio state** filled per frame by the bridge
   `sub_824B0DA8`. See §1.

## Shared image constants

| Address | Value | Use |
|---|---|---|
| `0x8209975C` | 0.5 | air-time scale; `Motion+200 < 0.5`; grind/seam speed offset |
| `0x8208EA70` | 0.12 | 309/310 hold timer speed scale |
| `0x82181B90` | 0.4 | hold-time scale |
| `0x820C6D98` | 0.25 | average of 4 skeleton floats |
| `0x822F889C` | 4096.0 | pitch output scale |
| `0x82165A00` | 0.05 | owner +1168 slew step |
| `0x82165A10` | 0.0 | |
| `0x8231A844` | 1.0 | |
| `0x8216DEE0` | −1.0 | unset marker (+468) |
| `0x822F8898` | 1/32767 | |
| `0x82256FE8` | 1000.0 | ms, ×1000 conversions |
| `0x822F8F00` | 29.97 | dt → frames |
| `0x821747FC` | 32767.0 | |
| `0x822F8628` | 3.6 | m/s → km/h |
| `0x821161A0` | 10000.0 | |
| `0x8208EDA4` | 0.08 | skid/squeak/seam speed scale |
| `0x822F9408` | 166.667 | jump height → word |
| `0x822F9414` | 114.5916 | radians → squeak angle word |
| `0x820BD5C4` | 500 | time scale → word |
| `0x820ED57C` | 100 | seam distance scale |
| `0x8220E13C` | 50 | seam rear-wheel extra travel |
| `0x82256FD8` | 90 | skid slip scale |

## 1. Audio-state bridge `sub_824B0DA8`

Virtual per-frame `Update(this = audio state, f1 = dt)` (only in the function tables, init
`sub_824B0C48`); ends with `sub_824B19C8(state, dt)` which writes the state's MixMap controller
inputs. Record: base `*(0x82083C38) + 0x2F070`; record = base + 240 + 544 × `[[state+36]+68]`
(skipped if −1 or ≥ `base+12`). Written by `sub_827A1B78` (`.35.cpp`, from frame builder
`sub_827A11B0`) from the PhysOut bundle `[[player+1808]]->vfunc92()` (B): B+4 Motion, B+8 Air,
B+20 Skeleton, B+24 Collision, B+28 State, B+32 Ground, B+36 SystemReckoning, B+52 Interaction,
B+72 OffBoard, B+40 unnamed (board dynamics), B+60 unnamed.

| State | Type | Source → math | Engine equivalent |
|---|---|---|---|
| 48/64 | vec | `[B+36]+64` position; old 48 → 64 | `reckoning.vector_64` |
| 324 | f32 | \|pos − `*[0x820CFDD4]`\| listener distance | none |
| 96/112/128 | vec | COM vel `[B+36]+16`; prev; delta | `reckoning.vector_16` |
| 200 | u32 | Collision `wheel_count_0` (R152 >> 20 & 7) | `physical.collision.wheel_count_0` |
| 204 | f32 | Ground+264 | not mapped |
| **208** | f32 | **Motion+164 ground speed** | `physical.skateboard.scalar_164` (not `board_speed`) |
| 212/216 | f32 | \|COM vel\|; prev | ≈ `rider_speed` |
| 220/224 | f32/u8 | time scale; paused/replay flag | none |
| 192 | u32 | `[B+40]+28` grind type | none |
| 228/232 | f32 | `[B+40]+32`; `[B+40]+20` = **slip** (PhysOut+116) | none |
| **236** | f32 | Air+176 `time_in_state` | `physical.air.time_in_state_176` |
| **240** | f32 | Air+184 time until landing | `physical.air.scalar_184` |
| **260** | f32 | Air+200 `jump_height` | `physical.air.jump_height_200` |
| 264 | f32 | Motion+184 (signed slide angle) | not published |
| 328 | f32 | \|Skeleton+288\| | none |
| 332 | u8 | Air byte438 (in known air) | none |
| 337 | u8 | State56 = flags2468 bit27 (push event present) | `state_flags[4]` |
| 338 | u8 | State57 = bit26 (push on right toe) | `state_flags[5]` |
| 334 | u8 | P && 338 (P = State55 = bit25 push planted) | `state_flags[3]` |
| 333 | u8 | P && !338 && 337 | derived |
| **335** | u8 | **P && !prev334 && !prev333** (computed before 333/334 update) — rattle trigger | derived |
| 336 | u8 | State52 = bit30 brake planted | `state_flags[0]` |
| 339 | u8 | State54 = bit29 ManualBrake | `state_flags[2]` |
| 340 | u8 | State60 balance ≠ 0 | `state_flags[8]` |
| 341/342 | u8 | `[B+40]` byte208 (grinding); prev | none |
| 343/344 | u8 | trick active (`[B+60]+152` ≠ −1); asset flag | none |
| 348/352/368 | u32 | audio trick id / last trick id (trick vault) | none |
| 464–467 | u8 | `[B+40]` bytes 68–71 per wheel | none |
| 480 | vec | R+272 = `[B+40]+0` board rotation rates | none |
| 528–548, 560–580, 593 | | body-part contact slides, materials, flag | none |
| 612–616 | u8 | Collision 3473/3474/3475; Skeleton 600/601 | `flag_3475` only |
| 620–660 | u32 | per-wheel material (surface−1, 0..143, 143 = none) | `WheelLineState.audio_surfaces` |
| 636/648 | u32 | seam pattern per wheel | `surface_tag >> 12 & 0xF` (discarded in `board_ground.rs:84`) |
| 672 | f32 | 0.25 × Σ Skeleton+560..572 | not published |
| 676/677 | u8 | State59 bail; Skeleton 599 end-of-bail | `state_flags[7]` for 676 |
| 684 | u32 | Motion+200 < 0.5 (flag used as w11/w8 by several) | none |
| 690 | u8 | State66 | `state_flags[14]` |
| 692 | u32 | `[B+40]+40` material | none |
| 716/718/720 | | state == 500; `[B+64]+0` == 7; countdown | `state` |
| **724** | u8 | OffBoard byte307 \|\| Air byte450 \|\| 334 \|\| 336 | `off_board.flags_306_307[1]`, `air.footplant_right_450` |
| **725** | u8 | OffBoard byte306 \|\| Air byte449 \|\| 333 | `flags_306_307[0]`, `footplant_left_449` |
| 728/732 | u32 | foot materials | different path today |
| 780 | u32 | (R156 >> 5) & 3 → loose-board slide state | none |
| **796** | f32 | Interaction+0 `AudibleFootStepStrength` | `footstep_strength` |
| 308/309/310/312/316 | | OffBoard 311/309; hold timer: 310 = 312−316 > (1 − min(1, v × 0.12)) × 0.4 | `off_board.flag_311` |

### State MixMap controller inputs (`sub_824B19C8`, into `[state+12]`)

v = state+208 ground speed.

| Id | Value |
|---|---|
| 0 | clamp(v × 23592.24, 0, 32767) (saturates 5 km/h) |
| 1 | v × 3932.04 (30 km/h) |
| 7 | v × 2359.22 (50 km/h) |
| 8 | v × 1685.16 (70 km/h) |
| 14 | state212 × 1685.16 |
| 2 | 32767 if wheel count (200) == 0 |
| 10 | wheel count 1/2/3/4 → 8191/16383/24575/32767 |
| 4 | 336; 5 = 339; 6 = 343 (each 0/32767) |
| 9 | 0 if `[state+16]+72` else 32767 |
| 11 | global flag |
| 13 | 32767 × slewed distance / max (`state+784`, rate `0x0395642CA6FC543A` × dt, cap `0x204DCC9296DC9400`) |
| 3 | listener-facing factor `sub_824B2088` (param 3 via bridge) |
| 12 | `sub_824B23C8() == 1 ? 32767 : 0` |

## 2. MixMap (EA PathFinder 5.03, `packages\engine\audio\path\5.03.00-sk8`)

- **Asset**: `data\audio\MixMapSK8.mxb`, 25,252 bytes, big-endian, loose file outside
  `audiofiles.big`. Present in the recomp's `out\build\windows-release\game\data\audio`; **not in
  the owner install's `private\stock\data\audio`** — the installer must stage it. Header: +4 = 14
  sections, +8 = 0x10, +0xC = −1, section offsets 0x44, 0x2A48, 0x42D8, 0x4418, 0x461C, 0x4B1C,
  0x534C, 0x54E0, 0x5640, 0x5850, 0x5BA8, 0x5CE4, 0x5D04, 0x5E10. Record kinds in top 3 bits;
  many −10000 (mB silence).
- **Loader**: `sub_82484FE8` → `sub_8294B918` (manager at `sys+4`), loads the file with
  `sub_8298ED88` (name at `0x8224CEC8`), data at `sys+580`, `sub_8294BA18(manager, data)` builds
  the 564-byte host (vtable `0x82316B78`). Builders: `sub_8294BDC8`, `sub_8294C548`,
  `sub_8294CC48`, `sub_8294E120`. Per frame: host slot 2 `sub_8294F5E8(host, dt)` from
  `sub_82485190`.
- **Controller** (16 bytes, vtable `0x82316B54`): +8 Set(id, v) `[ctrl+8][id] = v` (32-bit
  inputs); +4 SetFloat; +12 Get; +16 GetOutput(id, type) from packed int16 outputs at
  `[ctrl+12]` (id n in word n>>1, shift (n&1)×16): type 0 cents → 2^(c/1200) × 4096, 1/4 & 0x7FFF,
  3 & 0xFFFF; +20/+24 enable flag `[[ctrl+12]+60]`; +28 set inputs ptr; +32 set outputs ptr.
  `sub_8294CB48(host, key, outBlock)` finds/allocates by key (array host+160, count host+180,
  storage host+164, key at ctrl+4), sets ctrl+12, hands it to `[host+108]->vfunc4(ctrl)`
  (**how it reaches each component's owner+12 is unconfirmed**).
- **Evaluation `sub_8294F5E8`**: host+132 = dt, +136 = dt×1000, +556 = dt×29.97.
  1. Input stage (host+208 entries × 16 B at host+388): reads `[[e+12]]`, shapes with curve type
     `e+8 & 0xF` via `sub_8294B668` (linear, 1−x, x², inverted, or 512-entry table at
     `0x82FDC7B0`), stores e+20, converts to mB via log table `0x82FDBFB0`.
  2. Product stage (host+464 at host+400): (a×b)>>15 chains → +8.
  3. 2-D range/table lookups `sub_82951658`.
  4. Envelopes: `sub_82950250` → `sub_82951440`, `sub_82951148`, `sub_82950D40` (states 0–4,
     attack/hold/release ms at +32/+40/+44, advanced by host+136). **This is the smoothing.**
  5. Sum stage (host+472 at host+436), clamped per entry.
  6. Output write `sub_8294FC88` (host+476 at host+448): dest id bits 26–30, source bits 21–25,
     signed 16-bit offset, type header bits 24–27: 0 volume mB → linear 0..32767 (602 mB per
     halving, floor −10000); 1 cents clamp −4800..2400; 2 mB −10000..0 via `sub_8294B4D8`; 3 raw;
     4 clamp 0..25000.
- **Owner reads**: owner vtable `0x822FC770` slot 13 (vfunc52) `sub_824C2870` u16; slot 14
  (vfunc56) `sub_824C5910` cents → ×4096; slots 15/16 (vfunc60/64) `sub_824AF240` & 0x7FFF.
- **Owner controller inputs** (SkateBoard `[owner+12]`): id 0 surface-change pulse
  (`sub_824C5CA8`), id 4 = (333||334) ? 32767 : 0 (`sub_824C6198`), id 6 = surface(slot 0) == 9.
- Recorded rolling w11 dynamics (for tests): rise 747 → 5258 → 11282 → 12954 over 3–4 updates;
  contact loss 12999 → 10700 → 8402 → 6100 → 3800 → 1500 → 0.

## 3. Board owner `SFXObj_SkateBoard` and the grain player

- Constructor `sub_824C5058` (vtable `0x822FC770`): loads 13 `.grain` members via
  `sub_824C5BF8` → `sub_828DC158` into slot pairs 0..25 (`grains.big` mounted at 0x83020C2F), and
  the four `PatchBank_*` banks into slots 29..32. Per-surface vault record class
  `0x7AB23C11B6ADA2DE`: grain filename, max speed 55–74 km/h, 4×4 Bezier control points,
  `GrainPlayer::GrainParams`, rise/fall steps, filters.
- Surface routing `sub_824C5CA8` (per truck, on `sub_824C82A8` surface change): surfaces 7, 8,
  10–13 release/repost a per-surface `Class_rolling` (selectors 1, 2, 10, 12, 11, 9); surfaces
  1–6 and 9 go to grain players (`sub_824C8370` picks soft/hard by `sub_824B23C8`): 1
  asphalt_rough, 2 concrete_rough, 3 asphalt_smooth, 4 concrete_smooth, 5 wood_ramp, 6
  concrete_aggregate, 9 metal_smooth. 14 = no contact (byte 341, or a lifted truck in a manual) →
  stop that truck's grains. Two players per truck at owner+1176+8i / +1180+8i; `sub_828EC040`
  binds grain data, GrainParams copied to player+16..+32.
- Per frame `sub_824C6BD8`: record {gain, pitch, 0, position} per player. Position = cubic Bezier
  of t = clamp(v × 3.6 / maxKmh, 0, 1) with vault control points at attribute +52/+36/+20/+4
  (3.0 at 0x82063B08); second player position − 0.1 (0x820641A8) clamped ≥ 0. Gain =
  vfunc60(1)/32767 × (1 − max(+1164, +1168)) × optional vault/+1456 multipliers; second player
  mixes vfunc60(2), +1164, +1508. Pitch = vfunc56(3)/4096. `sub_824C8588` slews a speed intensity
  with per-surface vault steps (0.06); `sub_824C9058` pushes per-surface filter values.
- **GrainPlayer** (`sub_828EBD88` class, 372 bytes, `.47.cpp`; ported in
  `skate-audio-core/src/grain/player.rs`, layout confirmed bit-exact against the retail capture).
  Object: `+0..+12` the owner's record `{gain, pitch, 0, position}`; `+16..+32` GrainParams
  (attack, sustain, release, search window, drift threshold; constructor defaults 0.01, 0.5, 0.01,
  4.0, 0.05); `+36` hold byte; `+40` send target; `+44` bound byte; `+48..+64` grain data, header
  length, duration, seek table (`data+8`), EAAC stream (`data+H`); `+68..+84` classes SnP1, Rsp0,
  GaF0, Sen0, Gai0 (Gai0 unused); two 28-byte voice slots at `+88`/`+116` (`+0` module table, `+4`
  graph, `+8` state timer, `+12` start s, `+16` position at pick, `+20` state 1/2/3); `+144` active
  slot; `+148` the 24-byte scheduler instance; `+172` 16 recent-window entries `{start, end, u16
  next}`; `+364/+366/+368` used head, last inserted, free head.
- **GrainParams** (vault `0xD18D1174735E5CDE`, only in `default`): an array `{u16 capacity 2, u16
  count 2, u16 stride 0x14, pad}` of two 20-byte elements of five floats. Element 0 = A players
  (0.1, 0.2, 0.1, 1.6, 0.05), element 1 = B players (0.2, 0.1, 0.2, 1.5, 0.05); `sub_824C5CA8`
  copies element `which` (lookup `sub_82B72420(key, which)`) to player+16..+32.
- **Grain members per surface** (`sub_824C8370`, soft when `sub_824B23C8 == 1`; filename = vault
  layout `+64`; slots A/B = `owner+40+4·slot`, both players get the same file): 1
  `asphalt_rough_soft` (key `943B1CB05BA9A6BA`, 14/15) / `asphalt_rough_hard` (`7EB8015B4C02405E`,
  0/1); 2 `concrete_rough_soft` (`B29FADBBBC2F39C2`, 16/17) / `_hard` (`03721D0FA99A03C8`, 2/3); 3
  `asphalt_smooth_soft` (`DC1009D50E327F8F`, 18/19) / `_hard` (`7C5912FC2DABF98C`, 4/5); 4
  `concrete_smooth_soft` (`607A6BC3D427DA49`, 20/21) / `_hard` (`FFB5E3E62E0B4943`, 6/7); 5
  `wood_ramp_soft` (`382B12636ED9D8DA`, 22/23) / `_hard` (`7947A259F181FDB4`, 8/9); 6
  `concrete_aggregate_soft` (`863C58AC34BAD599`, 24/25) / `_hard` (`B303AED8241530E2`, 10/11); 9
  `metal_smooth_hard` only (`1C9C52CC0E1CD4CF`, 12/13). Layout: `+0` 4×4 Bezier matrix (column 1
  of rows 0..3 = P3..P0), `+64` filename, `+68` max km/h, `+72` B boost gain, `+76` boost ramp
  km/h, `+80` A shift per boost, `+84` intensity cap, `+88` B base shift.
- **Pick** `sub_828ECAB0`: grain length L = attack + sustain + release; target T = (duration −
  window) × position; the free gaps of the start-sorted recent list (initial sentinel {−2, −1}),
  clamped to [T, T + window], are cut into back-to-back windows of L (`sub_828ECA08`, ≤ 64);
  index = int(rand × 2⁻³¹ × 0.5 × count) with `sub_82A8AF10` (a global add-with-carry generator
  at `0x82FD7D74` shared by ~100 call sites, so retail pick sequences are not reproducible); no
  candidate or last entry 15 → the list collapses to its last entry and, if window/L > 1, the
  search repeats, else T is returned. The capture's `GP ret` value is the window end, not the start.
- **Per-block plug-in**: `sub_828EBE68` registers `player+148` in scheduler bucket 0
  (`sub_82481BE0`: pool `system+112`, 74 nodes, process `sub_828ECE98` → `sub_828EC6F0`, context
  the player, name "Grain Player"); the block driver `sub_82B48530` runs bucket 0 in phase 1,
  before the command drain, with f1 = `[system+176]` = `0x3BAEC33E` = (f32) 256/48000 (pinned by
  the capture's attack timer 0.1 → `3DAC0831` → `3D962FC9` …). Drift: if |slot[active].+16 −
  position| > `+32` (and hold clear) the other slot is released if busy, else the active one
  releases and a new grain starts in the other. Each live voice: gain → Sen0 prop 0, pitch → Rsp0
  prop 0 (stamp `0x82B463A8`); timer −= dt (sustain: −= dt × pitch, fused); attack → sustain;
  sustain → release, stop the other slot, pick, start there; release → stop (deferred
  `0x82B49238`).
- **Voice graph** `sub_828EC3F0`: `SnP1 → Rsp0 → GaF0 → Sen0`, 1 channel, order 0; SnP1 param 5
  with sample = EAAC stream, detail = seek table, start = clamp(start, 0, duration) as a double at
  block+16; fader to 0 at once, then attack (fade to 1 over `+16`); Sen0 → `+40`; gain stamped.
- `.grain` file: `+0` header length H (112..176), `+4` duration (f32, stored; 3 members are 1–3 ulp
  off num_samples/rate), `+8` seek table `00 10 0180 00000018` + run-length varint columns (reader
  `sub_82B470D0` / `sub_82B474B8`; in every grain row 0 spans the whole single-block stream, so a
  seek is "restart at 0, 384-sample preroll, skip target − 384"), `+H` mono XMA EAAC at 44.1/48
  kHz, 12–22 s, one block. The play command's start frame is `fctiwz(rate × start)`.
- **Bus chain** (`sub_824C8878`, per player, record at `owner+1192+24k`; ported in
  `grain/chain.rs`): graph 1 (order 2) `Sub0 → HI20 → LI20 → FrequencyShiftSsb(arg 0.0) → Sen0(→
  graph 3, level +1556 = 0) → Gai0 → Sen0(→ graph 2)`; graph 2 (order 5) `Sub0 → Sen0(→
  [[manager+116]], level 0) → Sen0(→ [[manager+52]]) → Pn21 (6 ch) → Sen0(→ eEQChain bus 8 = the
  default bus)`; graph 3 (order 3, local player) `Sub0 → DCl0(+1528 = 0.09) → Gai0 → HS20(+1548 =
  5000 Hz, +1552 = 0.65) → Sen0(→ graph 2)`. Voices send to graph 1's Sub0. FrequencyShiftSsb
  (`sub_82B22898`) = two cascades of two allpass biquads (Hilbert I/Q) then I·cos φ − Q·sin φ
  (`sub_824531C8` / `sub_82473930`), φ += 2π·shift/rate. `sub_824C9058` per truck: HI20 ←
  vfunc64(12), LI20 ← vfunc64(11) (capture 77 / 24971), FSS A ← (latch ? 150 : 0) + [+1152 if
  +1156 clear] + boost × `+80`, FSS B ← `+88` + [+1152] + boost × `0x7FFF3A8AD44809EF`, Pn21 ←
  vfunc52(0) × 360/65535, env send ← vfunc60(13)/32767, local sends ← vfunc60(21)/(22)/32767.
- **Record inputs resolved**: `owner+912`, `+1036`, `+1340` are 124-byte segment envelopes
  (`sub_8248D368` reset, `sub_8248D3C0` add, `sub_8248D498` add chained, `sub_8248D510` advance);
  `+1028`/`+1032` = the `+912` envelope's value/idle byte, `+1152`/`+1156` the `+1036` one's. On
  every push (state `+335`) `sub_824C6198` programs `+912`: current value (or 1.0) → 1.4 + t·(1.1 −
  1.4) over 35 ms, hold 200 ms, → 1.0 over 600 ms; and `+1036`: 0 → −52 + t·32 Hz over 30 ms, hold
  200, → 0 over 600 (t = clamp((v − 1)·3.6/45)); both advanced by dt each frame while not idle.
  So the grain speed is scaled by the push envelope while it runs. `sub_824CA688` (`+1504`): set
  while state `+340` (balance ≠ 0, manual), cleared once wheels in contact (`+200`) is 0 or 4.
  `sub_824CA6E0` (`+1505`): set while state `+372` (bit 23 of the airborne trick packet word
  `+152`, refreshed only while `+332`), cleared once `+615 && +616` (feet in the deck box).
- `Class_rolling` patch: random-range ops, shuffle bags, timer, window latch, oscillators, ramps,
  curves, one voice op; samples 0.09–0.75 s, loop bit clear. Retail opens ~25–30 per 166 s.
- Rolling updater `sub_824C9948` (holders +1304 layer 0, +1308 layer 3): w0 32767; w1
  vfunc52(0) 0..65536; w2 vfunc56(8) 0..8192; w3 speed = fctiwz(clamp(v/maxSpeed(layer) × 3.6,
  0, 1) × 10000) (maxSpeed vault `0x880C82E8EF647EC4` = 70 for layers 0/3); w5 0; w6 surface
  (both trucks 14 && 341 → 13); w7 = 340 || 339; w8 vfunc60(19); w9/w10 vfunc64(17)/(18) 0..25000;
  w11 = vfunc60(7) layer 0 / vfunc60(9) layer 3, 0..32767.
- Rattle updater `sub_824C80C0` (holder +1300): w0 32767; w1 vfunc52(0); w2 vfunc56(3); w7
  vfunc60(16); w8/w9 vfunc64(14)/(15); w10 vfunc60(6).
- Rattle poster `sub_824C6198` (from `sub_824C6A78` after `sub_824C5CA8`): every frame owner
  controller id 4 = push-foot planted; on 335: EQ objects +912/+1036 from lerps of t =
  clamp((v − 1) × 3.6 / A) (A = `0x2D751DEB_89BB5E33`), release +1300, and if the slot's enable
  `owner[1320 + 4 × slot]` (0 for surfaces 7, 9–13) post `sub_824B0248(speed, surfaceCode, tweak)`:
  speed = fctiwz(clamp((v − 1) × 3.6 / D) × 10000), D = `0x12275AA8AC4A63FB` = 30; surface code
  1..5 from owner+760 key vs `0x7C5912FC2DABF98C`, `0x03721D0FA99A03C8`, `0xFFB5E3E62E0B4943`,
  `0x7947A259F181FDB4`, `0xB303AED8241530E2`; tweak `0xC04832978CDED925` = 8. Then filters tick,
  owner+1164 = `sub_824C8588()`, owner+1168 slewed when 336.
- `SFXObj_Wheels` (`sub_824CD6F8`, update `sub_824CDC70` → `sub_824CDD28`): `wheels.big` holds
  `Whls_spins_Jump_1.snr`, `Whls_spins_Man_1.snr`; fires on rising 332 / 340 plus a manual combo;
  spin-down with gain × (1 − speed/max).
- `SFXObj_Moving` (`sub_824E48D0`) is the world moving-object emitter, not player.

## 4. Existing families — retail words

Components: vtable +36 poster/releaser (tick 1, dt), +40 updater (tick 2). Tuning holder
`*(0x830CFDA4)` (`sub_8289D5C8`): +40 grind material class `049861E8F9A8D16B`/default, +56
rolling `7B0ED922C779B74C`, +64 AudioSurfaceMap `C489459A0C07D154`, +68 Treatment
`EE7B8A8A893A4E30`, +72 tricks `1FA8AC006CABEF59`, +140 eEQChain class `42AFE160E647167C`.

### Class_Treatment (22 words, 92-byte obj; ctor `sub_824B0080`, poster `sub_824DD408`, updater `sub_824DD6F0`)
Posted once, never released. Companion `hall_of_meat_slo_mo` (15 words, `sub_824AF368`) at +40.

| w | ctor | updater |
|---|---|---|
| 0 | 0 | vfunc60(2) 0..32767 |
| 1, 2 | 32767, 0 | — |
| 3 | 0 | vfunc52(0) 0..65535 |
| 4 | 4096 | vfunc56(1) 0..8192 |
| 5, 6 | 25000, 0 | — |
| 7 | 0 | clamp(trunc(state236 × 1000), 0, 10000) |
| 8 | 0 | clamp(trunc(state240 × 1000), 0, 10000) |
| 9 | 0 | trunc(clamp(state260 × 166.667, 0, 1000)) |
| 10 | 500 | clamp(trunc(state220 × 500), 0, 1000) |
| 11 | 0 | G+16 ? clamp(trunc(G+24 × 1000), 0, 1000) : 0 |
| 12, 13 | 0 | G+164 ? (1, clamp(trunc(G+168 × 10000), 0, 10000)) : (0, 0) |
| 14 | 0 | state224 == 0 |
| 15 | vault `32F9111CBF746F34` = 7000 | — |
| 16 | `E4BC6FE030A553C4` = 28000 | — |
| 17 | `1F11951C2AF58CC7` = 32767 | — |
| 18 | bool class `11A631878B239355` via `sub_82484460` | — |
| 19 | owner+72 && owner+64 == 0 | — |
| 20 | owner+72 | — |
| 21 | 8 | — |

G = `*(*(0x83083C38) + 0x2FCB4)` (reset by `sub_8279E800` → `sub_827AB6E0`), unidentified. In play,
w13 = 10000 most of the time. The captured "landing curve" is the air phase (w7 + w8 = 702).

### Class_Flips (28 words, 116-byte obj; poster `sub_824CBFB8` via `sub_824CBEB0`; ctor `sub_824AFAD8`; updater/release `sub_824CC7D8` via `sub_824CBF78`)
Post when component+36 == 0 and ((332 && 343) || (!332 && owner && 310)); 310 forces id 34; ids
−1, 35, 36 never posted. Keep while (332 || (owner && 310)) and id unchanged; else release (next
frame reposts). rate(a, D, T) = v ≥ T ? v : 0, v = min(trunc(|a| / D × 1000), 1000).

| w | ctor / updater |
|---|---|
| 0 | 0 / vfunc60(1) |
| 1, 2 | 32767, 0 |
| 3 | 0 / vfunc52(0) 0..65536 |
| 4 | 4096 / vfunc56(2) 0..8192 |
| 5 | 25000 / vfunc64(3) 0..25000 |
| 6 | 0 |
| 7 | rate(state488, `8EDBACCBA6FE46AD` = 10.2, `9A0316625B63CD99` = 603) |
| 8 | rate(state484, `02D39586635FB1A3` = 4.4, `EE157886DE5D3C97` = 703) |
| 9 | rate(state480, `1494BB20854C155C` = 3.0, `5C73CF6A0D50C8D8` = 297) |
| 10 | clamp(trunc(state220 × 500), 0, 1000) |
| 11 | trick id 0..40 (ctor only) |
| 12 | 0 / clamp(component+72, 0, 1000): `sub_824CD170` target 1000 if `sub_824898C8`, else container+32 flag 0x8000 → 250, 0x4000 → 700, 0x2000 → 1000, else 0; slew +10000/s (`57AED5FBC374D8F1`) up, −1000/s (`6B57BD44C0E0B267`) down |
| 13–15 | 0 |
| 16 | state224 == 0 |
| 17 | `99E6FF024834E4C7` = 0x6BFE |
| 18 | `D2D0EBAC43842F6D` = 0x3A00 |
| 19 | `13C155A181A55BA4` = 0x1A00 |
| 20 | 0 / vfunc60(8) |
| 21 | `B565D4D763128252` = 0x2710 |
| 22 | 0 / vfunc60(7) |
| 23 | bool class `11A63187…` via `sub_824844B8` |
| 24 | owner+72 && owner+64 == 0 |
| 25 | owner+72 |
| 26 | owner ? vfunc60(6) |
| 27 | eEQChain `D9BE1F2F1A72FEE8` = 6 |

Trick ids (vault `eSk8AudioTricks` class `6918469984A8C596`, field `8C3025DB4D1761AF`, collection =
hash64(lowercase trick name), names table 0x820862A8 stride 24): kickflip/kickflip2..4/
latekickflip/darkslideout_* 0; heelflip/…/lateheelflip 1; popshuvit 2; fspopshuvit 3;
varialkickflip 4; varialheelflip 5; hardflip 6; inwardheelflip 7; 360popshuvit 8; fs360popshuvit
9; 360flip 10; laserflip/superman 11; 360hardflip 12; 360inwardheelflip 13; n_kickflip …
n_360inwardheelflip 14..27; ollie/handplants 28; nollie 29; *_darkcatch, late shuvs 30;
fsfastplant/footplants 31, 32; boneless/bsfastplant 33; nocomply 34; grabs 35; coffin 36;
fingerflips 37; underflips 38.

### Class_grind (17 words, 72-byte obj; ctor `sub_824AF8C8`; poster `sub_824C28B0`; updater `sub_824C39E0`)
While 341: recompute speed; post layer L into +36 and, for grind type 0, a layer-1 companion into
+40; surface 14 → no post. Release both on grind end. Layer: family 1/2/4 → 0, 5 → 3, else 2.
speed = min(trunc(clamp(3.6 × (v − 0.5) / 45, 0, 1) × 10000), 9000) (45 = `4890392C91829954`).

| w | ctor | updater |
|---|---|---|
| 0 | 0 | 32767 |
| 1 | 32767 | clamp(trunc((vfunc60(1) & 0x7FFF) × F[surface][layer]), 0, 32767) |
| 2 | 0 | vfunc60(5) |
| 3 | 0 | vfunc52(0) |
| 4 | 0 | vfunc56(2) |
| 5 | 25000 | vfunc64(3) |
| 6 | 0 | vfunc64(4) |
| 7 | speed | speed |
| 8 | 1024 | — |
| 9 | surface 0..14 | — |
| 10 | layer | +36: layer; +40: 1 |
| 11 | trunc(V[surface][layer] × 32767) | — |
| 12 | bool class `11A63187…` via `sub_824843B0` | — |
| 13 | owner+72 && owner+64 == 0 | — |
| 14 | owner+72 | — |
| 15 | owner ? vfunc60(6) | same |
| 16 | eEQChain `D489344CEDEE5036` = 5 | — |

Surface = 4 if state692 == 143, else AudioSurfaceMap[material].+16 (`sub_82494E18`, key
`4CA607558B1CF440`, default entry 94); ≥ 14 → 4; then indexes table 0x82249F90. V keys (layers
0..3) `0ECECDAC28B2B979`, `58070BF511809903`, `72BA0780A8FA25D6`, `721A50C80028AD69`; F keys
`69969AF1BE6BB367`, `0555484D6D4A3128`, `C21983A2160ED3AC`, `69A5FC53091ED1F8` (default 1.0; mostly
1.0, layer 3 0.5–1.0, surface 9 = 0).

| Surface | L0 | L1 | L2 | L3 |
|---|---|---|---|---|
| 0 | 0 | 23919 | 32767 | 32767 |
| 1 | 32767 | 0 | 25230 | 32767 |
| 2 | 32767 | 11140 | 23919 | 32767 |
| 3 | 17366 | 10157 | 20970 | 32767 |
| 4 | 11796 | 18349 | 21626 | 32767 |
| 5 | 23919 | 15728 | 26541 | 32767 |
| 6 | 17366 | 18021 | 27196 | 22936 |
| 7 | 12779 | 5242 | 15728 | 32767 |
| 8 | 12451 | 983 | 16383 | 32767 |
| 9 | 0 | 0 | 0 | 32767 |
| 10 | 16383 | 8191 | 32767 | 32767 |
| 11 | 32767 | 32767 | 15400 | 32767 |
| 12 | 32767 | 16383 | 32767 | 32767 |
| 13 | 21298 | 11140 | 20643 | 24575 |

### Boot utilities
Each allocates 8 bytes and posts object+4 (1 word, never written; allocator `sub_828E2730` doesn't
zero). No updater/release. `c_emitter_utility` `sub_824A1420` slot 0x8302EF58; `Start_up_Play_ctl`
`sub_82486AC8` slot 0x8302EEA8; `c_foley_utility` `sub_82488120` slot 0x8302EEC8. The traced words
3/7/11… are adjacent heap links. **Check each class's declared input word count before changing
the Rust 28-word posts** — `ON_PAYLOAD` copies the class-defined count.

### Footsteps (`sub_824B73E0` / `sub_824E9FD8` / updater `sub_824EAEA8` / `sub_824E9270`)
See `docs/wheel-audio-handoff-2026-09-17.md` addenda; foot down = 724/725 above.

## 5. Missing families

Class vtable slots 7 create, 8 teardown, 9 process, 10 rewrite+redeliver. Posts
`sub_828E2B48(0x8302EE28 + 8i, obj+4, obj)`, release `sub_828E2C78` + `sub_828E27B0`, redeliver
`sub_828E2D18`. Surface table vault `0x4CA607558B1CF440` (95 entries × 72 B): +4 rolling, +12
skid, +20 foot drag, +24 (`sub_82494F58`), +32 Seams, +36 Seams transition (+5), +40 body slide.
Material ≥ 143 = none; 94..142 → entry 94 (rolling 3, skid 2, foot drag 0, seams 2, transition 0,
body slide 2). Priority (posts/min play4/play1/play2): Seams held (291–690 triggers/min); skid
7.7/17/13; wind 12/18/7; cloth_trick 6/13/1.4; SoS rattle 4.7/15/7; squeaks 2.5/16/5.7; foot drag
1.6/1.9/0.3; body slide 2.1/3.4/2.6; cloth falls 0.9/0.8/1.7; board slide 0.9/1.5/2.0.

### Class_Seams (`sub_824AFDD0`, slot 6; 84-byte obj, 20 words; create `sub_824C13D0`, process `sub_824C14C8`, update `sub_824C1F18`)
Ctor: w1 32767, w4 4096, w5 25000, w10 surface 0..8, w11 flag 0..1, w14 wheel 0..3, w19
`5A837C613E3F41DC` = 8, rest 0. Four held (one per wheel, owner+52..+64), released only at
teardown. Process (gated `[[owner+16]+52]`): w7 = 0 each frame; airborne clears history;
material change on any wheel (state 620+4i) → `sub_824C1DF8(i, 0, 1)`; else if v >
`F7BCA67F0A1FC92E` (0.05) → `sub_824C1698(axis 0/1)`. Trigger: w7 alternates 1, 2 (toggle
owner+68+i, one frame); w9 = a5; w10 = `sub_824C2428` (table +32, or +36 + 5 on material change);
w11 = state684. Pattern trigger: pattern from state636 (or 648 when (339||340) && !464); 0 →
w13 = 0; else `sub_82497A58` → class `7242F32831ED3332`, mode `CA81764BF5A85E34`. Mode 1 grid
(`sub_824C1CA0`): rotate wheel world pos (state+384+16i) by record angle, compare floor(x/grid) or
floor(z/grid) to stored cell; fire if frame counter advanced > `DAE803A0CBD286D1` (1); rear wheels
skipped when 339||340. Mode 2 distance: d = v × 100 × min(dt, 2.0) × `[owner16+60]`; front
accumulator passes `A3BC1976039FA00A` → wheels 0, 1; rear after further 50.

| Id | Pattern | Mode | Grid m | Angle ° | w13 | Gain |
|---|---|---|---|---|---|---|
| 1 | spidercrack | 0 | 3 | 30 | 2 | 0.63 |
| 2 | square_2_x_2 | 1 | 2 | 30 | 1 | 0.98 |
| 3 | square_4_x_4 | 1 | 1.6 | 65 | 2 | 0.00 |
| 4 | square_8_x_8 | 1 | 0.5 | 14 | 3 | 0.63 |
| 5 | square_12_x_12 | 1 | 0.5 | 30 | 4 | 0.63 |
| 6 | square_24_x_24 | 1 | 0.2 | 30 | 5 | 1.00 |
| 7 | irregular_small | 1 | 0.3 | 30 | 6 | 1.00 |
| 8 | irregular_medium | 1 | 0.3 | 30 | 7 | 1.00 |
| 9 | irregular_large | 1 | 0.7 | 68 | 8 | 0.41 |
| 10 | slats | 2 (spacing 50) | 3 | 30 | 9 | 0.73 |
| 11 | sidewalk | 1 | 3 (6 on surface 3) | 85 | 10 | 0.90 |
| 12 | brick_tile_random_size | 1 | 0.3 | 30 | 11 | 0.58 |
| 13 | mini_tile | 1 | 0.08 | 30 | 12 | 1.00 |
| 14 | special_1 | 1 | 0.3 | 30 | 13 | 0.63 |
| 15 | special_2 | 1 | 0.3 | 30 | 14 | 0.63 |

Update: w0 32767; w1 vf60(1) (vf60(6) if w11); w2 vf60(5); w3 vf52(0); w4 vf56(2); w5/w6
vf64(3)/(4); w8 = trunc(clamp01((v − 0.5) × 0.08) × 10000); w15 = |state204 × 1000| slewed ±100
per frame, 0..1000; w16 = record gain × 32767; w17 = 339||340; w18 = `2F29F40384863C8C` (1.0) ×
32767; w12 = 333||334; w13 = record class. Redelivers twice per frame (slots 9 and 10).

### Class_wheels_skid (`sub_824AF678`, slot 1; 76-byte obj, 18 words; trigger `sub_824C72F0` via `sub_824C7438`, update `sub_824C7A20`, holder owner+1288)
Ctor: w1 32767, w5 25000, w7 trunc(clamp01(v × 0.08) × 10000), w8 `sub_824B23C8`, w9 table +12 of
state620 (0..4), w10 trunc(state232 × 90) 0..90, w11 `FB10048CCDD6ADFA` = 23000, w12
`8CE42E5A9388A4C8` = 32767, w13 global flag (0), w14 local && `[[owner+16]+64]` == 0, w15 local,
w16 local ? vf60(20) : 0, w17 `F52450E504250254` = 5. Predicate: slip (232) > 0 → (341 ? state192
== 4 : (716 && 308) ? false : trick id ∈ {−1, 35}); else 690 || counter(owner+1516) > 0. Post on
rising edge; release when false. Counter +5/frame while 690 (cap 45), else −15 (floor 0). Update:
w7; w10 = min(90, counter + trunc(90 × slip)); w9; w16; w0 32767; w1 vf60(4); w2 vf60(13); w3
vf52(0); w4 vf56(3); w5/w6 vf64(11)/(12). Compare only local posts (w15 = 1).

### SenseOfSpeed_wind (`sub_824B0388`, slot 12, 13 words) / SenseOfSpeed_rattle (`sub_824B0520`, slot 13, 11 words)
Process `sub_824E7980`, update `sub_824E7CB0`, local only; tuning `sub_824E80D0`, collection
`6E878344774A7999`. Wind: w2 4096, w3 intensity 0..1000, w4 25000, w8 `C1DC8556BA66CDCD` = 7, w9
32767, w10 17000, w11 17000, w12 6750. Rattle: w2 4096, w3 intensity, w4 25000, w8
`4D30A6CA8C9E1ABC` = 7, w9 32767, w10 23000. Rattle: post at v × 3.6 ≥ 30 (`F57A74AFD22AD030`),
release below; w3 = trunc(clamp01((kmh − 30) / (80 − 30)) × 1000) (80 = `12275AA8AC4A63FB`). Wind:
state212, 15..55 km/h (`8B4DECD646B13210`/`73E9A42D88C51169`), or 1..10 when bailing (676;
`3BDFAC298128131F`/`7BCB09DF227EDDAC`). Update rattle: w0 `EB224272A924C135` = 32767, w1
vf52(0), w2 vf56(2), w6 0, w7 vf60(1), w3 (fused multiply-subtract). Wind: w0 `1BBA9174BD1B2D88` =
27000, w1 unchanged, w2 vf56(3), w6 0, w7 vf60(4), w3. Also streams `x_jet_rolling.grain` ("AudBoard
Rocket", `sub_824E78F0`) above 35 km/h (`7508154FF73DDCED`), level 35..60 (`1185E9A69919B051`),
gain 22000.

### cloth_trick (`sub_824B71C0`, slot 22; 48-byte obj, 11 words)
Ctor: w2 4096, w4 25000, w8 1, w9 trick id 0..40, w10 `4B6E2D79A8452D9B` = 0. A (`sub_824CC590`,
owner+40): post when state348 ≠ −1 and none held; id change → release, repost next frame; updater
`sub_824CCE48` releases on 676 or (id −1 && grounded). B (`sub_824CC680`, owner+44): on id → −1,
post with w9 = state352, held for the trick's duration (owner+64 up, owner+68 down) or until bail
(`sub_824CCFE8`). Update: w0 32767, w4 25000, w5/w6 0, w7 vf60(4), w1 vf52(0), w2 vf56(5).

### Class_Squeaks (`sub_824AFF48`, slot 7; 48-byte obj, 11 words; trigger `sub_824C7738`, update `sub_824C7DD0`, holder owner+1292)
Ctor: w1 32767, w4 4096, w5 25000, w7 trunc(clamp01(v × 0.08) × 1000), w8 15, w9
trunc(|state480.z| / `AC87D592E7601134` (1.5) × 1000) (0 if < 50, cap 1000), w10
`22D0D4A5A14FFF7D` = 0. Gate: 615 && 616 && wheel count > 1, else release. Post when
trunc(|state264| × 114.5916) ≥ `A129B33B4A2C7961` (15). Sign change of state264 → release +
repost. Update: w7, w9, w4 vf56(3), w0 32767, w1 vf60(5), w2 0, w5/w6 vf64(11)/(12), w3 vf52(0).

### Class_foot_drag (`sub_824AF498`, slot 0; 64-byte obj, 15 words; trigger `sub_824BB540`, update `sub_824BEEE8`, holder owner+128)
Ctor: w1 32767, w5 25000, w7 trunc(clamp01((v − 0.5) / 50 × 3.6) × 10000) (0.5
`E5A6D8AC6EB9B5AB`, 50 `B2C81577820408BE`) or 500 on the brake-hold path, w8 `sub_824BA390`
surface (table +20 of state620, or state628 if 339; 1 → 0), w9 4000, w10 4500, w11 4000, w12 22500,
w13 !336, w14 7. Fires on 336 || 339 || 310; release when all clear. Update: w7; w0 32767; w1
vf60(336 ? 4 : 5); w2 vf60(18); w5/w6 vf64(16)/(17); w3 vf52(0); w4 vf56(22); w8.

### c_body_slide (`sub_824B7070`, slot 21; 52-byte obj, 12 words; helper `sub_824DC2B0`, update `sub_824DC578`)
Ctor: w3 speed int 0..1000, w4 25000, w8 type 0..4, w9 676, w11 7. Sliding = any state528..548 ≠ 0;
speed int = trunc(state212 / 4.5 × 1000); type = table +40 of first contact material
state560..580 (2 if none, 4 if state593). Post when sliding && speed > `B021DB338B89D0F2` (350);
release when not sliding or ≤ `746EA8EF187E1571` (150). Update: w1 vf52(0), w2 vf56(6), w3, w7
vf60(5), w8, w10 trunc(state328 / 8 × 1000), w9 676.

### c_cloth_falls (`sub_824B72D8`, slot 23; 44-byte obj, 10 words; trigger `sub_824DBF10`, update `sub_824DCA48`)
Ctor: w3 trunc(state328 / 5.0 × 1000) (5.0 `0A9A9BD1150FE838`), w4 25000, w9 `E633C8F009CAEFFC` = 5.
Post on 676 0 → 1; release on 677 or 676 cleared. Update: w3 = max(state328, state672) / 5 × 1000,
w1 vf52(0), w2 vf56(1), w7 vf60(2).

### c_board_slide (`sub_824B0670`, slot 15; 52-byte obj, 12 words; trigger `sub_824CB3C8`, update `sub_824CB4C0`)
Loose-board scrape during bails (rail boardslides are Class_grind). Ctor: w2 4096, w4 25000, w8
`F2B44F93BD91662E` = 7, w10 state780 == 2, rest 0. Post while state780 ≠ 0; release at 0. Update:
w3 = trunc(clamp01((v − 0.5) / 15 × 3.6) × 10000) (15 `9635B780C7472A6E`); w11 = 15000
(`662CEE73D2E3FE2F`/`AB87C3D1EDDDDCBC`); spatial vf ids 23–27.

## 6. Native sources of the audio-state fields (traced 2026-09-18)

PhysOut bundle slot sizes (reset `sub_82DE53F0`, templates `sub_82DE4940`): 0=288, 4 Motion=288,
8 Air=464, 12=48, 16 Grinds=336, 20 Skeleton=608, 24 Collision=3488, 28 State=88, 32 Ground=336,
36 Reckoning=176, **40=224**, 44=112, 48=128, 52 Interaction=80, 56 Animation=176, **60=14672**,
64 Filtered=88, 68=140, 72 OffBoard=336.

- **B+40** (template `sub_82DE3358`) is written after the physics tick by an 800-byte PhysOut
  audio conditioner (vtable `0x82310A74`, built in `sub_82DF2130`, owner+12 = slot 3, constructor
  tail `sub_827725E8`), run by `sub_82DF2710` after the chromosome conditioner (slot 1) and before
  the filtered one (slot 5). Update `sub_82772748` (writes +0, +24, +36, +209, +210); helpers
  `sub_82772E18` (+16/+20), `sub_82772FD8` (+52..+91), `sub_827731C8` (+28/+32/+40/+208),
  `sub_82773298` (+92..+207), `sub_82772B88` (+44), `sub_827729B8` (+211..+215), `sub_82772D30`
  (+48). The engine has no port; its inputs mostly exist.
- **B+60** is PhysOutScoring2 (engine publishes only +204 `scoring.capabilities_204`); writers
  `sub_82DA33E0`, `sub_82DA4238`, `sub_82DA6630`, `sub_82DA4A48`, trick id `sub_82DAC498`.

Corrections to §1: 615/616 = **Skeleton 600/601** (feet inside the deck box), not Collision;
612/613/614 = Collision 3473/3474/3475 (front truck, back truck, deck contact); 696 = B60+136 (if >
0); B60+140 is never read by the bridge.

| State | Source | Writer | Meaning | Engine |
|---|---|---|---|---|
| 192 | B40+28 | `827731C8`: Grinds+136 while Grinds+316, starts −1 | grind family, latched | `grinds.words_136_140[0]` gated by `grinding_316` + latch |
| 228 | B40+32 | `827731C8`: Grinds+128 if > 0 else previous | last positive grind impact | `grinds.impact_speed_128` + latch |
| 232 | B40+20 | `82772E18`: clamp((\|Motion80 · deckX\| − K2)/K1, 0, 1); 0 if wheel count 0; K1/K2 vault via `*(0x830CFDA4)`+84, keys `0x56D931D202881C2D` / `0xE6C3FFE8AA70F944` | slip (lateral deck speed) | missing: `skateboard.vector_80` · deck part basis X |
| 468 | B40+24 (only when Air440) | `82772748`: \|Air+112\| × K`0x822F8A44`, clamp to 1 above `0x821CCD60` | jump velocity delta | `air.jump_velocity_delta_112`, Air440 = `flags_2468` bit 22 |
| 480 | B40+0 | `82772748`: Motion+64 projected on deck-part basis rows | deck-local angular velocity | `skateboard.vector_64` + deck `part_transforms` basis |
| 668 | B40+36 | max of last 4 Collision+24 (`82C02A80`: clamp(\|deck accel · deck normal\| × k), deck contact only) | deck scrape | `BoardGroundState.accelerations[6]`, `parts[6].normal` |
| 664 | Collision+20 | `82C02A80`: \|tangential deck relative velocity\| | deck slide speed | `parts[Deck].relative_velocity`, `.normal` |
| 692 | B40+40 | Grinds+216 while grinding (−1, clamp 0..143) | grind material, latched | `grinds.audio_surface_216` + latch |
| 300 | B40+44 | `82772B88`: 4-sample ring of Reckoning+20, \|min(v,0)\| vs vault (+92 class, key `0x7385078DD3C063BA`, entries 2/1/0 → 4/3/2, else 1). Bridge forces 1 if (343 && 348 ≠ 31) or 720 == 0 (720 = 20 while filtered state 7, counts down) | landing bucket 1..4 | `reckoning.vector_16[1]`, `filtered_state_0` |
| 304 | B40+48 | `82772D30`: Ground+300 (jump strength) vs vault (+136 class, key `0x46875250BEE65CDB`) → 2/1/0 | jump bucket | `jump_strength` (Processed2624) |
| 448–460 | B40+52..64 | `82772FD8`: per-wheel touchdown after > 5 air frames: clamp(−(prev wheel vel Motion+208+16i · wheel normal Coll+3376+16i)/K, 0, 1) | wheel impact | per-wheel velocity + normal |
| 464–467 | B40+68..71 | `82772FD8` | per-wheel landed latch (cleared after > 5 air frames) | `collision.wheel_contact_3296_3299` |
| 496–611 | B40+92..207 | `82773298`: Collision+80..195 (ragdoll contacts, `sub_82BD60C8`) | body contacts (528 slides, 560 materials, 592/593 flags) | `audio_observation.rs:135` + `audio_state.rs:286`, stored `:798-806` |
| 341 | B40 byte208 | State+12 == 400 or (State+16 == 701 && Grinds+323) | grinding | `state.category_12`, `state_16`, `grinds.flag_323` |
| 740 | B40 211..213 | `827729B8`: 212 ? (211 ? 2 : 4) : 213 ? (211 ? 3 : 5) : 1 | step code | needs Skeleton+144/+160 + port |
| 343 / 348 / 352 / 344 | B60+152 (EScorableID from `sub_82DA5AC8`; valid only with packet flags bit 24/25) | builder: trick name `*(0x820862A8 + 24·id + 20)`, key hash64(lowercase) (`sub_82B69B68`), collection `sub_82B69B08(0x6918469984A8C596, key)`, layout `sub_82B6CC78` offsets +164 → 348, +172 → 352, byte +176 → 344 | audio trick ids | `score_packet` → `scoring.data.by_name(..).metadata.id` + flags gate |
| 368 | B60+12312 | builder, while 332 | spin bucket | missing |
| 717 | B60 byte14657 | `82DA33E0` | Processed2484 bit 31 or < 30 frames since State78 | derivable |
| 204 | Ground+264 = Processed+2676 | ProcessOutput `82DB6EC0` | turn input | `turn` attribute / `ControlFeedback.turn` |
| 264 | Motion+184 = SkateboardBody+256 (filtered steering, `sub_82C040F0`) | `82C02A80` | signed deck tilt | `skater.ground.steering.deck_tilt` |
| 684 | Motion+200 < 0.5 = Processed+2764 | `82C02A80` | soft wheels | `processed.scalar_2764 < 0.5` |
| 332 | Air byte438 | ProcessOutput: 200 ≤ state < 300 && Collision+0 == 0 | in known air | `state.state_16`, `collision.wheel_count_0` |
| 768 | Air byte448; foot materials from Air+224 | FootPlant | footplant | `air.flag_448`, `footplant_surface_224` |
| 760 / 764 | Air 451 / +228 | HandPlantManager +3056 / +3012 | handplant | missing |
| 328 | \|Skeleton+288\| | ragdoll body velocity | body speed | `audio_state.rs:822` (part 23 *angular* velocity; angular-vs-linear open, see defect #4) |
| 292 / 296 | \|Skeleton+324\| / \|Skeleton+308\| | ragdoll foot bodies (+1612 / +1996) | per-foot vertical speed | `audio_state.rs:820-821` |
| 268 / 272 | \|Skeleton+212\| / \|Skeleton+196\| | `82BF22A0` | local toe velocity Y | `foot_physical.output.local_velocity[1][1]` / `[0][1]` |
| 280 / 276 | max xz | same | local toe velocity XZ | `local_velocity[0]` / `[1]` x, z |
| 284 / 288 | max(\|224\|,\|232\|) / max(\|240\|,\|248\|) | same | world foot speed XZ | `foot_physical.output.world_velocity[0]` / `[1]` |
| 672 | 0.25 × Σ limb speeds rel. COM | `82BE1AE8` | limb speed | `audio_state.rs:797` |
| 677 | Skeleton 599 | `82D3EEE8` | end of bail | `physical.skeleton.over_599` |
| 615 / 616 | Skeleton 600 / 601 | `82BF22A0` | feet in deck box | `physical.skeleton.flag_600` / `flag_601` |
| 620–632 | Coll 3440.. = tag & 0x7F | `82C079E0` | per-wheel material | `WheelLineState.audio_surfaces` |
| 636–648 | Coll 3456.. = (tag >> 12) & 0xF | `82C079E0` | per-wheel seam pattern | **discarded at `board_ground.rs:84-85`** |
| 652 / 656 / 660 | Coll +4/+8/+12 | contact reports `82C07D20` | truck/deck materials | `board_ground.rs:234-240` keeps only physics bits |
| 780 | builder | (676 or state 500) && deck contact && deck material < 94; d = deck up · wheel normal: d < [0x822F8994] → 1; [0x8207268C] < d < 0.1 → 2 | loose board | derivable once Coll+12 exists |
| 718 | [B+64]+0 == 7 | — | OffboardAir | `physical.filtered_state_0 == 7` |
| 309 / 320 | OffBoard 309 / 310 | Processed2480 bits 18 / 7 | — | processed flags |

## 7. Open engine-side gaps that change how player audio sounds

Recorded 2026-09-19 from playtests of the ported path. These are **physics/engine** gaps, not audio
ports; audio only shows them up.

1. **Jump height — resolved, no workaround left.** Measuring Class_Treatment word 9 from a
   playtest trace showed it **pegged at its 1000 clamp** with the old 10x scale applied, against
   retail's 183–367: the scale was overdriving landings, not rescuing them. Back-solving the clamp
   puts reported height at ≥0.6 m rather than the 0.10–0.13 m originally measured. The scale is
   removed; word 9 now lands in retail's range on its own. Full history, including two wrong turns,
   in `docs/engine-defects.md` defect 1.
2. **The engine rarely enters KnownAir.** Retail is in KnownAir (state 201) for essentially every
   hop; this engine selects it only while Processed2468 bit 10 (trajectory valid) is published, and
   nothing publishes that bit except transiently, so ordinary ollies run in PhysicsAir (200/202).
   PhysicsAir publishes nothing for Air+176 and a constant 4.0 for Air+184, which left the audio
   state's `+236` at 0 — and with it every per-wheel landing bucket (`+244` → `+448`), i.e. no
   landing impact at all. Audio now reads the owning state's own timer/prediction instead
   (`audio_observation::air_timing`), but the physics-side fix (publish bit 10 so KnownAir runs)
   is still worth doing.
3. **Powerslide squeaks do not fire.** Retail's gate is both feet inside the deck box (`+615`/`+616`),
   more than one wheel down, and `|deck tilt +264| × 114.5916 ≥ 15` (retail's slides sit at
   0.14–0.28 rad). In a Rust playtest the squeak never posts, so one of those engine inputs does not
   reach retail's values during PhysicsSlideGround (101). Use `SKATE_AUDIO_TRACE` and compare its
   `AS` lines with the capture's `state.tsv`.

## 8. Output levels, measured against the recomp (2026-09-20)

Levels were being set by ear, which kept trading one complaint for another ("gunshots" → "too
quiet" → "balance is off"). They are now set from a like-for-like measurement of both sides.

**Method.** The recomp's output pass `sub_82B21F58` was hooked to log every finished six-channel
block as `OUT <p0..p5> | rms <r> frames <n> ch <c>` (`src/skate3_audio_capture.cpp`). The same line
is emitted by this engine from `player_audio/trace.rs::output`, followed by a `DEV` line for the
post-downmix stereo. Retail's in-game music was switched off so only the player mix is metered.
The owner played the same session on both sides: roll, ollies and flip tricks, landings. Each
side's levels are the p50 of a 12-frame window after each takeoff/landing edge, against the p50 of
the rolling stretches.

**Retail (music off), native six-channel:**

| | peak p50 | over rolling |
|---|---|---|
| rolling | −22.9 dBFS | — |
| takeoff | −5.9 dBFS | +17.0 dB |
| landing | +0.4 dBFS | +23.3 dB |

Two things follow. Retail's rolling bed is quiet — about 23 dB below its impacts — and retail's
native mix runs *past unity* on a landing, so the console's master/Dac stage supplies headroom that
this port does not have.

**This engine, same session, before the fix:** native rolling −22.7 dBFS (the grain bed already
matches retail to within 0.2 dB), takeoff −17.2 (+5.5 dB) and landing −13.6 (+9.1 dB). So the bed
was right all along and the *impacts* were 11–14 dB short; the earlier readings of "rolling too
loud" were an artefact of measuring after the host trim, which compresses the ratio.

**What changed as a result:**

* `authored::oneshot::CONTACT_TRIM` 0.125 → 0.625, raising a landing by the measured 14 dB onto
  retail's level. Still a deviation, because the Splice graph's own levels are unrecovered.
* The takeoff pops are on by default (`SKATE_AUDIO_CONTACT_POPS=0` disables them): retail's takeoff
  is +17 dB over its bed and the authored takeoff alone reached +5.5 dB.
* `player_audio::OUTPUT_GAIN` 2.5 → 0.6. It is headroom, not balance: the 6→2 downmix measures
  +2.6 dB over the native peak on an impact, which would put a retail-level landing near +3 dBFS
  and into the clamp — that clamp is what made landings read as "gunshots". Scaling the mix here
  for loudness would move it away from retail's measured levels.

Still not ported: the console's own downmix and master level, so absolute loudness at the speakers
may differ from a retail console's even though the internal balance now matches.


## 9. Open: the landing one-shot has no dynamics (found 2026-09-20)

**Symptom, owner's words:** landings "sound the same" whatever the drop.

**Not the Treatment layer.** With the old 10x scale removed, Class_Treatment word 9 measures
min 12, p50 163, p90 209, max 393 over 748 landings in a playtest trace, across 184 distinct
values — inside retail's 183–367 and clearly modulated by drop height. Implied jump height p50 is
0.978 m. That layer is working.

**The fixed layer is the Splice one-shot.** The landing impact that dominates what you hear is
opened by `ContactVoicePlayer` with `gain = member.values.gain` straight from the bank record,
scaled only by the constant `authored::oneshot::CONTACT_TRIM`. Nothing in that path varies with
drop height, landing velocity or wheel count, so every landing fires at one level — which is why
a curb drop and a roof gap are indistinguishable even though word 9 separates them cleanly.

**What retail does instead.** The one-shot's level is a MixMap output like every other player gain.
The Contacts component's controller inputs (ids 3, 6, 10, 17, 18, 21) are the ones still unwritten
— the same gap that keeps the takeoff pops' environment send on the `RETAIL_POPS_LEVEL` constant.
Until those inputs are fed from the audio state, contact one-shots cannot have retail's dynamics.

**Next step.** Write the Contacts MixMap inputs from `AudioState` (landing bucket `+448`, landed
mask `+464`, wheel count `+200`, and the air fields `+236`/`+240`/`+260`), then take the one-shot
gain from the controller instead of `CONTACT_TRIM`. Verify by re-measuring: a small hop and a big
drop must produce different `OUT` peaks, not just different word 9.

## 10. The on-board body impact: `sub_824BC188` (decoded 2026-09-21)

**Symptom this explains.** Riding along, your body clips a wall or a pole: retail thuds, this
engine is silent. The cause is not missing data — `body_impact_496` is computed and speed-scaled
every frame — it is that **nothing reads it**. `sub_824BC188` is the retail routine that does.

### Why it was hard to find

The read is a scaled indexed load, so a grep for `,496(` finds nothing:

```
addi   r11,r17,124      ; r17 = region index
lwz    r10,32(r31)      ; r31 = component object, obj+32 = audio state
rlwinm r9,r11,2,0,29    ; (124 + i) * 4  =  496 + i*4
lfsx   f31,r9,r10       ; f31 = state->body_impact[i]
```

The material read is the same shape with `addi r11,r17,140` (`560 + i*4`, `lwzx`).

### Gating — it runs while riding

`lbz r22,676(r10)`: when `+676` is clear (riding) the loop runs and `obj+422` is set. When `+676`
is set it runs only while `+677` is clear, and `+676` is then carried as a *modifier* (it gates the
`+592` groin latch and the `+596`/`+600` force latches), not as a gate. So this is the on-board
path; `sub_824E3BE0` is its ragdoll counterpart, fed straight from the `sub_82BD60C8` Collision
block (`+128` token, `+144` magnitude, `+148` material, `+48` position).

The loop covers **6 of the 8 region slots** (`cmpwi cr6,r17,6`), and each region has its own
re-trigger cooldown at `obj+260+i*4`, refilled from a vault count and decremented by `state+220`.

### `sub_824BCBA0` — region to body-part material

| region | primary | second | third |
|---|---|---|---|
| 0 | **97** head | (none, 143) | **110**, or **112** when `state+593` (face) |
| 1 | **98** torso | **109** cotton | **110** |
| 2, 3 | **100** arm | **107** skin | **111** |
| 4, 5 | **99** leg | **108** denim | **111** |

Those ids are confirmed twice over: from this switch, and independently by hashing
`tools/asset_pipeline/names.txt` against `MATERIAL_VAULT_KEY` — 95 `Board`, 96 `Truck`, 97 `head`,
98 `torso`, 99 `leg`, 100 `arm`, 101 `foot`, 107 `skin`, 108 `denim`, 109 `cotton`.

### `sub_82497088` — the impact band and its window

The missing helper. It resolves the material's `Hash_F64F891C312EFD5E` RefSpec to a record of class
`7DAFF70B3A91CD5D` holding four floats, and returns the band the impact falls in plus that band's
interpolation window — which is exactly the `tier` and the `lo`/`hi` that the already-ported
`sub_82496C58` (`CollisionMaterials::contact_level`) takes.

| record offset | vault field | default | role |
|---|---|---|---|
| +12 | `D660AC459139BDF4` | 0.005 | floor; below it the function returns **3** = silent |
| +8 | `7D8DEDD338D45482` | 0.5 | band 0/1 split |
| +0 | `C8DED1BC20B9D6A5` | 1.0 | band 1/2 split |
| +4 | `B8870D2001033E0F` | 2.0 | ceiling |

```
if impact <  [+12]            -> 3          (silent)
if impact >  [+0]             -> 2, window ([+0],  [+4])
else if impact > [+8]         -> 1, window ([+8],  [+0])
else                          -> 0, window ([+12], [+8])
```

The offset-to-field mapping is *derived*, not assumed: the four values must form three contiguous
ascending bands, and that holds for **all 45 records of the class with zero violations**. Asserted
against the owner's real vault for all 143 materials by
`the_vault_impact_bands_tile_for_every_material`, which also shows five distinct floors across the
table — the thresholds really are per-material.

Note the JSON export under `assets/private/stock/skater-collections.json` is missing records 77,
87, 92, 97 and 98 that the vault itself has, so read the vault (`Collections::load`), not the
export, when checking this.

The `.rdata` key table `sub_82497088` indexes (`0x8302D6F0`, 16-byte stride, material → vault key)
was read from a decrypted image dump and **matches `MATERIAL_VAULT_KEY` for all 143 materials with
zero mismatches** — which validates both that table and the dump. See the image-dump recipe in the
session notes; the collection is the same `D40CB4C0FFE45676` this port already loads.

### What each region plays

Five `sub_82486EF0` calls per contacting region — main body-vs-surface, a detail layer, then three
body-only layers using the second/third ids above with material 143. `sub_824BCEB0` adds a single
one-shot accent (`matA` 98, `matB` 143, both categories 1) when the `obj+420` first-contact latch
is set. All of them land in `sub_824D2318` / `sub_824D20E8`, the manager this port already has.

### Not this routine

- `sub_824BBB28` — a looping Splice voice keyed off `+333`/`+334`/`+620`. Unrelated.
- `sub_824BD000` — a single non-regional channel, position `+144`, magnitude `+668`, material
  `+660`, ids 95/113. The deck/board impact, not the body.
- `sub_824DC2B0` — reads `+528..+556` and `+560..+580`; the body *scrape/slide* loop, already
  ported as `c_body_slide`.

## 11. Measured retail transfer functions (2026-09-23)

Extracted from `<drive>:\skate3-retail-captures\extract` (`state.tsv` joined to `board.tsv` and
`updates/*.tsv` by mixer frame). These are the curves to compare a Rust playtest against; the
percentile summaries earlier in this document are the same data without the speed axis.

### Rolling: the grain players against ground speed (`+208`)

Truck 0, both players, 18,545 frames. `A` is the primary player, `B` the one offset in the grain
scan.

| km/h | gain A | gain B | B/A | position A | position B |
|---|---|---|---|---|---|
| 0 | 0.0047 | 0.0006 | −18.1 dB | 0.002 | 0.001 |
| 5 | 0.0613 | 0.0118 | −14.3 dB | 0.022 | 0.010 |
| 10 | 0.1245 | 0.0595 | −6.4 dB | 0.121 | 0.043 |
| 15 | 0.0721 | 0.0989 | **+2.7 dB** | 0.140 | 0.063 |
| 20 | 0.1700 | 0.0078 | −26.8 dB | 0.407 | 0.308 |
| 25 | 0.0950 | 0.0637 | −3.5 dB | 0.340 | 0.241 |
| 30 | 0.1121 | 0.0567 | −5.9 dB | 0.394 | 0.295 |
| 35 | 0.0829 | 0.0373 | −6.9 dB | 0.515 | 0.415 |
| 40 | 0.1214 | **0.0000** | silent | 0.634 | 0.534 |
| 45 | 0.0303 | **0.0000** | silent | 0.596 | 0.496 |

Two things this settles:

* **`position B = position A − 0.100`, exactly**, at every speed where both run. That confirms
  `SECOND_OFFSET` (`grain/board.rs:111`) and the sign it is applied with.
* **The B player is a *turning* layer, not a speed layer.** It peaks at 15 km/h — where a line has
  the most carving in it — and is **silent at 40 km/h and above**. Gain A, meanwhile, stays roughly
  flat at 0.08–0.17 from 10 km/h up; what changes with speed is the *position*, 0.12 → 0.63, which
  is the Bezier walking further into the grain recording.

That shape is what `f1164` predicts: `clamp(|COM velocity| × 0.24, cap) × turn_204`, slewed. Retail's
`+204` is zero in **87%** of frames (n = 18,553, range −1..+1), so the B player is off during
ordinary straight rolling however fast it is.

**Why it matters.** If a port's turn input is non-zero most of the time, `f1164` stays up, the B
player runs permanently, and because `gain_A` is scaled by `(1 − max(f1164, f1168))` the two layers
sit at comparable level — the same grain recording played from two positions 0.1 apart. That is a
doubled, phasey bed that gets worse with speed, and it is what "rolling fast sounds echoey" would
sound like. This engine feeds `turn` from the animation `Turn` attribute
(`physics/audio_observation.rs:336`), and every other consumer — `ControlFeedback.turn`
(`animation_phase.rs:388`) and `StraightenInput.turn` (`ground_runtime/input.rs:186`) — reads the
same field, so there is no better-recovered value sitting unused. **Whether it is wrong is
empirical and needs a `SKATE_AUDIO_TRACE` playtest to settle: compare our `+204` against the 87 %
figure above.**

### Wind: `SenseOfSpeed_wind` against COM speed (`+212`)

Posted frames only — the object is released below its 15 km/h bound.

| km/h | w3 intensity | `clamp01((kmh−15)/40)×1000` | w7 level | dBFS | n |
|---|---|---|---|---|---|
| 15 | 51 | 0 | 210 | −43.9 | 4472 |
| 20 | 154 | 125 | 2681 | −21.7 | 2203 |
| 25 | 272 | 250 | 678 | −33.7 | 2050 |
| 30 | 442 | 375 | 975 | −30.5 | 2631 |
| 35 | 509 | 500 | 1521 | −26.7 | 1245 |
| 40 | 624 | 625 | 3300 | −19.9 | 222 |
| 45 | 683 | 750 | 683† | −20.4 | 55 |
| 50 | 906 | 875 | 8246 | −12.0 | 34 |
| 55 | 964 | 1000 | 8091 | −12.1 | 37 |

† small n; the 45 and 50 km/h rows carry 55 and 34 frames.

The intensity column tracks the ported formula within the error of a 5 km/h bin, so `w3` is right.
The loudness is not in `w3` at all — it is **`w7`, the MixMap `level(4)`, a 32 dB ramp** from −44 dBFS
at the 15 km/h post to −12 dBFS at 55. `SFXObj_SenseOfSpeed`'s outputs depend on state controller
ids 7, 8, 9 and 14 (`mixmap_dump`'s dependency walk), which are the speed scales this port already
writes — so wind level should follow once the speeds do. Compare a playtest's `w7` against this
column before touching anything in `speed.rs`.

### Validated headlessly, 2026-09-23: the position path is correct once surfaces are real

`player_audio/headless.rs::headless_rolling_speed_and_surface_sweep` drives the retail path at a
chosen speed and surface tag, which a playtest cannot do: retail's table above averages a session
that crossed many surfaces, and the position is a Bezier of `speed / maxKmh` with `maxKmh` **per
surface**. Position at one speed, by surface (A player, `SurfacePolicy::Retail`):

| surface | 20 km/h | 30 km/h | 40 km/h |
|---|---|---|---|
| material 2 | 0.145 | 0.321 | 0.528 |
| material 3 | 0.255 | 0.481 | 0.710 |
| material 16 | 0.344 | 0.612 | 0.851 |
| **material 41** | 0.328 | 0.487 | **0.641** |
| material 53 | 0.145 | 0.321 | 0.528 |
| **retail, mixed** | **0.407** | **0.394** | **0.634** |

Retail's mixed average falls inside the per-surface spread at every speed, and material 41 — the
most common surface in the University traces — lands within 0.007 of it at 40 km/h. **So the
Bezier, `SECOND_OFFSET` and the per-surface `maxKmh` lookup are all right.** A playtest pinned to
one surface reads as a position error that is really a surface difference; do not chase it without
controlling the surface.

**Retracted 2026-09-24: the A player's gain is not demonstrably off.** The paragraph below read a
+2.6 dB gain-A error out of a traced session. That comparison is confounded the same way the
position one was, and the direct measurement does not support it.

`gain_A = (1 - max(f1164, f1168)) x level(1)/32767 x ...`, so with turn and brake at zero the
headless sweep measures `level(1)` itself: **0.215-0.254**, flat across all five surfaces. Retail's
own `level(1)` is in the capture — `vf.tsv` slot 60, id 1, board controller `4A26A8A0`, single
caller `0x824C6FEC` inside `sub_824C6BD8`, 19,304 reads:

| p10 | p50 | p90 | p99 | max |
|---|---|---|---|---|
| 0 | 1 | **7859** | 10924 | 17536 |

The near-zero median is the capture's idle and stationary time. The active-rolling value, p90, is
**7859/32767 = 0.240** — inside the port's 0.215-0.254 band. So the MixMap output feeding the
rolling bed matches retail, and what differs in a session is the `(1 - max(f1164, f1168))` term,
which depends on turn, brake and the **per-surface intensity cap** — and the traced session was
pinned to concrete with `SKATE_AUDIO_SURFACES=concrete` while retail's crossed many surfaces.

Before re-opening this, measure `f1164` directly on both sides at a matched surface; do not infer a
gain error from a session-average comparison again. *(Original paragraph retained below.)*

**What was recorded as off: the A player's gain.** Measured in a real traced session (30,060 frames,
`tools/audio_compare.py`), gain A is 0.1507 at 30 km/h against retail's 0.1121 — **+2.6 dB** — and
runs 3-5 dB high from 25 km/h up. It is *not* surface-dependent (the sweep gives an identical
0.215/0.226/0.254 on all five surfaces), so it is the `(1 - max(f1164, f1168)) x level(1)/32767`
term, not the grain selection. Note the headless figure looks worse (~+8 dB) only because the
fixture holds turn and brake at zero, which leaves `f1164`/`f1168` at zero and gain A at its
ceiling; the traced session is the fair comparison.

**Harness caveat.** The sweep faults with `SkateBoard process: at 0x40040000: no segment covers
this address` on its **eleventh** rung, in ascending *or* descending speed order, so it is
cumulative rather than speed-dependent — roughly 900 frames of teleporting the speed every 90
frames. The 30,060-frame traced playtest shows no such fault and no observation overflow, so treat
it as an artifact of the synthetic discontinuities, not a defect in play. Keep sweeps to under ten
rungs per worker (`SKATE_SWEEP_SPEEDS` overrides the ladder).

### The wind is correct, and how the bin-average comparison lied (2026-09-24)

`SenseOfSpeed_wind` w7 **is** the MixMap `level(4)` — confirmed in the capture as controller
`4A26A920`, caller `0x824E7FFC` inside `sub_824E7CB0`, 13,333 reads, exactly the number of wind
packets. So the packet table above is already the direct comparison.

Measured at **matched instantaneous speed** (retail binned to +/-0.5 km/h, median; this engine from
`headless_rolling_speed_and_surface_sweep` at the exact speed):

| km/h | retail n | retail median | this engine | error |
|---|---|---|---|---|
| 25 | 715 | 343 | 343 | **+0.0 dB** |
| 30 | 947 | 730 | 697 | −0.4 dB |
| 35 | 241 | 1194 | 1250 | +0.4 dB |
| 40 | 67 | 2017 | 2081 | +0.3 dB |

**The wind level tracks retail within 0.4 dB across its whole range.** Two earlier readings in this
document claimed otherwise — first "much too quiet", then "~3 dB hot" — and both were artifacts of
the comparison, not the engine.

**The methodological trap, worth avoiding for every other family.** Retail's per-speed distribution
is strongly right-skewed: at 40 km/h its mean is 3258 against a median of 2017, half a factor of
two apart. Binning at +/-2.5 km/h and comparing *means* therefore inflates retail by 3-6 dB, and
because the underlying curve is convex in speed a wide bin inflates it again. Use **narrow bins and
medians at a matched instantaneous speed**, and compare against a headless rung driven at that
exact speed. A session-average-versus-session-average comparison has now produced three false
findings in this document (the grain position, the A player's gain, and the wind), each of which
survived until it was measured this way.
