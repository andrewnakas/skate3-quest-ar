#!/usr/bin/env python3
"""
Skate 2 / Skate 3 (Xbox 360) RW4 ".rx2" model container -- skeleton extractor.

Big-endian EA RenderWare 4 ("\\x89RW4xb2\\0\\r\\n\\x1a\\n") export.

The bone data lives in the section whose RW4 type code is 0x00EB0001. That
section is a byte-for-byte image of the engine struct ``pegasus::tRModelData``
(confirmed against sk82_na_f.xex, which ships full C++ mangled symbols):

    struct pegasus::tRModelData {            // sizeof == 0x40
        tAABB      m_BBox;                   // +0x00  Vector4 min, Vector4 max
        Matrix44  *m_pIBPMatrices;           // +0x20  inverse bind pose, 64B each
        tMeshData *m_pMeshTable;             // +0x24  8 bytes per mesh
        uint8    **m_pBoneNameTable;         // +0x28  numBones * u32 offsets
        uint8     *m_pBoneNameList;          // +0x2C  NUL-separated ASCII pool
        uint16     m_iNumTotalBones;         // +0x30  bones actually skinned
        uint16     m_iNumMeshes;             // +0x32
        uint16     m_iNumBones;              // +0x34  IBP / name-table count
        uint16     m_iNumIslands;            // +0x36
        tAABB     *m_pIslandAABBs;           // +0x38
        Vector4   *m_pIslandAreas;           // +0x3C
    };

All pointer fields are stored on disk as offsets relative to the start of the
section. Bone names are plain NUL-terminated ASCII -- *not* EA's FastString36
base-38 packing used by the sibling .abin format.

The parent hierarchy is NOT stored in the .rx2. Sk8::RemapBones() resolves each
rx2 bone name against Andale::HierarchyData (which lives in the .abin animation
database) via HierarchyData::GetBone(FastString<30>). The parent indices emitted
here are therefore reconstructed from a canonical name table, then validated
geometrically against the bind poses. See RX2_FORMAT.md.

Usage:  python rx2_skeleton.py <file.rx2> [...]
        python rx2_skeleton.py --all <dir>
"""

from __future__ import annotations

import glob
import math
import os
import struct
import sys
from typing import Dict, List, Optional, Tuple

MAGIC = b"\x89RW4xb2\x00\r\n\x1a\x0a"

# ---------------------------------------------------------------------------
# RW4 section type codes seen in Skate .rx2 files.
# ---------------------------------------------------------------------------

TYPE_MODEL_DATA = 0x00EB0001   # pegasus::tRModelData  <- bones live here
TYPE_MESH_DESC  = 0x00EB0023   # per-mesh skin descriptor + used-bone index list
TYPE_RAW_BUFFER = 0x00010031   # vertex / index buffer (lives in the GPU arena)
TYPE_VTX_DESC   = 0x000200E9   # renderengine::VertexDescriptor (element list)
TYPE_VB_DESC    = 0x000200EA   # vertex buffer descriptor  (byte size)
TYPE_IB_DESC    = 0x000200EB   # index buffer descriptor   (byte size + count)
TYPE_VTX_DECL   = 0x00020081   # small vertex-declaration summary

TYPE_NAMES: Dict[int, str] = {
    0x00010031: "RawBuffer(vertex/index, GPU arena)",
    0x00020081: "VertexDeclSummary",
    0x000200E9: "VertexDescriptor (element list)",
    0x000200EA: "VertexBufferDesc",
    0x000200EB: "IndexBufferDesc",
    0x00EB0001: "tRModelData (bbox + IBP matrices + bone names)",
    0x00EB0005: "MaterialTable (ASCII key/value strings)",
    0x00EB0008: "ModelHeader",
    0x00EB000B: "ExternalReferenceTable (64-bit GUID imports)",
    0x00EB000D: "InstanceInfo (transform + asset path string)",
    0x00EB0023: "MeshSkinDesc (bbox + used-bone index list)",
}

# ---------------------------------------------------------------------------
# Big-endian readers
# ---------------------------------------------------------------------------


def _u16(d: bytes, o: int) -> int:
    return struct.unpack_from(">H", d, o)[0]


def _u32(d: bytes, o: int) -> int:
    return struct.unpack_from(">I", d, o)[0]


def _f32v(d: bytes, o: int, n: int) -> Tuple[float, ...]:
    return struct.unpack_from(">%df" % n, d, o)


def _cstr(d: bytes, o: int) -> str:
    e = d.index(b"\x00", o)
    return d[o:e].decode("ascii", "replace")


# ---------------------------------------------------------------------------
# Canonical parent table for the Skate character / board rig.
#
# Reconstructed from bone names + Andale::HierarchyData parent arrays read out
# of the shipped .abin skeletons, and cross-checked geometrically against the
# bind poses in these very files (every derived bone offset must come out at a
# plausible anatomical length -- see validate()).
# ---------------------------------------------------------------------------

_P: Dict[str, Optional[str]] = {
    "hips": None,
    "spine": "hips",
    "spine1": "spine",
    "spine2": "spine1",
    "spine3": "spine2",
    "neck": "spine3",
    "neck1": "neck",
    "head": "neck1",
    # board
    "skateboard_root": None,
    "truck_front": "skateboard_root",
    "truck_back": "skateboard_root",
    "left_wheelfront": "truck_front",
    "right_wheelfront": "truck_front",
    "left_wheelback": "truck_back",
    "right_wheelback": "truck_back",
    # face root chain
    "face": "head",
    "offset_jaw": "face",
    "jaw": "offset_jaw",
    "offset_chin": "jaw",
    "chin": "offset_chin",
    "tongue": "jaw",
    "tonguetip": "tongue",
}

for _s in ("left", "right"):
    _P.update({
        _s + "shoulder": "spine3",
        _s + "arm": _s + "shoulder",
        _s + "armtwist": _s + "arm",
        _s + "forearm": _s + "arm",
        _s + "forearmtwist": _s + "forearm",
        _s + "forearmtwist1": _s + "forearm",
        _s + "hand": _s + "forearm",
        _s + "upleg": "hips",
        _s + "uplegtwist": _s + "upleg",
        _s + "leg": _s + "upleg",
        _s + "foot": _s + "leg",
        _s + "toebase": _s + "foot",
        _s + "inhandring": _s + "hand",
        _s + "inhandpinky": _s + "hand",
    })
    for _f in ("thumb", "index", "middle"):
        _P[_s + "hand" + _f + "1"] = _s + "hand"
        _P[_s + "hand" + _f + "2"] = _s + "hand" + _f + "1"
        _P[_s + "hand" + _f + "3"] = _s + "hand" + _f + "2"
    for _f in ("ring", "pinky"):
        _P[_s + "hand" + _f + "1"] = _s + "inhand" + _f
        _P[_s + "hand" + _f + "2"] = _s + "hand" + _f + "1"
        _P[_s + "hand" + _f + "3"] = _s + "hand" + _f + "2"

# Facial detail bones are flat children of Face in the shipped face rigs
# (their .abin HierarchyData parent array is [-1, 0, 1, -1, -1, ...]).
for _f in (
    "leftlowerlip", "lowerlip", "rightlowerlip",
    "leftcheek", "leftcrease", "lefteye", "leftinnereyebrow", "leftloweyelid",
    "leftmouth", "leftnose", "leftoutereyebrow", "leftupcheek", "leftupeyelid",
    "leftupperlip",
    "rightcheek", "rightcrease", "righteye", "rightinnereyebrow",
    "rightloweyelid", "rightmouth", "rightnose", "rightoutereyebrow",
    "rightupcheek", "rightupeyelid", "rightupperlip", "upperlip",
):
    _P.setdefault(_f, "face")

CANONICAL_PARENT = _P


# ---------------------------------------------------------------------------
# 4x4 rigid-transform helpers.
#
# Matrices are stored row-major with a row-vector convention: rows 0..2 are the
# rotation basis, row 3 is the translation, column 3 is (0,0,0,1).
# ---------------------------------------------------------------------------

Mat4 = List[List[float]]


def _mat_from(m: Tuple[float, ...]) -> Mat4:
    return [list(m[0:4]), list(m[4:8]), list(m[8:12]), list(m[12:16])]


def mat_mul(a: Mat4, b: Mat4) -> Mat4:
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)]
            for i in range(4)]


def rigid_inverse(m: Mat4) -> Mat4:
    """Inverse of a rigid row-vector matrix: R' = R^T, t' = -t * R^T."""
    r = [[m[j][i] for j in range(3)] for i in range(3)]          # transpose
    t = m[3][:3]
    ti = [-(t[0] * r[0][i] + t[1] * r[1][i] + t[2] * r[2][i]) for i in range(3)]
    return [r[0] + [0.0], r[1] + [0.0], r[2] + [0.0], list(ti) + [1.0]]


def mat_to_quat(m: Mat4) -> Tuple[float, float, float, float]:
    """Rotation part -> quaternion (x, y, z, w)."""
    t = m[0][0] + m[1][1] + m[2][2]
    if t > 0.0:
        s = math.sqrt(t + 1.0) * 2.0
        w = 0.25 * s
        x = (m[1][2] - m[2][1]) / s
        y = (m[2][0] - m[0][2]) / s
        z = (m[0][1] - m[1][0]) / s
    elif m[0][0] > m[1][1] and m[0][0] > m[2][2]:
        s = math.sqrt(1.0 + m[0][0] - m[1][1] - m[2][2]) * 2.0
        w = (m[1][2] - m[2][1]) / s
        x = 0.25 * s
        y = (m[1][0] + m[0][1]) / s
        z = (m[2][0] + m[0][2]) / s
    elif m[1][1] > m[2][2]:
        s = math.sqrt(1.0 + m[1][1] - m[0][0] - m[2][2]) * 2.0
        w = (m[2][0] - m[0][2]) / s
        x = (m[1][0] + m[0][1]) / s
        y = 0.25 * s
        z = (m[2][1] + m[1][2]) / s
    else:
        s = math.sqrt(1.0 + m[2][2] - m[0][0] - m[1][1]) * 2.0
        w = (m[0][1] - m[1][0]) / s
        x = (m[2][0] + m[0][2]) / s
        y = (m[2][1] + m[1][2]) / s
        z = 0.25 * s
    n = math.sqrt(x * x + y * y + z * z + w * w) or 1.0
    return (x / n, y / n, z / n, w / n)


# ---------------------------------------------------------------------------
# Container parsing
# ---------------------------------------------------------------------------


def parse_header(d: bytes) -> dict:
    """RW4 container header. Offsets are absolute file offsets."""
    if d[:12] != MAGIC:
        raise ValueError("not an RW4 .rx2 file (bad magic)")
    hdr = {
        "magic_ok": True,
        "version_a": d[0x10:0x13].decode("ascii", "replace"),   # "454"
        "version_b": d[0x14:0x17].decode("ascii", "replace"),   # "000"
        "hash": _u32(d, 0x1C),
        "section_count": _u32(d, 0x20),
        "section_count2": _u32(d, 0x24),
        "section_index_offset": _u32(d, 0x30),
        "gpu_arena_offset": _u32(d, 0x44),   # base of the 0x00010031 arena
        "gpu_arena_size": _u32(d, 0x54),
        "type_count": _u32(d, 0xCC),         # inside the 0x00010005 sub-block
        "type_list_offset": 0xD4,
    }
    return hdr


def parse_sections(d: bytes, hdr: dict) -> List[dict]:
    """Section index: `section_count` entries of 24 bytes, big-endian:
         +0x00 u32 offset      (file offset, or GPU-arena offset for 0x10031)
         +0x04 u32 unused      (always 0 in shipped files)
         +0x08 u32 size
         +0x0C u32 alignment
         +0x10 u32 type_index  (index into the type list at 0xD4)
         +0x14 u32 type_code
    """
    types = [_u32(d, hdr["type_list_offset"] + 4 * i)
             for i in range(hdr["type_count"] + 1)]
    out: List[dict] = []
    base = hdr["section_index_offset"]
    for i in range(hdr["section_count"]):
        off, unused, size, align, ti, tc = struct.unpack_from(
            ">6I", d, base + i * 24)
        gpu = tc == TYPE_RAW_BUFFER
        out.append({
            "index": i,
            "offset": off,
            "file_offset": (hdr["gpu_arena_offset"] + off) if gpu else off,
            "size": size,
            "alignment": align,
            "type_index": ti,
            "type_code": tc,
            "type_name": TYPE_NAMES.get(tc, "unknown"),
            "arena": "gpu" if gpu else "cpu",
            "type_index_ok": ti < len(types) and types[ti] == tc,
        })
    return out


def _parse_model_data(d: bytes, base: int) -> dict:
    """pegasus::tRModelData at absolute file offset `base`."""
    bbox_min = _f32v(d, base + 0x00, 4)
    bbox_max = _f32v(d, base + 0x10, 4)
    p_ibp = _u32(d, base + 0x20)
    p_mesh = _u32(d, base + 0x24)
    p_names = _u32(d, base + 0x28)
    p_pool = _u32(d, base + 0x2C)
    n_total, n_mesh, n_bones, n_isl = struct.unpack_from(">4H", d, base + 0x30)
    p_isl_bb = _u32(d, base + 0x38)
    p_isl_area = _u32(d, base + 0x3C)

    if p_ibp + 64 * n_bones > p_mesh:
        raise ValueError("IBP array overruns mesh table")
    if (p_pool - p_names) // 4 != n_bones:
        raise ValueError("bone name table size (%d) != m_iNumBones (%d)"
                         % ((p_pool - p_names) // 4, n_bones))

    bones: List[dict] = []
    for i in range(n_bones):
        name = _cstr(d, base + _u32(d, base + p_names + 4 * i))
        ibp = _mat_from(_f32v(d, base + p_ibp + 64 * i, 16))
        bind = rigid_inverse(ibp)
        bones.append({
            "index": i,
            "name": name,
            "parent": -1,
            "parent_name": None,
            "ibp_matrix": ibp,
            "bind_matrix": bind,
            "bind_pos": tuple(bind[3][:3]),
            "bind_quat": mat_to_quat(bind),
        })

    return {
        "bbox_min": bbox_min[:3],
        "bbox_max": bbox_max[:3],
        "num_total_bones": n_total,
        "num_meshes": n_mesh,
        "num_bones": n_bones,
        "num_islands": n_isl,
        "off_ibp": p_ibp,
        "off_mesh_table": p_mesh,
        "off_name_table": p_names,
        "off_name_pool": p_pool,
        "off_island_bboxes": p_isl_bb,
        "off_island_areas": p_isl_area,
        "bones": bones,
    }


def _parse_mesh_desc(d: bytes, base: int) -> dict:
    """0x00EB0023: mesh bbox + the list of bone-palette slots this mesh skins to.
    +0x00 Vector4 bbox min, +0x10 Vector4 bbox max,
    +0x48 u32 used-bone count, +0x4C u32 offset to a u16[] of palette indices."""
    count = _u32(d, base + 0x48)
    off = _u32(d, base + 0x4C)
    return {
        "bbox_min": _f32v(d, base + 0x00, 3),
        "bbox_max": _f32v(d, base + 0x10, 3),
        "used_bone_count": count,
        "used_bone_indices": [_u16(d, base + off + 2 * i) for i in range(count)],
    }


# ---------------------------------------------------------------------------
# Mesh geometry: vertex declaration, vertex buffer, index buffer.
#
# The 0x000200E9 section is a byte-image of ``renderengine::VertexDescriptor``
# (layout confirmed from the executable's type info):
#
#     struct renderengine::VertexDescriptor {        // sizeof == 0x20
#         D3DVertexDeclaration *m_d3dVertexDeclaration;  // +0x00  0 on disk
#         uint32   m_typesFlags;                         // +0x04
#         uint16   m_numElements;                        // +0x08
#         int16    m_refCount;                           // +0x0A
#         uint16   m_instanceStreams;                    // +0x0C
#         uint16   m_pad0;                               // +0x0E
#         Element  m_elements[1];                        // +0x10  (numElements)
#     };
#     struct renderengine::VertexDescriptor::Element {   // sizeof == 0x10
#         uint16 stream;        // +0x00
#         uint16 offset;        // +0x02   byte offset within the vertex
#         uint32 format;        // +0x04   renderengine::VertexFormat
#         uint8  method;        // +0x08
#         uint8  usage;         // +0x09   D3DDECLUSAGE
#         uint8  usageIndex;    // +0x0A
#         uint8  type;          // +0x0B
#         uint32 elementClass;  // +0x0C
#     };
#
# Immediately after the element array the exporter appends one uint8 per
# element holding that element's stream stride (all equal in these files).
# ---------------------------------------------------------------------------

# D3DDECLUSAGE, confirmed empirically (weights sum to 1, indices < palette size)
USAGE_NAMES: Dict[int, str] = {
    0: "POSITION", 1: "BLENDWEIGHT", 2: "BLENDINDICES", 3: "NORMAL",
    4: "PSIZE", 5: "TEXCOORD", 6: "TANGENT", 7: "BINORMAL",
    8: "TESSFACTOR", 9: "POSITIONT", 10: "COLOR", 11: "FOG",
    12: "DEPTH", 13: "SAMPLE",
}

# renderengine::VertexFormat codes seen in these assets -> (name, byte size).
# Sizes are *proved* by the element offsets (they tile the stride exactly);
# the semantic names are inferred from the decoded values.
VTX_FORMATS: Dict[int, Tuple[str, int]] = {
    0x001A215A: ("SHORT4",       8),   # raw int16[4]  -- position
    0x001A2286: ("UBYTE4",       4),   # raw uint8[4]  -- blend indices / weights
    0x002C2159: ("SHORT2N",      4),   # int16[2] / 32768 -- texcoord
    0x002C23A5: ("FLOAT2",       8),   # Xenos format37: two big-endian float32 UVs
    0x002A2190: ("PACKED11_11_10N", 4),  # signed 11/11/10 -- tangent, binormal
}

# Position de-quantisation. Positions are raw int16 with a fixed scale of
# 2^-14 and a constant +0.8 m bias on Y. Derived empirically and cross-checked
# against the tRModelData / 0xEB0023 mesh bboxes on all 16 assets: every one of
# the 96 axis bounds agrees to under one quantisation step (6.1e-5 m). The
# constants are not stored anywhere in the file -- they are baked into the
# character vertex shader, so they are a property of the Sk8 character
# compressor rather than of this particular asset.
POS_SCALE = 1.0 / 16384.0
POS_BIAS = (0.0, 0.8, 0.0)


def _dec_11_11_10(u: int) -> Tuple[float, float, float]:
    """Signed 11/11/10 normalised, low bits first: x=[0:11] y=[11:22] z=[22:32].

    Component order and normalisation were settled geometrically: with this
    decode |tangent| has median 0.9992 and cross(tangent, binormal) agrees with
    the mesh's geometric vertex normals at a median dot product of +0.9939.
    Every other candidate ordering gives |t| ~ 0.83 and a dot near zero.
    """
    out = []
    for shift, bits in ((0, 11), (11, 11), (22, 10)):
        v = (u >> shift) & ((1 << bits) - 1)
        if v >= 1 << (bits - 1):
            v -= 1 << bits
        out.append(v / float((1 << (bits - 1)) - 1))
    return (out[0], out[1], out[2])


def _parse_vertex_descriptor(d: bytes, base: int, size: int) -> dict:
    """0x000200E9 -> renderengine::VertexDescriptor."""
    n = _u16(d, base + 0x08)
    elems: List[dict] = []
    for i in range(n):
        o = base + 0x10 + 16 * i
        stream, offset = struct.unpack_from(">2H", d, o)
        fmt = _u32(d, o + 4)
        method, usage, usage_index, etype = struct.unpack_from(">4B", d, o + 8)
        fname, fsize = VTX_FORMATS.get(fmt, ("UNKNOWN_0x%08X" % fmt, 0))
        elems.append({
            "stream": stream,
            "offset": offset,
            "format": fmt,
            "format_name": fname,
            "size": fsize,
            "method": method,
            "usage": usage,
            "usage_name": USAGE_NAMES.get(usage, "USAGE_%d" % usage),
            "usage_index": usage_index,
            "type": etype,
            "element_class": _u32(d, o + 0x0C),
        })
    tail = base + 0x10 + 16 * n
    strides = list(d[tail:tail + n]) if tail + n <= base + size else []
    return {
        "types_flags": _u32(d, base + 0x04),
        "num_elements": n,
        "instance_streams": _u16(d, base + 0x0C),
        "elements": elems,
        "stride": strides[0] if strides else 0,
        "element_strides": strides,
    }


def _parse_vb_desc(d: bytes, base: int) -> dict:
    """0x000200EA. Only the byte size at +0x20 is load-bearing."""
    return {"byte_size": _u32(d, base + 0x20), "flags": _u32(d, base + 0x18)}


def _parse_ib_desc(d: bytes, base: int) -> dict:
    """0x000200EB. +0x1C allocated byte size, +0x20 index count.
    The low byte of +0x00 (2) is the index width in bytes -- consistent with
    count*2 <= allocated size for every asset, exactly equal in some."""
    return {
        "index_bytes": _u32(d, base + 0x00) & 0xFF,
        "byte_size": _u32(d, base + 0x1C),
        "index_count": _u32(d, base + 0x20),
        "flags": _u32(d, base + 0x18),
    }


def _decode_vertices(d: bytes, vb: int, count: int, vdesc: dict) -> dict:
    """Walk the vertex buffer once, decoding every element we understand."""
    stride = vdesc["stride"]
    out: Dict[str, list] = {}
    for e in vdesc["elements"]:
        key = {
            ("POSITION", 0): "positions",
            ("BLENDWEIGHT", 0): "skin_weights",
            ("BLENDINDICES", 0): "skin_indices",
            ("NORMAL", 0): "normals",
            ("TEXCOORD", 0): "uvs",
            ("TEXCOORD", 1): "uvs2",
            ("TANGENT", 0): "tangents",
            ("BINORMAL", 0): "binormals",
        }.get((e["usage_name"], e["usage_index"]))
        if key is None:
            continue
        fmt, off = e["format_name"], e["offset"]
        vals: list = []
        if e["usage_name"] == "POSITION" and fmt == "SHORT4":
            for v in range(count):
                x, y, z = struct.unpack_from(">3h", d, vb + v * stride + off)
                vals.append((x * POS_SCALE + POS_BIAS[0],
                             y * POS_SCALE + POS_BIAS[1],
                             z * POS_SCALE + POS_BIAS[2]))
        elif fmt == "PACKED11_11_10N":
            for v in range(count):
                vals.append(_dec_11_11_10(_u32(d, vb + v * stride + off)))
        elif fmt == "SHORT2N":
            for v in range(count):
                u, w = struct.unpack_from(">2h", d, vb + v * stride + off)
                vals.append((u / 32768.0, w / 32768.0))
        elif fmt == "FLOAT2":
            for v in range(count):
                vals.append(struct.unpack_from(">2f", d, vb + v * stride + off))
        elif fmt == "SHORT4N":
            for v in range(count):
                q = struct.unpack_from(">4h", d, vb + v * stride + off)
                vals.append(tuple(c / 32768.0 for c in q))
        elif fmt == "UBYTE4" and e["usage_name"] == "BLENDWEIGHT":
            for v in range(count):
                q = struct.unpack_from(">4B", d, vb + v * stride + off)
                vals.append(tuple(c / 255.0 for c in q))
        elif fmt == "UBYTE4":
            for v in range(count):
                vals.append(struct.unpack_from(">4B", d, vb + v * stride + off))
        else:
            continue
        out[key] = vals

    return out


def _derive_normals(mesh: dict) -> None:
    """No NORMAL element is exported -- the shader rebuilds it from the tangent
    frame as cross(tangent, binormal). Validated geometrically: the result
    agrees with the mesh's own area-weighted face normals at a median dot
    product of +0.994.

    A handful of vertices in the low LODs ship a *degenerate* frame with
    binormal == -tangent (a collapsed UV shell), where the cross product is
    undefined. Those fall back to the geometric normal.
    """
    t_all, b_all = mesh.get("tangents"), mesh.get("binormals")
    if "normals" in mesh or not (t_all and b_all):
        return
    nrm: List[Optional[Tuple[float, float, float]]] = []
    degenerate: List[int] = []
    for i, (t, b) in enumerate(zip(t_all, b_all)):
        n = (t[1] * b[2] - t[2] * b[1],
             t[2] * b[0] - t[0] * b[2],
             t[0] * b[1] - t[1] * b[0])
        L = math.sqrt(n[0] ** 2 + n[1] ** 2 + n[2] ** 2)
        if L < 1e-3:
            degenerate.append(i)
            nrm.append(None)
        else:
            nrm.append((n[0] / L, n[1] / L, n[2] / L))

    if degenerate:
        pos, idx = mesh.get("positions"), mesh.get("indices")
        acc = {i: [0.0, 0.0, 0.0] for i in degenerate}
        if pos and idx:
            want = set(degenerate)
            for t in range(0, len(idx) - 2, 3):
                tri = (idx[t], idx[t + 1], idx[t + 2])
                if not want.intersection(tri):
                    continue
                a, b, c = pos[tri[0]], pos[tri[1]], pos[tri[2]]
                u = [b[k] - a[k] for k in range(3)]
                w = [c[k] - a[k] for k in range(3)]
                f = (u[1] * w[2] - u[2] * w[1],
                     u[2] * w[0] - u[0] * w[2],
                     u[0] * w[1] - u[1] * w[0])
                for vi in tri:
                    if vi in acc:
                        for k in range(3):
                            acc[vi][k] += f[k]
        for i in degenerate:
            g = acc[i]
            L = math.sqrt(sum(c * c for c in g))
            nrm[i] = (g[0] / L, g[1] / L, g[2] / L) if L > 1e-12 else (0.0, 1.0, 0.0)

    mesh["normals"] = nrm
    mesh["normals_derived"] = True
    mesh["degenerate_tangent_frames"] = degenerate


def _section_by_index(sections: List[dict], idx: int, want: int) -> Optional[dict]:
    """Resolve a section reference stored in the 0xEB0023 mesh descriptor.

    The four u32s at +0x28..+0x34 hold section-index numbers. Every shipped
    asset here uses the same section ordering, so this cannot be proved
    uniquely -- we therefore verify the type code and fall back to "the first
    section with the wanted type" if it does not match.
    """
    if 0 <= idx < len(sections) and sections[idx]["type_code"] == want:
        return sections[idx]
    for s in sections:
        if s["type_code"] == want:
            return s
    return None


def _attach_geometry(d: bytes, sections: List[dict], base: int, mesh: dict) -> None:
    """Decode the vertex/index buffers referenced by one 0xEB0023 descriptor."""
    vdesc_s = _section_by_index(sections, _u32(d, base + 0x28), TYPE_VTX_DESC)
    ibd_s = _section_by_index(sections, _u32(d, base + 0x30), TYPE_IB_DESC)
    vbd_s = _section_by_index(sections, _u32(d, base + 0x34), TYPE_VB_DESC)
    if not (vdesc_s and ibd_s and vbd_s):
        return

    vdesc = _parse_vertex_descriptor(d, vdesc_s["file_offset"], vdesc_s["size"])
    vbd = _parse_vb_desc(d, vbd_s["file_offset"])
    ibd = _parse_ib_desc(d, ibd_s["file_offset"])
    stride = vdesc["stride"]
    if not stride:
        return

    # Raw GPU-arena buffers are matched to their descriptors by byte size; the
    # vertex buffer always precedes the index buffer in the arena.
    raws = sorted([s for s in sections if s["type_code"] == TYPE_RAW_BUFFER],
                  key=lambda s: s["offset"])
    vb_s = next((s for s in raws if s["size"] == vbd["byte_size"]), None)
    ib_s = next((s for s in raws
                 if s["size"] == ibd["byte_size"] and s is not vb_s), None)
    if vb_s is None and raws:
        vb_s = raws[0]
    if ib_s is None and len(raws) > 1:
        ib_s = raws[1]
    if vb_s is None or ib_s is None:
        return

    count = vbd["byte_size"] // stride
    mesh.update(_decode_vertices(d, vb_s["file_offset"], count, vdesc))
    mesh["vertex_count"] = count
    mesh["stride"] = stride
    mesh["vertex_elements"] = vdesc["elements"]
    mesh["vb_padding"] = vbd["byte_size"] - count * stride

    n_idx = ibd["index_count"]
    ibo = ib_s["file_offset"]
    mesh["indices"] = list(struct.unpack_from(">%dH" % n_idx, d, ibo))
    mesh["index_count"] = n_idx
    mesh["triangle_count"] = n_idx // 3
    mesh["ib_padding"] = ibd["byte_size"] - n_idx * 2

    # Draw call: {primType(4 == D3DPT_TRIANGLELIST), ?, ?, indexCount}.
    # The two middle fields are 0 in every shipped asset, so their meaning
    # (start index / base vertex) is a guess.
    dc = _u32(d, base + 0x44)
    if dc:
        prim, a, b, cnt = struct.unpack_from(">4I", d, base + dc)
        mesh["draw_call"] = {"prim_type": prim, "field1": a, "field2": b,
                             "index_count": cnt}

    _derive_normals(mesh)   # needs the index buffer for the degenerate cases

    # Map palette-local blend indices onto the file's bone array.
    pal = mesh.get("used_bone_indices") or []
    if pal and "skin_indices" in mesh:
        mesh["skin_bones"] = [tuple(pal[i] if i < len(pal) else 0 for i in q)
                              for q in mesh["skin_indices"]]


def _assign_parents(bones: List[dict]) -> None:
    """Fill in `parent` / `parent_name` from CANONICAL_PARENT.

    A file only carries the bones its meshes skin to, so a bone's canonical
    parent is often absent from the palette. In that case we walk up the
    canonical chain to the nearest ancestor that *is* present, which keeps the
    emitted tree connected and acyclic. `parent_name` always reports the true
    canonical direct parent, present or not.
    """
    by_name = {b["name"].lower(): b["index"] for b in bones}
    for b in bones:
        key = b["name"].lower()
        direct = CANONICAL_PARENT.get(key, "__unknown__")
        b["parent_name"] = None if direct in (None, "__unknown__") else direct
        b["parent_known"] = direct != "__unknown__"
        cur, seen = direct, set()
        while cur not in (None, "__unknown__") and cur not in seen:
            seen.add(cur)
            if cur in by_name:
                b["parent"] = by_name[cur]
                break
            cur = CANONICAL_PARENT.get(cur, None)
        else:
            b["parent"] = -1


def parse_rx2(path: str, geometry: bool = True) -> dict:
    """Parse an .rx2 RW4 container and extract its skeleton and mesh geometry.

    Returns a dict with:
      'header'   -- container header fields
      'sections' -- list of section-index entries
      'model'    -- tRModelData fields (bbox, counts, sub-offsets)
      'bones'    -- [{'name','index','parent','parent_name','bind_matrix',
                      'ibp_matrix','bind_pos','bind_quat'}]
      'meshes'   -- 0x00EB0023 descriptors incl. used-bone index lists, plus
                    (when ``geometry``) the decoded buffers:
                      'positions'     [(x, y, z)]  model space, metres
                      'indices'       flat triangle-list vertex indices
                      'normals'       [(x, y, z)]  unit; derived cross(T, B)
                      'tangents' / 'binormals'     [(x, y, z)] unit
                      'uvs'  (and 'uvs2' on the hair meshes)
                      'skin_indices'  [(i, i, i, i)] into 'used_bone_indices'
                      'skin_bones'    [(b, b, b, b)] into 'bones'
                      'skin_weights'  [(w, w, w, w)] sums to 1.0
                      'vertex_count', 'triangle_count', 'stride',
                      'vertex_elements', 'draw_call'

    Pass ``geometry=False`` to skip the buffers (skeleton-only, as before).
    """
    with open(path, "rb") as fh:
        d = fh.read()

    hdr = parse_header(d)
    sections = parse_sections(d, hdr)

    model: Optional[dict] = None
    meshes: List[dict] = []
    for s in sections:
        if s["type_code"] == TYPE_MODEL_DATA and model is None:
            model = _parse_model_data(d, s["file_offset"])
        elif s["type_code"] == TYPE_MESH_DESC:
            m = _parse_mesh_desc(d, s["file_offset"])
            if geometry:
                _attach_geometry(d, sections, s["file_offset"], m)
            meshes.append(m)

    if model is None:
        raise ValueError("no 0x00EB0001 tRModelData section in %s" % path)

    bones = model.pop("bones")
    _assign_parents(bones)

    return {
        "path": path,
        "file_size": len(d),
        "header": hdr,
        "sections": sections,
        "model": model,
        "bones": bones,
        "meshes": meshes,
    }


def parse_rx2_meshes(path: str) -> List[dict]:
    """Convenience wrapper: just the renderable meshes of an .rx2."""
    return parse_rx2(path, geometry=True)["meshes"]


# ---------------------------------------------------------------------------
# Validation -- these checks are the ground truth for "did we decode it right".
# ---------------------------------------------------------------------------


def validate_geometry(res: dict) -> List[str]:
    """Geometric sanity checks on the decoded vertex/index buffers."""
    errs: List[str] = []
    for mi, m in enumerate(res["meshes"]):
        tag = "mesh %d" % mi
        pos = m.get("positions")
        idx = m.get("indices")
        if pos is None or idx is None:
            errs.append("%s: no geometry decoded" % tag)
            continue

        # Positions must sit inside the mesh bbox (1% slack on the extent).
        lo, hi = m["bbox_min"], m["bbox_max"]
        slack = [0.01 * (hi[k] - lo[k]) + 1e-4 for k in range(3)]
        bad = 0
        for p in pos:
            if any(not (lo[k] - slack[k] <= p[k] <= hi[k] + slack[k])
                   for k in range(3)):
                bad += 1
        if bad:
            errs.append("%s: %d/%d vertices outside bbox" % (tag, bad, len(pos)))

        if len(idx) % 3:
            errs.append("%s: index count %d not a multiple of 3"
                        % (tag, len(idx)))
        oor = [i for i in idx if i >= len(pos)]
        if oor:
            errs.append("%s: %d indices >= vertex count %d"
                        % (tag, len(oor), len(pos)))
            continue

        # Mean triangle edge length must be mm-to-cm scale.
        tot = n = 0.0
        for t in range(0, len(idx) - 2, 3):
            a, b, c = (pos[idx[t]], pos[idx[t + 1]], pos[idx[t + 2]])
            for u, v in ((a, b), (b, c), (c, a)):
                tot += math.sqrt(sum((u[k] - v[k]) ** 2 for k in range(3)))
                n += 1
        mean_edge = tot / n if n else 0.0
        m["mean_edge_length"] = mean_edge
        if not (1e-4 < mean_edge < 0.25):
            errs.append("%s: mean edge length %.6g m out of plausible range"
                        % (tag, mean_edge))

        # Degenerate triangles are a red flag for a wrong index stride.
        degen = sum(1 for t in range(0, len(idx) - 2, 3)
                    if len({idx[t], idx[t + 1], idx[t + 2]}) < 3)
        if degen:
            errs.append("%s: %d degenerate triangles" % (tag, degen))

        w = m.get("skin_weights")
        if w:
            worst = max(abs(sum(q) - 1.0) for q in w)
            m["max_weight_error"] = worst
            if worst > 0.01:
                errs.append("%s: blend weights sum off by %.4f" % (tag, worst))
        si = m.get("skin_indices")
        if si and m.get("used_bone_count"):
            mx = max(max(q) for q in si)
            if mx >= m["used_bone_count"]:
                errs.append("%s: blend index %d >= used-bone count %d"
                            % (tag, mx, m["used_bone_count"]))
        for key in ("normals", "tangents", "binormals"):
            vecs = m.get(key)
            if not vecs:
                continue
            worst = max(abs(math.sqrt(sum(c * c for c in v)) - 1.0)
                        for v in vecs)
            if worst > 0.05:
                errs.append("%s: %s off unit length by %.4f" % (tag, key, worst))
        uv = m.get("uvs")
        if uv and not all(-4.0 < c < 4.0 for q in uv for c in q):
            errs.append("%s: texcoords out of plausible range" % tag)
    return errs


def validate(res: dict) -> List[str]:
    """Return a list of problems; empty means every sanity check passed."""
    errs: List[str] = []
    bones = res["bones"]
    mdl = res["model"]

    for s in res["sections"]:
        if not s["type_index_ok"]:
            errs.append("section %d: type_index does not resolve to type_code"
                        % s["index"])
        if s["arena"] == "cpu" and s["offset"] + s["size"] > res["file_size"]:
            errs.append("section %d overruns file" % s["index"])

    if len(bones) != mdl["num_bones"]:
        errs.append("bone count mismatch")

    for mi, m in enumerate(res["meshes"]):
        if m["used_bone_count"] > mdl["num_bones"]:
            errs.append("mesh %d: used-bone count %d > numBones %d"
                        % (mi, m["used_bone_count"], mdl["num_bones"]))
        bad = [i for i in m["used_bone_indices"] if i >= mdl["num_bones"]]
        if bad:
            errs.append("mesh %d: used-bone indices out of range: %s"
                        % (mi, bad[:8]))
        if m["used_bone_indices"] != sorted(m["used_bone_indices"]):
            errs.append("mesh %d: used-bone index list not ascending" % mi)

    lo, hi = mdl["bbox_min"], mdl["bbox_max"]
    if not all(lo[i] <= hi[i] for i in range(3)):
        errs.append("bbox min > max")
    if not all(-10.0 < v < 10.0 for v in list(lo) + list(hi)):
        errs.append("bbox out of human scale: %s %s" % (lo, hi))

    for b in bones:
        m = b["ibp_matrix"]
        for r in range(3):
            n = math.sqrt(sum(c * c for c in m[r][:3]))
            if abs(n - 1.0) > 1e-3:
                errs.append("%s: IBP row %d not unit length (%.5f)"
                            % (b["name"], r, n))
        det = (m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
               - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
               + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]))
        if abs(det - 1.0) > 1e-3:
            errs.append("%s: IBP determinant %.5f != 1" % (b["name"], det))
        if abs(m[3][3] - 1.0) > 1e-6 or any(abs(m[r][3]) > 1e-6
                                            for r in range(3)):
            errs.append("%s: IBP last column not (0,0,0,1)" % b["name"])

        q = b["bind_quat"]
        qn = math.sqrt(sum(c * c for c in q))
        if abs(qn - 1.0) > 1e-4:
            errs.append("%s: bind quat not unit (%.5f)" % (b["name"], qn))

        p = b["bind_pos"]
        if not all(-4.0 < v < 4.0 for v in p):
            errs.append("%s: bind pos out of human scale %s" % (b["name"], p))
        if not (lo[0] - 1.0 <= p[0] <= hi[0] + 1.0
                and lo[1] - 1.5 <= p[1] <= hi[1] + 1.5
                and lo[2] - 1.0 <= p[2] <= hi[2] + 1.0):
            errs.append("%s: bind pos %s far outside model bbox"
                        % (b["name"], p))

        if not b["parent_known"]:
            errs.append("%s: no canonical parent known" % b["name"])

    # Tree validity: acyclic, and every edge a plausible bone length.
    for b in bones:
        p, seen = b["parent"], set()
        while p != -1:
            if p in seen:
                errs.append("%s: parent cycle" % b["name"])
                break
            seen.add(p)
            p = bones[p]["parent"]
        if b["parent"] != -1:
            a, c = bones[b["parent"]]["bind_pos"], b["bind_pos"]
            dist = math.sqrt(sum((c[i] - a[i]) ** 2 for i in range(3)))
            if dist > 0.75:
                errs.append("%s -> %s: implausible bone length %.3f m"
                            % (bones[b["parent"]]["name"], b["name"], dist))

    if any("positions" in m for m in res["meshes"]):
        errs.extend(validate_geometry(res))
    return errs


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def print_report(res: dict) -> bool:
    mdl = res["model"]
    errs = validate(res)          # also fills in per-mesh derived statistics
    print("=" * 78)
    print("%s  (%d bytes)" % (res["path"], res["file_size"]))
    h = res["header"]
    print("  RW4 v%s/%s hash=0x%08X  sections=%d @0x%X  gpu arena 0x%X+0x%X"
          % (h["version_a"], h["version_b"], h["hash"], h["section_count"],
             h["section_index_offset"], h["gpu_arena_offset"],
             h["gpu_arena_size"]))
    print("  sections:")
    for s in res["sections"]:
        print("    [%2d] %-4s off=0x%06X size=0x%06X align=0x%02X "
              "type=0x%08X  %s"
              % (s["index"], s["arena"], s["offset"], s["size"],
                 s["alignment"], s["type_code"], s["type_name"]))
    print("  tRModelData: numBones=%d numTotalBones=%d numMeshes=%d "
          "numIslands=%d" % (mdl["num_bones"], mdl["num_total_bones"],
                             mdl["num_meshes"], mdl["num_islands"]))
    print("    bbox min=(%.4f, %.4f, %.4f) max=(%.4f, %.4f, %.4f)  [metres]"
          % (tuple(mdl["bbox_min"]) + tuple(mdl["bbox_max"])))
    for mi, m in enumerate(res["meshes"]):
        print("    mesh skins %d palette slots: %s"
              % (m["used_bone_count"], m["used_bone_indices"]))
        if "positions" in m:
            print("    mesh %d: %d verts (stride %d) / %d tris, mean edge "
                  "%.2f mm, max weight err %.5f"
                  % (mi, m["vertex_count"], m["stride"], m["triangle_count"],
                     1000.0 * m.get("mean_edge_length", 0.0),
                     m.get("max_weight_error", 0.0)))
            print("      layout: " + ", ".join(
                "+%d %s[%d]=%s" % (e["offset"], e["usage_name"],
                                   e["usage_index"], e["format_name"])
                for e in m["vertex_elements"]))

    print("  bone tree (bind position in model space, metres):")
    kids: Dict[int, List[int]] = {}
    for b in res["bones"]:
        kids.setdefault(b["parent"], []).append(b["index"])

    def walk(i: int, depth: int) -> None:
        b = res["bones"][i]
        p = b["bind_pos"]
        q = b["bind_quat"]
        print("    %s%-3d %-24s parent=%-3d pos=(%8.4f,%8.4f,%8.4f) "
              "quat=(%6.3f,%6.3f,%6.3f,%6.3f)"
              % ("  " * depth, b["index"], b["name"], b["parent"],
                 p[0], p[1], p[2], q[0], q[1], q[2], q[3]))
        for c in kids.get(i, []):
            walk(c, depth + 1)

    for r in kids.get(-1, []):
        walk(r, 0)

    if errs:
        print("  VALIDATION FAILED (%d):" % len(errs))
        for e in errs[:20]:
            print("    ! " + e)
    else:
        print("  validation: OK (rigid IBP matrices, unit quats, human-scale "
              "positions, acyclic tree, plausible bone lengths)")
    return not errs


def main(argv: List[str]) -> int:
    args = argv[1:]
    if not args:
        print(__doc__)
        return 1
    if args[0] == "--all":
        root = args[1] if len(args) > 1 else "."
        paths = sorted(glob.glob(os.path.join(root, "**", "*.rx2"),
                                 recursive=True))
    else:
        paths = args

    ok = failed = 0
    for p in paths:
        try:
            if print_report(parse_rx2(p)):
                ok += 1
            else:
                failed += 1
        except Exception as exc:                       # noqa: BLE001
            print("=" * 78)
            print("%s: PARSE ERROR: %s" % (p, exc))
            failed += 1
    print("=" * 78)
    print("%d/%d file(s) parsed and validated" % (ok, ok + failed))
    return 0 if failed == 0 else 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
