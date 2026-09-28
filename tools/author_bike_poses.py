"""Author the freestyle-MX rider poses against the native skeleton, in Blender.

    blender -b --python tools/author_bike_poses.py -- \
        --skater <skater.glb> --bike sdk/examples/freestyle-mx/bike.glb \
        --out sdk/examples/freestyle-mx/rider.json --preview <abs-dir>

No Mixamo and no calibration profile: the native rig is a plain humanoid, so the
seated pose is fitted straight onto the bike's own measured grips and pegs, and
every other pose is that pose with the hips moved and named bones rotated. Each
pose is a single frame, which is what the host's blend wants -- it eases a base
pose towards several targets at once by scalar weights, so a rider standing on
the pegs with his weight back through a lean is all three of those poses at
their own weights rather than one authored pose per combination.

Four things here were measured rather than assumed, and each contradicts the
obvious guess:

* **The export is a world-space delta, not a bone matrix.** Blender's pose-bone
  matrices are in Blender's own bone convention; writing them out directly is
  what produced the disjointed rider. What transfers correctly is
  `posed @ rest.inverted()` -- a pure world-space rigid motion -- applied to the
  *reference GLB's* rest node. Bone roll and axis convention cancel, because
  both sides of the delta carry the same ones. This is the formula the working
  kart rider uses (`tools/export_kart_rider.py`), and `--verify` checks it by
  exporting an unposed rig and comparing against the reference rest pose.
* **Poses are relative to the seat, not to the hips.** The host puts the rider
  root at `definition.seat`, so that is the origin the matrices are written
  around. Recentring on HIPS instead pins the pelvis there forever, which makes
  standing up, crouching and shifting weight impossible to author at all.
* **Local X runs along each bone** and rotating about it does nothing. Y is the
  flexion axis (negative swings forward, the way the rig faces) and Z is
  lateral. Blender's usual "Y is the bone axis" does not hold for this import.
* **Blender's IK is not used.** With no pole target it settled the knees out
  sideways -- a foot 0.71 m off the bike's centreline -- and when a target is out
  of reach it silently points the limb at it rather than failing, so the pose
  looked fine in code and wrong on screen. The limbs are fitted by coordinate
  descent instead, and the residual at each hand and foot is printed.

Coordinates: Blender is Z-up and the imported rig faces -Y; the game is Y-up, +Z
forward, +X driver-left. A point converts as game = (bx, bz, -by), so the rig
already faces game +Z and needs no yaw.
"""
import argparse, json, math, os, struct, sys
import bpy
from mathutils import Matrix, Quaternion, Vector

argv = sys.argv[sys.argv.index("--") + 1 :]
p = argparse.ArgumentParser()
p.add_argument("--skater", required=True)
p.add_argument("--bike", required=True)
p.add_argument("--out", required=True)
p.add_argument("--preview", default=None)
args = p.parse_args(argv)

BONES = [
    "TRAJECTORY", "HIPS", "SPINE", "SPINE1", "SPINE2", "SPINE3", "NECK", "NECK1",
    "HEAD", "RIGHTSHOULDER", "RIGHTARM", "RIGHTFOREARM", "RIGHTHAND", "LEFTSHOULDER",
    "LEFTARM", "LEFTFOREARM", "LEFTHAND", "RIGHTUPLEG", "RIGHTLEG", "RIGHTFOOT",
    "RIGHTTOEBASE", "LEFTUPLEG", "LEFTLEG", "LEFTFOOT", "LEFTTOEBASE",
    "SKATEBOARD_ROOT", "TRUCK_FRONT", "RIGHT_WHEELFRONT", "LEFT_WHEELFRONT",
    "TRUCK_BACK", "LEFT_WHEELBACK", "RIGHT_WHEELBACK", "RIGHTTOEBASE_REPARENTED",
    "LEFTTOEBASE_REPARENTED", "RIGHTHAND_REPARENTED", "LEFTHAND_REPARENTED",
]

# Measured off this bike's own geometry, in chassis-local metres. Two traps
# here, both of which put the rider visibly off the bike when missed:
#
# * **The mesh nodes carry their own transform** -- this model's are scaled
#   0.989 and dropped 0.16 m -- so raw vertex coordinates are not chassis
#   local. Reading them directly put the seat 0.24 m too high and the rider
#   floating above the bike with his legs through the engine. Measure in the
#   scene, after the import has composed the transforms.
# * **The footpegs are the symmetric pair** of side protrusions below the seat.
#   An earlier guess caught a single-sided frame part 0.25 m lower and put the
#   pegs out of the rider's reach entirely.
#
# With the transform applied these come out as real motocross numbers against
# a ground plane 0.489 m below the chassis origin: seat 0.97 m, pegs 0.41 m,
# grips 1.11 m. Only the +X handlebar is modelled, so the grips are mirrored.
SEAT = Vector((0.0, 0.54, -0.12))    # the rider's hips, and the host's rider root
GRIP = Vector((0.44, 0.62, 0.18))
PEG = Vector((0.205, -0.077, -0.16))
# The fit aims a bone's HEAD, and the foot bone's head is the ankle. Aiming the
# ankle at the peg hangs the whole boot below it; the rig's ankle sits this far
# above its sole in rest, so this is what puts the sole on the peg.
ANKLE = 0.10
# Forward lean, shared down the chain. Measured: about 90 deg in total puts the
# shoulders ~0.30 m ahead of the hips, which is the attack stance, and is what
# brings the grips inside arm's length.
LEAN = [("SPINE", 18), ("SPINE1", 16), ("SPINE2", 13), ("SPINE3", 8)]
BEND, SPLAY = 1, 2   # local Y flexes, local Z swings sideways
# Blender Z-up <-> game Y-up. `BASIS` maps game axes into Blender's.
BASIS = Matrix(((1, 0, 0, 0), (0, 0, -1, 0), (0, 1, 0, 0), (0, 0, 0, 1)))


# --- the reference rig's rest pose, straight out of the GLB --------------------
def glb_rest(path):
    """Every named node's world matrix in the reference GLB, in game axes."""
    raw = open(path, "rb").read()
    length = struct.unpack_from("<I", raw, 12)[0]
    doc = json.loads(raw[20 : 20 + length])
    nodes = doc["nodes"]

    def local(node):
        if "matrix" in node:
            m = node["matrix"]
            return Matrix([[m[c * 4 + r] for c in range(4)] for r in range(4)])
        x, y, z, w = node.get("rotation", [0, 0, 0, 1])
        return Matrix.LocRotScale(
            Vector(node.get("translation", [0, 0, 0])),
            Quaternion((w, x, y, z)),
            Vector(node.get("scale", [1, 1, 1])),
        )

    parent = {}
    for i, node in enumerate(nodes):
        for child in node.get("children", []):
            parent[child] = i
    world = {}

    def resolve(i):
        if i in world:
            return world[i]
        m = local(nodes[i])
        if i in parent:
            m = resolve(parent[i]) @ m
        world[i] = m
        return m

    out = {}
    for i, node in enumerate(nodes):
        if "name" in node:
            out[node["name"].upper()] = resolve(i)
    return out


WORLD_REST = glb_rest(args.skater)
missing = [b for b in BONES if b not in WORLD_REST and not b.endswith("REPARENTED")]
if missing:
    print(f"NOTE reference GLB has no node for: {missing}")


def to_blender(g):
    """game (x, up, forward) -> blender (x, -forward, up)"""
    return Vector((g[0], -g[2], g[1]))


bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=args.skater)
arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
for o in [o for o in bpy.data.objects if o.type == "MESH"]:
    if len(o.data.vertices) < 100:      # the importer's helper icosphere
        bpy.data.objects.remove(o, do_unlink=True)

# The object transform before the rig is moved onto the bike. The export is a
# delta from this, so the move itself is part of the pose rather than something
# that has to be undone later.
REST_WORLD = arm.matrix_world.copy()
rest_hips = arm.matrix_world @ arm.data.bones["HIPS"].head_local
arm.location = arm.location + (to_blender(SEAT) - rest_hips)
bpy.context.view_layer.update()


def set_angle(bone, axis, deg):
    pb = arm.pose.bones[bone]
    pb.rotation_mode = "XYZ"
    e = list(pb.rotation_euler)
    e[axis] = math.radians(deg)
    pb.rotation_euler = e


def add_angle(bone, axis, deg):
    pb = arm.pose.bones[bone]
    pb.rotation_mode = "XYZ"
    e = list(pb.rotation_euler)
    e[axis] += math.radians(deg)
    pb.rotation_euler = e


def bend(bone, deg):
    """Flex a bone. Positive swings it forward, the way the rig faces."""
    add_angle(bone, BEND, -deg)


def splay(bone, deg):
    """Swing a bone sideways. Positive goes toward +x, the rider's left."""
    add_angle(bone, SPLAY, deg)


def reset_pose():
    for pb in arm.pose.bones:
        pb.rotation_mode = "XYZ"
        pb.rotation_euler = (0, 0, 0)
        pb.location = (0, 0, 0)
        pb.scale = (1, 1, 1)


def head_of(bone):
    bpy.context.view_layer.update()
    return arm.matrix_world @ arm.pose.bones[bone].head


def move_hips(offset):
    """Shift the pelvis in bike coordinates. The limbs refit afterwards, so
    this is what makes standing, crouching and weight shifts possible at all."""
    pb = arm.pose.bones["HIPS"]
    # `location` is in the bone's own space, so convert through its rest basis.
    pb.location = pb.bone.matrix_local.to_3x3().inverted() @ to_blender(offset)
    bpy.context.view_layer.update()


def stance():
    """Torso only. Every pose, base and trick alike, starts here."""
    reset_pose()
    for bone, deg in LEAN:
        bend(bone, deg)
    # Against ~90 deg of spine lean the neck has to counter most of it, or the
    # rider stares at the front tyre instead of the landing.
    bend("NECK", -24)
    bend("NECK1", -15)
    bend("HEAD", -12)


# --- fitting ----------------------------------------------------------------
# Coordinate descent over a handful of joint angles. Deterministic, converges in
# a few passes, and unlike IK it cannot quietly "succeed" at an impossible reach.
def fit_limb(knobs, end_bone, target, passes=16):
    # Start from whatever the limb is already doing rather than from the rest
    # pose. A refit after the hips move only has to correct a good solution;
    # restarting from the T-pose puts the descent a long way from the target
    # and it settles in a local minimum with the hands off the bars.
    state = {
        k: math.degrees(arm.pose.bones[k[0]].rotation_euler[k[1]]) for k in knobs
    }

    def apply():
        for (bone, axis), value in state.items():
            set_angle(bone, axis, value)

    def error():
        apply()
        # A tie-breaker, not a constraint: three knobs reaching a point in
        # space is exactly determined only up to sign, and without a
        # preference the descent is as happy to get there with the joint
        # swung out sideways as tucked in. A hundredth of a millimetre per
        # degree is far below the reach tolerance and still enough to settle
        # it the anatomical way.
        splay = sum(abs(v) for (_, axis), v in state.items() if axis == SPLAY)
        return (head_of(end_bone) - target).length + splay * 1e-5

    step, best = 32.0, error()
    for _ in range(passes):
        improved = False
        for knob in knobs:
            for delta in (step, -step):
                trial = state[knob] + delta
                if abs(trial) > 170:
                    continue
                keep, state[knob] = state[knob], trial
                if (e := error()) < best - 1e-5:
                    best, improved = e, True
                else:
                    state[knob] = keep
        if not improved:
            step /= 2.0
            if step < 0.05:
                break
    apply()
    return best


def hand_knobs(side):
    # Shoulder aims, elbow sets the distance. The elbow is a hinge: giving it
    # a lateral knob let the solver reach the grips with the forearm swung 60
    # degrees out, which reaches the target and looks broken.
    return [(f"{side}ARM", BEND), (f"{side}ARM", SPLAY), (f"{side}FOREARM", BEND)]


def foot_knobs(side):
    # Hip aims, knee sets the distance. Same hinge argument as the elbow.
    return [(f"{side}UPLEG", BEND), (f"{side}UPLEG", SPLAY), (f"{side}LEG", BEND)]


def grip_target(sign):
    return to_blender(Vector((sign * GRIP.x, GRIP.y, GRIP.z)))


def peg_target(sign):
    return to_blender(Vector((sign * PEG.x, PEG.y + ANKLE, PEG.z)))


BASE_FIT = []
stance()
for side, sign in (("RIGHT", -1.0), ("LEFT", 1.0)):
    ge = fit_limb(hand_knobs(side), f"{side}HAND", grip_target(sign))
    fe = fit_limb(foot_knobs(side), f"{side}FOOT", peg_target(sign))
    print(f"FIT {side:5s} hand miss {ge:.3f} m   foot miss {fe:.3f} m")
    for bone, axis in hand_knobs(side) + foot_knobs(side):
        BASE_FIT.append((bone, axis, math.degrees(arm.pose.bones[bone].rotation_euler[axis])))
    BASE_FIT.append((f"{side}FOOT", BEND, -20.0))     # sole flat on the peg


print("BASEFIT " + ", ".join(f"{b}.{'XYZ'[a]}={v:+.1f}" for b, a, v in BASE_FIT))


def base_pose():
    stance()
    for bone, axis, value in BASE_FIT:
        set_angle(bone, axis, value)


base_pose()
print(f"FIT base HIPS {tuple(round(v, 3) for v in head_of('HIPS'))} "
      f"SPINE3 {tuple(round(v, 3) for v in head_of('SPINE3'))} "
      f"RHAND {tuple(round(v, 3) for v in head_of('RIGHTHAND'))} "
      f"RFOOT {tuple(round(v, 3) for v in head_of('RIGHTFOOT'))}")

# --- poses ------------------------------------------------------------------
# Limb angles here are ABSOLUTE, not deltas. The seated stance is a fitted
# solution whose joint values fall out of the solver, so adding a delta to it
# partly cancels the fit and a "superman" comes out looking like someone sitting
# down -- which is exactly what the first attempt produced. Setting the limb
# outright is the only way to say where it should actually go.
#
#   legs = (upleg bend, upleg splay, knee bend, foot bend), splay mirrored
#   arms = (arm bend, arm splay, forearm bend), splay mirrored
#   torso = deltas on the lean, which IS the right form for the spine
#   hips = a bike-frame offset for the pelvis; limbs refit to the bike after
# bend is positive-forward; splay positive goes toward +x, the rider's left.
def set_bend(bone, deg):
    set_angle(bone, BEND, -deg)


def set_splay(bone, deg):
    set_angle(bone, SPLAY, deg)


def set_legs(spec, sides=("RIGHT", "LEFT")):
    up_b, up_s, knee_b, foot_b = spec
    for side in sides:
        sign = -1.0 if side == "RIGHT" else 1.0
        set_bend(f"{side}UPLEG", up_b)
        set_splay(f"{side}UPLEG", up_s * sign)
        set_bend(f"{side}LEG", knee_b)
        set_splay(f"{side}LEG", 0.0)
        set_bend(f"{side}FOOT", foot_b)


def set_arms(spec):
    arm_b, arm_s, fore_b = spec
    for side in ("RIGHT", "LEFT"):
        sign = -1.0 if side == "RIGHT" else 1.0
        set_bend(f"{side}ARM", arm_b)
        set_splay(f"{side}ARM", arm_s * sign)
        set_bend(f"{side}FOREARM", fore_b)
        set_splay(f"{side}FOREARM", 0.0)


POSES = {
    # --- riding postures. The host blends the seated pose towards these by how
    # much of each the ride is asking for, so they have to be compatible
    # stances that keep the rider attached to the bike, not gestures.
    # Up off the seat, knees and elbows soft: the attack position.
    "stand": dict(hips=(0.0, 0.22, 0.04),
                  torso=[("SPINE", 6), ("SPINE1", 4), ("NECK", -6)]),
    # Absorbing a landing, or loading the suspension before a jump.
    "crouch": dict(hips=(0.0, -0.07, -0.02),
                   torso=[("SPINE", 10), ("SPINE1", 8), ("NECK", -10)]),
    # Weight to the back of the seat, arms long: braking, or lofting the front.
    "weight_back": dict(hips=(0.0, -0.01, -0.15),
                        torso=[("SPINE", -8), ("NECK", 8)]),
    # Over the bars: front wheel weighted, chest down on the tank.
    "weight_forward": dict(hips=(0.0, 0.04, 0.17),
                           torso=[("SPINE", 16), ("SPINE1", 12), ("NECK", -16)]),
    # Hanging off the inside of a lean. The head counter-rolls to stay level,
    # which is what makes a leaned rider read as a rider and not a statue.
    "lean_left": dict(hips=(0.09, -0.02, 0.0),
                      torso=[("SPINE", 0, -9), ("SPINE1", 0, -7), ("SPINE2", 0, -5),
                             ("NECK", 0, 12), ("HEAD", 0, 8)]),
    "lean_right": dict(hips=(-0.09, -0.02, 0.0),
                       torso=[("SPINE", 0, 9), ("SPINE1", 0, 7), ("SPINE2", 0, 5),
                              ("NECK", 0, -12), ("HEAD", 0, -8)]),
    # Not tricks either: the host blends towards these by how far the bars are
    # turned, so they are seated poses with the shoulders following the bars.
    "steer_left": dict(torso=[("SPINE", 0, 14), ("SPINE1", 0, 11),
                              ("SPINE2", 0, 8), ("NECK", 0, 10)]),
    "steer_right": dict(torso=[("SPINE", 0, -14), ("SPINE1", 0, -11),
                               ("SPINE2", 0, -8), ("NECK", 0, -10)]),
    # --- tricks. Legs straight out behind, body flat along the bike. The
    # torso entries are deltas on the base lean, so they only mean what they
    # say relative to it: these push the chest down onto the bars from the
    # seated 55 degrees to roughly horizontal, and the neck holds the head up
    # to look down the track rather than at the tank.
    "superman": dict(legs=(-78, 0, 5, 25),
                     torso=[("SPINE", 14), ("SPINE1", 12), ("SPINE2", 8),
                            ("NECK", -20), ("HEAD", -12)]),
    # Feet forward past the bars, lying back on the seat.
    "lazyboy": dict(legs=(115, 10, 25, -10),
                    torso=[("SPINE", -34), ("SPINE1", -28), ("SPINE2", -22),
                           ("NECK", 26), ("HEAD", 16)]),
    # Right leg swung over the back, looking back over the shoulder.
    "nacnac": dict(right_leg=(-46, 68, -52, 10),
                   torso=[("SPINE2", 0, -30), ("SPINE3", 0, -22),
                          ("NECK", 0, -26), ("HEAD", 0, -20)]),
    # Right leg over the tank to the left side.
    "cancan": dict(right_leg=(34, 78, -62, 5), torso=[]),
    # Both legs up through the arms, heels together.
    "heelclicker": dict(legs=(128, 26, 12, -5),
                        torso=[("SPINE", -26), ("SPINE1", -22)]),
    # Inverted over the front: legs up and back, head down to the fender.
    "kissofdeath": dict(legs=(-128, 0, 8, 25),
                        torso=[("SPINE", 34), ("SPINE1", 28), ("SPINE2", 20),
                               ("NECK", -30), ("HEAD", -24)]),
    # Back arched, shins hooked forward over the bars.
    "cordova": dict(legs=(-116, 14, -118, 0),
                    torso=[("SPINE", -34), ("SPINE1", -28), ("SPINE2", -20),
                           ("NECK", 32), ("HEAD", 22)]),
    # Both hands off, arms out wide.
    "nohander": dict(arms=(-26, 84, -22), torso=[("SPINE", -16)]),
}


def pose(name):
    base_pose()
    spec = POSES.get(name)
    if not spec:
        bpy.context.view_layer.update()
        return
    if "hips" in spec:
        move_hips(Vector(spec["hips"]))
    if "legs" in spec:
        set_legs(spec["legs"])
    if "right_leg" in spec:
        set_legs(spec["right_leg"], sides=("RIGHT",))
    if "left_leg" in spec:
        set_legs(spec["left_leg"], sides=("LEFT",))
    if "arms" in spec:
        set_arms(spec["arms"])
    for entry in spec.get("torso", []):
        bend(entry[0], entry[1])
        if len(entry) > 2:
            splay(entry[0], entry[2])
    # Anything the pose did not take over stays attached to the bike. Moving
    # the hips or twisting the spine moves the shoulders, so arms left on their
    # fitted angles drift off the grips and read as a mistake rather than as a
    # pose; re-solving pins them back on. Same for a foot whose leg was never
    # touched. This is what lets the postures above be three numbers each.
    for side, sign in (("RIGHT", -1.0), ("LEFT", 1.0)):
        if "arms" not in spec:
            fit_limb(hand_knobs(side), f"{side}HAND", grip_target(sign))
        moved = "legs" in spec or f"{side.lower()}_leg" in spec
        if not moved:
            fit_limb(foot_knobs(side), f"{side}FOOT", peg_target(sign))
            set_bend(f"{side}FOOT", -20.0)
    bpy.context.view_layer.update()


def capture():
    """Every bone as a model-space matrix about the seat, in game axes.

    A world-space delta from the rig's own rest pose, applied to the reference
    GLB's rest node. Both sides of the delta carry the same bone convention, so
    it cancels: this is the one formulation that survives the round trip.
    """
    bpy.context.view_layer.update()
    inverse = BASIS.inverted()
    frame = []
    for name in BONES:
        if name in arm.pose.bones and name in WORLD_REST:
            pb = arm.pose.bones[name]
            posed = arm.matrix_world @ pb.matrix
            rest = REST_WORLD @ pb.bone.matrix_local
            delta = inverse @ (posed @ rest.inverted()) @ BASIS
            g = delta @ WORLD_REST[name]
            g.translation -= SEAT
            m = g @ BASIS
        else:
            m = Matrix.Identity(4)
        frame.append([m[r][c] for c in range(4) for r in range(4)])
    return frame


# The delta formulation is only correct if an unposed rig round-trips to the
# reference rest pose. Check it rather than trust it: a silent basis error here
# is exactly what produced the disjointed rider, and it looks fine in the data.
reset_pose()
saved, arm.matrix_world = arm.matrix_world.copy(), REST_WORLD.copy()
bpy.context.view_layer.update()
worst = 0.0
for name, values in zip(BONES, capture()):
    if name not in arm.pose.bones or name not in WORLD_REST:
        continue
    want = WORLD_REST[name].copy()
    want.translation -= SEAT
    want = want @ BASIS
    got = Matrix([[values[c * 4 + r] for c in range(4)] for r in range(4)])
    worst = max(worst, max(abs(a - b) for ra, rb in zip(got, want) for a, b in zip(ra, rb)))
arm.matrix_world = saved
bpy.context.view_layer.update()
print(f"VERIFY rest round trip worst element error {worst:.6f}")
if worst > 1e-4:
    raise SystemExit(f"Export basis is wrong: rest pose differs by {worst}")

clips = {}
for name in ["seated"] + list(POSES):
    pose(name)
    clips[name] = capture()
    spec = POSES.get(name, {})
    hips = head_of("HIPS")
    held = []
    if "arms" not in spec:
        held.append(f"hands {(head_of('RIGHTHAND') - grip_target(-1.0)).length:.3f}/"
                    f"{(head_of('LEFTHAND') - grip_target(1.0)).length:.3f}")
    if "legs" not in spec and "right_leg" not in spec:
        held.append(f"feet {(head_of('RIGHTFOOT') - peg_target(-1.0)).length:.3f}/"
                    f"{(head_of('LEFTFOOT') - peg_target(1.0)).length:.3f}")
    print(f"POSE {name:15s} hips {tuple(round(v, 2) for v in hips)} "
          + "  ".join(held) + ("  (off the bike by design)" if not held else ""))

with open(args.out, "w") as f:
    json.dump({"version": 1, "bone_names": BONES,
               "clips": {k: {"fps": 30, "frames": [v]} for k, v in clips.items()}}, f)
print(f"wrote {args.out} with {len(clips)} poses")

if args.preview:
    os.makedirs(args.preview, exist_ok=True)
    bpy.ops.import_scene.gltf(filepath=args.bike)
    for view, (location, rotation, scale) in {
        "side": ((5.0, -0.1, 0.75), (88, 0, 89), 2.9),
        "rear": ((0.0, -5.0, 0.9), (84, 0, 0), 2.6),
    }.items():
        bpy.ops.object.camera_add(
            location=location, rotation=tuple(math.radians(d) for d in rotation)
        )
        cam = bpy.context.object
        cam.data.type = "ORTHO"
        cam.data.ortho_scale = scale
        cam.name = f"cam_{view}"
    bpy.ops.object.light_add(type="SUN", location=(5, -4, 6))
    bpy.context.object.data.energy = 4
    s = bpy.context.scene
    s.render.engine = "BLENDER_WORKBENCH"
    s.render.resolution_x, s.render.resolution_y = 720, 560
    s.render.image_settings.file_format = "PNG"
    for name in ["seated"] + list(POSES):
        pose(name)
        for view in ("side", "rear"):
            s.camera = bpy.data.objects[f"cam_{view}"]
            s.render.filepath = os.path.join(args.preview, f"{name}-{view}.png")
            bpy.ops.render.render(write_still=True)
        print(f"preview {name}")
