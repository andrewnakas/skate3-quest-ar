"""The clean tier's skate animation set, authored here as poses on IK controls and baked to actions.

Everything is in the rider frame (Blender units, metres): origin on the deck top between the feet,
+X toward the board's nose (the left, leading foot is regular stance), -Y the way the chest faces,
+Z up. Each pose places the hips, bends the spine, turns the head and puts the feet and hands
(IK targets) somewhere; clips are poses at frames, eased between, then baked to plain bone keys.

Runtime (crates/skate-game/src/clean/skater.rs) blends these clips from the engine's state.
"""
import math

import bpy
from mathutils import Euler, Matrix, Quaternion, Vector

FPS = 30
DECK_TOP_TO_GROUND = -0.105  # the ground under the wheels, in the rider frame
ANKLE = 0.075                # ankle height above whatever the foot stands on

# Where the feet stand on the deck (board.rs has the same deck: trucks at +-0.2, kicks from +-0.245):
# the front foot over the front bolts, the back foot on the tail.
FRONT_X, BACK_X = 0.19, -0.31
# Foot yaw (degrees, + turns the toes toward the nose) of the riding stance.
FRONT_YAW, BACK_YAW = 15.0, 0.0
# The ankle sits toward the heel, so a foot centred across the deck has its ankle this far to the heel
# side (+Y; the toes point -Y): with the ankle on the centre line the toes hung over the toe edge.
FOOT_Y = 0.05


def foot(x, y=0.0, z=0.0, yaw=0.0, pitch=0.0):
    """Ankle target for a foot centred across the deck at x (y offsets it), standing on a surface at
    height z; yaw turns the toes, pitch tips them."""
    return {"at": (x, y + FOOT_Y, z + ANKLE), "rot": (pitch, 0.0, yaw)}


def P(**kw):
    """A pose. Missing parts take the riding stance."""
    pose = {
        "hips": (0.0, 0.01, 0.85),
        "hips_rot": (9.0, 0.0, 0.0),      # pitch (forward bend about X), roll, yaw (degrees)
        "spine": (18.0, 2.0, 10.0),        # bend forward, side, twist toward the nose
        "head": (-6.0, 3.0, 55.0),         # nod, tilt, turn toward the nose
        # skater's stance: front toes turned 15 deg toward the nose, back foot square across the tail
        "lf": foot(FRONT_X, 0.0, 0.0, yaw=FRONT_YAW),
        "rf": foot(BACK_X, 0.0, 0.0, yaw=BACK_YAW),
        "lh": (0.28, -0.12, 0.8),          # front hand: hanging easy, a little ahead
        "rh": (-0.28, -0.08, 0.78),        # back hand
        "knee_out": 0.35,                  # knees point toward the chest side
        "knee_down": 0.0,                  # lying face down: the knees bend toward the ground
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
            lf=foot(FRONT_X + 0.1, 0.0, 0.16, yaw=10, pitch=14), rf=foot(BACK_X - 0.02, 0.0, 0.0, yaw=0),
            # hands up and out but elbows kept bent: a near-straight arm lets the elbow flip about
            lh=(0.58, -0.14, 1.07), rh=(-0.56, -0.08, 1.04), knee_out=0.55)
AIR_TUCK = P(hips=(0.0, 0.03, 0.92), hips_rot=(16, 0, 0), spine=(16, 0, 10), head=(-20, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.22, yaw=15), rf=foot(BACK_X, 0.0, 0.21, yaw=0),
             lh=(0.68, -0.1, 1.2), rh=(-0.66, -0.04, 1.17), knee_out=0.6)
# Flip tricks: the legs split up and out of the way and the board turns underneath, arms up and
# open, then the feet come back down onto it. Kickflips kick the front foot out forward over the toe
# side, heelflips forward over the heel side; shove-its lift both feet straight up.
def flip_open(front_y, back_z):
    """Both feet up in a compact tuck over the spinning board (the front foot only flicks a little
    toward its side), the chest hunched, arms thrown up and out in a V (as in gameplay footage)."""
    return P(hips=(0.0, 0.04, 0.89), hips_rot=(16, 0, 0), spine=(15, 0, 8), head=(-26, 0, 50),
             lf=foot(FRONT_X + 0.08, front_y, 0.36, yaw=5, pitch=-10), rf=foot(BACK_X - 0.02, 0.0, back_z, yaw=2),
             lh=(0.68, -0.08, 1.3), rh=(-0.66, -0.02, 1.28), knee_out=0.7)


FLIP_KICK = flip_open(-0.12, 0.33)
FLIP_HEEL = flip_open(0.1, 0.34)
FLIP_SHUV = P(hips=(0.0, 0.04, 0.89), hips_rot=(16, 0, 0), spine=(15, 0, 8), head=(-26, 0, 50),
              lf=foot(FRONT_X + 0.02, 0.0, 0.34, yaw=15), rf=foot(BACK_X - 0.02, 0.0, 0.33, yaw=0),
              lh=(0.68, -0.1, 1.28), rh=(-0.66, -0.04, 1.26), knee_out=0.65)
FLIP_CATCH = P(hips=(0.0, 0.03, 0.88), hips_rot=(16, 0, 0), spine=(18, 0, 10), head=(-26, 0, 50),
               lf=foot(FRONT_X, 0.0, 0.1, yaw=15), rf=foot(BACK_X, 0.0, 0.09, yaw=0),
               lh=(0.64, -0.1, 1.32), rh=(-0.62, -0.04, 1.29), knee_out=0.55)  # arms still high at the catch
LAND_LOW = P(hips=(0.0, 0.04, 0.72), hips_rot=(22, 0, 0), spine=(22, 0, 12), lh=(0.4, -0.26, 0.8), rh=(-0.4, -0.2, 0.78))
MANUAL = P(hips=(-0.13, 0.0, 0.8), hips_rot=(-4, 0, 0), spine=(-2, 0, 12), lh=(0.38, -0.05, 1.02), rh=(-0.5, -0.05, 1.0))
NOSE_MANUAL = P(hips=(0.14, 0.0, 0.78), hips_rot=(14, 0, 0), spine=(24, 0, 14), lh=(0.52, -0.08, 1.0), rh=(-0.36, -0.08, 1.04))
GRIND = P(hips=(0.0, 0.02, 0.74), hips_rot=(12, 0, 0), spine=(18, 0, 12), lh=(0.62, -0.05, 1.08), rh=(-0.62, -0.05, 1.04))
BOARDSLIDE = P(hips=(0.0, 0.03, 0.74), hips_rot=(12, 0, 30), spine=(16, 0, 30), head=(-6, 0, 70),
               lh=(0.6, -0.15, 1.06), rh=(-0.58, 0.1, 1.04))
POWERSLIDE = P(hips=(0.0, 0.16, 0.68), hips_rot=(-12, 0, 10), spine=(-6, 0, 20),
               lf=foot(FRONT_X, 0.03, 0.0, yaw=10), rf=foot(BACK_X, 0.03, 0.0, yaw=-3),
               lh=(0.5, 0.05, 0.95), rh=(-0.5, 0.15, 0.9))
# Grabs: a compact tuck with the board pulled up under the feet and a hand on the deck. The deck top is
# at GRAB_DECK (the soles), its toe edge at y -0.11 and heel edge at +0.11, nose x +0.4, tail x -0.4.
# Shapes after the 3-angle grab footage; every number our own. Judo kicks the front foot off, and in
# No Foot and Superman both feet leave the board, which then follows the hands (runtime).
GRAB_DECK = 0.3


def grab_pose(**kw):
    base = dict(hips=(0.0, 0.02, 0.6), hips_rot=(34, 0, 0), spine=(38, 0, 6), head=(-14, 0, 45),
                lf=foot(FRONT_X - 0.02, 0.0, GRAB_DECK, yaw=15), rf=foot(BACK_X + 0.02, 0.0, GRAB_DECK - 0.01, yaw=0),
                lh=(0.5, -0.3, 0.92), rh=(-0.5, -0.26, 0.9), knee_out=0.65)
    base.update(kw)
    return P(**base)


D = GRAB_DECK - 0.01  # hand height on the edge
GRAB_POSES = {
    # back hand
    "indy": grab_pose(rh=(0.0, -0.12, D)),
    "crail": grab_pose(rh=(0.38, -0.1, D + 0.02), spine=(44, 0, 22), head=(-18, 0, 60),
                       lf=foot(FRONT_X - 0.02, 0.0, GRAB_DECK + 0.03, yaw=15), lh=(0.46, -0.26, 0.98)),  # free hand out front
    "tail": grab_pose(rh=(-0.42, -0.04, D - 0.01), hips_rot=(28, 0, 0), spine=(34, 0, -18), head=(-14, 0, 25),
                      lh=(0.52, -0.28, 0.98)),
    "stalefish": grab_pose(rh=(-0.08, 0.13, D), hips_rot=(30, -8, 0), spine=(32, 0, -12), head=(-14, 0, 40),
                           lh=(0.52, -0.3, 0.98)),
    # front hand
    "melon": grab_pose(lh=(0.02, 0.13, D), spine=(36, 0, 14), rh=(-0.5, -0.26, 0.94)),
    "nose": grab_pose(lh=(0.42, -0.02, D + 0.02), spine=(42, 0, 20), head=(-18, 0, 60),
                      lf=foot(FRONT_X + 0.04, 0.0, GRAB_DECK + 0.02, yaw=15), rh=(-0.5, -0.26, 0.96)),
    "tuck_knee": grab_pose(hips=(0.0, 0.03, 0.58), spine=(42, 0, 8), lh=(0.06, -0.12, D + 0.06),
                           lf=foot(FRONT_X - 0.04, 0.0, GRAB_DECK + 0.07, yaw=15),
                           rf=foot(BACK_X + 0.04, 0.0, GRAB_DECK + 0.06, yaw=0), rh=(-0.46, -0.24, 0.96)),
    "judo": grab_pose(hips=(0.0, 0.0, 0.66), spine=(30, 0, 18), lh=(0.36, 0.11, D - 0.02),
                      lf=foot(0.62, -0.34, 0.46, yaw=25, pitch=-25), rh=(-0.5, -0.08, 1.16), knee_out=0.5),
    # both hands
    "double": grab_pose(spine=(42, 0, 6), lh=(0.14, -0.12, D), rh=(-0.14, -0.12, D)),
    "superman": P(hips=(0.0, 0.16, 0.86), hips_rot=(55, 0, 0), spine=(14, 0, 4), head=(-40, 0, 20),
                  lf=foot(0.18, 0.46, 0.74, yaw=15, pitch=40), rf=foot(-0.22, 0.5, 0.8, yaw=0, pitch=40),
                  lh=(0.16, -0.5, 0.92), rh=(-0.16, -0.5, 0.92), knee_out=0.2),
    "no_foot": P(hips=(0.0, 0.04, 0.92), hips_rot=(16, 0, 0), spine=(16, 0, 6), head=(-22, 0, 40),
                 lf=foot(0.46, -0.16, 0.36, yaw=0, pitch=-15), rf=foot(-0.5, 0.14, 0.4, yaw=25, pitch=15),
                 lh=(0.13, -0.3, 0.72), rh=(-0.13, -0.3, 0.72), knee_out=0.45),  # legs kicked out wide
    "stiffy": grab_pose(hips=(0.0, 0.1, 0.8), hips_rot=(58, 0, 0), spine=(40, 0, 6), head=(-6, 0, 40),
                        lf=foot(FRONT_X + 0.04, 0.0, 0.2, yaw=15), rf=foot(BACK_X - 0.04, 0.0, 0.19, yaw=0),
                        lh=(0.12, -0.12, 0.19), rh=(-0.12, -0.12, 0.19), knee_out=0.2),  # legs near straight
}
# the back hand across the front of the body to the nose (the trunk turned to it), the free front
# hand up and back behind
GRAB_POSES["seatbelt"] = grab_pose(rh=(0.36, -0.1, D + 0.02), spine=(42, 0, 26), head=(-18, 0, 62),
                                   lf=foot(FRONT_X - 0.02, 0.0, GRAB_DECK + 0.03, yaw=15),
                                   lh=(0.02, 0.3, 1.06))
# the front hand to the toe edge between the feet, the back arm out for balance
GRAB_POSES["mute"] = grab_pose(lh=(0.05, -0.12, D), spine=(40, 0, 12), head=(-16, 0, 48), rh=(-0.52, -0.24, 0.98))
# Tilted grabs: the runtime turns the deck by these (skater.rs METHOD_ROLL / ROCKET_PITCH, degrees)
# about its centre at GRAB_DECK, and the feet are authored on the turned deck.
METHOD_ROLL, ROCKET_PITCH = 38.0, 42.0


def on_deck(x, y=0.0, roll=0.0, pitch=0.0, yaw=0.0, foot_pitch=0.0):
    """A foot standing at (x, y) on a grab-height deck rolled heel edge up by `roll` and pitched
    nose up by `pitch` (degrees)."""
    r, q = math.radians(roll), math.radians(pitch)
    z = y * math.sin(r) + x * math.sin(q)
    return foot(x * math.cos(q), y * math.cos(r) - FOOT_Y * (1 - math.cos(r)), GRAB_DECK + z, yaw=yaw, pitch=foot_pitch)


# Christ air: both feet off, legs together hanging long, the board held out in the back hand and
# both arms spread wide (the runtime hangs the board from that hand). Method: the front hand on the
# heel edge, knees bent and the board pulled up behind (rolled heel edge up), the chest arched and
# the back arm up and out. Rocket: both hands on the nose, the board pitched up in front, the feet
# on the tail, the body sitting back behind it.
GRAB_POSES["christ"] = P(hips=(0.0, 0.05, 0.92), hips_rot=(4, 0, 0), spine=(2, 0, 4), head=(-14, 0, 35),
                         lf=foot(0.05, 0.04, 0.2, yaw=10, pitch=35), rf=foot(-0.07, 0.04, 0.19, yaw=0, pitch=35),
                         lh=(0.66, 0.0, 1.22), rh=(-0.66, 0.02, 1.2), knee_out=0.15)
GRAB_POSES["method"] = P(hips=(0.0, -0.04, 0.66), hips_rot=(18, 0, -6), spine=(4, 0, -14), head=(-22, 0, 70),
                         lf=on_deck(FRONT_X - 0.02, 0.0, roll=METHOD_ROLL, yaw=15),
                         rf=on_deck(BACK_X + 0.04, 0.0, roll=METHOD_ROLL, yaw=0),
                         lh=(0.04, 0.17, GRAB_DECK + 0.08), rh=(-0.5, -0.24, 1.16), knee_out=0.7)
GRAB_POSES["rocket"] = P(hips=(-0.18, 0.02, 0.62), hips_rot=(30, 0, 0), spine=(36, 0, 4), head=(-20, 0, 30),
                         lf=on_deck(FRONT_X - 0.04, pitch=ROCKET_PITCH, yaw=15),
                         rf=on_deck(BACK_X + 0.06, pitch=ROCKET_PITCH, yaw=0),
                         lh=(0.38 * math.cos(math.radians(ROCKET_PITCH)), -0.05, GRAB_DECK + 0.38 * math.sin(math.radians(ROCKET_PITCH))),
                         rh=(0.38 * math.cos(math.radians(ROCKET_PITCH)), 0.05, GRAB_DECK + 0.38 * math.sin(math.radians(ROCKET_PITCH))),
                         knee_out=0.6)
# The old names, kept for the poses other code refers to.
GRAB_TOE = GRAB_POSES["indy"]
GRAB_HEEL = GRAB_POSES["melon"]


# Breakdowns between the key poses (motion reads smooth when the in-betweens are designed, not left
# to the curve): the pop drives up off the back foot before the front foot slides; the air holds
# the tuck then reaches the legs back down for the landing.
POP_DRIVE = P(hips=(-0.02, 0.03, 0.86), hips_rot=(16, 0, 0), spine=(14, 0, 10), head=(-20, 0, 55),
              lf=foot(FRONT_X + 0.04, 0.0, 0.05, yaw=13, pitch=6), rf=foot(BACK_X - 0.01, 0.0, 0.0, yaw=0),
              lh=(0.5, -0.2, 0.95), rh=(-0.49, -0.14, 0.93), knee_out=0.45)  # arms on their way up
POP_SLIDE = P(hips=(0.0, 0.02, 0.95), hips_rot=(12, 0, 0), spine=(12, 0, 9), head=(-20, 0, 55),
              lf=foot(FRONT_X + 0.06, 0.0, 0.17, yaw=13, pitch=4), rf=foot(BACK_X, 0.0, 0.1, yaw=0),
              lh=(0.68, -0.1, 1.22), rh=(-0.66, -0.04, 1.19), knee_out=0.6)
# Gameplay footage (3 fixed angles, pose-tracked): knees ~82 deg in the tuck and the tuck deepest late
# in the air, held into the landing (no reaching the legs down first); pop to landing ~0.8 s.
AIR_PEAK = P(hips=(0.0, 0.05, 0.84), hips_rot=(26, 0, 0), spine=(26, 0, 10), head=(-26, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.4, yaw=15), rf=foot(BACK_X, 0.0, 0.39, yaw=0),
             lh=(0.7, -0.1, 1.2), rh=(-0.68, -0.04, 1.17), knee_out=0.65)  # arms wide for balance
AIR_HOLD = P(hips=(0.0, 0.05, 0.84), hips_rot=(24, 0, 0), spine=(24, 0, 10), head=(-24, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.37, yaw=15), rf=foot(BACK_X, 0.0, 0.36, yaw=0),
             lh=(0.68, -0.12, 1.16), rh=(-0.66, -0.06, 1.13), knee_out=0.65)
AIR_REACH = P(hips=(0.0, 0.03, 0.94), hips_rot=(14, 0, 0), spine=(16, 0, 10), head=(-22, 0, 55),
              lf=foot(FRONT_X, 0.0, 0.07, yaw=15), rf=foot(BACK_X, 0.0, 0.06, yaw=0),
              lh=(0.48, -0.18, 0.98), rh=(-0.46, -0.12, 0.95), knee_out=0.5)
LAND_CONTACT = P(hips=(0.0, 0.03, 0.84), hips_rot=(16, 0, 0), spine=(18, 0, 11), head=(-18, 0, 55),
                 lh=(0.6, -0.16, 1.06), rh=(-0.6, -0.1, 1.03), knee_out=0.5)
# The arms come down from the air over the whole absorb, not in the first few frames.
LAND_DEEP = P(hips=(0.0, 0.05, 0.7), hips_rot=(24, 0, 0), spine=(24, 0, 12), head=(-14, 0, 55),
              lh=(0.54, -0.22, 0.94), rh=(-0.54, -0.16, 0.92), knee_out=0.45)
LAND_RISE = P(hips=(0.0, 0.01, 0.87), hips_rot=(9, 0, 0), spine=(15, 0, 10), lh=(0.4, -0.15, 0.9), rh=(-0.41, -0.12, 0.89))


# Nollie: the mirror of the pop: weight forward, the front leg snaps straight to slam the nose, the
# back knee drives up and the back foot drags toward the tail (after the port footage, our numbers).
NOLLIE_CROUCH = P(hips=(0.05, 0.05, 0.74), hips_rot=(24, 0, 0), spine=(20, 0, 14), head=(-22, 0, 60),
                  lh=(0.36, -0.28, 0.72), rh=(-0.32, -0.22, 0.7))
NOLLIE_DRIVE = P(hips=(0.04, 0.03, 0.86), hips_rot=(16, 0, 0), spine=(14, 0, 12), head=(-20, 0, 58),
                 lf=foot(FRONT_X + 0.01, 0.0, 0.0, yaw=15), rf=foot(BACK_X - 0.04, 0.0, 0.05, yaw=0, pitch=-6),
                 lh=(0.5, -0.2, 0.95), rh=(-0.49, -0.14, 0.93), knee_out=0.45)
NOLLIE_TOP = P(hips=(0.03, 0.02, 0.94), hips_rot=(12, 0, 0), spine=(10, 0, 10), head=(-18, 0, 58),
               lf=foot(FRONT_X + 0.02, 0.0, 0.0, yaw=15), rf=foot(BACK_X - 0.1, 0.0, 0.16, yaw=4, pitch=-14),
               lh=(0.58, -0.14, 1.07), rh=(-0.56, -0.08, 1.04), knee_out=0.55)
NOLLIE_SLIDE = P(hips=(0.0, 0.02, 0.95), hips_rot=(12, 0, 0), spine=(12, 0, 9), head=(-20, 0, 55),
                 lf=foot(FRONT_X, 0.0, 0.1, yaw=15), rf=foot(BACK_X - 0.06, 0.0, 0.17, yaw=2, pitch=-4),
                 lh=(0.68, -0.1, 1.22), rh=(-0.66, -0.04, 1.19), knee_out=0.6)
# Old school. No comply: the front foot steps down off the toe edge onto the ground, the back foot
# snaps the tail and the board pops up off it, the front leg springs off the ground and both feet
# come back over the deck into the tuck. Boneless: the same step down, but the back hand reaches
# down to the toe edge by the back foot and lifts the board up with the jump, letting go in the air.
NC_STEP = P(hips=(0.03, -0.04, 0.76), hips_rot=(20, 0, 8), spine=(20, 0, 14), head=(-20, 0, 50),
            lf=foot(FRONT_X - 0.04, -0.27, DECK_TOP_TO_GROUND, yaw=30), rf=foot(BACK_X, 0.0, 0.0, yaw=BACK_YAW),
            lh=(0.36, -0.26, 0.82), rh=(-0.32, -0.2, 0.8), knee_out=0.45)
NC_POP = P(hips=(0.01, -0.03, 0.9), hips_rot=(14, 0, 6), spine=(14, 0, 12), head=(-18, 0, 52),
           lf=foot(FRONT_X - 0.02, -0.25, DECK_TOP_TO_GROUND + 0.04, yaw=28, pitch=-14),
           rf=foot(BACK_X - 0.02, 0.0, 0.1, yaw=BACK_YAW), lh=(0.5, -0.16, 1.0), rh=(-0.48, -0.12, 0.98),
           knee_out=0.5)
NC_BACK_ON = P(hips=(0.0, 0.0, 0.94), hips_rot=(14, 0, 2), spine=(15, 0, 10), head=(-20, 0, 55),
               lf=foot(FRONT_X + 0.02, -0.12, 0.22, yaw=20), rf=foot(BACK_X, 0.0, 0.2, yaw=BACK_YAW),
               lh=(0.6, -0.12, 1.12), rh=(-0.58, -0.06, 1.1), knee_out=0.55)
BL_REACH = P(hips=(0.02, -0.06, 0.66), hips_rot=(30, 0, 10), spine=(34, 0, 16), head=(-24, 0, 45),
             lf=foot(FRONT_X - 0.05, -0.28, DECK_TOP_TO_GROUND, yaw=30), rf=foot(BACK_X, 0.0, 0.0, yaw=BACK_YAW),
             lh=(0.42, -0.3, 0.84), rh=(BACK_X + 0.16, -0.12, 0.02), knee_out=0.5)
BL_JUMP = P(hips=(0.0, -0.03, 0.92), hips_rot=(16, 0, 6), spine=(18, 0, 12), head=(-18, 0, 50),
            lf=foot(FRONT_X - 0.03, -0.24, DECK_TOP_TO_GROUND + 0.06, yaw=28, pitch=-16),
            rf=foot(BACK_X, 0.0, 0.14, yaw=BACK_YAW), lh=(0.56, -0.18, 1.06), rh=(BACK_X + 0.16, -0.12, 0.16),
            knee_out=0.5)
BL_HOLD = P(hips=(0.0, 0.0, 0.9), hips_rot=(18, 0, 2), spine=(20, 0, 10), head=(-20, 0, 52),
            lf=foot(FRONT_X, -0.1, 0.24, yaw=20), rf=foot(BACK_X, 0.0, 0.22, yaw=BACK_YAW),
            lh=(0.6, -0.14, 1.1), rh=(BACK_X + 0.18, -0.12, 0.24), knee_out=0.55)


# Hardflip: the legs open front to back (front foot forward and up, back foot up behind) and the board
# flips upright between them. Tre flip: both feet up high, the back foot scooping round behind.
FLIP_HARD = P(hips=(0.0, 0.03, 0.9), hips_rot=(16, 0, 0), spine=(15, 0, 8), head=(-28, 0, 50),
              lf=foot(FRONT_X + 0.18, -0.04, 0.42, yaw=10, pitch=-20), rf=foot(BACK_X - 0.1, 0.02, 0.36, yaw=4, pitch=10),
              lh=(0.7, -0.08, 1.26), rh=(-0.68, -0.02, 1.23), knee_out=0.7)
FLIP_TRE = P(hips=(0.0, 0.04, 0.89), hips_rot=(16, 0, 0), spine=(15, 0, 6), head=(-26, 0, 50),
             lf=foot(FRONT_X + 0.06, -0.06, 0.38, yaw=5, pitch=-10), rf=foot(BACK_X - 0.06, 0.1, 0.4, yaw=25),
             lh=(0.68, -0.08, 1.3), rh=(-0.66, -0.02, 1.28), knee_out=0.7)
# 5-0: on the back truck, weight back over the tail, the front leg long, arms out for balance.
GRIND_50 = P(hips=(-0.1, 0.02, 0.76), hips_rot=(6, 0, 0), spine=(12, 0, 12), head=(-14, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.0, yaw=15), rf=foot(BACK_X, 0.0, 0.0, yaw=0),
             lh=(0.6, -0.06, 1.1), rh=(-0.62, -0.06, 1.06), knee_out=0.4)


# More grinds and slides (the runtime holds the board on the rail per kind; the feet ride the
# turned deck). Truck grinds lean the weight over the truck on the rail; slides keep the body
# across the rail like the boardslide, shifted toward the end that's on it.
GRIND_NOSE = P(hips=(0.1, 0.02, 0.76), hips_rot=(16, 0, 0), spine=(22, 0, 14), head=(-18, 0, 60),
               lh=(0.6, -0.1, 1.06), rh=(-0.58, -0.06, 1.08), knee_out=0.45)
GRIND_CROOKED = P(hips=(0.1, 0.0, 0.75), hips_rot=(18, 0, 14), spine=(22, 0, 22), head=(-18, 0, 64),
                  rf=foot(BACK_X, 0.0, 0.03, yaw=BACK_YAW), lh=(0.58, -0.16, 1.04), rh=(-0.56, 0.02, 1.1),
                  knee_out=0.5)
GRIND_OVERCROOK = P(hips=(0.1, 0.06, 0.75), hips_rot=(16, -6, -12), spine=(20, 0, 2), head=(-16, 0, 50),
                    rf=foot(BACK_X, 0.0, 0.03, yaw=BACK_YAW), lh=(0.6, 0.0, 1.08), rh=(-0.56, -0.14, 1.04),
                    knee_out=0.4)
GRIND_SMITH = P(hips=(-0.1, -0.03, 0.74), hips_rot=(8, 8, 6), spine=(14, 0, 14), head=(-16, 0, 58),
                lf=foot(FRONT_X, -0.03, -0.04, yaw=FRONT_YAW), lh=(0.6, -0.14, 1.04), rh=(-0.62, -0.02, 1.08),
                knee_out=0.45)
GRIND_FEEBLE = P(hips=(-0.1, 0.05, 0.74), hips_rot=(8, -8, -4), spine=(14, 0, 8), head=(-14, 0, 52),
                 lf=foot(FRONT_X, 0.04, 0.0, yaw=FRONT_YAW), lh=(0.62, -0.02, 1.08), rh=(-0.6, -0.12, 1.04),
                 knee_out=0.4)
SLIDE_NOSE = P(hips=(0.12, 0.03, 0.75), hips_rot=(14, 0, 26), spine=(18, 0, 28), head=(-10, 0, 72),
               lh=(0.62, -0.14, 1.04), rh=(-0.5, 0.1, 1.06))
SLIDE_TAIL = P(hips=(-0.12, 0.03, 0.75), hips_rot=(10, 0, 32), spine=(14, 0, 32), head=(-6, 0, 76),
               lh=(0.56, -0.14, 1.06), rh=(-0.6, 0.08, 1.04))
SLIDE_LIP = P(hips=(0.0, -0.02, 0.74), hips_rot=(12, 0, -28), spine=(16, 0, -26), head=(-8, 0, 30),
              lh=(0.56, 0.1, 1.06), rh=(-0.6, -0.16, 1.04))
SLIDE_NOSEBLUNT = P(hips=(0.18, -0.02, 0.7), hips_rot=(26, 0, 18), spine=(28, 0, 22), head=(-20, 0, 66),
                    lh=(0.6, -0.2, 0.98), rh=(-0.5, 0.0, 1.08), knee_out=0.55)
SLIDE_BLUNT = P(hips=(-0.18, 0.04, 0.72), hips_rot=(2, 0, 26), spine=(8, 0, 26), head=(-4, 0, 70),
                lh=(0.6, -0.08, 1.08), rh=(-0.56, 0.06, 1.0), knee_out=0.55)


# Wallride: low and compact on the wheels, the board flat on the wall below the feet (the runtime
# leans the body out from the wall only part way), arms out for balance, eyes along the wall.
WALLRIDE = P(hips=(0.0, 0.04, 0.72), hips_rot=(16, 0, 4), spine=(20, 0, 12), head=(-12, 0, 62),
             lh=(0.58, -0.12, 1.04), rh=(-0.56, -0.04, 1.06), knee_out=0.55)


# On foot, carrying the board in the back hand: the body faces the way it walks (+X here, the
# board's nose), the feet step on the ground under the deck's level.
def walk_frame(t, stride, lift, lean, arm):
    s = math.sin(2 * math.pi * t)
    def step(ph):  # x along the walk, height: the foot on the ground goes back, the other swings
        c = math.cos(2 * math.pi * ph)
        return stride * math.sin(2 * math.pi * ph), lift * max(0.0, c) ** 1.5
    lx, lz = step(t)
    rx, rz = step(t + 0.5)
    bob = 0.02 * abs(math.cos(2 * math.pi * t))
    return P(hips=(0.0, 0.0, 0.84 + bob), hips_rot=(lean, 0, 90 + 4 * s), spine=(lean * 0.6, 0, -6 * s),
             head=(-4, 0, 4 * s), lf=foot(lx, 0.1 - FOOT_Y, DECK_TOP_TO_GROUND + lz, yaw=90),
             rf=foot(rx, -0.1 - FOOT_Y, DECK_TOP_TO_GROUND + rz, yaw=90),
             lh=(-arm * s * 0.9, 0.22, 0.74), rh=(0.02 + 0.03 * s, -0.27, 0.72), knee_out=0.2)


STAND = walk_frame(0.25, 0.0, 0.0, 2.0, 0.0)
# A jump on foot: knees tucked up, the board still in the back hand.
JUMP_FOOT = dict(STAND, hips=(0.0, 0.0, 0.86), hips_rot=(10, 0, 90), lf=foot(0.08, 0.1 - FOOT_Y, 0.16, yaw=90, pitch=-10),
                 rf=foot(-0.1, -0.1 - FOOT_Y, 0.12, yaw=90, pitch=-10), lh=(0.12, 0.26, 0.86), knee_out=0.25)


# Plants on an edge (the rider sits at the edge; the runtime puts the board in the back hand).
# Footplant: the front foot planted on the edge, the back knee pulled up with the board in hand,
# the front arm out. Handplant: the front hand planted on the edge, the body tipped up over it
# and the legs tucked up with the board.
FOOTPLANT = P(hips=(-0.02, 0.03, 0.72), hips_rot=(14, 0, 4), spine=(18, 0, 10), head=(-16, 0, 50),
              lf=foot(0.04, 0.0, -0.1, yaw=FRONT_YAW), rf=foot(-0.26, 0.06, 0.3, yaw=BACK_YAW, pitch=20),
              lh=(0.42, -0.24, 0.96), rh=(-0.2, -0.06, 0.42), knee_out=0.5)
HANDPLANT_REACH = P(hips=(0.02, 0.04, 0.66), hips_rot=(40, 0, 6), spine=(30, 0, 8), head=(-20, 0, 40),
                    lf=foot(FRONT_X - 0.02, 0.0, 0.22, yaw=FRONT_YAW), rf=foot(BACK_X + 0.04, 0.0, 0.22, yaw=BACK_YAW),
                    lh=(0.06, -0.06, -0.08), rh=(-0.12, -0.04, 0.6), knee_out=0.5)
HANDPLANT_TIP = P(hips=(0.02, 0.05, 0.8), hips_rot=(100, 0, 6), spine=(22, 0, 6), head=(5, 0, 25),
                  lf=foot(0.1, 0.06, 0.9, yaw=FRONT_YAW, pitch=-40), rf=foot(-0.14, 0.08, 0.86, yaw=BACK_YAW, pitch=-40),
                  lh=(0.06, -0.06, -0.08), rh=(-0.1, 0.06, 0.7), knee_out=0.3)
HANDPLANT = P(hips=(0.02, 0.06, 0.74), hips_rot=(158, 0, 6), spine=(14, 0, 6), head=(30, 0, 10),
              lf=foot(0.12, 0.08, 1.48, yaw=FRONT_YAW, pitch=-70), rf=foot(-0.16, 0.1, 1.42, yaw=BACK_YAW, pitch=-70),
              lh=(0.06, -0.06, -0.08), rh=(-0.08, 0.1, 0.66), knee_out=0.2)


# Primo: standing tall on the board's upturned edge (it's on its side), arms out to balance.
# Casper: the board upside down with its tail up, the back foot on the raised tail, the front on
# the underside by the front truck, weight back.
PRIMO = P(hips=(0.0, 0.0, 0.94), hips_rot=(6, 0, 0), spine=(8, 0, 8), head=(-14, 0, 52),
          lf=foot(FRONT_X - 0.04, -FOOT_Y, 0.1, yaw=FRONT_YAW), rf=foot(BACK_X + 0.06, -FOOT_Y, 0.1, yaw=BACK_YAW),
          lh=(0.36, -0.14, 1.0), rh=(-0.36, -0.1, 0.98), knee_out=0.3)
CASPER = P(hips=(-0.08, 0.02, 0.8), hips_rot=(10, 0, 4), spine=(14, 0, 10), head=(-14, 0, 55),
           lf=foot(FRONT_X - 0.06, 0.0, 0.0, yaw=FRONT_YAW), rf=foot(BACK_X + 0.04, 0.0, 0.13, yaw=BACK_YAW, pitch=-12),
           lh=(0.38, -0.16, 0.98), rh=(-0.38, -0.1, 0.96), knee_out=0.45)


def flip_part(open_pose, k):
    """Part-way from the pop's extension into the flip's open pose."""
    return mix(POP_TOP, open_pose, k)


# Push timing, shared with the engine (sk8.h SK8_PUSH_PLANT/RELEASE) and the runtime: a stroke is
# PUSH_LEN long (the engine's push_period), the foot is on the ground from PLANT to RELEASE; the first
# stroke follows a PUSH_LEAD step off the board (push_lead).
PUSH_LEN, PUSH_LEAD = 0.84, 0.3
PUSH_PLANT, PUSH_RELEASE = 0.18, 0.62
GROUND = DECK_TOP_TO_GROUND


def _smooth(k):
    k = min(1.0, max(0.0, k))
    return k * k * (3 - 2 * k)


def _hermite1(p0, m0, p1, m1, u):
    u2, u3 = u * u, u * u * u
    return (2 * u3 - 3 * u2 + 1) * p0 + (u3 - 2 * u2 + u) * m0 + (-2 * u3 + 3 * u2) * p1 + (u3 - u2) * m1


def stroke(phase, ahead, behind, lift, flick):
    """The pushing foot over a stroke: (x along the board, height above the ground, toe pitch).
    On the ground it drives straight back from `ahead` to `behind` at an even pace; off it, it rises
    with the heel flicking up behind (knee bent), swings forward and reaches down to plant, on one
    smooth curve whose speed matches the drive's at both ends (no kink at plant or lift-off)."""
    drive = PUSH_RELEASE - PUSH_PLANT
    v = (behind - ahead) / drive                      # per unit of phase (negative: going back)
    if PUSH_PLANT <= phase <= PUSH_RELEASE:
        k = (phase - PUSH_PLANT) / drive
        # the toes tip down as the foot passes under and behind the hips, pushing off them
        return ahead + (behind - ahead) * k, 0.0, -18.0 * _smooth((k - 0.55) / 0.45)
    swing = 1.0 - drive
    u = ((phase - PUSH_RELEASE) % 1.0) / swing
    # leaving and arriving at a third of the drive's speed: carried back briefly, then forward
    x = _hermite1(behind, v * swing * 0.35, ahead, v * swing * 0.35, u)
    z = lift * math.sin(math.pi * u) ** 1.2 * (1.0 + flick * math.sin(math.pi * u) * (1 - u))
    # pushed off the toes, the sole turns up behind (heel flick), then levels to plant flat
    pitch = -18.0 * (1 - _smooth(u / 0.3))
    return x, z, pitch


def _drive_bump(phase, early=0.07, late=0.1):
    """0..1..0 over the drive, eased, starting a little before the plant (the standing knee loads)
    and fading after the lift-off; periodic, so the loop has no seam."""
    a, b = PUSH_PLANT - early, PUSH_RELEASE + late
    k = ((phase - a) % 1.0) / (b - a)
    return math.sin(math.pi * k) ** 2 if k < 1.0 else 0.0


def push_frame(phase):
    """Kick-push, after reference footage: the front foot turns to point down the board and the hips
    and chest face the way of travel; the back foot swings forward, plants beside the front truck,
    drives straight back along the ground past the tail, then lifts with a heel flick and comes
    forward again. The standing knee loads before the plant and the body leans forward into each
    drive and rises out of it; the shoulders counter-turn against the pushing leg and the arms swing
    with them (right arm forward as the right leg drives back). The runtime pins the planted foot to
    the ground, so the stroke on screen gets longer the faster the board rolls."""
    x, lift, pitch = stroke(phase, 0.16, -0.62, 0.15, 0.6)
    d = _drive_bump(phase)
    # the leg's counter-swing: +1 with the pushing foot furthest back, -1 furthest forward
    sw = math.cos(2 * math.pi * (phase - 0.6))
    return P(hips=(0.05 + 0.04 * d, -0.06, 0.84 - 0.1 * d), hips_rot=(10 + 8 * d, 0, 75 - 4 * sw),
             spine=(10 + 9 * d, 0, 12 + 7 * sw), head=(-8 - 6 * d, 0, 10 - 8 * sw),
             lf=foot(FRONT_X - 0.02, 0.0, 0.0, yaw=80),
             rf=foot(x, -0.22, GROUND + lift, yaw=76, pitch=pitch),  # beside the toe edge
             # about the shoulders, which the lean carries ahead of the hips
             lh=(0.22 - 0.17 * sw, 0.28, 0.78 + 0.08 * max(0.0, -sw) - 0.03 * d),
             rh=(0.22 + 0.22 * sw, -0.42, 0.78 + 0.12 * max(0.0, sw) - 0.03 * d), knee_out=0.5)


# Stepping off to push: the hips and chest turn to face down the board while the front foot pivots
# on the bolts to point the same way, then the back foot lifts off the tail and swings out over the
# toe edge into the first stroke's forward swing.
PUSH_PIVOT = P(hips=(0.03, -0.03, 0.83), hips_rot=(10, 0, 40), spine=(12, 0, 12), head=(-8, 0, 30),
               lf=foot(FRONT_X - 0.01, 0.0, 0.0, yaw=55), rf=foot(BACK_X, 0.0, 0.0, yaw=24),
               lh=(0.2, 0.06, 0.8), rh=(-0.12, -0.3, 0.8), knee_out=0.45)
PUSH_STEP_OFF = P(hips=(0.05, -0.05, 0.82), hips_rot=(11, 0, 66), spine=(12, 0, 12), head=(-8, 0, 16),
                  lf=foot(FRONT_X - 0.02, 0.0, 0.0, yaw=78), rf=foot(BACK_X + 0.01, -0.15, 0.11, yaw=60),
                  lh=(0.14, 0.2, 0.8), rh=(-0.02, -0.38, 0.8), knee_out=0.5)
# Stopping pushing: the back foot steps back onto the tail (still pointing ahead), then the front
# foot pivots back across the board and the hips and chest turn back to the side.
PUSH_STEP_ON = P(hips=(0.03, -0.04, 0.84), hips_rot=(9, 0, 45), spine=(10, 0, 12), head=(-8, 0, 30),
                 lf=foot(FRONT_X - 0.02, 0.0, 0.0, yaw=80), rf=foot(BACK_X, 0.0, 0.0, yaw=30),
                 lh=(0.14, 0.12, 0.76), rh=(-0.06, -0.3, 0.76), knee_out=0.45)
PUSH_TURN = P(hips=(0.01, -0.01, 0.84), hips_rot=(10, 0, 12), spine=(15, 0, 10), head=(-9, 0, 48),
              lf=foot(FRONT_X, 0.0, 0.0, yaw=40), rf=foot(BACK_X, 0.0, 0.0, yaw=4),
              lh=(0.28, -0.08, 0.84), rh=(-0.28, -0.12, 0.84))

# Mongo: the back foot stays on the board and the front foot pushes. The back foot slides up onto
# the back bolts and turns to point down the board, the hips and chest turn to face the way of
# travel, and the front (left) foot steps off on its own side, the heel edge, plants ahead beside
# the front truck and drives back to beside the back foot; the weight sits over the back leg.
MONGO_BACK_X = -0.17


def mongo_push_frame(phase):
    x, lift, pitch = stroke(phase, 0.42, -0.12, 0.13, 0.5)
    d = _drive_bump(phase)
    sw = math.cos(2 * math.pi * (phase - 0.6))
    # the body faces down the board (+X): its left side is +Y (heel edge), its right -Y
    return P(hips=(-0.07 + 0.03 * d, 0.04, 0.83 - 0.09 * d), hips_rot=(10 + 8 * d, 0, 68 + 4 * sw),
             spine=(10 + 9 * d, 0, 10 - 7 * sw), head=(-8 - 6 * d, 0, 12 + 8 * sw),
             lf=foot(x, 0.24 - FOOT_Y * 2, GROUND + lift, yaw=78, pitch=pitch),  # beside the heel edge
             rf=foot(MONGO_BACK_X, 0.0, 0.0, yaw=76),
             # counter-swing: the right arm forward as the left leg comes through, and back as it drives
             lh=(0.1 + 0.2 * sw, 0.38, 0.78 + 0.12 * max(0.0, sw) - 0.03 * d),
             rh=(0.1 - 0.17 * sw, -0.3, 0.78 + 0.08 * max(0.0, -sw) - 0.03 * d), knee_out=0.5)


# Stepping off mongo: the back foot slides up to the bolts as the hips turn to face down the board,
# then the front foot lifts off and steps out over the heel edge into the stroke's forward swing.
MONGO_SHIFT = P(hips=(-0.05, 0.02, 0.83), hips_rot=(10, 0, 40), spine=(12, 0, 12), head=(-8, 0, 30),
                lf=foot(FRONT_X - 0.01, 0.0, 0.0, yaw=40), rf=foot(MONGO_BACK_X - 0.04, 0.0, 0.0, yaw=55),
                lh=(0.16, 0.22, 0.8), rh=(0.1, -0.3, 0.8), knee_out=0.45)
MONGO_STEP_OFF = P(hips=(-0.07, 0.04, 0.82), hips_rot=(11, 0, 64), spine=(11, 0, 11), head=(-8, 0, 16),
                   lf=foot(FRONT_X + 0.04, 0.15, 0.11, yaw=70), rf=foot(MONGO_BACK_X, 0.0, 0.0, yaw=74),
                   lh=(0.12, 0.36, 0.8), rh=(0.16, -0.3, 0.8), knee_out=0.5)
# Stopping: the front foot steps back on over the front bolts, then the back foot slides back to the
# tail and both turn back across the board.
MONGO_STEP_ON = P(hips=(-0.05, 0.02, 0.84), hips_rot=(9, 0, 42), spine=(10, 0, 12), head=(-8, 0, 30),
                  lf=foot(FRONT_X, 0.0, 0.0, yaw=45), rf=foot(MONGO_BACK_X, 0.0, 0.0, yaw=60),
                  lh=(0.16, 0.2, 0.78), rh=(0.06, -0.3, 0.78), knee_out=0.45)


# The bail: the back foot steps off and the body pitches forward over the toe side (stumble), the
# hands go out to catch the fall, the body lands on them and the knees, then sinks and settles.
BAIL_STUMBLE = P(hips=(0.05, -0.2, 0.74), hips_rot=(34, 8, 15), spine=(30, 0, 4), head=(-10, 0, 20),
                 lf=foot(FRONT_X, -0.05, 0.0, yaw=15), rf=foot(BACK_X + 0.1, -0.35, DECK_TOP_TO_GROUND + 0.08, yaw=0),
                 lh=(0.35, -0.55, 0.8), rh=(-0.2, -0.55, 0.82), knee_out=0.5)
BAIL_REACH = P(hips=(0.05, -0.42, 0.5), hips_rot=(58, 10, 15), spine=(46, 0, 0), head=(4, 0, 5),
               lf=foot(FRONT_X - 0.05, -0.2, DECK_TOP_TO_GROUND + 0.05, yaw=15), rf=foot(BACK_X, -0.4, DECK_TOP_TO_GROUND, yaw=0),
               lh=(0.3, -0.95, 0.1), rh=(-0.2, -0.95, 0.12), knee_out=0.6)
BAIL_DOWN = P(hips=(0.0, -0.5, 0.3), hips_rot=(66, 10, 15), spine=(40, 0, 0), head=(-30, 0, 0),
              lf=foot(FRONT_X - 0.05, -0.22, DECK_TOP_TO_GROUND, yaw=15), rf=foot(BACK_X, -0.18, DECK_TOP_TO_GROUND + 0.04, yaw=0),
              lh=(0.28, -1.0, -0.06), rh=(-0.22, -1.0, -0.05), knee_out=0.7)
BAIL_REST = P(hips=(0.0, -0.5, 0.28), hips_rot=(64, 12, 15), spine=(36, 0, 0), head=(-34, 0, 8),
              lf=foot(FRONT_X - 0.05, -0.22, DECK_TOP_TO_GROUND, yaw=15), rf=foot(BACK_X, -0.18, DECK_TOP_TO_GROUND + 0.02, yaw=0),
              lh=(0.3, -0.95, -0.07), rh=(-0.24, -0.95, -0.07), knee_out=0.7)


# Staged bail (the engine's bail stages: flail, tumble, lie, get up). Thrown forward the body turns
# toward where it was going, the hands go out to catch it and it ends face down, sliding; slipped
# out (the board shot ahead) it sits down hard and rolls onto its back, the feet ahead. Getting up
# pushes up to a knee, stands and steps back onto the board (the runtime puts the board under it).
G = DECK_TOP_TO_GROUND
BAIL_FLAIL_A = P(hips=(0.05, 0.0, 0.86), hips_rot=(-8, 6, 30), spine=(-6, 4, 10), head=(-24, 0, 30),
                 lf=foot(FRONT_X + 0.12, -0.08, 0.16, yaw=30, pitch=-15), rf=foot(BACK_X - 0.05, 0.1, 0.06, yaw=10),
                 lh=(0.42, -0.25, 1.45), rh=(-0.52, 0.12, 0.98), knee_out=0.5)
BAIL_FLAIL_B = P(hips=(0.05, 0.0, 0.84), hips_rot=(-4, -6, 36), spine=(-2, -4, 14), head=(-20, 0, 34),
                 lf=foot(FRONT_X + 0.08, -0.12, 0.1, yaw=30, pitch=-10), rf=foot(BACK_X - 0.1, 0.06, 0.14, yaw=10),
                 lh=(0.55, 0.12, 0.98), rh=(-0.4, -0.28, 1.45), knee_out=0.55)
BAIL_TOSS = P(hips=(0.14, -0.08, 0.7), hips_rot=(38, 6, 50), spine=(28, 0, 10), head=(-24, 0, 30),
              lf=foot(FRONT_X + 0.12, -0.05, 0.0, yaw=40), rf=foot(BACK_X + 0.3, -0.18, G + 0.14, yaw=60),
              lh=(0.62, -0.32, 0.78), rh=(0.36, -0.48, 0.72), knee_out=0.5)
BAIL_CATCH = P(hips=(0.3, -0.1, 0.42), hips_rot=(64, 6, 80), spine=(36, 0, 0), head=(-34, 0, 0),
               lf=foot(-0.02, 0.04, G + 0.06, yaw=90, pitch=35), rf=foot(-0.16, -0.2, G + 0.1, yaw=90, pitch=35),
               lh=(0.86, 0.16, G + 0.1), rh=(0.84, -0.24, G + 0.1), knee_out=0.6, knee_down=0.5)
BAIL_PRONE = P(hips=(0.36, -0.04, G + 0.17), hips_rot=(84, 4, 90), spine=(6, 0, 0), head=(-44, 0, 10),
               lf=foot(-0.44, 0.12, G + 0.04, yaw=90, pitch=70), rf=foot(-0.47, -0.14, G + 0.05, yaw=90, pitch=70),
               lh=(0.76, 0.3, G + 0.06), rh=(0.72, -0.32, G + 0.06), knee_out=0.0, knee_down=1.0)
BAIL_PRONE_REST = P(hips=(0.36, -0.04, G + 0.16), hips_rot=(86, 2, 92), spine=(4, 0, 0), head=(-26, 4, 20),
                    lf=foot(-0.45, 0.14, G + 0.04, yaw=95, pitch=72), rf=foot(-0.47, -0.16, G + 0.04, yaw=85, pitch=72),
                    lh=(0.7, 0.34, G + 0.05), rh=(0.66, -0.34, G + 0.05), knee_out=0.0, knee_down=1.0)
BAIL_SLIP = P(hips=(-0.08, 0.04, 0.72), hips_rot=(-26, 0, 40), spine=(-10, 0, 10), head=(-6, 0, 30),
              lf=foot(FRONT_X + 0.26, -0.02, 0.2, yaw=60, pitch=-30), rf=foot(0.1, 0.06, 0.26, yaw=60, pitch=-30),
              lh=(0.45, -0.3, 1.1), rh=(-0.56, -0.3, 0.78), knee_out=0.5)
BAIL_SIT = P(hips=(-0.16, 0.0, G + 0.15), hips_rot=(-34, 0, 90), spine=(-4, 0, 0), head=(4, 0, 0),
             lf=foot(0.52, 0.12, G + 0.02, yaw=90, pitch=-40), rf=foot(0.46, -0.13, G + 0.03, yaw=90, pitch=-40),
             lh=(-0.42, 0.26, G + 0.05), rh=(-0.42, -0.26, G + 0.05), knee_out=0.3)
BAIL_SUPINE = P(hips=(-0.16, 0.0, G + 0.15), hips_rot=(-82, 0, 90), spine=(-6, 0, 0), head=(16, 0, 0),
                lf=foot(0.62, 0.14, G + 0.03, yaw=90, pitch=-60), rf=foot(0.58, -0.16, G + 0.05, yaw=90, pitch=-60),
                lh=(-0.3, 0.52, G + 0.05), rh=(-0.32, -0.52, G + 0.05), knee_out=0.2)
BAIL_SUPINE_REST = P(hips=(-0.16, 0.0, G + 0.14), hips_rot=(-84, 2, 92), spine=(-6, 0, 0), head=(10, 6, 14),
                     lf=foot(0.6, 0.18, G + 0.03, yaw=100, pitch=-62), rf=foot(0.57, -0.17, G + 0.04, yaw=80, pitch=-62),
                     lh=(-0.26, 0.56, G + 0.05), rh=(-0.3, -0.5, G + 0.05), knee_out=0.25)
GETUP_PUSH = P(hips=(0.18, -0.04, 0.3), hips_rot=(68, 0, 90), spine=(14, 0, 0), head=(-36, 0, 0),
               lf=foot(-0.36, 0.12, G + 0.05, yaw=90, pitch=70), rf=foot(-0.38, -0.14, G + 0.05, yaw=90, pitch=70),
               lh=(0.62, 0.22, G + 0.02), rh=(0.6, -0.22, G + 0.02), knee_out=0.0, knee_down=1.0)
# the front knee drawn up under the body before the foot plants ahead (a leg swung straight
# through flipped the knee)
GETUP_TUCK = P(hips=(0.1, -0.02, 0.36), hips_rot=(52, 0, 90), spine=(18, 0, 0), head=(-28, 0, 0),
               lf=foot(-0.02, 0.1, G + 0.1, yaw=90, pitch=40), rf=foot(-0.36, -0.13, G + 0.05, yaw=90, pitch=65),
               lh=(0.5, 0.22, G + 0.02), rh=(0.5, -0.22, G + 0.02), knee_out=0.1, knee_down=0.6)
GETUP_KNEE = P(hips=(0.0, 0.0, 0.5), hips_rot=(24, 0, 90), spine=(14, 0, 0), head=(-14, 0, 0),
               lf=foot(0.28, 0.1, G, yaw=90), rf=foot(-0.34, -0.12, G + 0.06, yaw=90, pitch=60),
               lh=(0.3, 0.14, 0.52), rh=(0.22, -0.18, 0.46), knee_out=0.2)
GETUP_SIT_UP = P(hips=(-0.12, 0.0, G + 0.17), hips_rot=(-20, 0, 90), spine=(10, 0, 0), head=(-8, 0, 0),
                 lf=foot(0.4, 0.12, G + 0.02, yaw=90, pitch=-20), rf=foot(0.22, -0.13, G, yaw=90),
                 lh=(-0.36, 0.26, G + 0.04), rh=(0.1, -0.3, 0.2), knee_out=0.4)
GETUP_RISE = P(hips=(0.0, 0.0, 0.8), hips_rot=(14, 0, 60), spine=(12, 0, 10), head=(-8, 0, 30),
               lf=foot(FRONT_X, 0.0, G + 0.04, yaw=50), rf=foot(BACK_X, 0.0, G + 0.04, yaw=30),
               lh=(0.3, -0.12, 0.78), rh=(-0.28, -0.1, 0.76), knee_out=0.3)


# Fakie into switch: the feet walk round on the deck to a stance the same both ways (feet the same
# distance either side of the middle, toes square, chest and head straight over the toe edge), the
# back foot first. The runtime plays this, then the same clip backwards on the mirrored body (the
# other foot forward), so the shuffle carries on out to the switch stance without a seam.
SWITCH_X = 0.25
SWITCH_SYM = P(hips=(0.0, 0.01, 0.84), hips_rot=(10, 0, 0), spine=(18, 0, 0), head=(-10, 0, 0),
               lf=foot(SWITCH_X, 0.0, 0.0, yaw=6), rf=foot(-SWITCH_X, 0.0, 0.0, yaw=-6),
               lh=(0.3, -0.1, 0.79), rh=(-0.3, -0.1, 0.79), knee_out=0.35)
# the back foot steps up toward the middle first, the weight over the front foot, the head coming round
SWITCH_STEP = P(hips=(0.03, 0.01, 0.86), hips_rot=(9, 0, 0), spine=(18, 1, 5), head=(-8, 1, 30),
                lf=foot(FRONT_X + 0.02, 0.0, 0.0, yaw=12), rf=foot((BACK_X - SWITCH_X) / 2, 0.0, 0.035, yaw=-3),
                lh=(0.29, -0.11, 0.8), rh=(-0.29, -0.09, 0.79), knee_out=0.35)
# then the front foot slides out to its spot
SWITCH_SLIDE = P(hips=(0.0, 0.01, 0.85), hips_rot=(10, 0, 0), spine=(18, 0, 2), head=(-9, 0, 12),
                 lf=foot((FRONT_X + SWITCH_X) / 2 + 0.01, 0.0, 0.02, yaw=8), rf=foot(-SWITCH_X, 0.0, 0.0, yaw=-6),
                 lh=(0.3, -0.1, 0.79), rh=(-0.3, -0.1, 0.79), knee_out=0.35)

def ride_frame(t):
    """Rolling along, loose: a knee bounce twice a loop, the weight rocking between the feet, the
    arms swinging a little out of time with it and the head easy."""
    s, b = math.sin(2 * math.pi * t), math.sin(4 * math.pi * t)
    base = P()
    lh, rh = base["lh"], base["rh"]
    return P(hips=(0.018 * s, 0.01, 0.85 + 0.014 * b), hips_rot=(9 + 1.5 * b, 0.8 * s, 0),
             spine=(18 + 2.5 * s, 2 + 1.5 * math.sin(2 * math.pi * t + 0.8), 10 + 2 * s),
             head=(-6 + 2 * math.sin(4 * math.pi * t + 1.4), 3, 55 + 4 * math.sin(2 * math.pi * t + 1.0)),
             lh=(lh[0] + 0.03 * math.sin(2 * math.pi * t + 0.6), lh[1], lh[2] + 0.025 * math.sin(2 * math.pi * t + 1.2)),
             rh=(rh[0] - 0.03 * math.sin(2 * math.pi * t + 0.3), rh[1], rh[2] - 0.025 * math.sin(2 * math.pi * t + 0.9)))


def idle(pose, t, sway=0.016, arms=0.045):
    """`pose` alive: a slow breath and balance sway (t = 0..1 around the loop)."""
    s, c = math.sin(2 * math.pi * t), math.sin(4 * math.pi * t + 0.7)
    h = pose["hips"]
    sp = pose["spine"]
    lh, rh = pose["lh"], pose["rh"]
    return dict(pose, hips=(h[0] + sway * s, h[1], h[2] + 0.6 * sway * c), spine=(sp[0] + 2.5 * c, sp[1] + 1.5 * s, sp[2] + 2.5 * s),
                lh=(lh[0] + 0.4 * arms * c, lh[1], lh[2] + arms * s), rh=(rh[0] - 0.4 * arms * c, rh[1], rh[2] - arms * s))


def loop(fn, length, n=8):
    return [(i / n * length, fn(i / n)) for i in range(n + 1)]


IDLE_LEN = 2.0
# name -> (loop, [(seconds, pose)]). Keys are passed through on smooth curves, not stopped at.
CLIPS = {
    "ride": (True, loop(ride_frame, IDLE_LEN)),
    "crouch": (True, loop(lambda t: idle(CROUCH, t, 0.008, 0.02), IDLE_LEN)),
    "pop": (False, [(0.0, CROUCH), (0.05, POP_DRIVE), (0.1, POP_TOP), (0.18, POP_SLIDE), (0.3, AIR_TUCK)]),
    "pop_no_comply": (False, [(0.0, NC_STEP), (0.08, NC_POP), (0.2, NC_BACK_ON), (0.34, AIR_TUCK)]),
    "pop_boneless": (False, [(0.0, BL_REACH), (0.1, BL_JUMP), (0.24, BL_HOLD), (0.4, BL_HOLD), (0.52, AIR_TUCK)]),
    "pop_nollie": (False, [(0.0, NOLLIE_CROUCH), (0.05, NOLLIE_DRIVE), (0.1, NOLLIE_TOP), (0.18, NOLLIE_SLIDE),
                           (0.3, AIR_TUCK)]),
    "grind_5_0": (True, loop(lambda t: idle(GRIND_50, t, 0.012, 0.06), IDLE_LEN)),
    "footplant": (False, [(0.0, AIR_TUCK), (0.14, FOOTPLANT), (0.7, idle(FOOTPLANT, 0.25, 0.01, 0.0))]),
    # the hand goes down to the edge first, still upright, then the body tips over it
    "handplant": (False, [(0.0, AIR_TUCK), (0.22, HANDPLANT_REACH), (0.36, HANDPLANT_TIP), (0.52, HANDPLANT),
                          (1.1, idle(HANDPLANT, 0.25, 0.01, 0.0))]),
    "primo": (True, loop(lambda t: idle(PRIMO, t, 0.016, 0.05), IDLE_LEN)),
    "casper": (True, loop(lambda t: idle(CASPER, t, 0.012, 0.04), IDLE_LEN)),
    "jump_foot": (False, [(0.0, STAND), (0.12, JUMP_FOOT), (0.5, JUMP_FOOT)]),
    # half of the shuffle into switch (0.35 s; the mirrored half plays it backwards)
    "switch": (False, [(0.0, STANCE), (0.13, SWITCH_STEP), (0.25, SWITCH_SLIDE), (0.35, SWITCH_SYM)]),
    "stand": (True, loop(lambda t: idle(STAND, t, 0.006, 0.02), IDLE_LEN)),
    "walk": (True, [(i / 16 * 1.05, walk_frame(i / 16, 0.26, 0.09, 4.0, 0.16)) for i in range(17)]),
    "run": (True, [(i / 16 * 0.7, walk_frame(i / 16, 0.42, 0.17, 12.0, 0.26)) for i in range(17)]),
    "wallride": (True, loop(lambda t: idle(WALLRIDE, t, 0.01, 0.05), IDLE_LEN)),
    **{name: (True, loop(lambda t, pose=pose: idle(pose, t, 0.012, 0.06), IDLE_LEN)) for name, pose in (
        ("grind_nose", GRIND_NOSE), ("grind_crooked", GRIND_CROOKED), ("grind_overcrook", GRIND_OVERCROOK),
        ("grind_smith", GRIND_SMITH), ("grind_feeble", GRIND_FEEBLE), ("slide_nose", SLIDE_NOSE),
        ("slide_tail", SLIDE_TAIL), ("slide_lip", SLIDE_LIP), ("slide_noseblunt", SLIDE_NOSEBLUNT),
        ("slide_blunt", SLIDE_BLUNT))},
    "air": (False, [(0.0, AIR_TUCK), (0.18, AIR_PEAK), (0.6, AIR_HOLD)]),
    # Absorb deep, then rise slowly: in gameplay footage the recovery takes about half a second.
    "land": (False, [(0.0, LAND_CONTACT), (0.07, LAND_DEEP), (0.18, LAND_DEEP), (0.42, LAND_RISE), (0.6, STANCE)]),
    "push": (True, [(i / 20 * PUSH_LEN, push_frame(i / 20)) for i in range(21)]),
    "push_start": (False, [(0.0, STANCE), (0.13, PUSH_PIVOT), (0.22, PUSH_STEP_OFF), (PUSH_LEAD, push_frame(0.0))]),
    "push_end": (False, [(0.0, push_frame(1.0)), (0.16, PUSH_STEP_ON), (0.32, PUSH_TURN), (0.46, STANCE)]),
    "push_mongo": (True, [(i / 20 * PUSH_LEN, mongo_push_frame(i / 20)) for i in range(21)]),
    "push_start_mongo": (False, [(0.0, STANCE), (0.13, MONGO_SHIFT), (0.22, MONGO_STEP_OFF),
                                 (PUSH_LEAD, mongo_push_frame(0.0))]),
    "push_end_mongo": (False, [(0.0, mongo_push_frame(1.0)), (0.16, MONGO_STEP_ON), (0.32, PUSH_TURN), (0.46, STANCE)]),
    "manual": (True, loop(lambda t: idle(MANUAL, t, 0.02, 0.05), IDLE_LEN)),
    "nose_manual": (True, loop(lambda t: idle(NOSE_MANUAL, t, 0.018, 0.05), IDLE_LEN)),
    "grind": (True, loop(lambda t: idle(GRIND, t, 0.012, 0.06), IDLE_LEN)),
    "boardslide": (True, loop(lambda t: idle(BOARDSLIDE, t, 0.012, 0.06), IDLE_LEN)),
    "powerslide": (True, loop(lambda t: idle(POWERSLIDE, t, 0.01, 0.03), IDLE_LEN)),
    # thrown forward: from the flail, the hands out to catch it, face down sliding, then still
    "bail": (False, [(0.0, BAIL_FLAIL_A), (0.16, BAIL_TOSS), (0.42, BAIL_CATCH), (0.72, BAIL_PRONE), (1.4, BAIL_PRONE_REST)]),
    "bail_flail": (True, [(0.0, BAIL_FLAIL_A), (0.25, BAIL_FLAIL_B), (0.5, BAIL_FLAIL_A)]),
    # slipped out: sits down hard, rolls onto the back, feet ahead
    "bail_back": (False, [(0.0, BAIL_FLAIL_A), (0.14, BAIL_SLIP), (0.38, BAIL_SIT), (0.7, BAIL_SUPINE), (1.4, BAIL_SUPINE_REST)]),
    "get_up": (False, [(0.0, BAIL_PRONE_REST), (0.32, GETUP_PUSH), (0.6, GETUP_TUCK), (0.92, GETUP_KNEE), (1.25, GETUP_RISE),
                       (1.6, STANCE)]),
    "get_up_back": (False, [(0.0, BAIL_SUPINE_REST), (0.4, GETUP_SIT_UP), (0.8, GETUP_KNEE), (1.2, GETUP_RISE), (1.6, STANCE)]),
}
# Overlapping action: parts sample the clip this much later (s), so the head and arms trail the
# hips instead of every joint arriving on the same frame.
PART_DELAY = {"hips": 0.0, "hips_rot": 0.01, "lf": 0.0, "rf": 0.0, "knee_out": 0.0, "spine": 0.03,
              "head": 0.07, "lh": 0.075, "rh": 0.075}
# Parts on curves flattened at every turning point (they arrive and stop on each key): the feet,
# which must land exactly where the deck is. Everything else flows through its keys on plain
# Catmull-Rom curves, carrying a little past a pose before it settles (follow-through), which is
# most of the difference between a stiff and a fluid look.
FLAT_PARTS = {"lf", "rf"}

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


def _hermite(times, values, t, looped, length, flat=True):
    """Smooth curve through (times, values) with Catmull-Rom tangents; `flat` flattens them where a
    key is a turning point, so it never overshoots a pose. The ends of a one-shot clip are always
    flat (it settles into its last pose)."""
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
        if flat and d_lo * d_hi <= 0:
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
                            [_hermite(times, [f[c] for f in flat], tp, looped, length, part in FLAT_PARTS)
                             for c in range(len(flat[0]))])
    return out


SHOULDER_UP, SHOULDER_HALF = 0.47, 0.18
ARM_REACH_MAX = 0.41
# The bail's hands reach for the ground and are left alone; in a grab only the free hand (above the
# board) relaxes.
REACHING = ("bail", "get_up")
GRABBING = ("grab_", "pop_boneless", "footplant", "handplant")


def shoulder(pose, side):
    """Where the shoulder is (rider frame), estimated from the hips and the trunk's bend and turn;
    side +1 the front (left) shoulder, -1 the back one."""
    hx, hy, hz = pose["hips"]
    pitch = math.radians(pose["hips_rot"][0] + pose["spine"][0] * 0.8)
    yaw = math.radians(pose["hips_rot"][2])
    cy, sy = math.cos(yaw), math.sin(yaw)
    up = (SHOULDER_UP * math.sin(pitch) * sy, -SHOULDER_UP * math.sin(pitch) * cy, SHOULDER_UP * math.cos(pitch))
    return (hx + up[0] + side * SHOULDER_HALF * cy, hy + up[1] + side * SHOULDER_HALF * sy, hz + up[2])


def relaxed(pose, above=-9.0):
    """The arms never quite straight: each hand is pulled in toward its shoulder in proportion to
    how far it reaches, so the elbows always keep some bend and the arms read loose."""
    hx, hy, hz = pose["hips"]
    pitch = math.radians(pose["hips_rot"][0] + pose["spine"][0] * 0.8)
    yaw = math.radians(pose["hips_rot"][2])
    cy, sy = math.cos(yaw), math.sin(yaw)
    # the trunk leans toward the chest (-Y before the yaw), the shoulder line runs along X
    up = (SHOULDER_UP * math.sin(pitch) * sy, -SHOULDER_UP * math.sin(pitch) * cy, SHOULDER_UP * math.cos(pitch))
    out = dict(pose)
    for key, side in (("lh", 1.0), ("rh", -1.0)):
        # fades out below `above` (a grabbing hand going down to the board), never a step: a
        # hand target that jumped there flipped the elbow mid-reach
        weight = min(1.0, max(0.0, (pose[key][2] - above) / 0.2))
        if weight <= 0.0:
            continue
        sh = (hx + up[0] + side * SHOULDER_HALF * cy, hy + up[1] + side * SHOULDER_HALF * sy, hz + up[2])
        d = [a - b for a, b in zip(pose[key], sh)]
        reach = math.sqrt(sum(c * c for c in d))
        if reach < 1e-4:
            continue
        k = 1.0 - 0.3 * min(1.0, max(0.0, (reach - 0.24) / 0.3))
        # and never past ~80% of the arm's length (0.52 m), however far out the pose put the hand
        k = min(k, ARM_REACH_MAX / reach)
        fwd = (0.03 * sy, -0.03 * cy)  # every hand rides slightly in front of the body
        hang = min(1.0, max(0.0, (sh[2] - pose[key][2] - 0.2) / 0.15))
        if hang > 0:
            # A hanging arm isn't pulled up toward the shoulder (that bends it into hands on hips):
            # the hand comes a little up and forward, so the elbow softens with the forearm ahead.
            pulled = (sh[0] + d[0] * k, sh[1] + d[1] * k, sh[2] + d[2] * k)
            low = (pose[key][0] + 2 * fwd[0], pose[key][1] + 2 * fwd[1], pose[key][2] + 0.04)
            new = tuple(p + (q - p) * hang for p, q in zip(pulled, low))
        else:
            new = (sh[0] + d[0] * k, sh[1] + d[1] * k, sh[2] + d[2] * k)
        # Raised arms come down toward chest height: out for balance, never held up high (the
        # elbows and shoulders stay down).
        lift = max(0.0, new[2] - hz)
        new = (new[0] + fwd[0], new[1] + fwd[1], new[2] - 0.9 * lift)
        # and kept in near the body: reach out to the side past ~28 cm from the middle is cut
        # by more than half (arms held out wide read stiff)
        rel = (new[0] - hx, new[1] - hy)
        out_side = rel[0] * cy + rel[1] * sy
        if abs(out_side) > 0.28:
            cut = (abs(out_side) - 0.28) * 0.55 * (1 if out_side > 0 else -1)
            new = (new[0] - cut * cy, new[1] - cut * sy, new[2])
        out[key] = tuple(o + (n - o) * weight for o, n in zip(pose[key], new))
    return out


# One clip per grab style: reach for the board over a quarter second, then hold with a slight sway.
for _name, _pose in GRAB_POSES.items():
    CLIPS[f"grab_{_name}"] = (False, [(0.0, AIR_TUCK), (0.3, _pose), (0.62, idle(_pose, 0.25, 0.01, 0.0))])
for _name, _open in (("flip_kick", FLIP_KICK), ("flip_heel", FLIP_HEEL), ("flip_shuv", FLIP_SHUV),
                     ("flip_hard", FLIP_HARD), ("flip_tre", FLIP_TRE)):
    # The arms take most of a fifth of a second to go up into the V.
    CLIPS[_name] = (False, [(0.0, POP_TOP), (0.1, flip_part(_open, 0.5)), (0.2, _open), (0.28, _open),
                            (0.38, FLIP_CATCH), (0.45, AIR_TUCK)])


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


# The elbow pole's direction last frame, per arm (reset at each clip's start).
_PREV_SIDE = {}
# Shoulders held down: the collars turn down this much (degrees), more with the arm raised.
COLLAR_DOWN, COLLAR_DOWN_RAISED = 3.0, 4.0
# Fingers curled in a relaxed loose fist (degrees per joint, base to tip; + curls toward the palm).
FINGER_CURL = (30.0, 45.0, 30.0)
THUMB_CURL = (12.0, 22.0, 18.0)
FINGER_BONES = [f"sk_{f}_0{j}_{s}" for s in ("l", "r") for f in ("index", "middle", "ring", "pinky", "thumb")
                for j in (1, 2, 3)]


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
    for side_name, hand, sign in (("L", "lh", 1.0), ("R", "rh", -1.0)):
        b = pb[f"collar.{side_name}"]
        b.rotation_mode = "QUATERNION"
        rest = rig.data.bones[f"collar.{side_name}"].matrix_local.to_3x3()
        axis = (rest.inverted() @ Vector((0.0, 1.0, 0.0))).normalized()
        raised = min(1.0, max(0.0, (pose[hand][2] - pose["hips"][2] - 0.1) / 0.3))
        b.rotation_quaternion = Quaternion(axis, sign * math.radians(COLLAR_DOWN + COLLAR_DOWN_RAISED * raised))
    for name in FINGER_BONES:
        b = pb[name]
        b.rotation_mode = "XYZ"
        j = int(name.split("_")[2]) - 1
        b.rotation_euler = (math.radians((THUMB_CURL if "thumb" in name else FINGER_CURL)[j]), 0, 0)
    nod, tilt, turn = pose["head"]
    for name, share in (("neck", 0.4), ("skull", 0.6)):
        b = pb[name]
        b.rotation_mode = "XYZ"
        b.rotation_euler = (math.radians(nod * share), math.radians(turn * share), math.radians(tilt * share))
    for side_name, key, hand in (("l", "lf", "lh"), ("r", "rf", "rh")):
        side = side_name
        f = pose[key]
        c[f"foot_{side}"].location = f["at"]
        pitch, _, yaw = f["rot"]
        # The ankle's rest orientation, turned by the foot's yaw about Z and pitch about the toe axis.
        c[f"foot_{side}"].rotation_mode = "QUATERNION"
        c[f"foot_{side}"].rotation_quaternion = (
            Euler((0, 0, math.radians(yaw))).to_quaternion() @ Euler((math.radians(pitch), 0, 0)).to_quaternion()
            @ rest_ankle[side])
        hx = 1 if side == "l" else -1
        # The knees point the way the pelvis faces: toward the chest, turned with the hips' yaw (a
        # body facing down the board to push bends its knees forward, not out over the toe edge).
        kyaw = math.radians(pose["hips_rot"][2])
        kx, ky = 0.15 * hx * 0.3, -1.0 * pose["knee_out"] - 0.4
        kx, ky = kx * math.cos(kyaw) - ky * math.sin(kyaw), kx * math.sin(kyaw) + ky * math.cos(kyaw)
        kp = (f["at"][0] + kx, ky, 0.5)
        # tipped well over (an invert) the knees still bend toward where the chest faces: the
        # pole goes out in front of the chest in the hips' own frame
        kpitch = math.radians(pose["hips_rot"][0])
        w = min(1.0, max(0.0, (pose["hips_rot"][0] - 80) / 40))
        if w > 0:
            hx_, hy_, hz_ = pose["hips"]
            front = (0.0, -math.cos(kpitch), -math.sin(kpitch))
            fp = (f["at"][0] + kx, hy_ + front[1] * 1.0, hz_ + front[2] * 1.0)
            kp = tuple(a + (b - a) * w for a, b in zip(kp, fp))
        # face down on the ground (a bail): the pole goes under the leg, so the knees bend down into
        # the ground the whole way through instead of flipping over as the legs stretch out behind
        kd = pose.get("knee_down", 0.0)
        if kd > 0:
            hx_, hy_, _ = pose["hips"]
            dp = ((hx_ + f["at"][0]) * 0.5, (hy_ + f["at"][1]) * 0.5, DECK_TOP_TO_GROUND - 0.6)
            kp = tuple(a + (b - a) * kd for a, b in zip(kp, dp))
        c[f"kpole_{side}"].location = kp
        c[f"hand_{side}"].location = pose[hand]
        # Elbows bend out, back and down from the body (turned with the hips, so a body facing down
        # the board to push bends them back from itself). The pole is that preferred direction with
        # its part along the shoulder-to-hand line removed, put beside the arm's middle: it can never
        # lie along the arm, which is what flipped elbows whenever a hand reached the way a fixed
        # pole pointed (grabs, a hand passing in front of the shoulder).
        sh = shoulder(pose, hx)
        yaw = math.radians(pose["hips_rot"][2])
        cy, sy = math.cos(yaw), math.sin(yaw)
        want = (hx * 0.2, 0.3, -1.0)  # down: elbows low and in
        # as the body leans (an invert turns it right over), "down" goes with the trunk
        pt = math.radians(pose["hips_rot"][0])
        want = (want[0], want[1] * math.cos(pt) - want[2] * math.sin(pt), want[1] * math.sin(pt) + want[2] * math.cos(pt))
        want = (want[0] * cy - want[1] * sy, want[0] * sy + want[1] * cy, want[2])
        arm = [h - q for h, q in zip(pose[hand], sh)]
        n = math.sqrt(sum(a * a for a in arm)) or 1.0
        arm = [a / n for a in arm]

        def perp(v):
            d = sum(a * b for a, b in zip(v, arm))
            return [a - d * b for a, b in zip(v, arm)]

        side = perp(want)
        size = math.sqrt(sum(a * a for a in side))
        if size < 0.35:  # the arm points nearly that way: lean on "down", then "back"
            for extra in ((0.0, 0.0, -1.0), (-sy, cy, 0.0)):
                e = perp(extra)
                k = (0.35 - size) / 0.35
                side = [a + b * k for a, b in zip(side, e)]
                size = math.sqrt(sum(a * a for a in side))
        side = [a / (size or 1.0) for a in side]
        # and as last frame's, as far as it can be: the bend turns smoothly frame to frame
        prev = _PREV_SIDE.get(side_name)
        if prev is not None:
            side = perp([0.65 * p + 0.35 * q for p, q in zip(prev, side)])
            size = math.sqrt(sum(a * a for a in side)) or 1.0
            side = [a / size for a in side]
        _PREV_SIDE[side_name] = side
        mid = [(h + q) * 0.5 for h, q in zip(pose[hand], sh)]
        c[f"epole_{side_name}"].location = tuple(m + 0.5 * d for m, d in zip(mid, side))


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
        _PREV_SIDE.clear()
        for f in range(last + 1):
            frame = 1 + f
            scene.frame_set(frame)
            pose = sample(keys, f / FPS, looped)
            if not name.startswith(REACHING):
                pose = relaxed(pose, GRAB_DECK + 0.25 if name.startswith(GRABBING) else -9.0)
            apply_pose(rig, c, pose, rest_ankle)
            for ob in c.values():
                ob.keyframe_insert("location", frame=frame)
                ob.keyframe_insert("rotation_quaternion" if ob.rotation_mode == "QUATERNION" else "rotation_euler",
                                   frame=frame)
            for bname in ("hips", "back_low", "back_mid", "chest", "neck", "skull"):
                b = rig.pose.bones[bname]
                b.keyframe_insert("location", frame=frame)
                b.keyframe_insert("rotation_euler", frame=frame)
            for bname in ("collar.L", "collar.R"):
                rig.pose.bones[bname].keyframe_insert("rotation_quaternion", frame=frame)
            for bname in FINGER_BONES:
                rig.pose.bones[bname].keyframe_insert("rotation_euler", frame=frame)
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
