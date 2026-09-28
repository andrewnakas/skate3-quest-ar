"""
Skate 3 ABIN Animation Importer for Blender
Platform: PS3 (big-endian) / X360 (big-endian) - EA ANT AnimCodecs 1.01.00

VBR codec decoder verified against:
  - EA AnimCodecs vbrdecompressor.cpp / vbranimation.cpp source
  - Reverse-engineered structs from animtest.cpp
  - sk82_na_m.xex IDA decompilation (InitPerAnim / UnPackHeaderBits)
  - Direct byte-level analysis of OnBoard.abin

Supported:
  - Multiple animations per .abin (hundreds of type=2 clips)
  - VBR codec (0x00524256) bit-packed DCT
  - RAW codec (0x00004452) for static poses
  - Type 3 pose blocks, type 4 hierarchy, type 5 physics t-pose
"""

from __future__ import annotations

bl_info = {
    "name": "Skate 3 ABIN Importer",
    "version": (2, 3, 0),
    "blender": (3, 6, 0),
    "location": "File > Import > Skate 3 Animation (.abin)",
    "description": "Import Skate 3 PS3 .abin animation clips onto a selected armature",
    "category": "Import-Export",
}

__VERSION__ = "3.0.0-animtest-port"
print(f"[ABIN] abin_importer loaded, version {__VERSION__}")

import math
import struct
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple

try:
    import bpy
except ModuleNotFoundError:
    bpy = None


# ---------------------------------------------------------------------------
# Canonical Skate 3 bone names. Decoded frames are keyed by absolute
# skeleton bone index (engine sqtOffset); index i -> BONE_NAMES[i].
# ---------------------------------------------------------------------------

BONE_NAMES: List[str] = [
    "TRAJECTORY", "HIPS", "SPINE", "SPINE1", "SPINE2", "SPINE3",
    "NECK", "NECK1", "HEAD",
    "RIGHTSHOULDER", "RIGHTARM", "RIGHTFOREARM", "RIGHTHAND",
    "LEFTSHOULDER", "LEFTARM", "LEFTFOREARM", "LEFTHAND",
    "RIGHTUPLEG", "RIGHTLEG", "RIGHTFOOT", "RIGHTTOEBASE",
    "LEFTUPLEG", "LEFTLEG", "LEFTFOOT", "LEFTTOEBASE",
    "SKATEBOARD_ROOT",
    "TRUCK_FRONT", "RIGHT_WHEELFRONT", "LEFT_WHEELFRONT",
    "TRUCK_BACK", "LEFT_WHEELBACK", "RIGHT_WHEELBACK",
    "LEFTHANDTHUMB1", "LEFTHANDTHUMB2", "LEFTHANDTHUMB3",
    "LEFTHANDINDEX1", "LEFTHANDINDEX2", "LEFTHANDINDEX3",
    "LEFTHANDMIDDLE1", "LEFTHANDMIDDLE2", "LEFTHANDMIDDLE3",
    "LEFTINHANDRING", "LEFTHANDRING1", "LEFTHANDRING2", "LEFTHANDRING3",
    "LEFTINHANDPINKY", "LEFTHANDPINKY1", "LEFTHANDPINKY2", "LEFTHANDPINKY3",
    "RIGHTHANDTHUMB1", "RIGHTHANDTHUMB2", "RIGHTHANDTHUMB3",
    "RIGHTHANDINDEX1", "RIGHTHANDINDEX2", "RIGHTHANDINDEX3",
    "RIGHTHANDMIDDLE1", "RIGHTHANDMIDDLE2", "RIGHTHANDMIDDLE3",
    "RIGHTINHANDRING", "RIGHTHANDRING1", "RIGHTHANDRING2", "RIGHTHANDRING3",
    "RIGHTINHANDPINKY", "RIGHTHANDPINKY1", "RIGHTHANDPINKY2", "RIGHTHANDPINKY3",
    "RIGHTTOEBASE_REPARENTED", "LEFTTOEBASE_REPARENTED",
    "RIGHTHAND_REPARENTED", "LEFTHAND_REPARENTED",
    "RIGHTSHOULDERHLP", "RIGHTARMTWIST", "RIGHTFOREARMTWIST", "RIGHTFOREARMTWIST1",
    "LEFTSHOULDERHLP", "LEFTARMTWIST", "LEFTFOREARMTWIST", "LEFTFOREARMTWIST1",
    "RIGHTUPLEGHLP", "RIGHTUPLEGTWIST", "LEFTUPLEGHLP", "LEFTUPLEGTWIST",
    "FACE", "OFFSET_JAW", "JAW", "OFFSET_CHIN", "CHIN",
    "OFFSET_LOWERLIP", "OFFSET_LEFTLOWERLIP", "OFFSET_RIGHTLOWERLIP",
    "OFFSET_TONGUE", "OFFSET_LEFTCHEEK", "OFFSET_LEFTEYE", "OFFSET_LEFTMOUTH",
    "OFFSET_LEFTUPCHEEK", "OFFSET_LEFTUPPERLIP",
    "OFFSET_RIGHTCHEEK", "OFFSET_RIGHTEYE", "OFFSET_RIGHTMOUTH",
    "OFFSET_RIGHTUPCHEEK", "OFFSET_RIGHTUPPERLIP", "OFFSET_UPPERLIP",
    "TONGUE", "OFFSET_TONGUETIP",
    "LEFTLOWERLIP", "LOWERLIP", "RIGHTLOWERLIP", "TONGUETIP",
    "LEFTCHEEK", "LEFTCREASE", "LEFTEYE", "LEFTINNEREYEBROW",
    "LEFTLOWEYELID", "LEFTMOUTH", "LEFTNOSE", "LEFTOUTEREYEBROW",
    "LEFTUPCHEEK", "LEFTUPEYELID", "LEFTUPPERLIP",
    "RIGHTCHEEK", "RIGHTCREASE", "RIGHTEYE", "RIGHTINNEREYEBROW",
    "RIGHTLOWEYELID", "RIGHTMOUTH", "RIGHTNOSE", "RIGHTOUTEREYEBROW",
    "RIGHTUPCHEEK", "RIGHTUPEYELID", "RIGHTUPPERLIP", "UPPERLIP",
]
assert len(BONE_NAMES) == 131


# ---------------------------------------------------------------------------
# Binary helpers (big-endian)
# ---------------------------------------------------------------------------

def _u8(d, o):  return d[o]
def _u16(d, o): return struct.unpack_from(">H", d, o)[0]
def _u32(d, o): return struct.unpack_from(">I", d, o)[0]
def _i16(d, o): return struct.unpack_from(">h", d, o)[0]
def _i32(d, o): return struct.unpack_from(">i", d, o)[0]
def _f32(d, o): return struct.unpack_from(">f", d, o)[0]


# FastString36 (EA's 6-char-per-u32 base-38 encoding)
_FS_SEED    = 79235168
_FS_CHARSET = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_"

def _decode6(v: int) -> str:
    out, seed = [], _FS_SEED
    while v > 0 and seed > 0:
        idx = v // seed
        if idx == 0 or idx > 37:
            break
        out.append(_FS_CHARSET[idx - 1])
        v %= seed
        seed //= 38
    return "".join(out)

def _decode_name36(d: bytes, off: int) -> str:
    return "".join(_decode6(_u32(d, off + i * 4)) for i in range(6)).rstrip("\x00")


# ---------------------------------------------------------------------------
# Block / clip / part dataclasses
# ---------------------------------------------------------------------------

CODEC_VBR = 0x00524256   # "\x00RBV" big-endian = "VBR\0" little-endian
CODEC_RAW = 0x00004452   # "\x00\x00DR"
CODEC_TAG = 0x00000001   # type 4/5 marker codec


@dataclass
class BlockHeader:
    offset:  int
    size:    int
    type_id: int
    codec:   int
    name:    str


@dataclass
class SkelPart:
    """One skeleton per-part record (36 bytes, big-endian) at
    skel_base + 32 + 8*num_bones + idx*36:
      +0  u32 total_bone_count
      +4  s32 sqt_offset      (SQTData bone index this part writes to)
      +10 u16 bone_id         (engine matches RD partIDs against this; VBR
                               parts carry partID 0 so VBR uses positional)
    """
    bone_id:          int
    sqt_offset:       int
    total_bone_count: int


@dataclass
class HierarchyData:
    num_bones: int
    has_traj:  bool
    num_parts: int
    parents:   List[int]
    skel_base: int               # file offset of HierarchyData (= block + 0x30)
    parts:     List[SkelPart]    # per-part records, in skeleton order


@dataclass
class AnimPart:
    abs_offset:       int    # file offset of the 16-byte AnimationPart
    m_header:         int    # mHeader  (packed bits)
    csize:            int    # mCompressedDataSize
    comp_hdr_rel:     int    # mCompressionHeader (offset from abs_offset)
    comp_data_rel:    int    # mCompressedData    (offset from abs_offset)
    channel_count:    int    # ((mHeader >> 6) & 0x3F) + 1
    part_id:          int    # parts-table entry >> 24 (0 for VBR; RD carries id)
    table_index:     int    # positional index in the parts table


@dataclass
class AnimClip:
    header:     BlockHeader
    fps:        float
    num_frames: int
    parts:      List[AnimPart]
    base_off:   int


@dataclass
class Pose:
    header: BlockHeader
    parts:  List[AnimPart]


@dataclass
class SQT:
    scale: Tuple[float, float, float]        = (1.0, 1.0, 1.0)
    quat:  Tuple[float, float, float, float] = (0.0, 0.0, 0.0, 1.0)  # x,y,z,w
    trans: Tuple[float, float, float]        = (0.0, 0.0, 0.0)


# ---------------------------------------------------------------------------
# IDCT + quant tables (faithful port of VBRDecompressor::Init, animtest.cpp)
# ---------------------------------------------------------------------------

def _build_idct_coef_tables() -> List[List[float]]:
    """mIdctCoefTables[k][n] = cos((k + 0.5)*n*pi/8) * 0.25 ;  [k][0] *= 0.5.
    Row k = output sample (frame-in-block); DecompressFrameBlock uses
    idctCoef = table[frameinBlk]."""
    tbl: List[List[float]] = []
    for k in range(8):
        row = [math.cos(((k + 0.5) * n * math.pi) / 8.0) * 0.25 for n in range(8)]
        row[0] *= 0.5
        tbl.append(row)
    return tbl

_IDCT_COEF_TABLE = _build_idct_coef_tables()


def _build_initial_quant_tables() -> List[List[float]]:
    """mInitialQuantizeTables[ch][i] = (ln(i+2)/32768) * (1 + ch*0.2),
    ch in 0..255. The per-block quant table is then
      quant[i] = init[i]*scale**2 + scale*(1/32768)
    where scale = max(|dctMin|,|dctMax|) (rexglue runtime bias = 1/32768)."""
    base = [math.log(float(i + 2)) / 32768.0 for i in range(8)]
    # Init @0x82c1c78c accumulates the multiplier in float32 (v += 0.2f), it
    # does not compute 1.0 + ch*0.2. Replicate the accumulation and rounding.
    f32 = lambda x: struct.unpack("f", struct.pack("f", x))[0]
    step = f32(0.2)
    tables: List[List[float]] = []
    v31 = 1.0
    for _ in range(256):
        tables.append([f32(base[i] * v31) for i in range(8)])
        v31 = f32(v31 + step)
    return tables

_INITIAL_QUANT_TABLES = _build_initial_quant_tables()


# ---------------------------------------------------------------------------
# ABIN file parser
# ---------------------------------------------------------------------------

class AbinFile:
    """Parses an EA ANT .abin database (self-contained: one HT_HIERARCHY
    skeleton + many clips/poses authored against it).

    Outer: type=1 wrapper covering the file; inner block chain starts at
    0x30. Each block is a DataHeader (size@0, type@4, codec@8, name@0x10)
    followed by its payload. type 2 = animation clip, 3 = pose,
    4 = hierarchy, 5 = physics t-pose (skipped)."""

    def __init__(self, data: bytes) -> None:
        self.data = data
        self.hierarchy: Optional[HierarchyData] = None
        self.clips:     List[AnimClip]          = []
        self.poses:     List[Pose]              = []
        self._parse()

    def _parse(self) -> None:
        ptr = 0x30
        while ptr + 0x30 <= len(self.data):
            size = _u32(self.data, ptr)
            if size == 0 or size > len(self.data) - ptr:
                break
            type_id = _u32(self.data, ptr + 4)
            codec   = _u32(self.data, ptr + 8)
            name    = _decode_name36(self.data, ptr + 0x10)
            hdr = BlockHeader(ptr, size, type_id, codec, name)
            if type_id == 4:
                try:
                    self.hierarchy = self._parse_hierarchy(hdr)
                except Exception as e:
                    print(f"[ABIN] Hierarchy parse failed: {e}")
            elif type_id == 2:
                try:
                    clip = self._parse_clip(hdr)
                    if clip is not None:
                        self.clips.append(clip)
                except Exception as e:
                    print(f"[ABIN] Clip '{name}' parse failed: {e}")
            elif type_id == 3:
                try:
                    pose = self._parse_pose(hdr)
                    if pose is not None:
                        self.poses.append(pose)
                except Exception as e:
                    print(f"[ABIN] Pose '{name}' parse failed: {e}")
            # type_id 5 (phys t-pose) — skip
            ptr += size

    def _aligned_data(self, blk: int) -> int:
        """Engine's alignedData = (&hdr[1].mGUID + 3) & ~0xF.
        sizeof(DataHeader)=0x28, offsetof(mGUID)=0x0C -> blk + 0x34 + 3, then
        floor-aligned to 16. For 16-aligned blocks this equals blk + 0x30
        (matches loadAnimationDataBase's `mHierarchy + 0x30`)."""
        return (blk + 0x34 + 3) & ~0xF

    def _parse_hierarchy(self, hdr: BlockHeader) -> HierarchyData:
        skel = self._aligned_data(hdr.offset)
        num_bones = _u16(self.data, skel + 0)
        has_traj  = _u16(self.data, skel + 2) != 0
        num_parts = _u32(self.data, skel + 4)
        parents = [_i32(self.data, skel + 8 + i * 4) for i in range(num_bones)]

        # Per-part record table: base = skel + 32 + 8*num_bones, stride 36.
        rec_base = skel + 32 + 8 * num_bones
        parts: List[SkelPart] = []
        for p in range(num_parts):
            rec = rec_base + p * 36
            total = _u32(self.data, rec + 0)
            sqt   = _i32(self.data, rec + 4)
            bid   = _u16(self.data, rec + 10)
            parts.append(SkelPart(bid, sqt, total))
        return HierarchyData(num_bones, has_traj, num_parts, parents, skel, parts)

    def _read_parts(self, base: int, table_off: int,
                     max_entries: int) -> List[AnimPart]:
        """Parts table: each entry is a big-endian u32:
          high byte   = partID (RD carries a real id; VBR is always 0)
          low 24 bits = AnimationPart offset, relative to `base`(=alignedData)
        AnimationPart (16 bytes, BE): mHeader, mCompressedDataSize,
        mCompressionHeader, mCompressedData."""
        parts: List[AnimPart] = []

        # The table is not always zero-terminated. In retail RAW clips the
        # first AnimationPart can immediately follow the final table entry,
        # so reading a fixed 64 entries interprets part bytes as more entries.
        # The first entry gives us a hard upper bound: no table entry can
        # overlap the first part it points at.
        first_entry = _u32(self.data, base + table_off)
        first_part_off = first_entry & 0xFFFFFF
        if first_part_off > table_off:
            table_capacity = (first_part_off - table_off) // 4
            max_entries = min(max_entries, table_capacity)

        for i in range(max_entries):
            entry = _u32(self.data, base + table_off + i * 4)
            off   = entry & 0xFFFFFF
            if off == 0:
                break
            p_abs = base + off
            if p_abs + 0x10 > len(self.data):
                break
            m_hdr = _u32(self.data, p_abs)
            if (m_hdr & 0xFFFFF000) == 0:   # engine sanity: header sizing bits
                break
            csize    = _u32(self.data, p_abs + 4)
            hdr_rel  = _u32(self.data, p_abs + 8)
            data_rel = _u32(self.data, p_abs + 12)
            ch_count = ((m_hdr >> 6) & 0x3F) + 1
            parts.append(AnimPart(p_abs, m_hdr, csize, hdr_rel, data_rel,
                                  ch_count, entry >> 24, i))
        return parts

    def _parse_clip(self, hdr: BlockHeader) -> Optional[AnimClip]:
        ad = self._aligned_data(hdr.offset)
        fps      = _f32(self.data, ad + 0x20)
        frames_f = _f32(self.data, ad + 0x24)
        if not (1.0 <= fps <= 240.0) or not (1.0 <= frames_f <= 10000.0):
            return None
        num_frames = max(1, int(round(frames_f)))
        parts = self._read_parts(ad, table_off=0x38, max_entries=64)
        if not parts:
            return None
        return AnimClip(hdr, fps, num_frames, parts, ad)

    def _parse_pose(self, hdr: BlockHeader) -> Optional[Pose]:
        """Pose (type=3): alignedData[0] = u32 totalParts, then the parts
        table at alignedData + 4 (engine `isPose` layout)."""
        ad = self._aligned_data(hdr.offset)
        total_parts = _u32(self.data, ad + 0)
        cap = total_parts if 1 <= total_parts <= 64 else 64
        parts = self._read_parts(ad, table_off=0x04, max_entries=cap)
        if not parts:
            return None
        return Pose(hdr, parts)


# ---------------------------------------------------------------------------
# VBR data structures (disk layout)
# ---------------------------------------------------------------------------

@dataclass
class VBRChannelBlock:
    """One channel-info entry (16 bytes on disk, big-endian):
      +0  u16 NumEntPerFrm     (animated joints this channel)
      +2  u16 NumConstEntPerFrm (constant joints this channel)
      +4  f32 Min
      +8  f32 Max
      +12 u8  Size             (floats per joint: scale=3, quat=4, trans=3)
    """
    num_ent:   int
    num_const: int
    ch_min:    float
    ch_max:    float
    size:      int


@dataclass
class VBRPartInfo:
    """Parsed VBR part, mirroring AnimTest's InitPerAnim + UnPackHeaderBits."""
    num_frames:    int
    dct_min:       float
    dct_max:       float
    num_channels:  int
    palette_size:  int
    channels:      List[VBRChannelBlock]
    frame_block_sz: List[int]            # per-block compressed byte size
    # Derived (InitPerAnim)
    block_starts:  List[int]             # per-channel start in the frame row
    block_ends:    List[int]
    frame_offset:  int                   # sum(num_ent * size) = bit-decoder bones
    # UnPackHeaderBits output
    const_values:  List[float]           # flat, channel-major (numConst*size each)
    is_const:      List[int]             # one flag per joint, channel-major order
    num_bits_off:  int                   # bytes consumed (= ctrl-word table start)
    mem_hdr_size:  int                   # InitPerAnim return: ctrl table + framedata
    # Absolute file offsets
    comp_data_off: int                   # part + mCompressedData (compressed base)
    channel_info_off: int                # inline channel-info table
    palette_off:   int                   # inline constant palette
    frame_block_sz_off: int              # inline mFrameBlockSizes


def _compute_vbr_inline(data: bytes, hdr_off: int) -> Tuple[int, int]:
    """Port of ComputeVBRInline. Returns (palette_off, channel_info_off).
      palette_off      = hdr + 16
      channel_info_off = align16(palette + 4*paletteSize + 2*ceil(numFrames/8))
    The engine floor-aligns an absolute 16-aligned pointer, so aligning the
    file offset the same way is equivalent (blocks are 16-aligned)."""
    palette_size = data[hdr_off + 13]
    num_frames   = _u16(data, hdr_off + 2)
    palette_off  = hdr_off + 16
    palette_bytes = 4 * palette_size
    map_bytes     = 2 * ((num_frames + 7) // 8)
    ci = (palette_off + palette_bytes + map_bytes + 0xF) & ~0xF
    return palette_off, ci


def parse_vbr_part(data: bytes, part: AnimPart) -> VBRPartInfo:
    """Faithful port of InitPerAnim + UnPackHeaderBits + ComputeVBRInline.

    VBRDataHeaderValue (at part + mCompressionHeader), big-endian:
      +0  u16 mConstChanMapSize
      +2  u16 mNumFrames
      +4  f32 mDctMin
      +8  f32 mDctMax
      +12 u8  mNumChannels
      +13 u8  mConstantPaletteSize
    Inline after it: palette[paletteSize] f32, frame-block-size u16 table,
    16-byte-aligned channel-info table (16 bytes per channel).

    Compressed data (at part + mCompressedData) begins with the memory header
    consumed by UnPackHeaderBits: per-channel constant palette indices, then
    the constant-channel-map RLE runs.
    """
    hdr = part.abs_offset + part.comp_hdr_rel
    dat = part.abs_offset + part.comp_data_rel

    const_map_sz = _u16(data, hdr + 0)
    num_frames   = _u16(data, hdr + 2)
    dct_min      = _f32(data, hdr + 4)
    dct_max      = _f32(data, hdr + 8)
    num_ch       = data[hdr + 12]
    pal_size     = data[hdr + 13]

    palette_off, ci_off = _compute_vbr_inline(data, hdr)
    num_fblocks = (num_frames + 7) // 8
    fbs_off = hdr + 16 + pal_size * 4
    frame_block_sz = [_u16(data, fbs_off + i * 2) for i in range(num_fblocks)]

    channels: List[VBRChannelBlock] = []
    for i in range(num_ch):
        o = ci_off + i * 16
        channels.append(VBRChannelBlock(
            num_ent   = _u16(data, o + 0),
            num_const = _u16(data, o + 2),
            ch_min    = _f32(data, o + 4),
            ch_max    = _f32(data, o + 8),
            size      = data[o + 12],
        ))

    # InitPerAnim: per-channel block start/end (cumulative num_ent*size),
    # frame_offset = final end, capped to first 4 channels like the engine.
    block_starts: List[int] = []
    block_ends:   List[int] = []
    last_end = 0
    for i, c in enumerate(channels):
        if i >= 4:
            block_starts.append(last_end)
            block_ends.append(last_end)
            continue
        start = 0 if i == 0 else last_end
        last_end = start + c.num_ent * c.size
        block_starts.append(start)
        block_ends.append(last_end)
    frame_offset = last_end

    # UnPackHeaderBits: constant palette decode + constant-channel-map RLE.
    p = dat
    const_values: List[float] = []
    for c in channels:
        rng = c.ch_max - c.ch_min
        n = c.size * c.num_const
        for _ in range(n):
            idx = data[p]; p += 1
            sca = _f32(data, palette_off + idx * 4)
            const_values.append(sca * rng + c.ch_min)

    is_const: List[int] = []
    cur = 0
    for _ in range(const_map_sz):
        count = data[p]; p += 1
        if count:
            is_const.extend([cur] * count)
        cur ^= 1
    if const_map_sz == 0 and num_ch > 0:
        is_const.extend([0] * num_ch)

    num_bits_off = p - dat                       # bytes consumed
    mem_hdr_size = num_bits_off + ((frame_offset * 4) & 0x7FFFFFFC)

    return VBRPartInfo(
        num_frames=num_frames, dct_min=dct_min, dct_max=dct_max,
        num_channels=num_ch, palette_size=pal_size, channels=channels,
        frame_block_sz=frame_block_sz,
        block_starts=block_starts, block_ends=block_ends,
        frame_offset=frame_offset,
        const_values=const_values, is_const=is_const,
        num_bits_off=num_bits_off, mem_hdr_size=mem_hdr_size,
        comp_data_off=dat, channel_info_off=ci_off,
        palette_off=palette_off, frame_block_sz_off=fbs_off,
    )


def _vbr_decode_block(data: bytes, p_comp: int, ctrl_off: int,
                      bone_count: int) -> List[float]:
    """Port of Vector_UnPackFrameBlockAndDecompressOneFrame's bit decoder.

    Two independent little-endian bit streams:
      Stream A (selector): 32-bit reg from pComp[1..4]; refill 24 bits from
        pComp[5+1 ..] (selRefillPtr starts at pComp+5, reads [1..3] then +=3)
        when count <= 8 after each bone.
      Stream B (magnitude): 64-bit lazy buffer; starts at
        pComp + 1 + ceil(pComp[0]/8); refill while count <= 56.
    ctrl words: big-endian u32 at ctrl_off, one per bone, nibble k = bits
      [k*4 .. k*4+4).  nb==0 -> 0.  Else 1 selector bit; if 1: 1 sign bit
      then nb magnitude bits; value = float(mag), negative when sign bit == 0.
    Returns flat list of bone_count*8 floats (the IEEE values, sign applied)."""
    sel_buf = (data[p_comp + 1]
               | (data[p_comp + 2] << 8)
               | (data[p_comp + 3] << 16)
               | (data[p_comp + 4] << 24))
    sel_cnt = 32
    sel_ref = p_comp + 5

    sel_hdr_bits = data[p_comp + 0]
    mag_ptr = p_comp + 1 + ((sel_hdr_bits + 7) >> 3)
    mag_buf = 0
    mag_cnt = 0

    out: List[float] = [0.0] * (bone_count * 8)
    cp = ctrl_off
    for bone in range(bone_count):
        ctrl = ((data[cp] << 24) | (data[cp + 1] << 16)
                | (data[cp + 2] << 8) | data[cp + 3])
        cp += 4
        b8 = bone * 8
        for k in range(8):
            nb = (ctrl >> (k * 4)) & 0xF
            if nb == 0:
                continue
            sel = sel_buf & 1
            sel_buf >>= 1
            sel_cnt -= 1
            if sel == 1:
                while mag_cnt <= 56:
                    mag_buf |= data[mag_ptr] << mag_cnt
                    mag_ptr += 1
                    mag_cnt += 8
                sign = mag_buf & 1
                mag_buf >>= 1
                mag_cnt -= 1
                mag = mag_buf & ((1 << nb) - 1)
                mag_buf >>= nb
                mag_cnt -= nb
                fval = float(mag)
                if sign == 0:
                    fval = -fval
                out[b8 + k] = fval
        if sel_cnt <= 8:
            # Engine reads selPtr[0..2] (Vector_UnPackFrameBlockAndDecompress-
            # OneFrame @0x82c1e0f8). Reading [1..3] here skipped a byte per
            # refill and desynced the selector stream after the first 24 bits.
            chunk = (data[sel_ref + 0]
                     | (data[sel_ref + 1] << 8)
                     | (data[sel_ref + 2] << 16))
            sel_ref += 3
            sel_buf = (sel_buf | (chunk << sel_cnt)) & 0xFFFFFFFF
            sel_cnt += 24
    return out


class VBRPartDecoder:
    """Decodes one VBR part. decode_frame(f) returns List[SQT] (one per local
    bone in this part), reproducing AnimTest's DecompressFrame:
      Vector_UnPack (bit decode) -> DecompressFrameBlock (IDCT) -> PutData
      distribution via the constant-channel map."""

    def __init__(self, data: bytes, part: AnimPart):
        self.data = data
        self.part = part
        self.info: Optional[VBRPartInfo] = None
        self._block_cache: Dict[int, List[float]] = {}  # blockIdx -> scratch

    def _ensure(self) -> VBRPartInfo:
        if self.info is None:
            self.info = parse_vbr_part(self.data, self.part)
        return self.info

    def _scratch_for_block(self, info: VBRPartInfo, block_idx: int) -> List[float]:
        cached = self._block_cache.get(block_idx)
        if cached is not None:
            return cached
        data = self.data
        # perFrameStream = compData + memHeaderSize + sum(frameBlockSizes[<idx])
        stream = info.comp_data_off + info.mem_hdr_size
        for b in range(block_idx):
            if b < len(info.frame_block_sz):
                stream += info.frame_block_sz[b]
        # bit stream = perFrameStream + numChannels (first bytes = channel idx)
        p_comp = stream + info.num_channels
        ctrl_off = info.comp_data_off + info.num_bits_off
        scratch = _vbr_decode_block(data, p_comp, ctrl_off, info.frame_offset)
        self._block_cache[block_idx] = scratch
        return scratch

    def _channel_index(self, info: VBRPartInfo, block_idx: int,
                       ch: int) -> int:
        # perFrameStream[ch] = quant-table channelIndex (read by
        # DecompressFrameBlock as compressed[ch]).
        stream = info.comp_data_off + info.mem_hdr_size
        for b in range(block_idx):
            if b < len(info.frame_block_sz):
                stream += info.frame_block_sz[b]
        return self.data[stream + ch]

    def decode_frame(self, frame_idx: int) -> List[SQT]:
        info = self._ensure()
        nframes = max(1, info.num_frames)
        if frame_idx >= nframes:
            frame_idx = nframes - 1
        block_idx    = frame_idx >> 3
        frame_in_blk = frame_idx & 7

        # ---- DecompressFrameBlock IDCT -> per-channel animated values ----
        # normalized[e] for e in [start,end) per channel; e indexes scratch.
        frame_off_aln = (info.frame_offset + 3) & ~3
        normalized = [0.0] * (frame_off_aln if frame_off_aln else 1)

        if info.frame_offset > 0 and info.frame_block_sz:
            scratch = self._scratch_for_block(info, block_idx)
            scale = max(abs(info.dct_min), abs(info.dct_max))
            scale_sq = scale * scale
            quant_bias = scale * (1.0 / 32768.0)     # rexglue runtime bias
            idct_row = _IDCT_COEF_TABLE[frame_in_blk & 7]
            for ch, c in enumerate(info.channels):
                start = info.block_starts[ch]
                end   = info.block_ends[ch]
                if start == end:
                    continue
                ch_idx = self._channel_index(info, block_idx, ch) & 0xFF
                init = _INITIAL_QUANT_TABLES[ch_idx]
                quant = [init[i] * scale_sq + quant_bias for i in range(8)]
                coef  = [quant[i] * idct_row[i] for i in range(8)]
                ch_min = c.ch_min
                ch_rng = c.ch_max - c.ch_min
                for e in range(start, end):
                    base = e * 8
                    s = 0.0
                    for k in range(8):
                        s += scratch[base + k] * coef[k]
                    normalized[e] = (s + 0.5) * ch_rng + ch_min

        # ---- DecompressFrame distribution (PutData) ----
        # primary (anim) / fallback (const) / flag cursors advance GLOBALLY
        # across channels, exactly like the engine.
        chans = info.channels
        bone_count = 0
        for c in chans:
            bone_count = max(bone_count, c.num_ent + c.num_const)

        out = [SQT() for _ in range(bone_count)]
        anim_cur = 0          # float index into `normalized`
        const_cur = 0         # float index into info.const_values
        flag_cur = 0          # joint index into info.is_const
        flags = info.is_const
        consts = info.const_values

        for ch, c in enumerate(chans):
            total = c.num_ent + c.num_const
            if total == 0:
                continue
            size = c.size
            for j in range(total):
                is_c = flags[flag_cur] if flag_cur < len(flags) else (
                    1 if j >= c.num_ent else 0)
                flag_cur += 1
                if is_c:
                    vals = consts[const_cur:const_cur + size]
                    const_cur += size
                else:
                    vals = normalized[anim_cur:anim_cur + size]
                    anim_cur += size
                if len(vals) < size:
                    vals = list(vals) + [0.0] * (size - len(vals))
                if j < bone_count:
                    sqt = out[j]
                    if ch == 0:        # scale
                        out[j] = SQT((vals[0], vals[1], vals[2]),
                                     sqt.quat, sqt.trans)
                    elif ch == 1:      # quaternion (x,y,z,w)
                        out[j] = SQT(sqt.scale,
                                     (vals[0], vals[1], vals[2], vals[3]),
                                     sqt.trans)
                    elif ch == 2:      # translation
                        out[j] = SQT(sqt.scale, sqt.quat,
                                     (vals[0], vals[1], vals[2]))
        return out


# ---------------------------------------------------------------------------
# RAW (RD) codec — port of ExtractPackedN (animtest.cpp), with the fixes:
#   * NO `mNbBonesM1 == 0` early-exit (not in the engine)
#   * header == 0 -> identity-fill the bone group (engine behaviour)
#   * identity quat is (0,0,0,1), not (1,0,0,0)
# ---------------------------------------------------------------------------

@dataclass
class RawCompressionHeader:
    masks:        List[int]
    frame_size:   int
    nb_bones_m1:  int
    compressed_t: bool
    compressed_q: bool


def _raw_read_hdr(data: bytes, part: AnimPart) -> RawCompressionHeader:
    base = part.abs_offset + part.comp_hdr_rel
    masks = [_u32(data, base + i * 4) for i in range(6)]
    frame_size   = _u16(data, base + 24)
    nb_bones_m1  = data[base + 26]
    compressed_t = data[base + 27] != 0
    compressed_q = data[base + 28] != 0
    return RawCompressionHeader(masks, frame_size, nb_bones_m1,
                                compressed_t, compressed_q)


def _raw_channel_has(hdr: RawCompressionHeader, channel: int, kind: int) -> bool:
    word = channel >> 5
    bit = 1 << (channel & 31)
    return bool(hdr.masks[kind + word] & bit)


def _raw_quat_decompress(qcomp: int) -> Tuple[float, float, float, float]:
    pi = math.pi
    qa = (qcomp >> 21) & 0x7FF
    qb = (qcomp >> 11) & 0x3FF
    qc =  qcomp        & 0x7FF
    angle_a = (qa / 2047.0) * 2 * pi - pi
    sca     =  qb / 1023.0
    angle_b = (qc / 2047.0) * 2 * pi - pi
    x = math.sin(angle_b) * sca
    y = math.cos(angle_b) * sca
    z = math.sin(angle_a)
    w = math.cos(angle_a)
    n = math.sqrt(x*x + y*y + z*z + w*w)
    if n < 1e-8:
        return (0.0, 0.0, 0.0, 1.0)
    return (x/n, y/n, z/n, w/n)


def _raw_extract_frame(data: bytes, part: AnimPart, frame_idx: int) -> List[SQT]:
    """Port of ExtractPackedN. channelCount comes from the part's mHeader
    (((mHeader>>6)&0x3F)+1), NOT the compression header's bone count."""
    hdr = _raw_read_hdr(data, part)
    ch_count = ((part.m_header >> 6) & 0x3F) + 1
    base = part.abs_offset + part.comp_data_rel + frame_idx * hdr.frame_size
    out: List[SQT] = []
    p = base
    for ch in range(ch_count):
        scale = (1.0, 1.0, 1.0)
        quat  = (0.0, 0.0, 0.0, 1.0)
        trans = (0.0, 0.0, 0.0)
        if _raw_channel_has(hdr, ch, 0):
            scale = (_f32(data, p), _f32(data, p + 4), _f32(data, p + 8))
            p += 12
        if _raw_channel_has(hdr, ch, 2):
            if hdr.compressed_q:
                quat = _raw_quat_decompress(_u32(data, p)); p += 4
            else:
                quat = (_f32(data, p), _f32(data, p + 4),
                        _f32(data, p + 8), _f32(data, p + 12))
                p += 16
        if _raw_channel_has(hdr, ch, 4):
            if hdr.compressed_t:
                tx = struct.unpack_from(">h", data, p)[0]
                ty = struct.unpack_from(">h", data, p + 2)[0]
                tz = struct.unpack_from(">h", data, p + 4)[0]
                rr = _u16(data, p + 6); p += 8
                inv = float(rr) / 32768.0
                trans = (tx * inv, ty * inv, tz * inv)
            else:
                trans = (_f32(data, p), _f32(data, p + 4), _f32(data, p + 8))
                p += 12
        out.append(SQT(scale, quat, trans))
    return out


# ---------------------------------------------------------------------------
# Clip-level frame decode (dispatch RAW vs VBR; skeleton-positional placement)
# ---------------------------------------------------------------------------

def _is_vbr(hdr: BlockHeader) -> bool:
    return hdr.codec == CODEC_VBR or (hdr.codec & 0x00FFFFFF) == 0x00524256


def _is_raw(hdr: BlockHeader) -> bool:
    return hdr.codec == CODEC_RAW or (hdr.codec & 0x0000FFFF) == 0x4452


def _place_part_sqts(result: Dict[int, SQT], sqts: List[SQT],
                     part: AnimPart, hier: Optional[HierarchyData]) -> None:
    """Place a part's decoded bones at their skeleton bone indices.

    VBR/RD parts carry no usable partID (high byte 0 for VBR), so the i-th
    part maps positionally to the i-th skeleton part; that part's sqtOffset
    is the absolute SQTData bone index its bones start at (engine
    Vector_ExtractPackedNVBR: sqtBuff + 48*sqtOffset)."""
    if hier and 0 <= part.table_index < len(hier.parts):
        base = hier.parts[part.table_index].sqt_offset
    else:
        base = 0
    for j, sqt in enumerate(sqts):
        result[base + j] = sqt


def decode_clip_frame(data: bytes, clip: AnimClip,
                       decoders: Dict[int, VBRPartDecoder],
                       frame_idx: int,
                       hier: Optional[HierarchyData] = None) -> Dict[int, SQT]:
    """Decode one clip frame -> {absolute_bone_index: SQT}."""
    result: Dict[int, SQT] = {}
    vbr = _is_vbr(clip.header)
    for part in clip.parts:
        sqts: List[SQT] = []
        if part.csize > 0:
            if vbr:
                dec = decoders.get(part.abs_offset)
                if dec is None:
                    dec = VBRPartDecoder(data, part)
                    decoders[part.abs_offset] = dec
                try:
                    sqts = dec.decode_frame(frame_idx)
                except Exception as e:
                    print(f"[ABIN] VBR decode err (part 0x{part.abs_offset:X} "
                          f"frame {frame_idx}): {e}")
                    sqts = []
            else:
                try:
                    sqts = _raw_extract_frame(data, part, frame_idx)
                except Exception as e:
                    print(f"[ABIN] RAW decode err: {e}")
                    sqts = []
        _place_part_sqts(result, sqts, part, hier)
    return result


def decode_pose_frame(data: bytes, pose: Pose,
                      hier: Optional[HierarchyData] = None) -> Dict[int, SQT]:
    """Decode a pose (single-frame, all-constant VBR) -> {bone_index: SQT}."""
    result: Dict[int, SQT] = {}
    for part in pose.parts:
        if part.csize > 0:
            try:
                dec = VBRPartDecoder(data, part)
                sqts = dec.decode_frame(0)
            except Exception as e:
                print(f"[ABIN] Pose part decode err at "
                      f"0x{part.abs_offset:X}: {e}")
                sqts = []
            _place_part_sqts(result, sqts, part, hier)
    return result


def find_pose_by_name(abin: "AbinFile", name: str) -> Optional[Pose]:
    """Find the first pose block matching a name (case-insensitive)."""
    if not name:
        return None
    u = name.upper()
    for p in abin.poses:
        if p.header.name.upper() == u:
            return p
    return None


def find_default_base_pose(abin: "AbinFile") -> Optional[Pose]:
    """Pick the best non-TPOSE pose to use as base, or None."""
    priority = ["BOARD_BACKWARDS_IK", "BOARD_BACKWARDS",
                "POSTURE_BUFF_POSE", "POSTURE_STIFF_POSE",
                "POSTURE_SLOUCH_POSE", "RIG_TPOSE"]
    for name in priority:
        p = find_pose_by_name(abin, name)
        if p is not None:
            return p
    return abin.poses[0] if abin.poses else None


# ---------------------------------------------------------------------------
# Channel → bone name lookup
# ---------------------------------------------------------------------------

def build_channel_bone_name_map(hier: HierarchyData,
                                 bone_names: List[str]) -> Dict[int, str]:
    """Decoded frames are now keyed by ABSOLUTE skeleton bone index (the
    engine's SQTData index, via each part's skeleton sqtOffset). The skeleton
    is in canonical BONE_NAMES order, so the map is the identity index->name,
    bounded by the .abin's actual bone count."""
    mapping: Dict[int, str] = {}
    n = min(hier.num_bones, len(bone_names))
    for i in range(n):
        mapping[i] = bone_names[i]
    return mapping


# ---------------------------------------------------------------------------
# Blender application
# ---------------------------------------------------------------------------

if bpy is not None:
    import mathutils

    # Quaternion axis-convention table. EA ANT stores quats as (x,y,z,w).
    # Blender mathutils.Quaternion takes (w, x, y, z).
    # Depending on how rw4 skel importer built bones (bone-length axis,
    # bone roll), the EA ANT → Blender remap differs.
    QUAT_CONVENTIONS = [
        ("DIRECT",    "Direct (no swap)",         lambda q: (q[3],  q[0],  q[1],  q[2])),
        ("XY_SWAP",   "Swap X<->Y, negate Y",     lambda q: (q[3], -q[1],  q[0],  q[2])),
        ("XY_SWAP2",  "Swap X<->Y, negate X",     lambda q: (q[3],  q[1], -q[0],  q[2])),
        ("NEG_Y",     "Negate Y",                 lambda q: (q[3],  q[0], -q[1],  q[2])),
        ("NEG_X",     "Negate X",                 lambda q: (q[3], -q[0],  q[1],  q[2])),
        ("NEG_Z",     "Negate Z",                 lambda q: (q[3],  q[0],  q[1], -q[2])),
        ("YZ_SWAP",   "Swap Y<->Z",               lambda q: (q[3],  q[0],  q[2],  q[1])),
        ("XZ_SWAP",   "Swap X<->Z",               lambda q: (q[3],  q[2],  q[1],  q[0])),
    ]
    QUAT_ITEMS = [(k, desc, desc) for k, desc, _ in QUAT_CONVENTIONS]
    QUAT_FUNCS = {k: f for k, _, f in QUAT_CONVENTIONS}

    TRANS_CONVENTIONS = [
        ("DIRECT",    "Direct (no swap)",    lambda t: (t[0],  t[1],  t[2])),
        ("XY_SWAP",   "Swap X<->Y, neg Y",   lambda t: (-t[1], t[0],  t[2])),
        ("XZ_SWAP",   "Swap X<->Z",          lambda t: (t[2],  t[1],  t[0])),
        ("NEG_Y",     "Negate Y",            lambda t: (t[0], -t[1],  t[2])),
        ("NEG_Z",     "Negate Z",            lambda t: (t[0],  t[1], -t[2])),
        ("YZ_SWAP",   "Swap Y<->Z",          lambda t: (t[0],  t[2],  t[1])),
    ]
    TRANS_ITEMS = [(k, desc, desc) for k, desc, _ in TRANS_CONVENTIONS]
    TRANS_FUNCS = {k: f for k, _, f in TRANS_CONVENTIONS}

    # Bone name substrings that identify "finger/face" bones that may
    # need a different quat convention than body bones.
    _FINGER_TOKENS = (
        "HAND", "THUMB", "INDEX", "MIDDLE", "RING", "PINKY",
        "FINGER", "FACE", "JAW", "LIP", "EYE", "MOUTH", "TONGUE",
        "BROW", "CHEEK", "NOSE", "CHIN", "CREASE", "LID",
    )

    def _is_finger_bone(name: str) -> bool:
        u = name.upper()
        return any(tok in u for tok in _FINGER_TOKENS)

    def _apply_clip_to_armature(
        arm_obj, data: bytes, clip: AnimClip, hier: HierarchyData,
        action_name: str, quat_conv: str = "XZ_SWAP", trans_conv: str = "XZ_SWAP",
        finger_quat_conv: str = "XZ_SWAP",
        push_to_nla: bool = True,
        base_pose_data: Optional[Dict[int, SQT]] = None,
        compose_base: bool = False,
    ) -> None:
        if arm_obj.animation_data is None:
            arm_obj.animation_data_create()

        action = bpy.data.actions.new(name=action_name)
        arm_obj.animation_data.action = action

        pb_map = {pb.name.upper(): pb for pb in arm_obj.pose.bones}
        ch_to_name = build_channel_bone_name_map(hier, BONE_NAMES)
        ch_to_pb = {ch: pb_map[name.upper()]
                    for ch, name in ch_to_name.items()
                    if name.upper() in pb_map}

        print(f"[ABIN] '{action_name}': {clip.num_frames} frames, "
              f"{len(ch_to_pb)}/{len(ch_to_name)} bones matched")

        qfn_body   = QUAT_FUNCS[quat_conv]
        qfn_finger = QUAT_FUNCS[finger_quat_conv]
        tfn = TRANS_FUNCS[trans_conv]

        ch_qfn: Dict[int, callable] = {}
        for ch, name in ch_to_name.items():
            ch_qfn[ch] = qfn_finger if _is_finger_bone(name) else qfn_body

        base_q: Dict[int, "mathutils.Quaternion"] = {}
        base_t: Dict[int, "mathutils.Vector"] = {}
        if compose_base and base_pose_data:
            for ch, sqt in base_pose_data.items():
                qfn = ch_qfn.get(ch, qfn_body)
                bw, bx, by, bz = qfn(sqt.quat)
                base_q[ch] = mathutils.Quaternion((bw, bx, by, bz))
                base_t[ch] = mathutils.Vector(tfn(sqt.trans))

        for pb in ch_to_pb.values():
            pb.rotation_mode = "QUATERNION"

        vbr_decoders: Dict[int, VBRPartDecoder] = {}
        # Previous-frame quaternion per channel, for sign continuity.
        # Blender linearly interpolates keyframes — if adjacent quats have
        # dot(q_prev, q_curr) < 0, interpolation passes through origin and
        # the skeleton explodes.  Negate q_curr when that happens.
        prev_q: Dict[int, "mathutils.Quaternion"] = {}

        for fi in range(clip.num_frames):
            frame_data = decode_clip_frame(data, clip, vbr_decoders, fi, hier)

            for ch, sqt in frame_data.items():
                pb = ch_to_pb.get(ch)
                if pb is None:
                    continue
                qfn = ch_qfn.get(ch, qfn_body)
                qw, qx, qy, qz = qfn(sqt.quat)
                q = mathutils.Quaternion((qw, qx, qy, qz))
                t = mathutils.Vector(tfn(sqt.trans))

                if compose_base and ch in base_q:
                    q = base_q[ch] @ q
                    t = base_t[ch] + t

                # Sign continuity: keep dot(prev, curr) >= 0
                p = prev_q.get(ch)
                if p is not None:
                    if p.w * q.w + p.x * q.x + p.y * q.y + p.z * q.z < 0.0:
                        q = mathutils.Quaternion((-q.w, -q.x, -q.y, -q.z))
                prev_q[ch] = q

                pb.rotation_quaternion = q
                pb.location            = t
                pb.scale               = sqt.scale

                pb.keyframe_insert("location",            frame=fi + 1)
                pb.keyframe_insert("rotation_quaternion", frame=fi + 1)
                pb.keyframe_insert("scale",               frame=fi + 1)

        if push_to_nla:
            track = arm_obj.animation_data.nla_tracks.new()
            track.name = action_name
            track.strips.new(action_name, 1, action)
            arm_obj.animation_data.action = None

        print(f"[ABIN] '{action_name}' done.")


    def _apply_pose_to_armature(
        arm_obj, data: bytes, pose: Pose, hier: HierarchyData,
        quat_conv: str = "XY_SWAP", trans_conv: str = "XY_SWAP",
    ) -> None:
        pb_map = {pb.name.upper(): pb for pb in arm_obj.pose.bones}
        ch_to_name = build_channel_bone_name_map(hier, BONE_NAMES)
        ch_to_pb = {ch: pb_map[name.upper()]
                    for ch, name in ch_to_name.items()
                    if name.upper() in pb_map}

        qfn = QUAT_FUNCS[quat_conv]
        tfn = TRANS_FUNCS[trans_conv]

        pose_sqts = decode_pose_frame(data, pose, hier)
        for ch, sqt in pose_sqts.items():
            pb = ch_to_pb.get(ch)
            if pb is None:
                continue
            pb.rotation_mode = "QUATERNION"
            qw, qx, qy, qz = qfn(sqt.quat)
            pb.rotation_quaternion = mathutils.Quaternion((qw, qx, qy, qz))
            pb.location            = mathutils.Vector(tfn(sqt.trans))
            pb.scale               = sqt.scale
        print(f"[ABIN] Applied pose '{pose.header.name}'")


    class IMPORT_OT_skate3_abin(bpy.types.Operator):
        bl_idname  = "import_anim.skate3_abin"
        bl_label   = "Import Skate 3 Animation (.abin)"
        bl_options = {"REGISTER", "UNDO"}

        filepath: bpy.props.StringProperty(subtype="FILE_PATH")
        filter_glob: bpy.props.StringProperty(default="*.abin", options={"HIDDEN"})

        clip_index: bpy.props.IntProperty(
            name="Clip Index",
            description="Clip to import (-1 = all, 0 = first, N = Nth)",
            default=-1, min=-1,
        )
        max_clips: bpy.props.IntProperty(
            name="Max Clips (when all)",
            description="Cap total clips when clip_index=-1 (0 = no cap). "
                        "Some .abin have thousands of clips; capping "
                        "prevents multi-hour imports. Default 50.",
            default=50, min=0,
        )
        debug: bpy.props.BoolProperty(name="Print Debug", default=True)
        world_up_correction: bpy.props.BoolProperty(
            name="Fix Y-up to Z-up (rotate armature +90° X)",
            default=False,
        )
        apply_base_pose: bpy.props.BoolProperty(
            name="Apply Base Pose",
            description="Set the selected pose as the character's base stance "
                        "(what the game uses before animations play).",
            default=True,
        )
        base_pose_name: bpy.props.StringProperty(
            name="Base Pose Name",
            description="Name of pose to use as base. Common values: "
                        "RIG_TPOSE, BOARD_BACKWARDS, BOARD_BACKWARDS_IK, "
                        "POSTURE_BUFF_POSE. Leave blank for first non-TPOSE.",
            default="BOARD_BACKWARDS_IK",
        )
        compose_base_with_anim: bpy.props.BoolProperty(
            name="Compose Base Onto Animation",
            description="Multiply each animation frame's rotation by the "
                        "base pose rotation. Needed if animations are stored "
                        "as deltas from the base pose.",
            default=False,
        )
        push_to_nla: bpy.props.BoolProperty(
            name="Push Each Clip to NLA",
            description="Each clip becomes an NLA strip so multi-clip "
                        "imports don't overwrite the active action.",
            default=True,
        )
        quat_conv: bpy.props.EnumProperty(
            name="Quat Axis Convention",
            description="Axis swap for body bones. "
                        "XZ_SWAP confirmed correct for skate 3 Skel.blend.",
            items=QUAT_ITEMS, default="XZ_SWAP",
        )
        trans_conv: bpy.props.EnumProperty(
            name="Trans Axis Convention",
            description="Axis swap for translations.",
            items=TRANS_ITEMS, default="XZ_SWAP",
        )
        finger_quat_conv: bpy.props.EnumProperty(
            name="Finger Quat Convention",
            description="Axis swap for hand/finger/face bones. Fingers "
                        "have different bind-pose roll from body bones, so "
                        "they often need a different swap than body.",
            items=QUAT_ITEMS, default="XZ_SWAP",
        )

        def execute(self, context):
            arm_obj = context.object
            if arm_obj is None or arm_obj.type != "ARMATURE":
                self.report({"ERROR"}, "Select an armature first.")
                return {"CANCELLED"}
            try:
                data = open(self.filepath, "rb").read()
            except OSError as e:
                self.report({"ERROR"}, f"Read failed: {e}")
                return {"CANCELLED"}

            abin = AbinFile(data)

            if self.debug:
                print(f"[ABIN] File: {self.filepath}")
                print(f"[ABIN] Clips: {len(abin.clips)}  Poses: {len(abin.poses)}  "
                      f"Hier: {'yes' if abin.hierarchy else 'no'}")
                if abin.hierarchy:
                    h = abin.hierarchy
                    print(f"[ABIN] Hier: {h.num_bones} bones, "
                          f"has_traj={h.has_traj}, parts={h.num_parts}")
                for i, c in enumerate(abin.clips[:10]):
                    active = sum(1 for p in c.parts if p.csize > 0)
                    codec = "VBR" if _is_vbr(c.header) else ("RAW" if _is_raw(c.header) else "???")
                    print(f"[ABIN] Clip[{i}]: '{c.header.name}' {codec} "
                          f"fps={c.fps} frames={c.num_frames} parts={len(c.parts)} active={active}")

            if not abin.clips:
                self.report({"ERROR"}, "No animation clips found.")
                return {"CANCELLED"}
            if abin.hierarchy is None:
                self.report({"ERROR"}, "No hierarchy block.")
                return {"CANCELLED"}

            # World-up rotation applied ONCE to armature object (not per clip).
            if self.world_up_correction:
                arm_obj.rotation_mode  = "XYZ"
                arm_obj.rotation_euler = (math.pi / 2, 0.0, 0.0)

            # Select base pose.  Prefer user-named pose, fall back to
            # first non-TPOSE, final fallback to first pose.
            base_pose: Optional[Pose] = None
            if self.apply_base_pose and abin.poses:
                if self.base_pose_name.strip():
                    base_pose = find_pose_by_name(abin, self.base_pose_name)
                    if base_pose is None and self.debug:
                        print(f"[ABIN] Pose '{self.base_pose_name}' not found, "
                              f"available: {sorted({p.header.name for p in abin.poses})}")
                if base_pose is None:
                    base_pose = find_default_base_pose(abin)
                if base_pose is not None:
                    print(f"[ABIN] Using base pose: '{base_pose.header.name}'")
                    try:
                        _apply_pose_to_armature(
                            arm_obj, data, base_pose, abin.hierarchy,
                            quat_conv=self.quat_conv, trans_conv=self.trans_conv,
                        )
                    except Exception as e:
                        if self.debug:
                            print(f"[ABIN] Pose apply failed: {e}")

            # Decode base pose once for composition use
            base_pose_data: Optional[Dict[int, SQT]] = None
            if self.compose_base_with_anim and base_pose is not None:
                try:
                    base_pose_data = decode_pose_frame(data, base_pose, abin.hierarchy)
                except Exception as e:
                    if self.debug:
                        print(f"[ABIN] Base pose decode err: {e}")

            if self.clip_index == -1:
                clips_to_import = abin.clips
                if self.max_clips > 0:
                    clips_to_import = clips_to_import[:self.max_clips]
            else:
                idx = max(0, min(self.clip_index, len(abin.clips) - 1))
                clips_to_import = [abin.clips[idx]]

            # Use clip[0]'s fps/frames for scene settings.
            if clips_to_import:
                c0 = clips_to_import[0]
                scene = bpy.context.scene
                scene.render.fps = max(1, int(round(c0.fps)))
                scene.frame_start = 1
                scene.frame_end   = max(c0.num_frames, scene.frame_end)

            imported = 0
            for idx, clip in enumerate(clips_to_import):
                if not (_is_vbr(clip.header) or _is_raw(clip.header)):
                    continue
                name = clip.header.name or f"abin_clip_{idx}"
                try:
                    _apply_clip_to_armature(
                        arm_obj, data, clip, abin.hierarchy,
                        action_name=name,
                        quat_conv=self.quat_conv,
                        trans_conv=self.trans_conv,
                        finger_quat_conv=self.finger_quat_conv,
                        push_to_nla=self.push_to_nla and len(clips_to_import) > 1,
                        base_pose_data=base_pose_data,
                        compose_base=self.compose_base_with_anim,
                    )
                    imported += 1
                except Exception as e:
                    print(f"[ABIN] Failed to import '{name}': {e}")

            self.report({"INFO"}, f"Imported {imported}/{len(clips_to_import)} clip(s).")
            return {"FINISHED"}

        def invoke(self, context, event):
            context.window_manager.fileselect_add(self)
            return {"RUNNING_MODAL"}


    def _menu_func(self, context):
        self.layout.operator(
            IMPORT_OT_skate3_abin.bl_idname,
            text="Skate 3 Animation (.abin)",
        )


    def register():
        bpy.utils.register_class(IMPORT_OT_skate3_abin)
        bpy.types.TOPBAR_MT_file_import.append(_menu_func)


    def unregister():
        bpy.types.TOPBAR_MT_file_import.remove(_menu_func)
        bpy.utils.unregister_class(IMPORT_OT_skate3_abin)


    if __name__ == "__main__":
        register()


# ---------------------------------------------------------------------------
# Standalone test harness (run outside Blender)
# ---------------------------------------------------------------------------

else:
    def _standalone_test(filepath: str) -> None:
        data = open(filepath, "rb").read()
        abin = AbinFile(data)
        print(f"File: {filepath} ({len(data):,} bytes)")
        if abin.hierarchy:
            h = abin.hierarchy
            print(f"Hierarchy: {h.num_bones} bones has_traj={h.has_traj} parts={h.num_parts}")
        print(f"Clips: {len(abin.clips)}   Poses: {len(abin.poses)}")
        for i, clip in enumerate(abin.clips[:5]):
            codec = "VBR" if _is_vbr(clip.header) else ("RAW" if _is_raw(clip.header) else "???")
            print(f"\nClip[{i}] '{clip.header.name}' {codec} fps={clip.fps} frames={clip.num_frames} parts={len(clip.parts)}")
            for pi, part in enumerate(clip.parts):
                print(f"  Part[{pi}] mHdr=0x{part.m_header:08X} cds=0x{part.csize:X} "
                      f"hdr=0x{part.comp_hdr_rel:X} data=0x{part.comp_data_rel:X} ch={part.channel_count}")
                if _is_vbr(clip.header) and part.csize > 0:
                    try:
                        info = parse_vbr_part(data, part)
                        total_ch = sum(c.num_ent + c.num_const
                                       for c in info.channels)
                        print(f"    VBR: NF={info.num_frames} "
                              f"Dct=[{info.dct_min:.3f},{info.dct_max:.3f}] "
                              f"nch={info.num_channels} pal={info.palette_size} "
                              f"FO={info.frame_offset} total_ch={total_ch}")
                        for bi, b in enumerate(info.channels):
                            print(f"      blk[{bi}]: ent={b.num_ent} const={b.num_const} "
                                  f"range=[{b.ch_min:.3f},{b.ch_max:.3f}] size={b.size}")
                        print(f"      FBS: {info.frame_block_sz} (sum={sum(info.frame_block_sz)})")
                        print(f"      mem_hdr={info.mem_hdr_size} bytes "
                              f"(const+chanmap={info.num_bits_off}, "
                              f"nibbles={4*info.frame_offset})")
                    except Exception as e:
                        print(f"    VBR parse err: {e}")
            # Decode a few frames
            if clip.num_frames > 0 and _is_vbr(clip.header):
                decoders: Dict[int, VBRPartDecoder] = {}
                for fi in [0, clip.num_frames // 2, clip.num_frames - 1]:
                    try:
                        frame = decode_clip_frame(data, clip, decoders, fi, abin.hierarchy)
                        print(f"  Frame {fi}: {len(frame)} channels")
                        for ch in list(frame.keys())[:3]:
                            s = frame[ch]
                            print(f"    ch {ch}: s=({s.scale[0]:.3f},{s.scale[1]:.3f},{s.scale[2]:.3f}) "
                                  f"q=({s.quat[0]:.3f},{s.quat[1]:.3f},{s.quat[2]:.3f},{s.quat[3]:.3f}) "
                                  f"t=({s.trans[0]:.3f},{s.trans[1]:.3f},{s.trans[2]:.3f})")
                    except Exception as e:
                        print(f"  Frame {fi} err: {e}")

    if __name__ == "__main__":
        import sys
        if len(sys.argv) < 2:
            print("Usage: python abin_importer.py <file.abin>")
            sys.exit(1)
        _standalone_test(sys.argv[1])
