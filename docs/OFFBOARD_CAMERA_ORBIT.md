# Offboard orbit centre

The compass adapter used the current deck transform even though the camera
subject had switched to the skeleton root. This gives the orbit and its
30-degree camera-relative limit a different centre from the camera's subject.
The discrepancy can pull or constrain the orbit as the board moves away.

Static native evidence comes from the TU3 image
`default_82000000_011B0000.bin`, base `0x82000000`, SHA-256
`f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4`:

- `82DF69C0` first calls `82DF80D8` to select publisher cache80.
- `82DF80D8` selects Skeleton output5+432 when State output7 byte59 or75
  is set. Otherwise it copies subject getter380 (previous physical transform).
- `82DF69C0` passes cache80 to setter24 (subject transform376), then passes
  current physical output0+64 to the separate setter28. This does not replace
  cache80.
- `82DF7B90` copies cache80's four vectors into compass input32..80.
- `82DF4100` uses compass80 as the orbit centre and measures both the retained
  orbit and previous camera position relative to it.

The adapter now binds the compass to the already selected subject transform,
sharing the native skeleton/physical-history selection instead of republishing
the current deck. The native orbit speed, deadzone and angular limit remain.
The source mismatch is confirmed by static analysis; the reported gameplay
symptom still requires the user's check. No automated or gameplay tests were run.
