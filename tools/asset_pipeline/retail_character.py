"""Pure RX2/ABIN rig and morph decoding extracted from the project converters.
See tools/vendor/skate3_anim/PROVENANCE.md and THIRD_PARTY_NOTICES.md.
No scene editor or rendering application is used.
"""
from __future__ import annotations
import glob,math,os,struct,sys
from pathlib import Path
import numpy as np
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'vendor/skate3_anim'))
import abin_importer as ABIN
import rx2_skeleton as RX2
TYPE_RAW_BUFFER=0x00010031
TYPE_VTX_DESC=0x000200E9
TYPE_VB_DESC=0x000200EA
TYPE_VTX_DECL=0x00020081
TYPE_MESH_DESC=0x00EB0023
TYPE_MORPH_DESC=0x00EB000E
MORPH_VERTEX_STRIDE=16
MORPH_POSITION_SCALE=1.0/16384.0
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

    def __init__(self, folder, parsed_models=None):
        self.models = []
        self.bind = {}
        self.ibp = {}
        self.errors = []
        files = sorted(
            glob.glob(os.path.join(folder, "**", "*.rx2"), recursive=True)
        )
        for path in files:
            try:
                parsed = parsed_models[path] if parsed_models is not None else RX2.parse_rx2(path)
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

def be_u32(data: bytes, offset: int) -> int:
    return struct.unpack_from(">I", data, offset)[0]

def morph_weight(name: str, manifest: dict) -> float:
    body = manifest["preset"]["body_mods"]
    if name == "fat" or name.startswith("fat_"):
        return float(body["fatness"])
    if name == "thin" or name.startswith("thin_"):
        return float(body["skinniness"])
    if name in manifest["morph_assembly"]["face_targets"]:
        return float(body.get("targets", {}).get(name, body["face_fields"]))
    raise RuntimeError(f"Retail morph target has no recipe mapping: {name}")

def decode_dense_morphs(
    path: Path, parsed: dict, vertex_count: int, rx2_skeleton
) -> list[dict]:
    data = path.read_bytes()
    sections = parsed["sections"]
    mesh_descs = [
        section
        for section in sections
        if section["type_code"] == TYPE_MESH_DESC
    ]
    if len(mesh_descs) != 1:
        raise RuntimeError(
            f"{path.name} expected one mesh descriptor, found {len(mesh_descs)}"
        )
    base = mesh_descs[0]["file_offset"]
    morph_count = be_u32(data, base + 0x50)
    morph_table = be_u32(data, base + 0x54)
    morphs = []
    for morph_index in range(morph_count):
        entry = base + morph_table + morph_index * 16
        target_hash, descriptor_index, name_offset = struct.unpack_from(
            ">QII", data, entry
        )
        name_start = base + name_offset
        name_end = data.find(b"\0", name_start)
        if name_end < 0:
            raise RuntimeError(
                f"{path.name} morph {morph_index} has no terminated name"
            )
        name = data[name_start:name_end].decode("ascii")
        if descriptor_index < 4 or descriptor_index >= len(sections):
            raise RuntimeError(
                f"{path.name} morph {name} has invalid descriptor index "
                f"{descriptor_index}"
            )
        expected_types = (
            TYPE_RAW_BUFFER,
            TYPE_VB_DESC,
            TYPE_VTX_DECL,
            TYPE_VTX_DESC,
            TYPE_MORPH_DESC,
        )
        stream_sections = sections[descriptor_index - 4 : descriptor_index + 1]
        actual_types = tuple(
            section["type_code"] for section in stream_sections
        )
        if actual_types != expected_types:
            raise RuntimeError(
                f"{path.name} morph {name} stream layout changed: "
                f"{actual_types} != {expected_types}"
            )
        raw, vb_desc, _vtx_decl, vtx_desc, _morph_desc = stream_sections
        byte_size = be_u32(data, vb_desc["file_offset"] + 0x20)
        if byte_size != vertex_count * MORPH_VERTEX_STRIDE:
            raise RuntimeError(
                f"{path.name} morph {name} byte size changed: "
                f"{byte_size} != {vertex_count * MORPH_VERTEX_STRIDE}"
            )
        if raw["size"] != byte_size:
            raise RuntimeError(
                f"{path.name} morph {name} raw-buffer size changed"
            )
        descriptor = rx2_skeleton._parse_vertex_descriptor(
            data, vtx_desc["file_offset"], vtx_desc["size"]
        )
        expected_elements = [
            ("POSITION", 0, "SHORT4", 0),
            ("NORMAL", 0, "PACKED11_11_10N", 8),
            ("BLENDWEIGHT", 0, "UNKNOWN_0x002C83A4", 12),
        ]
        actual_elements = [
            (
                element["usage_name"],
                element["usage_index"],
                element["format_name"],
                element["offset"],
            )
            for element in descriptor["elements"]
        ]
        if (
            descriptor["stride"] != MORPH_VERTEX_STRIDE
            or actual_elements != expected_elements
        ):
            raise RuntimeError(
                f"{path.name} morph {name} vertex declaration changed"
            )

        deltas = []
        normal_deltas = []
        nonzero_vertices = 0
        nonzero_normal_deltas = 0
        max_delta = 0.0
        for vertex_index in range(vertex_count):
            vertex_offset = (
                raw["file_offset"] + vertex_index * MORPH_VERTEX_STRIDE
            )
            x, y, z, _remap = struct.unpack_from(
                ">4h", data, vertex_offset
            )
            packed_normal = be_u32(data, vertex_offset + 8)
            normal_deltas.append(rx2_skeleton._dec_11_11_10(packed_normal))
            if packed_normal != 0:
                nonzero_normal_deltas += 1
            if be_u32(data, vertex_offset + 12) != 0x3F800000:
                raise RuntimeError(
                    f"{path.name} morph {name} has an unhandled vertex factor"
                )
            delta = (
                x * MORPH_POSITION_SCALE,
                y * MORPH_POSITION_SCALE,
                z * MORPH_POSITION_SCALE,
            )
            length = math.sqrt(sum(value * value for value in delta))
            if length > 0.0:
                nonzero_vertices += 1
                max_delta = max(max_delta, length)
            deltas.append(delta)
        morphs.append(
            {
                "name": name,
                "hash": f"{target_hash:016x}",
                "descriptor_index": descriptor_index,
                "deltas": deltas,
                "normal_deltas": normal_deltas,
                "nonzero_vertices": nonzero_vertices,
                "nonzero_normal_deltas": nonzero_normal_deltas,
                "max_source_delta": max_delta,
            }
        )
    return morphs
