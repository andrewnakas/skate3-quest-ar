# Edge-case parity probes

Same idea as `../feel.py`: run one input script through the port (s3trace, your own converted pack)
and through sk8 started at the port's state, and print both side by side. Measured behaviour only;
nothing from the pack is stored. Paths to s3trace / sk8trace / the pack are at the top of `edge.py`.

| script | what it compares |
| --- | --- |
| `sweep_kind.py START MAG c|n STEP` | grind picked by the right stick held from tick START (catch ~60), swept round |
| `roll.py 5-0|nose MAG` | from a 5-0 / nosegrind, the stick rolled to each side mid-rail |
| `lroll.py 50|5-0|nose` | the left stick leaning a grind over |
| `rails.py`, `decel.py` | grind kinds and speed lost along the rail |
| `magnet*.py` | rail magnet: aim from the pop, reach, catch distance |
| `glide.py` | coasting at several speeds |
| `pushgain.py`, `pushhold.py` | push gain by speed and by how long the button is held |
| `fakie.py`, `fakie4.py` | a 180 to fakie, then pushing (slow fakie pushes turn back) |
