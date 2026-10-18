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
            lh=(0.52, -0.14, 1.1), rh=(-0.5, -0.06, 1.06), knee_out=0.55)
AIR_TUCK = P(hips=(0.0, 0.03, 0.92), hips_rot=(16, 0, 0), spine=(16, 0, 10), head=(-20, 0, 55),
             lf=foot(FRONT_X, 0.0, 0.16, yaw=-20), rf=foot(BACK_X, 0.0, 0.15, yaw=8),
             lh=(0.5, -0.16, 1.06), rh=(-0.48, -0.1, 1.02), knee_out=0.6)
# Flip tricks: the legs split up and out of the way and the board turns underneath, arms up and
# open, then the feet come back down onto it. Kickflips kick the front foot out forward over the toe
# side, heelflips forward over the heel side; shove-its lift both feet straight up.
def flip_open(front_y, back_z):
    return P(hips=(0.0, 0.02, 0.96), hips_rot=(8, 0, 0), spine=(8, 0, 8), head=(-26, 0, 50),
             lf=foot(FRONT_X + 0.22, front_y, 0.34, yaw=-40, pitch=-15), rf=foot(BACK_X - 0.08, 0.0, back_z, yaw=12),
             lh=(0.6, -0.08, 1.24), rh=(-0.58, -0.02, 1.2), knee_out=0.7)


FLIP_KICK = flip_open(-0.24, 0.2)
FLIP_HEEL = flip_open(0.18, 0.24)
FLIP_SHUV = P(hips=(0.0, 0.02, 0.96), hips_rot=(10, 0, 0), spine=(10, 0, 8), head=(-24, 0, 50),
              lf=foot(FRONT_X + 0.02, 0.0, 0.28, yaw=-20), rf=foot(BACK_X - 0.02, 0.0, 0.26, yaw=8),
              lh=(0.56, -0.1, 1.18), rh=(-0.54, -0.04, 1.14), knee_out=0.6)
FLIP_CATCH = P(hips=(0.0, 0.03, 0.88), hips_rot=(16, 0, 0), spine=(18, 0, 10), head=(-26, 0, 50),
               lf=foot(FRONT_X, 0.0, 0.1, yaw=-20), rf=foot(BACK_X, 0.0, 0.09, yaw=8),
               lh=(0.52, -0.12, 1.06), rh=(-0.5, -0.06, 1.02), knee_out=0.55)
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
             rh=(0.0, -0.13, 0.22), lh=(0.45, -0.05, 0.95))
GRAB_HEEL = P(hips=(0.0, 0.05, 0.6), hips_rot=(26, 0, 0), spine=(34, 0, 14),
              lf=foot(FRONT_X, 0.0, 0.2, yaw=-20), rf=foot(BACK_X, 0.0, 0.18, yaw=8),
              lh=(0.02, 0.12, 0.22), rh=(-0.45, -0.05, 0.95))


def push_frame(phase):
    """Kick-push, after reference footage: the front foot turns to point down the board and the hips
    and chest face the way of travel; the pushing foot swings forward, plants beside the front truck,
    drives straight back along the ground, then lifts and comes forward again. The front knee bends
    as the foot plants; the arms swing a little against the leg."""
    plant, release = 0.18, 0.62
    if phase < plant:                       # swinging forward to plant
        k = phase / plant
        x, lift = -0.35 + 0.47 * k, 0.1 * math.sin(math.pi * (0.5 + 0.5 * k))
    elif phase < release:                   # driving back on the ground
        k = (phase - plant) / (release - plant)
        x, lift = 0.12 - 0.62 * k, 0.0
    else:                                   # lifting off and coming forward
        k = (phase - release) / (1 - release)
        x, lift = -0.5 + 0.15 * k, 0.12 * math.sin(math.pi * k)
    drive = math.sin(math.pi * min(1.0, max(0.0, (phase - plant) / (release - plant))))
    swing = math.sin(2 * math.pi * phase)
    return P(hips=(0.06, -0.06, 0.78 - 0.11 * drive), hips_rot=(20 + 8 * drive, 0, 75), spine=(18 + 6 * drive, 0, 12),
             head=(-16, 0, 8),
             lf=foot(FRONT_X - 0.02, 0.0, 0.0, yaw=80),
             rf=foot(x, -0.17, DECK_TOP_TO_GROUND + lift, yaw=78),
             lh=(0.36 + 0.1 * swing, -0.3, 0.74), rh=(-0.1 - 0.12 * swing, -0.26, 0.72), knee_out=0.5)


def bail_frame(t):
    """Falling forward off the toe side onto the hands, then lying there."""
    k = min(1.0, t)
    return P(hips=(0.0, -0.5 * k, 0.86 - 0.62 * k), hips_rot=(20 + 50 * k, 10 * k, 15), spine=(20 + 40 * k, 0, 0),
             head=(10, 0, 0), lf=foot(FRONT_X, -0.2 * k, DECK_TOP_TO_GROUND * k, yaw=-20),
             rf=foot(BACK_X, -0.15 * k, DECK_TOP_TO_GROUND * k + 0.1 * math.sin(math.pi * k), yaw=8),
             lh=(0.25, -0.9 * k - 0.1, 0.9 - 0.95 * k), rh=(-0.25, -0.85 * k - 0.1, 0.9 - 0.95 * k))


def ride_frame(t):
    s = math.sin(2 * math.pi * t)
    return P(hips=(0.01 * s, 0.0, 0.86 + 0.008 * math.sin(4 * math.pi * t)), spine=(12 + 1.5 * s, 0, 10))


# name -> (loop, [(seconds, pose)])
CLIPS = {
    "ride": (True, [(t / 8 * 2.0, ride_frame(t / 8)) for t in range(9)]),
    "crouch": (False, [(0.0, CROUCH), (0.1, CROUCH)]),
    "pop": (False, [(0.0, CROUCH), (0.1, POP_TOP), (0.3, AIR_TUCK)]),
    "air": (False, [(0.0, AIR_TUCK), (0.1, AIR_TUCK)]),
    "flip_kick": (False, [(0.0, POP_TOP), (0.1, FLIP_KICK), (0.26, FLIP_KICK), (0.38, FLIP_CATCH), (0.45, AIR_TUCK)]),
    "flip_heel": (False, [(0.0, POP_TOP), (0.1, FLIP_HEEL), (0.26, FLIP_HEEL), (0.38, FLIP_CATCH), (0.45, AIR_TUCK)]),
    "flip_shuv": (False, [(0.0, POP_TOP), (0.1, FLIP_SHUV), (0.26, FLIP_SHUV), (0.38, FLIP_CATCH), (0.45, AIR_TUCK)]),
    "land": (False, [(0.0, AIR_TUCK), (0.08, LAND_LOW), (0.35, STANCE)]),
    "push": (True, [(i / 10 * 0.62, push_frame(i / 10)) for i in range(11)]),
    "manual": (False, [(0.0, MANUAL), (0.1, MANUAL)]),
    "nose_manual": (False, [(0.0, NOSE_MANUAL), (0.1, NOSE_MANUAL)]),
    "grind": (False, [(0.0, GRIND), (0.1, GRIND)]),
    "boardslide": (False, [(0.0, BOARDSLIDE), (0.1, BOARDSLIDE)]),
    "powerslide": (False, [(0.0, POWERSLIDE), (0.1, POWERSLIDE)]),
    "grab_toe": (False, [(0.0, GRAB_TOE), (0.1, GRAB_TOE)]),
    "grab_heel": (False, [(0.0, GRAB_HEEL), (0.1, GRAB_HEEL)]),
    "bail": (False, [(i / 10 * 0.9, bail_frame(i / 9)) for i in range(11)]),
}

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
        # Elbows bend out to the side and a little back, below the hand.
        c[f"epole_{side}"].location = (pose[hand][0] + hx * 0.35, pose[hand][1] + 0.3, pose[hand][2] - 0.25)


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
    for name, (loop, keys) in CLIPS.items():
        last = max(1, round(keys[-1][0] * FPS))
        for t, pose in keys:
            frame = 1 + round(t * FPS)
            scene.frame_set(frame)
            apply_pose(rig, c, pose, rest_ankle)
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
        bpy.ops.nla.bake(frame_start=1, frame_end=1 + last, only_selected=False, visual_keying=True,
                         clear_constraints=False, use_current_action=False, bake_types={"POSE"})
        action = rig.animation_data.action
        action.name = f"sk8_{name}"
        action.use_fake_user = True
        if loop:
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
