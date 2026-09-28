# Live grind integration

The default world now supplies seven straight grind paths shared with rail/coping collision geometry. This pass connects the paired-truck 50-50 acquisition path to the production state selector, board forces, skeleton update, stock grind animations and physical output. The runtime now builds and consumes the same Pegasus spline data for default-world and imported `.skate` rails (authored polylines, closed paths, and preserved retail cubic records).

## Source and implementation

TU3 image: default_82000000_011B0000.bin, SHA256 f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4. Analysis used the local generated PPC and the copied IDA database; the reference game was not launched for this implementation.

- 82C1FDC0 and 82D89150: actual truck probe rectangles, paired contacts including the native cross-segment separation branch, depth and balance rejection. No nearest-distance snap or positional teleport.
- 82D886B8: air and ordinary level-ground admission. The host currently covers straight level rails; sloped approach classification, tipslides, boardslides and darkslides still need their own live adapters.
- 82D41D70 / 82D3FA18 / 82D3FD88: 50-50 lateral pin and friction, using the stock PinVsSlope and FrictionVsTime curves. Authored static rail support uses the upward normal and default surface multiplier.
- 82D41E38 / 82D40890: rail-aligned target blended through the existing native matrix interpolation and angular-only board drive. User-driven grind twist forces and the remaining grind-manager transition scoring are not complete.
- 82D73AB0: straight static 50-50 pop branch, stock physics-mode launch speeds, lateral nudge and launch velocity publication.
- 82D3F430: disables the grind hook and restores wheel drag on exit. The original selector retains Nonspecific701 briefly on contact loss before choosing air/ground from actual observations; the host now supports that ordinary continuation.
- 82BAF208, 82BB0A30, 82BB0D40 and 82BB10F8: selected grind intent, stock twist endpoints and retained fade, and grind crouch. Physical twist uses the native parts10/6 direction relative to Processed352 and Reckoning up. Canonical 50-50 names select the existing B_GRIND5050/B_BF_GRIND5050 animation trees; full chromosome naming/scorable identifiers are not ported.
- 82BA8238: DistToEdge reads the existing OffBoard116 output. This resolves the mounting graph condition encountered in the launched build.

## Visual check

The user's 2026-09-07 18:59:36 run confirmed acquisition: both truck contacts
passed at depths0.0753/0.0664m and entered BS_50_50 at18:00:19.516 UTC.
The next animation tick2437 stopped on unsupported IsDroppingIn. Factory82BC2D58
uses vtable8231E898 whose evaluator82BA41F0 reads PhysOut bundle16 (Grinds),
byte324 for nonzero. Reset82DE3518 clears this byte; tipslide Fill82D42788
copies state97 into it. This condition now consumes that physical publication;
the ordinary50-50 retains the reset value. This is not Ground324 (pumping) or
OffBoard334 (camera drop-in). No gameplay test was run for this fix.

Build with Build.ps1; launch bin/skate-game.exe --assets assets (no map argument). The low straight rail is at x=-7, z=7..15, 0.45m above the lower floor; the higher rail is at x=4.5, z=8..16, 0.70m above it. Approach along the rail and land both trucks on its top. The halfpipe coping and three starting-platform edges also have authored paths.

Build completed. No automated tests or controller-driven gameplay tests were run. Visual behavior, catch consistency, pop timing, naming and solver stability while grinding require the user's playtest; this is not a claim of complete Skate 3 grind parity.

## Spline integration correction (2026-09-07)

The newer `SK8R15/Source/owned/world/src/grind_spline.cpp` revealed that the previous default-world blob incorrectly put delta in coefficient A and populated auxiliary words as length fields. Authored segments now use A=-2*delta, B=3*delta, C=0, D=start. The old independent endpoint list bypassed that blob, so this malformed blob alone does not explain the failed default-world catches.

Both map paths now build Pegasus data and feed the live query from it. TU3 AssetRecord82C1E568 constructs exactly one contact chord per cubic, D to (A+B)+(C+D); curved payloads are retained without an invented tessellation scheme. Imported native IDs, flags and all30 payload words are preserved. The query uses the native local1.2m bounds and40-entry limit. The run preceding this correction recorded no entered grind; the subsequent user run confirmed acquisition and exposed the IsDroppingIn binding described above. Passive GRIND_QUERY logging now records those observations during the user's visual run. No gameplay success is claimed from a build.
