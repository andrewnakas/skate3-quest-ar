"""The clean tier's skate animation set, authored here as poses on IK controls and baked to actions.

Everything is in the rider frame (Blender units, metres): origin on the deck top between the feet,
+X toward the board's nose (the left, leading foot is regular stance), -Y the way the chest faces,
+Z up. Each pose places the hips, bends the spine, turns the head and puts the feet and hands
(IK targets) somewhere; clips are poses at frames, eased between, then baked to plain bone keys.

Runtime (crates/skate-game/src/clean/skater.rs) blends these clips from the engine's state.
"""
import math

import bpy
from mathutils import Euler, Matrix, Vector

FPS = 30
DECK_TOP_TO_GROUND = -0.105  # the ground under the wheels, in the rider frame
ANKLE = 0.075                # ankle height above whatever the foot stands on

FRONT_X, BACK_X = 0.22, -0.25


def foot(x, y=0.0, z=0.0, yaw=0.0, pitch=0.0):
    """Ankle target for a foot standing at (x, y) on a surface at height z; yaw turns the toes."""
    return {"at": (x, y, z + ANKLE), "rot": (pitch, 0.0, yaw)}


def P(**kw):
    """A pose. Missing parts take the riding stance."""
    pose = {
        "hips": (0.0, 0.0, 0.86),
        "hips_rot": (8.0, 0.0, 0.0),      # pitch (forward bend about X), roll, yaw (degrees)
        "spine": (16.0, 0.0, 10.0),        # bend forward, side, twist toward the nose
        "head": (-8.0, 0.0, 55.0),         # nod, tilt, turn toward the nose
        "lf": foot(FRONT_X, 0.0, 0.0, yaw=-20.0),
        "rf": foot(BACK_X, 0.0, 0.0, yaw=8.0),
        "lh": (0.32, -0.12, 0.88),         # front hand
        "rh": (-0.34, -0.1, 0.86),         # back hand
        "knee_out": 0.35,                  # knees point toward the chest side
    }
    pose.update(kw)
    return pose


STANCE = P()
# Key poses fitted to median measurements of pose-estimated reference clips (scripts/cleanroom/
# reference, many skaters): crouch leg extension ~0.8 of leg length with ~80 deg knees and the trunk
# leaning 30-50 deg; pop fully extended; in the air the body stays tall (extension ~0.85, knees
# ~65-70 deg) and the board comes up ~15 cm; hands around chest height, a little out to the sides.
CROUCH = P(hips=(0.0, 0.05, 0.74), hips_rot=(22, 0, 0), spine=(20, 0, 12), head=(-20, 0, 55),
           lh=(0.34, -0.28, 0.72), rh=(-0.34, -0.22, 0.7))
# The pop: the back leg snaps straight to slam the tail, the front knee drives up and the front foot
# drags toward the nose, the arms lift and open.
POP_TOP = P(hips=(-0.03, 0.02, 0.94), hips_rot=(12, 0, 0), spine=(10, 0, 8), head=(-18, 0, 55),
            lf=foot(FRONT_X + 0.1, 0.0, 0.16, yaw=-25, pitch=14), rf=foot(BACK_X - 0.02, 0.0, 0.0, yaw=8),
            # hands up and out but elbows kept bent: a near-straight arm lets the elbow flip about
            lh=(0.62, -0.12, 1.16), rh=(-0.6, -0.06, 1.13), knee_out=0.55)
AIR_TUCK = P(hips=(0.0, 0.03, 0.92), hips_rot=(16, 0, 0), spine=(16, 0, 10), head=(-20, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.22, yaw=-20), rf=foot(BACK_X, 0.0, 0.21, yaw=8),
             lh=(0.68, -0.1, 1.2), rh=(-0.66, -0.04, 1.17), knee_out=0.6)
# Flip tricks: the legs split up and out of the way and the board turns underneath, arms up and
# open, then the feet come back down onto it. Kickflips kick the front foot out forward over the toe
# side, heelflips forward over the heel side; shove-its lift both feet straight up.
def flip_open(front_y, back_z):
    return P(hips=(0.0, 0.02, 0.96), hips_rot=(8, 0, 0), spine=(8, 0, 8), head=(-26, 0, 50),
             lf=foot(FRONT_X + 0.22, front_y, 0.34, yaw=-40, pitch=-15), rf=foot(BACK_X - 0.08, 0.0, back_z, yaw=12),
             lh=(0.7, -0.08, 1.24), rh=(-0.68, -0.02, 1.21), knee_out=0.7)


FLIP_KICK = flip_open(-0.24, 0.2)
FLIP_HEEL = flip_open(0.18, 0.24)
FLIP_SHUV = P(hips=(0.0, 0.02, 0.96), hips_rot=(10, 0, 0), spine=(10, 0, 8), head=(-24, 0, 50),
              lf=foot(FRONT_X + 0.02, 0.0, 0.28, yaw=-20), rf=foot(BACK_X - 0.02, 0.0, 0.26, yaw=8),
              lh=(0.68, -0.1, 1.2), rh=(-0.66, -0.04, 1.17), knee_out=0.6)
FLIP_CATCH = P(hips=(0.0, 0.03, 0.88), hips_rot=(16, 0, 0), spine=(18, 0, 10), head=(-26, 0, 50),
               lf=foot(FRONT_X, 0.0, 0.1, yaw=-20), rf=foot(BACK_X, 0.0, 0.09, yaw=8),
               lh=(0.66, -0.1, 1.16), rh=(-0.64, -0.04, 1.13), knee_out=0.55)
LAND_LOW = P(hips=(0.0, 0.04, 0.72), hips_rot=(22, 0, 0), spine=(22, 0, 12), lh=(0.4, -0.26, 0.8), rh=(-0.4, -0.2, 0.78))
MANUAL = P(hips=(-0.13, 0.0, 0.8), hips_rot=(-4, 0, 0), spine=(-2, 0, 12), lh=(0.38, -0.05, 1.02), rh=(-0.5, -0.05, 1.0))
NOSE_MANUAL = P(hips=(0.14, 0.0, 0.78), hips_rot=(14, 0, 0), spine=(24, 0, 14), lh=(0.52, -0.08, 1.0), rh=(-0.36, -0.08, 1.04))
GRIND = P(hips=(0.0, 0.02, 0.74), hips_rot=(12, 0, 0), spine=(18, 0, 12), lh=(0.62, -0.05, 1.08), rh=(-0.62, -0.05, 1.04))
BOARDSLIDE = P(hips=(0.0, 0.03, 0.74), hips_rot=(12, 0, 30), spine=(16, 0, 30), head=(-6, 0, 70),
               lh=(0.6, -0.15, 1.06), rh=(-0.58, 0.1, 1.04))
POWERSLIDE = P(hips=(0.0, 0.16, 0.68), hips_rot=(-12, 0, 10), spine=(-6, 0, 20),
               lf=foot(FRONT_X, 0.03, 0.0, yaw=-25), rf=foot(BACK_X, 0.03, 0.0, yaw=5),
               lh=(0.5, 0.05, 0.95), rh=(-0.5, 0.15, 0.9))
GRAB_TOE = P(hips=(0.0, 0.0, 0.58), hips_rot=(32, 0, 0), spine=(40, 0, 6),
             lf=foot(FRONT_X, 0.0, 0.2, yaw=-20), rf=foot(BACK_X, 0.0, 0.18, yaw=8),
             rh=(0.0, -0.13, 0.22), lh=(0.42, -0.32, 0.86))  # free arm out in front for balance
GRAB_HEEL = P(hips=(0.0, 0.05, 0.6), hips_rot=(26, 0, 0), spine=(34, 0, 14),
              lf=foot(FRONT_X, 0.0, 0.2, yaw=-20), rf=foot(BACK_X, 0.0, 0.18, yaw=8),
              lh=(0.02, 0.12, 0.22), rh=(-0.42, -0.3, 0.86))


# Breakdowns between the key poses (motion reads smooth when the in-betweens are designed, not left
# to the curve): the pop drives up off the back foot before the front foot slides; the air holds
# the tuck then reaches the legs back down for the landing.
POP_DRIVE = P(hips=(-0.02, 0.03, 0.86), hips_rot=(16, 0, 0), spine=(14, 0, 10), head=(-20, 0, 55),
              lf=foot(FRONT_X + 0.04, 0.0, 0.05, yaw=-22, pitch=6), rf=foot(BACK_X - 0.01, 0.0, 0.0, yaw=8),
              lh=(0.42, -0.22, 0.86), rh=(-0.42, -0.16, 0.84), knee_out=0.45)
POP_SLIDE = P(hips=(0.0, 0.02, 0.95), hips_rot=(12, 0, 0), spine=(12, 0, 9), head=(-20, 0, 55),
              lf=foot(FRONT_X + 0.06, 0.0, 0.17, yaw=-22, pitch=4), rf=foot(BACK_X, 0.0, 0.1, yaw=8),
              lh=(0.68, -0.1, 1.22), rh=(-0.66, -0.04, 1.19), knee_out=0.6)
# Gameplay footage (3 fixed angles, pose-tracked): knees ~82 deg in the tuck and the tuck deepest late
# in the air, held into the landing (no reaching the legs down first); pop to landing ~0.8 s.
AIR_PEAK = P(hips=(0.0, 0.05, 0.84), hips_rot=(26, 0, 0), spine=(26, 0, 10), head=(-26, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.4, yaw=-20), rf=foot(BACK_X, 0.0, 0.39, yaw=8),
             lh=(0.7, -0.1, 1.2), rh=(-0.68, -0.04, 1.17), knee_out=0.65)  # arms wide for balance
AIR_HOLD = P(hips=(0.0, 0.05, 0.84), hips_rot=(24, 0, 0), spine=(24, 0, 10), head=(-24, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.37, yaw=-20), rf=foot(BACK_X, 0.0, 0.36, yaw=8),
             lh=(0.68, -0.12, 1.16), rh=(-0.66, -0.06, 1.13), knee_out=0.65)
AIR_REACH = P(hips=(0.0, 0.03, 0.94), hips_rot=(14, 0, 0), spine=(16, 0, 10), head=(-22, 0, 55),
              lf=foot(FRONT_X, 0.0, 0.07, yaw=-20), rf=foot(BACK_X, 0.0, 0.06, yaw=8),
              lh=(0.48, -0.18, 0.98), rh=(-0.46, -0.12, 0.95), knee_out=0.5)
LAND_CONTACT = P(hips=(0.0, 0.03, 0.84), hips_rot=(16, 0, 0), spine=(18, 0, 11), head=(-18, 0, 55),
                 lh=(0.6, -0.16, 1.06), rh=(-0.6, -0.1, 1.03), knee_out=0.5)
LAND_DEEP = P(hips=(0.0, 0.05, 0.7), hips_rot=(24, 0, 0), spine=(24, 0, 12), head=(-14, 0, 55),
              lh=(0.42, -0.27, 0.76), rh=(-0.42, -0.21, 0.74), knee_out=0.45)
LAND_RISE = P(hips=(0.0, 0.01, 0.87), hips_rot=(9, 0, 0), spine=(15, 0, 10), lh=(0.34, -0.14, 0.86), rh=(-0.35, -0.11, 0.85))


def flip_part(open_pose, k):
    """Part-way from the pop's extension into the flip's open pose."""
    return mix(POP_TOP, open_pose, k)


def push_frame(phase):
    """Kick-push, after reference footage: the front foot turns to point down the board and the hips
    and chest face the way of travel; the pushing foot swings forward, plants beside the front truck,
    drives straight back along the ground, then lifts and comes forward again. The front knee bends
    as the foot plants and the hips bob with each drive; the shoulders counter-turn against the
    pushing leg and the arms swing with them."""
    plant, release = 0.18, 0.62
    if phase < plant:                       # swinging forward to plant
        k = phase / plant
        x, lift = -0.35 + 0.47 * k, 0.1 * math.sin(math.pi * (0.5 + 0.5 * k))
    elif phase < release:                   # driving back on the ground
        k = (phase - plant) / (release - plant)
        x, lift = 0.12 - 0.62 * k, 0.0
    else:                                   # lifting off and coming forward
        k = (phase - release) / (1 - release)
        # ends at the swing's starting height so the loop has no seam
        # the heel kicks up behind (knee bent) as the leg swings forward, as in gameplay footage
        x, lift = -0.5 + 0.15 * k, 0.24 * math.sin(math.pi * k) + 0.1 * k * k * (3 - 2 * k)
    drive = math.sin(math.pi * min(1.0, max(0.0, (phase - plant) / (release - plant))))
    # The body faces down the board (+X) while pushing, so its left side is +Y and its right -Y. The
    # arms hang at those sides and swing against the pushing leg: right arm forward as it drives back.
    arm = max(-0.12, min(0.12, (-0.19 - x) * 0.35))
    # Upright: the trunk leans only a little into each drive (a deep forward bend read as hunched),
    # and the standing knee bends to let the pushing foot reach the ground.
    return P(hips=(0.05, -0.05, 0.83 - 0.09 * drive), hips_rot=(8 + 6 * drive, 0, 75),
             spine=(8 + 5 * drive, 0, 12 + 20 * arm), head=(-6, 0, 8 - 15 * arm),
             lf=foot(FRONT_X - 0.02, 0.0, 0.0, yaw=80),
             rf=foot(x, -0.17, DECK_TOP_TO_GROUND + lift, yaw=78),
             lh=(0.12 - arm, 0.24, 0.8 + 0.15 * max(0.0, -arm)),
             rh=(0.12 + arm, -0.38, 0.8 + 0.15 * max(0.0, arm)), knee_out=0.5)


# Stopping pushing: the back foot steps back onto the tail (still pointing ahead), then the front
# foot pivots back across the board and the hips and chest turn back to the side.
PUSH_STEP_ON = P(hips=(0.03, -0.04, 0.84), hips_rot=(9, 0, 45), spine=(10, 0, 12), head=(-8, 0, 30),
                 lf=foot(FRONT_X - 0.02, 0.0, 0.0, yaw=80), rf=foot(BACK_X, 0.0, 0.0, yaw=30),
                 lh=(0.14, 0.12, 0.76), rh=(-0.06, -0.3, 0.76), knee_out=0.45)
PUSH_TURN = P(hips=(0.01, -0.01, 0.84), hips_rot=(10, 0, 12), spine=(15, 0, 10), head=(-9, 0, 48),
              lf=foot(FRONT_X, 0.0, 0.0, yaw=0), rf=foot(BACK_X, 0.0, 0.0, yaw=12),
              lh=(0.28, -0.08, 0.84), rh=(-0.28, -0.12, 0.84))


# The bail: the back foot steps off and the body pitches forward over the toe side (stumble), the
# hands go out to catch the fall, the body lands on them and the knees, then sinks and settles.
BAIL_STUMBLE = P(hips=(0.05, -0.2, 0.74), hips_rot=(34, 8, 15), spine=(30, 0, 4), head=(-10, 0, 20),
                 lf=foot(FRONT_X, -0.05, 0.0, yaw=-20), rf=foot(BACK_X + 0.1, -0.35, DECK_TOP_TO_GROUND + 0.08, yaw=8),
                 lh=(0.35, -0.55, 0.8), rh=(-0.2, -0.55, 0.82), knee_out=0.5)
BAIL_REACH = P(hips=(0.05, -0.42, 0.5), hips_rot=(58, 10, 15), spine=(46, 0, 0), head=(4, 0, 5),
               lf=foot(FRONT_X - 0.05, -0.2, DECK_TOP_TO_GROUND + 0.05, yaw=-20), rf=foot(BACK_X, -0.4, DECK_TOP_TO_GROUND, yaw=8),
               lh=(0.3, -0.95, 0.1), rh=(-0.2, -0.95, 0.12), knee_out=0.6)
BAIL_DOWN = P(hips=(0.0, -0.5, 0.3), hips_rot=(66, 10, 15), spine=(40, 0, 0), head=(-30, 0, 0),
              lf=foot(FRONT_X - 0.05, -0.22, DECK_TOP_TO_GROUND, yaw=-20), rf=foot(BACK_X, -0.18, DECK_TOP_TO_GROUND + 0.04, yaw=8),
              lh=(0.28, -1.0, -0.06), rh=(-0.22, -1.0, -0.05), knee_out=0.7)
BAIL_REST = P(hips=(0.0, -0.5, 0.28), hips_rot=(64, 12, 15), spine=(36, 0, 0), head=(-34, 0, 8),
              lf=foot(FRONT_X - 0.05, -0.22, DECK_TOP_TO_GROUND, yaw=-20), rf=foot(BACK_X, -0.18, DECK_TOP_TO_GROUND + 0.02, yaw=8),
              lh=(0.3, -0.95, -0.07), rh=(-0.24, -0.95, -0.07), knee_out=0.7)


def ride_frame(t):
    s = math.sin(2 * math.pi * t)
    return P(hips=(0.01 * s, 0.0, 0.86 + 0.008 * math.sin(4 * math.pi * t)), spine=(12 + 1.5 * s, 0, 10),
             head=(-8, 0, 55 + 3 * math.sin(2 * math.pi * t + 1.0)))


def idle(pose, t, sway=0.012, arms=0.03):
    """`pose` alive: a slow breath and balance sway (t = 0..1 around the loop)."""
    s, c = math.sin(2 * math.pi * t), math.sin(4 * math.pi * t + 0.7)
    h = pose["hips"]
    sp = pose["spine"]
    lh, rh = pose["lh"], pose["rh"]
    return dict(pose, hips=(h[0] + sway * s, h[1], h[2] + 0.4 * sway * c), spine=(sp[0] + 1.5 * c, sp[1], sp[2] + 2 * s),
                lh=(lh[0], lh[1], lh[2] + arms * s), rh=(rh[0], rh[1], rh[2] - arms * s))


def loop(fn, length, n=8):
    return [(i / n * length, fn(i / n)) for i in range(n + 1)]


IDLE_LEN = 2.0
# name -> (loop, [(seconds, pose)]). Keys are passed through on smooth curves, not stopped at.
CLIPS = {
    "ride": (True, loop(ride_frame, IDLE_LEN)),
    "crouch": (True, loop(lambda t: idle(CROUCH, t, 0.008, 0.02), IDLE_LEN)),
    "pop": (False, [(0.0, CROUCH), (0.05, POP_DRIVE), (0.1, POP_TOP), (0.18, POP_SLIDE), (0.3, AIR_TUCK)]),
    "air": (False, [(0.0, AIR_TUCK), (0.18, AIR_PEAK), (0.6, AIR_HOLD)]),
    # Absorb deep, then rise slowly: in gameplay footage the recovery takes about half a second.
    "land": (False, [(0.0, LAND_CONTACT), (0.07, LAND_DEEP), (0.18, LAND_DEEP), (0.42, LAND_RISE), (0.6, STANCE)]),
    "push": (True, [(i / 10 * 0.62, push_frame(i / 10)) for i in range(11)]),
    "push_end": (False, [(0.0, push_frame(1.0)), (0.16, PUSH_STEP_ON), (0.32, PUSH_TURN), (0.46, STANCE)]),
    "manual": (True, loop(lambda t: idle(MANUAL, t, 0.02, 0.05), IDLE_LEN)),
    "nose_manual": (True, loop(lambda t: idle(NOSE_MANUAL, t, 0.018, 0.05), IDLE_LEN)),
    "grind": (True, loop(lambda t: idle(GRIND, t, 0.012, 0.06), IDLE_LEN)),
    "boardslide": (True, loop(lambda t: idle(BOARDSLIDE, t, 0.012, 0.06), IDLE_LEN)),
    "powerslide": (True, loop(lambda t: idle(POWERSLIDE, t, 0.01, 0.03), IDLE_LEN)),
    # The reach for the board takes a quarter second (a faster one whipped the arms).
    "grab_toe": (False, [(0.0, AIR_TUCK), (0.26, GRAB_TOE), (0.6, idle(GRAB_TOE, 0.25, 0.01, 0.0))]),
    "grab_heel": (False, [(0.0, AIR_TUCK), (0.26, GRAB_HEEL), (0.6, idle(GRAB_HEEL, 0.25, 0.01, 0.0))]),
    "bail": (False, [(0.0, STANCE), (0.18, BAIL_STUMBLE), (0.42, BAIL_REACH), (0.62, BAIL_DOWN), (1.2, BAIL_REST)]),
}
# Overlapping action: parts sample the clip this much later (s), so the head and arms trail the
# hips instead of every joint arriving on the same frame.
PART_DELAY = {"hips": 0.0, "hips_rot": 0.0, "lf": 0.0, "rf": 0.0, "knee_out": 0.0, "spine": 0.02,
              "head": 0.05, "lh": 0.05, "rh": 0.05}

# ---------------------------------------------------------------- sampling


def _flat(pose, part):
    v = pose[part]
    if isinstance(v, dict):
        return list(v["at"]) + list(v["rot"])
    if isinstance(v, (int, float)):
        return [float(v)]
    return list(v)


def _unflat(like, vals):
    if isinstance(like, dict):
        return {"at": tuple(vals[:3]), "rot": tuple(vals[3:])}
    if isinstance(like, (int, float)):
        return vals[0]
    return tuple(vals)


def mix(a, b, k):
    return {part: _unflat(a[part], [x + (y - x) * k for x, y in zip(_flat(a, part), _flat(b, part))]) for part in a}


def _hermite(times, values, t, looped, length):
    """Smooth curve through (times, values) with Catmull-Rom tangents, flattened where a key is a
    turning point or at the ends of a one-shot clip, so it never overshoots a pose."""
    n = len(times)
    if looped:
        t %= length
    t = min(max(t, times[0]), times[-1])
    i = max(0, min(n - 2, next((k for k in range(n - 1) if times[k + 1] >= t), n - 2)))

    def tangent(k):
        if 0 < k < n - 1:
            lo, hi = k - 1, k + 1
            dt_lo, dt_hi = times[k] - times[lo], times[hi] - times[k]
            v_lo, v_hi = values[lo], values[hi]
        elif looped:  # first and last keys are the same pose: neighbours across the seam
            dt_lo, dt_hi = times[-1] - times[-2], times[1] - times[0]
            v_lo, v_hi = values[-2], values[1]
        else:
            return 0.0
        d_lo = (values[k] - v_lo) / max(dt_lo, 1e-6)
        d_hi = (v_hi - values[k]) / max(dt_hi, 1e-6)
        if d_lo * d_hi <= 0:
            return 0.0
        return (v_hi - v_lo) / max(dt_lo + dt_hi, 1e-6)

    t0, t1 = times[i], times[i + 1]
    h = max(t1 - t0, 1e-6)
    u = (t - t0) / h
    p0, p1, m0, m1 = values[i], values[i + 1], tangent(i) * h, tangent(i + 1) * h
    u2, u3 = u * u, u * u * u
    return (2 * u3 - 3 * u2 + 1) * p0 + (u3 - 2 * u2 + u) * m0 + (-2 * u3 + 3 * u2) * p1 + (u3 - u2) * m1


def sample(keys, t, looped):
    """The clip's pose at time t: every part on its own smooth curve, delayed by PART_DELAY."""
    times = [k[0] for k in keys]
    length = times[-1]
    out = {}
    for part in keys[0][1]:
        flat = [_flat(p, part) for _, p in keys]
        tp = t - PART_DELAY.get(part, 0.0)
        out[part] = _unflat(keys[0][1][part],
                            [_hermite(times, [f[c] for f in flat], tp, looped, length) for c in range(len(flat[0]))])
    return out


for _name, _open in (("flip_kick", FLIP_KICK), ("flip_heel", FLIP_HEEL), ("flip_shuv", FLIP_SHUV)):
    CLIPS[_name] = (False, [(0.0, POP_TOP), (0.07, flip_part(_open, 0.6)), (0.14, _open), (0.26, _open),
                            (0.37, FLIP_CATCH), (0.45, AIR_TUCK)])


# ---------------------------------------------------------------- rig controls


def _empty(name):
    e = bpy.data.objects.new(name, None)
    bpy.context.scene.collection.objects.link(e)
    return e


def add_controls(rig):
    """IK targets and poles; returns the control objects by name."""
    c = {}
    for side, leg, knee, ankle, arm, elbow, wrist in (
        ("l", "leg.L", "knee.L", "ankle.L", "arm.L", "elbow.L", "wrist.L"),
        ("r", "leg.R", "knee.R", "ankle.R", "arm.R", "elbow.R", "wrist.R"),
    ):
        for kind in ("foot", "kpole", "hand", "epole"):
            c[f"{kind}_{side}"] = _empty(f"ctl_{kind}_{side}")
        pb = rig.pose.bones
        ik = pb[knee].constraints.new("IK")
        ik.target, ik.pole_target, ik.chain_count = c[f"foot_{side}"], c[f"kpole_{side}"], 2
        # pole angles are set by calibrate_poles (left and right rolls mirror each other)
        rot = pb[ankle].constraints.new("COPY_ROTATION")
        rot.target = c[f"foot_{side}"]
        ik = pb[elbow].constraints.new("IK")
        ik.target, ik.pole_target, ik.chain_count = c[f"hand_{side}"], c[f"epole_{side}"], 2
    return c


def calibrate_poles(rig, c, rest_ankle):
    """Pick each IK chain's pole angle so the knee or elbow points straight at its pole target
    (the bones' rolls differ per side, so no single angle works for both)."""
    apply_pose(rig, c, STANCE, rest_ankle)
    pb = rig.pose.bones
    for mid, root, end, pole in (("knee.L", "leg.L", "ankle.L", "kpole_l"), ("knee.R", "leg.R", "ankle.R", "kpole_r"),
                                 ("elbow.L", "arm.L", "wrist.L", "epole_l"), ("elbow.R", "arm.R", "wrist.R", "epole_r")):
        ik = next(con for con in pb[mid].constraints if con.type == "IK")
        best = None
        for deg in range(-180, 180, 2):
            ik.pole_angle = math.radians(deg)
            bpy.context.view_layer.update()
            a = rig.matrix_world @ pb[root].head
            axis = ((rig.matrix_world @ pb[end].head) - a).normalized()
            off = (rig.matrix_world @ pb[mid].head) - a
            off -= axis * off.dot(axis)
            want = c[pole].location - a
            want -= axis * want.dot(axis)
            err = off.angle(want) if off.length > 1e-5 and want.length > 1e-5 else 9.0
            if best is None or err < best[0]:
                best = (err, deg)
        ik.pole_angle = math.radians(best[1])
        print(f"SKATER pole {mid}: {best[1]} deg (off by {math.degrees(best[0]):.1f})")


def apply_pose(rig, c, pose, rest_ankle):
    """Put the rig and its controls in `pose` (rider frame = rig object space)."""
    pb = rig.pose.bones
    hips = pb["hips"]
    rest = rig.data.bones["hips"].head_local
    hips.location = (0, 0, 0)
    hips.rotation_mode = "XYZ"
    # Move the hips in armature space: hips' parent is the root at the origin with identity rest.
    target = Vector(pose["hips"])
    world_off = target - rest
    hips.matrix = Matrix.Translation(target) @ (
        # +pitch tips the pelvis (and everything above) toward the chest side (-Y); checked from the side.
        Euler([math.radians(a) for a in (pose["hips_rot"][0], pose["hips_rot"][1], pose["hips_rot"][2])], "XYZ")
        .to_matrix().to_4x4()) @ Matrix.Translation(-rest) @ rig.data.bones["hips"].matrix_local
    del world_off
    bend, side, twist = pose["spine"]
    for name, share in (("back_low", 0.3), ("back_mid", 0.3), ("chest", 0.4)):
        b = pb[name]
        b.rotation_mode = "XYZ"
        b.rotation_euler = (math.radians(bend * share), math.radians(twist * share), math.radians(side * share))
    nod, tilt, turn = pose["head"]
    for name, share in (("neck", 0.4), ("skull", 0.6)):
        b = pb[name]
        b.rotation_mode = "XYZ"
        b.rotation_euler = (math.radians(nod * share), math.radians(turn * share), math.radians(tilt * share))
    for side, key, hand in (("l", "lf", "lh"), ("r", "rf", "rh")):
        f = pose[key]
        c[f"foot_{side}"].location = f["at"]
        pitch, _, yaw = f["rot"]
        # The ankle's rest orientation, turned by the foot's yaw about Z and pitch about the toe axis.
        c[f"foot_{side}"].rotation_mode = "QUATERNION"
        c[f"foot_{side}"].rotation_quaternion = (
            Euler((0, 0, math.radians(yaw))).to_quaternion() @ Euler((math.radians(pitch), 0, 0)).to_quaternion()
            @ rest_ankle[side])
        hx = 1 if side == "l" else -1
        c[f"kpole_{side}"].location = (f["at"][0] + 0.15 * hx * 0.3, -1.0 * pose["knee_out"] - 0.4, 0.5)
        c[f"hand_{side}"].location = pose[hand]
        # Elbows bend out to the side and back, aimed from behind the shoulder line at a height between
        # shoulder and hand: the pole then moves smoothly with the hand and never crosses the arm
        # (a pole hung just below the hand flipped the elbow on low reaches like grabs and the bail).
        # The offset turns with the hips, so a body facing down the board (pushing) still bends its
        # elbows out and back from itself.
        yaw = math.radians(pose["hips_rot"][2])
        dx, dy = hx * 0.45, 0.55
        dx, dy = dx * math.cos(yaw) - dy * math.sin(yaw), dx * math.sin(yaw) + dy * math.cos(yaw)
        c[f"epole_{side}"].location = (pose[hand][0] + dx, pose[hand][1] + dy, 0.5 * (1.38 + pose[hand][2]))


def _fcurves(action):
    """An action's F-curves (Blender 4.4+ keeps them in layered channelbags)."""
    if hasattr(action, "layers") and action.layers:
        return [fc for layer in action.layers for strip in layer.strips for bag in strip.channelbags for fc in bag.fcurves]
    return list(action.fcurves)


def bake_clips(rig):
    """Author every clip and bake it to an action on the rig; returns the action names."""
    scene = bpy.context.scene
    scene.render.fps = FPS
    rest_ankle = {s: (rig.matrix_world @ rig.data.bones[f"ankle.{s.upper()}"].matrix_local).to_quaternion()
                  for s in ("l", "r")}
    c = add_controls(rig)
    calibrate_poles(rig, c, rest_ankle)
    bpy.context.view_layer.objects.active = rig
    made = []
    for name, (looped, keys) in CLIPS.items():
        last = max(1, round(keys[-1][0] * FPS))
        for f in range(last + 1):
            frame = 1 + f
            scene.frame_set(frame)
            apply_pose(rig, c, sample(keys, f / FPS, looped), rest_ankle)
            for ob in c.values():
                ob.keyframe_insert("location", frame=frame)
                ob.keyframe_insert("rotation_quaternion" if ob.rotation_mode == "QUATERNION" else "rotation_euler",
                                   frame=frame)
            for bname in ("hips", "back_low", "back_mid", "chest", "neck", "skull"):
                b = rig.pose.bones[bname]
                b.keyframe_insert("location", frame=frame)
                b.keyframe_insert("rotation_euler", frame=frame)
        bpy.ops.object.mode_set(mode="POSE")
        bpy.ops.pose.select_all(action="SELECT")
        # Bake from a few frames early and drop them: the IK solver starts from its last solution
        # (the previous clip's), which showed as a jump out of every clip's first frame.
        bpy.ops.nla.bake(frame_start=-3, frame_end=1 + last, only_selected=False, visual_keying=True,
                         clear_constraints=False, use_current_action=False, bake_types={"POSE"})
        action = rig.animation_data.action
        for fc in _fcurves(action):
            for kp in [k for k in fc.keyframe_points if k.co.x < 0.5]:
                fc.keyframe_points.remove(kp)
        action.name = f"sk8_{name}"
        action.use_fake_user = True
        if looped:
            action["loop"] = True
        track = rig.animation_data.nla_tracks.new()
        track.name = action.name
        track.strips.new(action.name, 1, action)
        track.mute = True
        rig.animation_data.action = None
        for ob in c.values():
            ob.animation_data_clear()
        bpy.ops.object.mode_set(mode="OBJECT")
        made.append(action.name)
    # The controls only exist to author; the export carries the baked bone keys.
    for pb in rig.pose.bones:
        for con in list(pb.constraints):
            pb.constraints.remove(con)
    for ob in c.values():
        bpy.data.objects.remove(ob)
    return made
