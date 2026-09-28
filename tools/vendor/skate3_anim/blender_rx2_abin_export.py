"""Export one Skate 3 ABIN clip to Blender using the viewer's RX2 rig.

Run with Blender:
  blender --background --python blender_rx2_abin_export.py -- \
    --abin anim/OnBoard.abin --rx2 ai_skater_01 \
    --clip KICKFLIP_IN_LOW_G --output out/Kickflip.blend
"""

from __future__ import annotations

import argparse
import glob
import json
import math
import os
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector
import numpy as np


SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import abin_importer as ABIN
import rx2_skeleton as RX2


SOURCE_TO_BLENDER = Matrix(
    (
        (1.0, 0.0, 0.0, 0.0),
        (0.0, 0.0, -1.0, 0.0),
        (0.0, 1.0, 0.0, 0.0),
        (0.0, 0.0, 0.0, 1.0),
    )
)
BLENDER_TO_SOURCE = SOURCE_TO_BLENDER.inverted()


def quat_matrix(quaternion, scale=(1.0, 1.0, 1.0)):
    """Viewer-equivalent row-vector SQT rotation matrix."""
    x, y, z, w = quaternion
    norm = x * x + y * y + z * z + w * w
    if norm < 1e-12:
        x = y = z = 0.0
        w = 1.0
    else:
        inverse_norm = 1.0 / math.sqrt(norm)
        x, y, z, w = (
            x * inverse_norm,
            y * inverse_norm,
            z * inverse_norm,
            w * inverse_norm,
        )
    xx, yy, zz = x * x, y * y, z * z
    xy, xz, yz = x * y, x * z, y * z
    wx, wy, wz = w * x, w * y, w * z
    matrix = np.identity(4, dtype=np.float64)
    matrix[0, 0] = 1 - 2 * (yy + zz)
    matrix[0, 1] = 2 * (xy + wz)
    matrix[0, 2] = 2 * (xz - wy)
    matrix[1, 0] = 2 * (xy - wz)
    matrix[1, 1] = 1 - 2 * (xx + zz)
    matrix[1, 2] = 2 * (yz + wx)
    matrix[2, 0] = 2 * (xz + wy)
    matrix[2, 1] = 2 * (yz - wx)
    matrix[2, 2] = 1 - 2 * (xx + yy)
    sx, sy, sz = scale
    matrix[0, :3] *= sx if abs(sx) >= 1e-9 else 1.0
    matrix[1, :3] *= sy if abs(sy) >= 1e-9 else 1.0
    matrix[2, :3] *= sz if abs(sz) >= 1e-9 else 1.0
    return matrix


def rigid_inverse(matrix):
    result = np.identity(4, dtype=np.float64)
    rotation = matrix[:3, :3]
    result[:3, :3] = rotation.T
    result[3, :3] = -(matrix[3, :3] @ rotation.T)
    return result


class AnimSource:
    def __init__(self, path):
        self.path = path
        self.data = Path(path).read_bytes()
        self.abin = ABIN.AbinFile(self.data)
        self.hier = self.abin.hierarchy
        self.parents = list(self.hier.parents) if self.hier else []
        self.clips = list(self.abin.clips)
        self.poses = list(self.abin.poses)
        self._decoders = {}
        self._cache = {}
        self._cache_key = None
        self.ref_pose = self._load_ref_pose()

    def _load_ref_pose(self):
        for pose in self.poses:
            if pose.header.name == "RIG_TPOSE":
                return ABIN.decode_pose_frame(
                    self.data, pose, self.hier
                )
        return {}

    def frame(self, clip_index, frame_index):
        if self._cache_key != clip_index:
            self._cache_key = clip_index
            self._cache = {}
            self._decoders = {}
        if frame_index not in self._cache:
            self._cache[frame_index] = ABIN.decode_clip_frame(
                self.data,
                self.clips[clip_index],
                self._decoders,
                frame_index,
                self.hier,
            )
        return self._cache[frame_index]


class SkeletonSet:
    """Blender-safe copy of SkateAnimViewer's RX2-to-ABIN mapper."""

    def __init__(self, folder):
        self.models = []
        self.bind = {}
        self.ibp = {}
        self.errors = []
        files = sorted(
            glob.glob(os.path.join(folder, "**", "*.rx2"), recursive=True)
        )
        for path in files:
            try:
                parsed = RX2.parse_rx2(path)
            except Exception as error:
                self.errors.append(f"{Path(path).name}: {error}")
                continue
            bones = parsed.get("bones") or []
            meshes = parsed.get("meshes") or []
            self.models.append(
                (Path(path).parent.name, Path(path).name, bones, meshes)
            )
            for bone in bones:
                name = (bone.get("name") or "").upper()
                if not name:
                    continue
                if name not in self.bind and bone.get("bind_matrix") is not None:
                    self.bind[name] = np.asarray(
                        bone["bind_matrix"], dtype=np.float64
                    ).reshape(4, 4)
                if name not in self.ibp and bone.get("ibp_matrix") is not None:
                    self.ibp[name] = np.asarray(
                        bone["ibp_matrix"], dtype=np.float64
                    ).reshape(4, 4)

    def _rx2_parent_names(self):
        if getattr(self, "_parent_names", None) is not None:
            return self._parent_names
        parent_names = {}
        for _, _, bones, _ in self.models:
            by_index = {bone.get("index", -1): bone for bone in bones}
            for bone in bones:
                name = (bone.get("name") or "").upper()
                if not name or name in parent_names:
                    continue
                parent = by_index.get(bone.get("parent", -1))
                parent_names[name] = (
                    (parent.get("name") or "").upper() if parent else None
                )
        self._parent_names = parent_names
        return parent_names

    def index_map(self, names, parents=None):
        upper = [name.upper() for name in names]
        result = {}
        self.rejected = []
        if parents is None:
            for index, name in enumerate(upper):
                result.setdefault(name, index)
            return result
        known = set(upper[: len(parents)])
        rx2_parents = self._rx2_parent_names()
        for index in range(min(len(parents), len(upper))):
            name = upper[index]
            if not name or name in result:
                continue
            rx2_parent = rx2_parents.get(name)
            if rx2_parent is not None and rx2_parent in known:
                parent = parents[index]
                expected = upper[parent] if 0 <= parent < len(upper) else None
                if expected != rx2_parent:
                    self.rejected.append(
                        (index, name, expected, rx2_parent)
                    )
                    continue
            result[name] = index
        return result

    def local_bind_matrices(self, parents, names):
        mapping = self.index_map(names, parents)
        world = {
            index: matrix
            for name, matrix in self.bind.items()
            if (index := mapping.get(name)) is not None
            and index < len(parents)
        }
        return {
            index: (
                matrix @ rigid_inverse(world[parent])
                if (parent := parents[index]) in world
                else matrix.copy()
            )
            for index, matrix in world.items()
        }

    def renderable_meshes(self, names, bone_count, parents=None):
        mapping = self.index_map(names, parents)
        output = []
        for folder, filename, bones, meshes in self.models:
            bones_by_index = {
                bone.get("index", -1): bone for bone in bones
            }
            resolved = {}

            def resolve(bone_index):
                if bone_index in resolved:
                    return resolved[bone_index]
                current = bone_index
                seen = set()
                answer = None
                while (
                    current is not None
                    and current >= 0
                    and current not in seen
                ):
                    seen.add(current)
                    bone = bones_by_index.get(current)
                    if bone is None:
                        break
                    candidate = mapping.get(
                        (bone.get("name") or "").upper()
                    )
                    if candidate is not None and candidate < bone_count:
                        answer = candidate
                        break
                    current = bone.get("parent", -1)
                resolved[bone_index] = answer
                return answer

            local_to_abin = {
                bone.get("index", -1): answer
                for bone in bones
                if (answer := resolve(bone.get("index", -1))) is not None
            }
            for mesh in meshes:
                positions = mesh.get("positions")
                indices = mesh.get("indices")
                if not positions or not indices:
                    continue
                triangles = [
                    tuple(indices[index : index + 3])
                    for index in range(0, len(indices) - 2, 3)
                ]
                triangles = [
                    triangle
                    for triangle in triangles
                    if max(triangle) < len(positions)
                ]
                raw_indices = mesh.get("skin_indices")
                raw_weights = mesh.get("skin_weights")
                used = mesh.get("used_bone_indices") or []
                skin = None
                if (
                    raw_indices
                    and raw_weights
                    and len(raw_indices) >= len(positions)
                    and len(raw_weights) >= len(positions)
                ):
                    skin = []
                    for vertex_index in range(len(positions)):
                        influences = []
                        for palette_index, weight in zip(
                            raw_indices[vertex_index],
                            raw_weights[vertex_index],
                        ):
                            if weight <= 1e-4:
                                continue
                            palette_index = int(palette_index)
                            model_bone = (
                                used[palette_index]
                                if palette_index < len(used)
                                else palette_index
                            )
                            abin_bone = local_to_abin.get(int(model_bone))
                            if abin_bone is not None:
                                influences.append(
                                    (abin_bone, float(weight))
                                )
                        skin.append(influences)
                fallback = next(
                    (
                        local_to_abin[bone]
                        for bone in used
                        if bone in local_to_abin
                    ),
                    None,
                )
                if fallback is None:
                    fallback = self._nearest_bone(
                        positions, names, bone_count, parents
                    )
                if skin is not None and fallback is not None:
                    skin = [
                        influences or [(fallback, 1.0)]
                        for influences in skin
                    ]
                output.append(
                    {
                        "name": filename,
                        "folder": folder,
                        "pos": positions,
                        "tris": triangles,
                        "skin": skin,
                    }
                )
        best = {}
        for mesh in output:
            key = mesh["folder"]
            if key not in best or len(mesh["pos"]) > len(best[key]["pos"]):
                best[key] = mesh
        return [best[key] for key in sorted(best)]

    def _nearest_bone(self, positions, names, bone_count, parents=None):
        mapping = self.index_map(names, parents)
        candidates = [
            (index, matrix[3, :3])
            for name, matrix in self.bind.items()
            if (index := mapping.get(name)) is not None
            and index < bone_count
        ]
        if not candidates:
            return None
        centroid = np.asarray(positions, dtype=np.float64).mean(axis=0)
        return min(
            candidates,
            key=lambda item: float(np.linalg.norm(item[1] - centroid)),
        )[0]


def parse_args() -> argparse.Namespace:
    argv = sys.argv
    argv = argv[argv.index("--") + 1 :] if "--" in argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument("--abin", required=True)
    parser.add_argument(
        "--reference-abin",
        help=(
            "Optional ABIN supplying RIG_TPOSE when --abin is a minimal "
            "clip-only bank. Its hierarchy must match --abin."
        ),
    )
    parser.add_argument(
        "--extra-abin",
        help=(
            "Optional second ABIN with the same hierarchy. Clips requested "
            "with --extra-clip are appended to the same armature."
        ),
    )
    parser.add_argument("--rx2", required=True)
    parser.add_argument(
        "--clip",
        required=True,
        action="append",
        help="Exact clip name; repeat to append clips sequentially in NLA.",
    )
    parser.add_argument(
        "--extra-clip",
        action="append",
        default=[],
        help=(
            "Exact clip name from --extra-abin; repeat to append multiple "
            "cross-bank actions."
        ),
    )
    parser.add_argument("--output", required=True)
    parser.add_argument(
        "--preview-fps",
        type=int,
        default=0,
        help="Scene FPS override; 0 preserves the native clip FPS.",
    )
    return parser.parse_args(argv)


def row_source_to_blender(matrix) -> Matrix:
    """Convert a Y-up row-vector source matrix to Z-up Blender columns."""
    source_column = Matrix(np.asarray(matrix, dtype=np.float64).T.tolist())
    return SOURCE_TO_BLENDER @ source_column @ BLENDER_TO_SOURCE


def point_source_to_blender(point) -> Vector:
    return Vector((float(point[0]), -float(point[2]), float(point[1])))


def clear_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in (
        bpy.data.meshes,
        bpy.data.armatures,
        bpy.data.materials,
        bpy.data.actions,
    ):
        for datablock in list(collection):
            collection.remove(datablock)


def compute_world_pose(source, skeleton, clip_index: int, frame_index: int):
    """Mirror SkateAnimViewer.Viewer._pose through world-matrix creation."""
    frame = source.frame(clip_index, frame_index)
    parents = source.parents
    ref = source.ref_pose
    bind = skeleton.local_bind_matrices(parents, ABIN.BONE_NAMES)
    matrices = {}
    for bone in range(len(parents)):
        sqt = frame.get(bone)
        bind_local = bind.get(bone)
        if sqt is None and bind_local is None:
            continue
        if sqt is None:
            local = bind_local.copy()
        else:
            local = quat_matrix(sqt.quat, sqt.scale)
            local[3, :3] = np.asarray(sqt.trans, dtype=np.float64)
            bind_translation = (
                bind_local[3, :3]
                if bind_local is not None
                else np.zeros(3, dtype=np.float64)
            )
            reference = ref.get(bone)
            if reference is not None:
                reference_matrix = quat_matrix(
                    reference.quat, reference.scale
                )
                reference_matrix[3, :3] = bind_translation
                local = local @ reference_matrix
            else:
                local[3, :3] += bind_translation
        parent = parents[bone]
        matrices[bone] = (
            local @ matrices[parent] if parent in matrices else local
        )
    return matrices


def mapped_bind_world(skeleton, parents):
    name_to_index = skeleton.index_map(ABIN.BONE_NAMES, parents)
    result = {}
    for name, matrix in skeleton.bind.items():
        index = name_to_index.get(name)
        if index is not None and index < len(parents):
            result[index] = matrix
    return result


def bone_lengths(bind_world, parents):
    children = {}
    for child, parent in enumerate(parents):
        children.setdefault(parent, []).append(child)
    result = {}
    for index, matrix in bind_world.items():
        origin = np.asarray(matrix[3, :3], dtype=np.float64)
        distances = []
        for child in children.get(index, []):
            child_matrix = bind_world.get(child)
            if child_matrix is None:
                continue
            distance = float(
                np.linalg.norm(
                    np.asarray(child_matrix[3, :3], dtype=np.float64)
                    - origin
                )
            )
            if distance > 1e-4:
                distances.append(distance)
        result[index] = min(0.25, max(0.035, min(distances, default=0.08)))
    return result


def create_armature(bind_world, parents):
    armature_data = bpy.data.armatures.new("Skate3_RX2_Rig")
    armature = bpy.data.objects.new("Skate3_RX2_Rig", armature_data)
    bpy.context.collection.objects.link(armature)
    bpy.context.view_layer.objects.active = armature
    armature.select_set(True)
    armature.show_in_front = True
    armature.data.display_type = "STICK"

    lengths = bone_lengths(bind_world, parents)
    bpy.ops.object.mode_set(mode="EDIT")
    edit_by_index = {}
    intended_rest = {}
    for index in sorted(bind_world):
        matrix = row_source_to_blender(bind_world[index])
        intended_rest[index] = matrix
        edit_bone = armature_data.edit_bones.new(ABIN.BONE_NAMES[index])
        head = matrix.translation
        rotation = matrix.to_3x3().normalized()
        y_axis = (rotation @ Vector((0.0, 1.0, 0.0))).normalized()
        z_axis = (rotation @ Vector((0.0, 0.0, 1.0))).normalized()
        edit_bone.head = head
        edit_bone.tail = head + y_axis * lengths[index]
        edit_bone.align_roll(z_axis)
        edit_by_index[index] = edit_bone

    for index, edit_bone in edit_by_index.items():
        parent = parents[index]
        if parent in edit_by_index:
            edit_bone.parent = edit_by_index[parent]
            edit_bone.use_connect = False
    bpy.ops.object.mode_set(mode="OBJECT")

    rest_error = 0.0
    for index, intended in intended_rest.items():
        actual = armature.data.bones[ABIN.BONE_NAMES[index]].matrix_local
        rest_error = max(
            rest_error,
            max(
                abs(actual[row][column] - intended[row][column])
                for row in range(4)
                for column in range(4)
            ),
        )
    return armature, edit_by_index, rest_error


def create_material(name: str, color):
    material = bpy.data.materials.new(name)
    material.diffuse_color = (*color, 1.0)
    material.roughness = 0.8
    return material


def create_meshes(armature, skeleton, parents):
    renderable = skeleton.renderable_meshes(
        ABIN.BONE_NAMES, len(parents), parents
    )
    palette = (
        (0.20, 0.32, 0.55),
        (0.26, 0.42, 0.68),
        (0.36, 0.48, 0.64),
        (0.24, 0.28, 0.36),
    )
    objects = []
    total_vertices = 0
    total_triangles = 0
    for mesh_index, source_mesh in enumerate(renderable):
        positions = source_mesh.get("pos") or []
        triangles = source_mesh.get("tris") or []
        if not positions or not triangles:
            continue
        mesh_data = bpy.data.meshes.new(
            f"{source_mesh.get('folder', 'Part')}_{mesh_index}"
        )
        mesh_data.from_pydata(
            [point_source_to_blender(position) for position in positions],
            [],
            triangles,
        )
        mesh_data.update()
        mesh_object = bpy.data.objects.new(mesh_data.name, mesh_data)
        bpy.context.collection.objects.link(mesh_object)
        mesh_object.data.materials.append(
            create_material(
                f"{mesh_data.name}_Material", palette[mesh_index % len(palette)]
            )
        )

        groups = {}
        skin = source_mesh.get("skin")
        if skin:
            for vertex_index, influences in enumerate(skin):
                combined = {}
                for bone_index, weight in influences:
                    if bone_index >= len(ABIN.BONE_NAMES) or weight <= 1e-5:
                        continue
                    combined[bone_index] = (
                        combined.get(bone_index, 0.0) + float(weight)
                    )
                for bone_index, weight in combined.items():
                    name = ABIN.BONE_NAMES[bone_index]
                    group = groups.get(name)
                    if group is None:
                        group = mesh_object.vertex_groups.new(name=name)
                        groups[name] = group
                    group.add([vertex_index], float(weight), "REPLACE")

        modifier = mesh_object.modifiers.new("Skate3_RX2_Skin", "ARMATURE")
        modifier.object = armature
        modifier.use_vertex_groups = True
        mesh_object.parent = armature
        objects.append((mesh_object, source_mesh))
        total_vertices += len(positions)
        total_triangles += len(triangles)
    return objects, total_vertices, total_triangles


def mapped_inverse_bind(skeleton, parents):
    mapping = skeleton.index_map(ABIN.BONE_NAMES, parents)
    return {
        index: matrix
        for name, matrix in skeleton.ibp.items()
        if (index := mapping.get(name)) is not None
        and index < len(parents)
    }


def validate_deformed_meshes(
    mesh_pairs, armature, source, skeleton, baked
):
    """Compare Blender armature output with viewer-equivalent LBS samples."""
    depsgraph = bpy.context.evaluated_depsgraph_get()
    max_error = 0.0
    worst = None
    global_start = 1
    for entry in baked:
        clip = entry["clip"]
        clip_source = entry.get("source", source)
        clip_index = entry["clip_index"]
        inverse_bind = mapped_inverse_bind(
            skeleton, clip_source.parents
        )
        sample_frames = sorted(
            {0, clip.num_frames // 2, clip.num_frames - 1}
        )
        for local_frame in sample_frames:
            scene_frame = global_start + local_frame
            bpy.context.scene.frame_set(scene_frame)
            bpy.context.view_layer.update()
            world = compute_world_pose(
                clip_source, skeleton, clip_index, local_frame
            )
            palette = {
                index: inverse_bind[index] @ matrix
                for index, matrix in world.items()
                if index in inverse_bind
            }
            for mesh_object, source_mesh in mesh_pairs:
                skin = source_mesh.get("skin")
                if not skin:
                    continue
                evaluated_object = mesh_object.evaluated_get(depsgraph)
                evaluated_mesh = evaluated_object.to_mesh()
                try:
                    stride = max(1, len(evaluated_mesh.vertices) // 96)
                    for vertex_index in range(
                        0, len(evaluated_mesh.vertices), stride
                    ):
                        influences = skin[vertex_index]
                        total = sum(
                            weight
                            for bone, weight in influences
                            if bone in palette
                        )
                        if total <= 1e-6:
                            continue
                        source_point = np.array(
                            [
                                *source_mesh["pos"][vertex_index],
                                1.0,
                            ],
                            dtype=np.float64,
                        )
                        expected_source = np.zeros(4, dtype=np.float64)
                        for bone, weight in influences:
                            matrix = palette.get(bone)
                            if matrix is not None:
                                expected_source += (
                                    source_point @ matrix
                                ) * weight
                        expected_source /= total
                        expected = point_source_to_blender(
                            expected_source[:3]
                        )
                        actual = evaluated_mesh.vertices[
                            vertex_index
                        ].co
                        error = float((actual - expected).length)
                        if error > max_error:
                            max_error = error
                            worst = {
                                "clip": clip.header.name,
                                "local_frame": local_frame,
                                "scene_frame": scene_frame,
                                "mesh": mesh_object.name,
                                "vertex": vertex_index,
                                "error_metres": error,
                                "actual": list(actual),
                                "expected": list(expected),
                                "source": list(source_point[:3]),
                                "influences": influences,
                            }
                finally:
                    evaluated_object.to_mesh_clear()
        global_start += clip.num_frames
    return max_error, worst


def bake_action(
    armature,
    source,
    skeleton,
    clip_index: int,
    clip,
    mapped_indices,
):
    action = bpy.data.actions.new(clip.header.name)
    armature.animation_data_create()
    armature.animation_data.action = action
    bpy.context.preferences.edit.keyframe_new_interpolation_type = "LINEAR"
    for pose_bone in armature.pose.bones:
        pose_bone.rotation_mode = "QUATERNION"

    max_pose_error = 0.0
    worst_pose = None
    for frame_index in range(clip.num_frames):
        blender_frame = frame_index + 1
        bpy.context.scene.frame_set(blender_frame)
        world = compute_world_pose(
            source, skeleton, clip_index, frame_index
        )
        targets = {}
        for index in mapped_indices:
            matrix = world.get(index)
            if matrix is None:
                continue
            target = row_source_to_blender(matrix)
            targets[index] = target
            pose_bone = armature.pose.bones[ABIN.BONE_NAMES[index]]
            rest = armature.data.bones[pose_bone.name].matrix_local
            parent = source.parents[index]
            if parent in targets:
                parent_name = ABIN.BONE_NAMES[parent]
                parent_rest = armature.data.bones[parent_name].matrix_local
                local_rest = parent_rest.inverted() @ rest
                pose_bone.matrix_basis = (
                    local_rest.inverted()
                    @ targets[parent].inverted()
                    @ target
                )
            else:
                pose_bone.matrix_basis = rest.inverted() @ target
        for index in targets:
            pose_bone = armature.pose.bones[ABIN.BONE_NAMES[index]]
            pose_bone.keyframe_insert(
                data_path="location", frame=blender_frame, group=pose_bone.name
            )
            pose_bone.keyframe_insert(
                data_path="rotation_quaternion",
                frame=blender_frame,
                group=pose_bone.name,
            )
            pose_bone.keyframe_insert(
                data_path="scale", frame=blender_frame, group=pose_bone.name
            )
        bpy.context.view_layer.update()
        for index, target in targets.items():
            pose_bone = armature.pose.bones[ABIN.BONE_NAMES[index]]
            actual = pose_bone.matrix
            bone_error = max(
                abs(actual[row][column] - target[row][column])
                for row in range(4)
                for column in range(4)
            )
            if bone_error > max_pose_error:
                max_pose_error = bone_error
                worst_pose = {
                    "frame": blender_frame,
                    "bone_index": index,
                    "bone": pose_bone.name,
                    "error": bone_error,
                    "target": [
                        [target[row][column] for column in range(4)]
                        for row in range(4)
                    ],
                    "actual": [
                        [actual[row][column] for column in range(4)]
                        for row in range(4)
                    ],
                }

    return action, max_pose_error, worst_pose


def main() -> None:
    args = parse_args()
    abin_path = Path(args.abin).resolve()
    reference_abin_path = (
        Path(args.reference_abin).resolve()
        if args.reference_abin
        else None
    )
    rx2_path = Path(args.rx2).resolve()
    output_path = Path(args.output).resolve()

    source = AnimSource(str(abin_path))
    extra_abin_path = (
        Path(args.extra_abin).resolve()
        if args.extra_abin
        else None
    )
    extra_source = (
        AnimSource(str(extra_abin_path))
        if extra_abin_path is not None
        else None
    )
    reference_source = None
    if reference_abin_path is not None:
        reference_source = AnimSource(str(reference_abin_path))
        if reference_source.parents != source.parents:
            raise RuntimeError(
                "--reference-abin hierarchy does not match --abin"
            )
        if not reference_source.ref_pose:
            raise RuntimeError(
                "--reference-abin does not contain a decodable RIG_TPOSE"
            )
        source.ref_pose = reference_source.ref_pose
    if extra_source is not None:
        if extra_source.parents != source.parents:
            raise RuntimeError(
                "--extra-abin hierarchy does not match --abin"
            )
        if not extra_source.ref_pose:
            extra_source.ref_pose = source.ref_pose
    elif args.extra_clip:
        raise RuntimeError("--extra-clip requires --extra-abin")
    skeleton = SkeletonSet(str(rx2_path))
    matches = []
    for requested_name in args.clip:
        found = [
            (index, clip)
            for index, clip in enumerate(source.clips)
            if clip.header.name == requested_name
        ]
        if len(found) != 1:
            raise RuntimeError(
                f"Expected one clip named {requested_name!r}, "
                f"found {len(found)}"
            )
        clip_index, clip = found[0]
        matches.append((source, abin_path, clip_index, clip))
    for requested_name in args.extra_clip:
        found = [
            (index, clip)
            for index, clip in enumerate(extra_source.clips)
            if clip.header.name == requested_name
        ]
        if len(found) != 1:
            raise RuntimeError(
                f"Expected one extra clip named {requested_name!r}, "
                f"found {len(found)}"
            )
        clip_index, clip = found[0]
        matches.append((
            extra_source,
            extra_abin_path,
            clip_index,
            clip,
        ))
    bind_world = mapped_bind_world(skeleton, source.parents)
    if not bind_world:
        raise RuntimeError("RX2 skeleton did not map to the ABIN hierarchy")

    clear_scene()
    armature, _, rest_error = create_armature(
        bind_world, source.parents
    )
    meshes, vertex_count, triangle_count = create_meshes(
        armature, skeleton, source.parents
    )
    baked = []
    for clip_source, clip_abin_path, clip_index, clip in matches:
        action, pose_error, worst_pose = bake_action(
            armature,
            clip_source,
            skeleton,
            clip_index,
            clip,
            sorted(bind_world),
        )
        baked.append(
            {
                "clip": clip,
                "clip_index": clip_index,
                "source": clip_source,
                "source_abin": clip_abin_path,
                "action": action,
                "pose_error": pose_error,
                "worst_pose": worst_pose,
            }
        )
    for entry in baked:
        entry["action"].use_fake_user = True

    scene = bpy.context.scene
    first_clip = baked[0]["clip"]
    scene.render.fps = (
        args.preview_fps
        if args.preview_fps > 0
        else max(1, round(first_clip.fps))
    )
    scene.frame_start = 1
    if len(baked) == 1:
        armature.animation_data.action = baked[0]["action"]
        scene.frame_end = first_clip.num_frames
        scene.timeline_markers.new(
            f"{first_clip.header.name} native {first_clip.fps:g} FPS",
            frame=1,
        )
    else:
        armature.animation_data.action = None
        frame_start = 1
        for index, entry in enumerate(baked, start=1):
            clip = entry["clip"]
            action = entry["action"]
            track = armature.animation_data.nla_tracks.new()
            track.name = f"{index:02d} {clip.header.name}"
            strip = track.strips.new(
                clip.header.name, frame_start, action
            )
            strip.extrapolation = "NOTHING"
            strip.blend_type = "REPLACE"
            scene.timeline_markers.new(
                f"{clip.header.name} native {clip.fps:g} FPS",
                frame=frame_start,
            )
            frame_start += clip.num_frames
        scene.frame_end = frame_start - 1
    scene.frame_set(1)

    mesh_error, worst_mesh = validate_deformed_meshes(
        meshes, armature, source, skeleton, baked
    )
    scene.frame_set(1)

    note = bpy.data.texts.new("SKATE3_EXPORT_INFO.json")
    clip_reports = [
        {
            "name": entry["clip"].header.name,
            "source_abin": str(entry["source_abin"]),
            "native_fps": entry["clip"].fps,
            "frames": entry["clip"].num_frames,
            "pose_matrix_max_abs_error": entry["pose_error"],
            "worst_pose": entry["worst_pose"],
        }
        for entry in baked
    ]
    report = {
        "schema_version": 1,
        "source_abin": str(abin_path),
        "reference_abin": (
            str(reference_abin_path)
            if reference_abin_path is not None
            else None
        ),
        "extra_abin": (
            str(extra_abin_path)
            if extra_abin_path is not None
            else None
        ),
        "source_rx2": str(rx2_path),
        "clips": clip_reports,
        "preview_fps": scene.render.fps,
        "frames": scene.frame_end,
        "mapped_bones": len(bind_world),
        "mesh_objects": len(meshes),
        "vertices": vertex_count,
        "triangles": triangle_count,
        "rest_matrix_max_abs_error": rest_error,
        "pose_matrix_max_abs_error": max(
            entry["pose_error"] for entry in baked
        ),
        "mesh_deformation_max_error_metres": mesh_error,
        "worst_mesh_deformation": worst_mesh,
        "transform_reference": "SkateAnimViewer._pose",
        "coordinate_conversion": "source Y-up row vectors -> Blender Z-up columns",
    }
    note.write(json.dumps(report, indent=2))

    output_path.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(output_path))
    report_path = output_path.with_suffix(".validation.json")
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("SKATE3_BLENDER_EXPORT " + json.dumps(report, sort_keys=True))


if __name__ == "__main__":
    main()
