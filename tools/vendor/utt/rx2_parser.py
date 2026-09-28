"""Standalone parser for EA Skate 1/2/3 RenderWare4 .rx2 files (Xbox 360).

Pure-Python reimplementation of the Xbox 360 RX2 reading logic from the
Noesis plugin mdl_EA_SKATE_4_7_0.py (Beedy / GHFear / tuukkas). It does not
require Noesis or the plugin.

Scope: the .rx2 container (Xbox 360). The PS3 .psg container is not covered.

What it can do:
  * read the RX2 header (file count, file table, data base, container type)
  * walk the 24-byte file table (TOC) records
  * find and decode texture entries (DXT1 / DXT3 / DXT5 / ATI2 /
    A8R8G8B8 / B5G6R5 / A8) into plain RGBA8888 byte buffers, including the
    Xbox 360 GPU tiled-memory layout (XGAddress2DTiled swizzle)

Typical use:

    import rx2_parser
    rx2 = rx2_parser.parse_rx2("some_file.rx2")
    for tex in rx2.textures:
        print(tex)          # e.g. "tex0 | 512x512 | DXT1"
        tex.save_png("tex%d.png" % tex.index)

Run `python rx2_parser.py file.rx2 --toc --out-dir out` for a CLI dump.
See rx2parserexample.txt for the file format notes.
"""

import struct
from pathlib import Path

MAGIC_X360 = b"\x89RW4xb2"

TYPE_SIM = 0x00000001
TYPE_MESH = 0x00000004
TYPE_TEXTURE = 0x00001000
TYPE_NHL_LEGACY_MESH = 0x00000800

TYPE_NAMES = {
    TYPE_SIM: "Sim / collision",
    TYPE_MESH: "3D model",
    TYPE_TEXTURE: "Texture",
    TYPE_NHL_LEGACY_MESH: "3D model (NHL legacy)",
}

TOC_TEXTURE_SKATE = 0x000200E8
TOC_TEXTURE_NHL = 0x00020003
TOC_MESH_INFO = 0x000200E9
TOC_VERTICES = 0x000200EA
TOC_FACES = 0x000200EB
TOC_MATERIALS = 0x00EB0005
TOC_BONES = 0x00EB0001
TOC_SIM = 0x00EB000B

TOC_TYPE_NAMES = {
    TOC_TEXTURE_SKATE: "texture (Skate)",
    TOC_TEXTURE_NHL: "texture (NHL)",
    TOC_MESH_INFO: "mesh info",
    TOC_VERTICES: "vertices",
    TOC_FACES: "faces",
    TOC_MATERIALS: "materials",
    TOC_BONES: "bones",
    TOC_SIM: "sim",
}

FMT_DXT1 = 0x52
FMT_DXT3 = 0x53
FMT_DXT5 = 0x54
FMT_ATI2 = 0x71
FMT_DXT1_NORMAL = 0x7C
FMT_A8R8G8B8 = 0x86
FMT_B5G6R5 = 0x44
FMT_A8 = 0x02

FMT_NAMES = {
    FMT_DXT1: "DXT1",
    FMT_DXT3: "DXT3",
    FMT_DXT5: "DXT5",
    FMT_ATI2: "ATI2",
    FMT_DXT1_NORMAL: "DXT1 normal",
    FMT_A8R8G8B8: "A8R8G8B8",
    FMT_B5G6R5: "B5G6R5",
    FMT_A8: "A8",
}


class RX2ParseError(Exception):
    pass


class TOCEntry(object):
    """One 24-byte record from the RX2 file table.

    Records come in pairs: the record directly before a type record carries
    the payload pointer/size for that type. Concretely, when record N's type
    matches:
        data offset (relative to the header-size value at 0x44) = record N-1 f0
        data size                                           = record N-1 f2
        info block offset (absolute)                        = record N   f0
    """

    __slots__ = ("index", "f0", "f1", "f2", "f3", "f4", "type_id",
                 "data_offset", "buffer_size", "info_offset", "is_texture")

    def __init__(self, index, f0, f1, f2, f3, f4, type_id):
        self.index = index
        self.f0 = f0
        self.f1 = f1
        self.f2 = f2
        self.f3 = f3
        self.f4 = f4
        self.type_id = type_id
        self.data_offset = None
        self.buffer_size = None
        self.info_offset = f0
        self.is_texture = type_id in (TOC_TEXTURE_SKATE, TOC_TEXTURE_NHL)

    @property
    def type_name(self):
        return TOC_TYPE_NAMES.get(self.type_id, "unknown")

    def __repr__(self):
        return ("TOCEntry(%d) type=0x%08X (%s) data_off=%s size=%s info_off=%s"
                % (self.index, self.type_id, self.type_name,
                   "0x%X" % self.data_offset if self.data_offset is not None else "-",
                   self.buffer_size if self.buffer_size is not None else "-",
                   "0x%X" % self.info_offset if self.info_offset is not None else "-"))


class Texture(object):
    """A decoded texture: width x height RGBA8888 pixels in `rgba`."""

    __slots__ = ("index", "width", "height", "fmt_id", "rgba",
                 "data_offset", "buffer_size", "cube_faces", "face_stride")

    def __init__(self, index, width, height, fmt_id, rgba, data_offset, buffer_size):
        self.index = index
        self.width = width
        self.height = height
        self.fmt_id = fmt_id
        self.rgba = rgba
        self.data_offset = data_offset
        self.buffer_size = buffer_size
        self.cube_faces = 1
        self.face_stride = 0

    @property
    def name(self):
        return "tex%d" % self.index

    @property
    def fmt_name(self):
        return FMT_NAMES.get(self.fmt_id, "0x%02X" % self.fmt_id)

    @property
    def size(self):
        return (self.width, self.height)

    def to_pil(self):
        """Return a Pillow RGBA image (requires Pillow)."""
        from PIL import Image
        return Image.frombytes("RGBA", self.size, self.rgba)

    def save_png(self, path):
        """Save the texture as a PNG (requires Pillow)."""
        self.to_pil().save(path, "PNG", compress_level=1)

    def __str__(self):
        return "%s | %dx%d | %s" % (self.name, self.width, self.height, self.fmt_name)


class RX2File(object):
    """A parsed RX2 container: header fields, TOC entries, decoded textures."""

    def __init__(self, data, path=None):
        if isinstance(data, (bytes, bytearray)):
            self.data = bytes(data)
        else:
            self.data = Path(data).read_bytes()
            path = str(data)
        self.path = path
        self.file_count = 0
        self.file_table_offset = 0
        self.data_base = 0
        self.type_id = 0
        self.entries = []
        self.textures = []
        self.warnings = []

    @property
    def magic(self):
        return self.data[0:7]

    @property
    def type_name(self):
        return TYPE_NAMES.get(self.type_id, "unknown (0x%08X)" % self.type_id)

    def parse(self):
        """Read the header and the file table. Idempotent."""
        data = self.data
        if len(data) < 0x5C:
            raise RX2ParseError("file is too small to be an RX2")
        if data[0:7] != MAGIC_X360:
            raise RX2ParseError("not an Xbox 360 RX2 (bad magic 0x%s)" % data[0:7].hex())

        self.file_count = int.from_bytes(data[0x20:0x24], "big")
        self.file_table_offset = int.from_bytes(data[0x30:0x34], "big")
        self.data_base = int.from_bytes(data[0x44:0x48], "big")
        self.type_id = int.from_bytes(data[0x58:0x5C], "big")

        if self.file_count > (1 << 20):
            raise RX2ParseError("implausible file count: %d" % self.file_count)
        if self.file_table_offset + self.file_count * 24 > len(data):
            raise RX2ParseError("file table runs past the end of the file")

        self.entries = []
        prev_f0 = None
        prev_f2 = None
        for i in range(self.file_count):
            p = self.file_table_offset + i * 24
            f0, f1, f2, f3, f4 = struct.unpack_from(">iiiii", data, p)
            type_id = int.from_bytes(data[p + 20:p + 24], "big")
            entry = TOCEntry(i, f0, f1, f2, f3, f4, type_id)
            if i > 0:
                entry.data_offset = prev_f0
                entry.buffer_size = prev_f2
            self.entries.append(entry)
            prev_f0, prev_f2 = f0, f2
        return self

    def decode_textures(self):
        """Decode every texture entry found in the TOC. Idempotent."""
        self.textures = []
        for entry in self.entries:
            if entry.is_texture:
                tex = self._decode_texture(entry)
                if tex is not None:
                    self.textures.append(tex)
        return self.textures

    def _decode_texture(self, entry):
        data = self.data
        hdr_off = entry.info_offset
        if hdr_off is None or hdr_off + 40 > len(data):
            self.warnings.append("texture entry %d: bad info offset" % entry.index)
            return None
        hdr = data[hdr_off:hdr_off + 40]
        fmt = hdr[35]
        size = (hdr[36] << 24) | (hdr[37] << 16) | (hdr[38] << 8) | hdr[39]
        width = (size & 0x1FFF) + 1
        height = ((size >> 13) & 0x1FFF) + 1
        dxt5_variant = hdr[28]
        base = self.data_base + entry.data_offset
        size = entry.buffer_size or 0
        if width <= 1 and height <= 8:
            genrx2 = self._genrx2_geometry(hdr)
            if genrx2 is None:
                self.warnings.append(
                    "texture entry %d: bad dimensions %dx%d" % (entry.index, width, height)
                )
                return None
            width, height, fmt, dxt5_variant = genrx2
        raw = data[base:base + size]
        try:
            rgba = _decode_texture_data(raw, width, height, fmt, dxt5_variant)
        except Exception as exc:
            self.warnings.append("texture entry %d: decode failed: %s" % (entry.index, exc))
            return None
        # Xbox fetch word 5, bits 9..10: kCube=3. A base mip contains
        # six consecutive tiled face slices. Keep their native face/row order
        # in a vertical strip; the map writer marks this as a cube source.
        cube = hdr_off + 52 <= len(data) and (int.from_bytes(data[hdr_off+48:hdr_off+52], 'big') >> 9) & 3 == 3
        stride = 0
        if cube:
            if width != height or fmt not in (FMT_DXT1, FMT_A8R8G8B8, FMT_B5G6R5):
                raise RX2ParseError('Unsupported cube texture shape/format: %dx%d fmt=%x fetch=%s' % (width,height,fmt,data[hdr_off+28:hdr_off+52].hex()))
            block = 4 if fmt == FMT_DXT1 else 1
            pitch = 8 if fmt == FMT_DXT1 else 2 if fmt == FMT_B5G6R5 else 4
            cols = ((width + block - 1)//block + 31) & ~31
            rows = ((height + block - 1)//block + 31) & ~31
            stride = (cols * rows * pitch + 4095) & ~4095
            if len(raw) < stride * 6:
                raise RX2ParseError('Truncated cube base faces')
            rgba = b''.join(_decode_texture_data(raw[face*stride:(face+1)*stride], width, height, fmt, dxt5_variant) for face in range(6))
            height *= 6
        result = Texture(entry.index, width, height, fmt, rgba, base, size)
        result.cube_faces = 6 if cube else 1
        result.face_stride = stride
        return result

    @staticmethod
    def _genrx2_geometry(hdr):
        """Recover dimensions for RX2 files written by the genrx2 converter.

        The tool writes a header variant that omits the D3D-style fields the
        game exporter uses. Its two 32-bit fields encode, empirically:

            [28:32] = 64 * (mip_count - 1)          -> max_dim = 1 << (value / 64)
            [32:36] = 0xA00 + max(0x4000, max_dim^2) -> padding + tiled size

        genrx2 always emits byte-swapped DXT5 with the linear (until-360-tiled)
        layout, which the parser selects with variant 84 (the game's plain
        DXT5 value). The aspect ratio is not stored; square textures decode
        exactly, non-square ones decode upscaled to the square extent.
        """
        mip_field = int.from_bytes(hdr[28:32], "big")
        size_field = int.from_bytes(hdr[32:36], "big")
        if mip_field <= 0 or mip_field % 64 != 0:
            return None
        max_dim = 1 << (mip_field // 64)
        if max_dim > 8192:
            return None
        expected = 0xA00 + max(0x4000, max_dim * max_dim)
        if size_field != expected:
            return None
        return max_dim, max_dim, FMT_DXT5, 84

    def __str__(self):
        return ("RX2File(%s)\n"
                "  type: %s\n"
                "  file count: %d\n"
                "  file table: 0x%X\n"
                "  data base: 0x%X\n"
                "  TOC entries: %d\n"
                "  textures: %d"
                % (self.path, self.type_name, self.file_count,
                   self.file_table_offset, self.data_base,
                   len(self.entries), len(self.textures)))


def parse_rx2(source):
    """Parse an RX2 (path or raw bytes) and decode every texture in it."""
    rx2 = RX2File(source)
    rx2.parse()
    rx2.decode_textures()
    return rx2


def patch_texture_entry(data, width, height):
    """Repair the texture entry of a converted RX2 so third-party readers
    (the Noesis plugin) see the real format and dimensions.

    genrx2 writes the payload and the TOC record, but leaves the game-layout
    fields of the 40-byte texture info block at zero (format byte at +35,
    height byte at +37 and the 16-bit width at +38), so external tools report
    a 1x8 image with an unhandled format. This restores those fields using
    the encoding the Noesis plugin reads, and trims the advertised buffer
    size (previous TOC record field 2) to the actual payload length so
    readers never read past the end of the file.

    Returns a new byte string; the input is not modified.
    """
    data = bytearray(data)
    rx2 = RX2File(bytes(data))
    rx2.parse()
    entry = next((e for e in rx2.entries if e.is_texture), None)
    if entry is None:
        raise RX2ParseError("no texture entry to patch")
    if entry.info_offset is None or entry.info_offset + 40 > len(data):
        raise RX2ParseError("texture info block runs past the end of the file")
    if not 8 <= height <= 2048 or not 1 <= width <= 8192:
        raise RX2ParseError("unsupported texture size %dx%d" % (width, height))
    p = entry.info_offset
    data[p + 35] = FMT_DXT5
    data[p + 36:p + 40] = struct.pack(">I", ((height - 1) << 13) | (width - 1))
    if entry.buffer_size is not None:
        payload = len(data) - rx2.data_base
        if payload >= 0:
            f2_pos = rx2.file_table_offset + (entry.index - 1) * 24 + 8
            data[f2_pos:f2_pos + 4] = struct.pack(">I", payload)
    return bytes(data)


# ---------------------------------------------------------------------------
# Byte order / tiling helpers (mirror the Noesis plugin's behaviour)
# ---------------------------------------------------------------------------

def _swap16(data):
    """Byte-swap every 16-bit word (X360 DXT/RGB565 storage order)."""
    b = bytearray(data)
    for i in range(0, len(b) - 1, 2):
        b[i], b[i + 1] = b[i + 1], b[i]
    return bytes(b)


def _x360_tiled_x(offset, width_units, texel_pitch):
    """XGAddress2DTiledX: swizzled column of a tiled 2D surface (X360 GPU)."""
    aligned_width = (width_units + 31) & ~31
    log_bpp = (texel_pitch >> 2) + ((texel_pitch >> 1) >> (texel_pitch >> 2))
    off_b = offset << log_bpp
    off_t = ((off_b & ~4095) >> 3) + ((off_b & 1792) >> 2) + (off_b & 63)
    off_m = off_t >> (7 + log_bpp)
    macro_x = (off_m % (aligned_width >> 5)) << 2
    tile = (((off_t >> (5 + log_bpp)) & 2) + (off_b >> 6)) & 3
    macro = (macro_x + tile) << 3
    micro = ((((off_t >> 1) & ~15) + (off_t & 15)) & ((texel_pitch << 3) - 1)) >> log_bpp
    return macro + micro


def _x360_tiled_y(offset, width_units, texel_pitch):
    """XGAddress2DTiledY: swizzled row of a tiled 2D surface (X360 GPU)."""
    aligned_width = (width_units + 31) & ~31
    log_bpp = (texel_pitch >> 2) + ((texel_pitch >> 1) >> (texel_pitch >> 2))
    off_b = offset << log_bpp
    off_t = ((off_b & ~4095) >> 3) + ((off_b & 1792) >> 2) + (off_b & 63)
    off_m = off_t >> (7 + log_bpp)
    macro_y = (off_m // (aligned_width >> 5)) << 2
    tile = ((off_t >> (6 + log_bpp)) & 1) + ((off_b & 2048) >> 10)
    macro = (macro_y + tile) << 3
    micro = ((((off_t & (((texel_pitch << 6) - 1) & ~31)) + ((off_t & 15) << 1)) >> (3 + log_bpp)) & ~1)
    return macro + micro + ((off_t & 16) >> 4)


def _untile360(src, width_units, texel_pitch):
    """Reverse the X360 GPU tiled-memory layout (XGAddress2DTiled swizzle).

    ``width_units`` is the surface width in units (4x4-pixel blocks for DXT
    formats, texels for raw formats) and ``texel_pitch`` is the size of one
    unit in bytes (8 or 16 for DXT, bytes-per-pixel for raw).
    """
    dst = bytearray(len(src))
    row_bytes = width_units * texel_pitch
    units = len(src) // texel_pitch
    for j in range(units // width_units):
        for i in range(width_units):
            x = _x360_tiled_x(j * width_units + i, width_units, texel_pitch)
            y = _x360_tiled_y(j * width_units + i, width_units, texel_pitch)
            dst_idx = (y * width_units + x) * texel_pitch
            src_idx = (j * width_units + i) * texel_pitch
            if x < width_units and dst_idx + texel_pitch <= len(dst) and src_idx + texel_pitch <= len(src):
                dst[dst_idx:dst_idx + texel_pitch] = src[src_idx:src_idx + texel_pitch]
    return bytes(dst)


def _untile360_dxt(src, tex_w, tex_h, blk_size):
    """Reverse the X360 GPU tiling for DXT-style formats (units = 4x4 blocks)."""
    return _untile360(src, (tex_w + 3) >> 2, blk_size)


def _untile360_raw(src, tex_w, tex_h, bpp):
    """Reverse the X360 GPU tiling for raw pixel formats (units = texels)."""
    return _untile360(src, tex_w, bpp)


# ---------------------------------------------------------------------------
# DXT / raw decoders (all produce RGBA8888 row-major bytes)
# ---------------------------------------------------------------------------

def _rgb565(value):
    """Expand a 16-bit 565 value to 888 (v*255/denom, truncated).

    Verified against Noesis's imageDecodeDXT output: truncation, not
    D3DX-style rounding (rounding adds a constant +1 offset to ~1 in 10
    pixels vs the game's decoder)."""
    return ((((value >> 11) & 0x1F) * 255) // 31,
            (((value >> 5) & 0x3F) * 255) // 63,
            (((value & 0x1F) * 255) // 31))


def _rgb565_interp(a5, b5, a6, b6, a1, b1):
    """The two interpolated palette colours, computed from the 888 values:
    (2a+b+1)/3 resp. (a+2b+1)/3, integer rounding (matches Noesis)."""
    r0 = (a5 * 255) // 31
    r1 = (b5 * 255) // 31
    g0 = (a6 * 255) // 63
    g1 = (b6 * 255) // 63
    b0 = (a1 * 255) // 31
    b1v = (b1 * 255) // 31
    return ((2 * r0 + r1 + 1) // 3, (2 * g0 + g1 + 1) // 3, (2 * b0 + b1v + 1) // 3), \
           ((r0 + 2 * r1 + 1) // 3, (g0 + 2 * g1 + 1) // 3, (b0 + 2 * b1v + 1) // 3)


def _dxt1_block(blk, transparent):
    c0, c1 = struct.unpack_from(">HH", blk, 0)
    r0, g0, b0 = _rgb565(c0)
    r1, g1, b1 = _rgb565(c1)
    # DXT3/DXT5 have independent alpha and always use four colour entries.
    if c0 > c1 or not transparent:
        c2, c3 = _rgb565_interp((c0 >> 11) & 0x1F, (c1 >> 11) & 0x1F,
                                (c0 >> 5) & 0x3F, (c1 >> 5) & 0x3F,
                                c0 & 0x1F, c1 & 0x1F)
        palette = [(r0, g0, b0), (r1, g1, b1), c2, c3]
        alpha3 = 255
    else:
        palette = [(r0, g0, b0), (r1, g1, b1),
                   ((r0 + r1) // 2, (g0 + g1) // 2, (b0 + b1) // 2),
                   (0, 0, 0)]
        alpha3 = 0 if transparent else 255
    out = bytearray(64)
    for y in range(4):
        idx_byte = blk[4 + (y ^ 1)]
        for x in range(4):
            i = (idx_byte >> (x * 2)) & 3
            o = (y * 4 + x) * 4
            r, g, b = palette[i]
            out[o] = r
            out[o + 1] = g
            out[o + 2] = b
            out[o + 3] = alpha3 if i == 3 else 255
    return bytes(out)


def _dxt3_block(blk):
    color = _dxt1_block(blk[8:16], transparent=False)
    out = bytearray(64)
    for p in range(16):
        # alpha nibbles are word-swapped like the DXT5 alpha bits
        a = (blk[(p // 2) ^ 1] >> (4 * (p & 1))) & 0xF
        o = p * 4
        out[o] = color[o]
        out[o + 1] = color[o + 1]
        out[o + 2] = color[o + 2]
        out[o + 3] = a * 17
    return bytes(out)


def _alpha_table(a0, a1):
    if a0 > a1:
        return [a0, a1] + [((6 - i) * a0 + (i + 1) * a1) // 7 for i in range(6)]
    return [a0, a1,
            (4 * a0 + a1) // 5, (3 * a0 + 2 * a1) // 5,
            (2 * a0 + 3 * a1) // 5, (a0 + 4 * a1) // 5, 0, 255]


def _dxt5_block(blk):
    # the alpha endpoint bytes and the 48 index bits are 16-bit-word-swapped
    # (same swapEndianArray(data, 2) the Noesis plugin applies before decode)
    table = _alpha_table(blk[1], blk[0])
    bits = int.from_bytes(_swap16(blk[2:8]), "little")
    color = _dxt1_block(blk[8:16], transparent=False)
    out = bytearray(64)
    for p in range(16):
        o = p * 4
        out[o] = color[o]
        out[o + 1] = color[o + 1]
        out[o + 2] = color[o + 2]
        out[o + 3] = table[(bits >> (3 * p)) & 7]
    return bytes(out)


def _ati2_channel(blk, off):
    table = _alpha_table(blk[off + 1], blk[off])
    bits = int.from_bytes(_swap16(blk[off + 2:off + 8]), "little")
    return [table[(bits >> (3 * p)) & 7] for p in range(16)]


def _ati2_block(blk):
    r = _ati2_channel(blk, 0)
    g = _ati2_channel(blk, 8)
    out = bytearray(64)
    for p in range(16):
        o = p * 4
        out[o] = r[p]
        out[o + 1] = g[p]
        out[o + 2] = 0
        out[o + 3] = 255
    return bytes(out)


def _decode_blocks(data, width, height, block_size, block_fn):
    bw = (width + 3) >> 2
    bh = (height + 3) >> 2
    out = bytearray(width * height * 4)
    for by in range(bh):
        for bx in range(bw):
            off = (by * bw + bx) * block_size
            blk = data[off:off + block_size]
            if len(blk) < block_size:
                continue
            px = block_fn(blk)
            x0 = bx * 4
            y0 = by * 4
            for yy in range(4):
                row = y0 + yy
                if row >= height:
                    break
                base = (row * width + x0) * 4
                if x0 + 4 <= width:
                    out[base:base + 16] = px[yy * 16:(yy + 1) * 16]
                else:
                    for xx in range(width - x0):
                        s = (yy * 4 + xx) * 4
                        d = base + xx * 4
                        out[d:d + 4] = px[s:s + 4]
    return bytes(out)


def decode_dxt1(data, width, height):
    return _decode_blocks(_untile360_dxt(data, width, height, 8),
                          width, height, 8, lambda b: _dxt1_block(b, True))


def decode_dxt1_normal(data, width, height):
    rgba = decode_dxt1(data, width, height)
    out = bytearray(rgba)
    for i in range(0, len(rgba), 4):
        r = out[i] / 255.0
        g = out[i + 1] / 255.0
        z2 = 1.0 - r * r - g * g
        out[i + 2] = int(round((z2 ** 0.5 if z2 > 0 else 0.0) * 255))
        out[i + 3] = 255
    return bytes(out)


def decode_dxt3(data, width, height):
    return _decode_blocks(_untile360_dxt(data, width, height, 16),
                          width, height, 16, _dxt3_block)


def decode_dxt5(data, width, height, tiled=True):
    blocks = _untile360_dxt(data, width, height, 16) if tiled else data
    return _decode_blocks(blocks, width, height, 16, _dxt5_block)


def decode_ati2(data, width, height):
    return _decode_blocks(_untile360_dxt(data, width, height, 16),
                          width, height, 16, _ati2_block)


def _decode_raw_a8r8g8b8(data, width, height):
    out = bytearray(width * height * 4)
    n = min(len(data), width * height * 4) // 4 * 4
    for i in range(0, n, 4):
        o = i
        out[o] = data[i + 1]
        out[o + 1] = data[i + 2]
        out[o + 2] = data[i + 3]
        out[o + 3] = data[i]
    return bytes(out)


def _decode_raw_b5g6r5(data, width, height):
    out = bytearray(width * height * 4)
    n = min(len(data), width * height * 2) // 2 * 2
    for i in range(0, n, 2):
        v = (data[i] << 8) | data[i + 1]
        o = (i // 2) * 4
        out[o] = ((v >> 11) & 0x1F) * 255 // 31
        out[o + 1] = ((v >> 5) & 0x3F) * 255 // 63
        out[o + 2] = (v & 0x1F) * 255 // 31
        out[o + 3] = 255
    return bytes(out)


def _decode_raw_a8(data, width, height):
    out = bytearray(width * height * 4)
    n = min(len(data), width * height)
    for i in range(n):
        o = i * 4
        v = data[i]
        out[o] = v
        out[o + 1] = v
        out[o + 2] = v
        out[o + 3] = v
    return bytes(out)


def _rgb565_pack(r, g, b):
    """Pack 8-bit RGB into a 565 word (top of the range)."""
    return ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)


def _encode_dxt5_block(px):
    """Encode 16 RGBA pixels into one 16-byte X360 DXT5 block.

    The Xbox 360 stores DXT blocks with the alpha part first and every
    16-bit word byte-swapped; ``_dxt5_block`` / ``_dxt1_block`` decode the
    same layout, so encode -> decode round-trips exactly.

    ``px`` is a list of 16 ``(r, g, b, a)`` tuples in row-major order.
    """
    alphas = [p[3] for p in px]
    a_min = min(alphas)
    a_max = max(alphas)
    if a_max > a_min:
        a0 = a_max
        a1 = a_min
        table = [a0, a1] + [((6 - i) * a0 + (i + 1) * a1) // 7 for i in range(6)]
        bits = 0
        for p in range(16):
            value = alphas[p]
            best = 0
            best_d = abs(table[0] - value)
            for i in range(1, 8):
                d = abs(table[i] - value)
                if d < best_d:
                    best_d = d
                    best = i
            bits |= best << (3 * p)
        aidx = _swap16(bits.to_bytes(6, "little"))
    else:
        a0 = a1 = alphas[0]
        aidx = b"\x00" * 6

    lo = (min(p[0] for p in px), min(p[1] for p in px), min(p[2] for p in px))
    hi = (max(p[0] for p in px), max(p[1] for p in px), max(p[2] for p in px))
    c0 = _rgb565_pack(*hi)
    c1 = _rgb565_pack(*lo)
    r0, g0, b0 = _rgb565(c0)
    r1, g1, b1 = _rgb565(c1)
    c2, c3 = _rgb565_interp((c0 >> 11) & 0x1F, (c1 >> 11) & 0x1F,
                            (c0 >> 5) & 0x3F, (c1 >> 5) & 0x3F,
                            c0 & 0x1F, c1 & 0x1F)
    palette = [(r0, g0, b0), (r1, g1, b1), c2, c3]

    blk = bytearray(16)
    blk[0] = a1
    blk[1] = a0
    blk[2:8] = aidx
    blk[8:10] = struct.pack(">H", c0)
    blk[10:12] = struct.pack(">H", c1)
    for y in range(4):
        byte = 0
        for x in range(4):
            r, g, b = px[y * 4 + x][:3]
            best = 0
            best_d = (palette[0][0] - r) ** 2 + (palette[0][1] - g) ** 2 + (palette[0][2] - b) ** 2
            for i in range(1, 4):
                pr, pg, pb = palette[i]
                d = (pr - r) ** 2 + (pg - g) ** 2 + (pb - b) ** 2
                if d < best_d:
                    best_d = d
                    best = i
            byte |= best << (x * 2)
        blk[12 + (y ^ 1)] = byte
    return bytes(blk)


def _tile360(src, width_units, texel_pitch):
    """Apply the X360 GPU tiled-memory layout (inverse of ``_untile360``)."""
    dst = bytearray(len(src))
    units = len(src) // texel_pitch
    for j in range(units // width_units):
        for i in range(width_units):
            x = _x360_tiled_x(j * width_units + i, width_units, texel_pitch)
            y = _x360_tiled_y(j * width_units + i, width_units, texel_pitch)
            src_idx = (y * width_units + x) * texel_pitch
            dst_idx = (j * width_units + i) * texel_pitch
            if src_idx + texel_pitch <= len(src) and dst_idx + texel_pitch <= len(dst):
                dst[dst_idx:dst_idx + texel_pitch] = src[src_idx:src_idx + texel_pitch]
    return bytes(dst)


def _encode_dxt5_np(rgba, width, height):
    """numpy-accelerated ``encode_dxt5`` (vectorised block encoder)."""
    import numpy as np

    arr = np.frombuffer(rgba, dtype=np.uint8).reshape(height, width, 4)
    bh = (height + 3) >> 2
    bw = (width + 3) >> 2
    if height % 4 or width % 4:
        pad = np.zeros((bh * 4, bw * 4, 4), dtype=np.uint8)
        pad[:height, :width] = arr
        arr = pad
    blk = arr.reshape(bh, 4, bw, 4, 4).transpose(0, 2, 1, 3, 4)
    blk = blk.reshape(bh * bw, 16, 4)

    alpha = blk[:, :, 3].astype(np.int16)
    a_max = alpha.max(axis=1)
    a_min = alpha.min(axis=1)
    same = a_max == a_min
    a0 = np.where(same, alpha[:, 0], a_max)
    a1 = np.where(same, alpha[:, 0], a_min)
    idx6 = np.arange(6, dtype=np.int16)
    t2 = ((6 - idx6) * a_max[:, None] + (idx6 + 1) * a_min[:, None]) // 7
    table = np.concatenate([a_max[:, None], a_min[:, None], t2], axis=1)
    best = np.argmin(np.abs(table[:, None, :] - alpha[:, :, None]), axis=2)
    bits = (best.astype(np.int64) << np.arange(16, dtype=np.int64)[None, :] * 3).sum(axis=1)
    aidx_le = ((bits[:, None] >> (8 * np.arange(6, dtype=np.int64))[None, :]) & 0xFF).astype(np.uint8)
    aidx = aidx_le[:, [1, 0, 3, 2, 5, 4]]

    rgb = blk[:, :, :3].astype(np.int32)
    hi = rgb.max(axis=1)
    lo = rgb.min(axis=1)
    c0 = ((hi[:, 0] >> 3) << 11) | ((hi[:, 1] >> 2) << 5) | (hi[:, 2] >> 3)
    c1 = ((lo[:, 0] >> 3) << 11) | ((lo[:, 1] >> 2) << 5) | (lo[:, 2] >> 3)
    r0 = ((c0 >> 11) & 0x1F) * 255 // 31
    g0 = ((c0 >> 5) & 0x3F) * 255 // 63
    b0 = (c0 & 0x1F) * 255 // 31
    r1 = ((c1 >> 11) & 0x1F) * 255 // 31
    g1 = ((c1 >> 5) & 0x3F) * 255 // 63
    b1 = (c1 & 0x1F) * 255 // 31
    pal0 = np.stack([r0, g0, b0], axis=1)
    pal1 = np.stack([r1, g1, b1], axis=1)
    pal2 = (2 * pal0 + pal1 + 1) // 3
    pal3 = (pal0 + 2 * pal1 + 1) // 3
    palette = np.stack([pal0, pal1, pal2, pal3], axis=1)
    dist = ((palette[:, None, :, :] - rgb[:, :, None, :]) ** 2).sum(axis=-1)
    cidx = np.argmin(dist, axis=2).reshape(-1, 4, 4)
    row_bits = (cidx << np.array([0, 2, 4, 6], dtype=np.uint8)[None, None, :]).sum(axis=2)
    row_bytes = row_bits[:, [1, 0, 3, 2]].astype(np.uint8)

    out = np.zeros((bh * bw, 16), dtype=np.uint8)
    out[:, 0] = a1.astype(np.uint8)
    out[:, 1] = a0.astype(np.uint8)
    out[:, 2:8] = aidx
    out[:, 8] = ((c0 >> 8) & 0xFF).astype(np.uint8)
    out[:, 9] = (c0 & 0xFF).astype(np.uint8)
    out[:, 10] = ((c1 >> 8) & 0xFF).astype(np.uint8)
    out[:, 11] = (c1 & 0xFF).astype(np.uint8)
    out[:, 12:16] = row_bytes

    units = bh * bw
    k = np.arange(units, dtype=np.int64)
    pitch = 16
    log_bpp = (pitch >> 2) + ((pitch >> 1) >> (pitch >> 2))
    aligned_width = (bw + 31) & ~31
    off_b = k << log_bpp
    off_t = ((off_b & ~4095) >> 3) + ((off_b & 1792) >> 2) + (off_b & 63)
    off_m = off_t >> (7 + log_bpp)
    macro_x = (off_m % (aligned_width >> 5)) << 2
    tile_x = (((off_t >> (5 + log_bpp)) & 2) + (off_b >> 6)) & 3
    micro_x = ((((off_t >> 1) & ~15) + (off_t & 15)) & ((pitch << 3) - 1)) >> log_bpp
    xs = ((macro_x + tile_x) << 3) + micro_x
    macro_y = (off_m // (aligned_width >> 5)) << 2
    tile_y = ((off_t >> (6 + log_bpp)) & 1) + ((off_b & 2048) >> 10)
    micro_y = ((((off_t & (((pitch << 6) - 1) & ~31)) + ((off_t & 15) << 1)) >> (3 + log_bpp)) & ~1)
    ys = ((macro_y + tile_y) << 3) + micro_y + ((off_t & 16) >> 4)
    perm = ys * bw + xs
    valid = perm < units
    tiled = np.zeros((units, 16), dtype=np.uint8)
    tiled[valid] = out[perm[valid]]
    return tiled.tobytes()


def encode_dxt5(rgba, width, height):
    """Encode an RGBA8888 byte buffer to a tiled DXT5 mip (X360 layout).

    ``rgba`` must be exactly ``width * height * 4`` bytes, row-major.
    Returns the tiled DXT5 byte buffer, ``bw * bh * 16`` bytes where
    ``bw = (width + 3) >> 2`` etc.
    """
    try:
        return _encode_dxt5_np(rgba, width, height)
    except ImportError:
        pass

    bw = (width + 3) >> 2
    bh = (height + 3) >> 2
    linear = bytearray(bw * bh * 16)
    for by in range(bh):
        for bx in range(bw):
            px = []
            for yy in range(4):
                y = by * 4 + yy
                row = y * width * 4 if y < height else -1
                for xx in range(4):
                    x = bx * 4 + xx
                    if row >= 0 and x < width:
                        o = row + x * 4
                        px.append((rgba[o], rgba[o + 1], rgba[o + 2], rgba[o + 3]))
                    else:
                        px.append((0, 0, 0, 0))
            blk = _encode_dxt5_block(px)
            off = (by * bw + bx) * 16
            linear[off:off + 16] = blk
    return _tile360(bytes(linear), bw, 16)


def _box_downsample(rgba, w, h, w2, h2):
    """Average 2x2 pixel blocks into a smaller RGBA buffer."""
    try:
        import numpy as _np
        arr = _np.frombuffer(rgba, dtype=_np.uint8).reshape(h, w, 4)
        if h & 1:
            arr = _np.concatenate([arr, arr[-1:]], axis=0)
        if w & 1:
            arr = _np.concatenate([arr, arr[:, -1:]], axis=1)
        quad = (arr[0:h2 * 2:2, 0:w2 * 2:2].astype(_np.uint16)
                + arr[0:h2 * 2:2, 1:w2 * 2 + 1:2]
                + arr[1:h2 * 2 + 1:2, 0:w2 * 2:2]
                + arr[1:h2 * 2 + 1:2, 1:w2 * 2 + 1:2])
        return (quad >> 2).astype(_np.uint8).tobytes()
    except ImportError:
        pass

    out = bytearray(w2 * h2 * 4)
    for y in range(h2):
        y0 = y * 2
        y1 = min(y0 + 1, h - 1)
        for x in range(w2):
            x0 = x * 2
            x1 = min(x0 + 1, w - 1)
            total = [0, 0, 0, 0]
            for yy in (y0, y1):
                r1 = yy * w * 4
                for xx in (x0, x1):
                    o = r1 + xx * 4
                    total[0] += rgba[o]
                    total[1] += rgba[o + 1]
                    total[2] += rgba[o + 2]
                    total[3] += rgba[o + 3]
            o = (y * w2 + x) * 4
            for c in range(4):
                out[o + c] = total[c] >> 2
    return bytes(out)


def encode_rx2_texture(template, width, height, rgba, min_mip=128):
    """Build a complete RX2 file from an RGBA image (pure Python encoder).

    ``template`` is the byte content of a known-good RX2 container; its
    header and file table are reused verbatim and the payload is replaced
    with a freshly encoded DXT5 mip chain. ``rgba`` is ``width * height * 4``
    row-major RGBA bytes (apply any opacity to the alpha channel before
    calling). Mips are generated down to ``min_mip`` (the X360 swizzle is
    only exact at widths of 32 block units, i.e. 128px).

    Returns the complete RX2 file as bytes.
    """
    rx2 = RX2File(template)
    rx2.parse()
    entry = next((e for e in rx2.entries if e.is_texture), None)
    if entry is None:
        raise RX2ParseError("the container template has no texture entry")
    if entry.info_offset is None or entry.info_offset + 40 > rx2.data_base:
        raise RX2ParseError("the container template stores its texture info "
                            "outside the header region")
    if len(rgba) < width * height * 4:
        raise RX2ParseError("RGBA buffer is too small for %dx%d" % (width, height))
    if (width & (width - 1)) or (height & (height - 1)):
        raise RX2ParseError("only square power-of-two textures are supported")
    if width != height:
        raise RX2ParseError("only square textures are supported")
    if not 16 <= width <= 4096:
        raise RX2ParseError("unsupported texture size %dx%d" % (width, height))

    mips = []
    w, h = width, height
    cur = rgba
    while True:
        mips.append((w, h, cur))
        if max(w, h) <= min_mip:
            break
        w2, h2 = max(1, w >> 1), max(1, h >> 1)
        cur = _box_downsample(cur, w, h, w2, h2)
        w, h = w2, h2

    chain = b"".join(encode_dxt5(m, w, h) for (w, h, m) in mips)

    out = bytearray(template[:rx2.data_base])
    out += chain
    p = entry.info_offset
    out[p + 28:p + 32] = struct.pack(">I", 64 * (len(mips) - 1))
    out[p + 32:p + 36] = struct.pack(">I", 0xA00 + max(0x4000, width * width))
    out[p + 35] = FMT_DXT5
    # The 32-bit size field packs width:13 = w-1 (bits 0-12) and
    # height:13 = h-1 (bits 13-25); the noesis plugin reads its byte 37 as
    # (h-1)>>3 and the width H with the (h-1)&7 bits masked off, which both
    # fall out of this single dword.
    out[p + 36:p + 40] = struct.pack(">I", ((height - 1) << 13) | (width - 1))
    f2_pos = rx2.file_table_offset + (entry.index - 1) * 24 + 8
    out[f2_pos:f2_pos + 4] = struct.pack(">I", len(chain))
    return bytes(out)


def _decode_texture_data(raw, width, height, fmt, dxt5_variant):
    if fmt == FMT_DXT1:
        return decode_dxt1(raw, width, height)
    if fmt == FMT_DXT3:
        return decode_dxt3(raw, width, height)
    if fmt == FMT_DXT5:
        return decode_dxt5(raw, width, height, tiled=dxt5_variant not in (1, 2, 84))
    if fmt == FMT_ATI2:
        return decode_ati2(raw, width, height)
    if fmt == FMT_DXT1_NORMAL:
        return decode_dxt1_normal(raw, width, height)
    if fmt == FMT_A8R8G8B8:
        return _decode_raw_a8r8g8b8(_untile360_raw(raw, width, height, 4), width, height)
    if fmt == FMT_B5G6R5:
        return _decode_raw_b5g6r5(_untile360_raw(_swap16(raw), width, height, 2),
                                  width, height)
    if fmt == FMT_A8:
        return _decode_raw_a8(_untile360_raw(raw, width, height, 1), width, height)
    raise RX2ParseError("unhandled texture format 0x%02X" % fmt)


def _main(argv=None):
    import argparse
    parser = argparse.ArgumentParser(description="Parse an EA Skate RX2 file.")
    parser.add_argument("input", help="path to the .rx2 / .dat file")
    parser.add_argument("--toc", action="store_true", help="dump the file table")
    parser.add_argument("--out-dir", help="save every texture as PNG into this directory")
    args = parser.parse_args(argv)

    rx2 = parse_rx2(args.input)
    print(rx2)
    for w in rx2.warnings:
        print("WARNING:", w)
    if args.toc:
        for entry in rx2.entries:
            print("  ", entry)
    for tex in rx2.textures:
        print("  ", tex)
    if args.out_dir:
        out = Path(args.out_dir)
        out.mkdir(parents=True, exist_ok=True)
        for tex in rx2.textures:
            p = out / (tex.name + ".png")
            tex.save_png(p)
            print("saved", p)
    return 0


try:
    from rx2_fast import (decode_dxt1, decode_dxt1_normal, decode_dxt3,
                         decode_dxt5, decode_ati2, _untile360,
                         _decode_raw_a8r8g8b8, _decode_raw_b5g6r5, _decode_raw_a8)
except ImportError:
    pass  # Standalone authoring tools without NumPy use the scalar decoder.

if __name__ == "__main__":
    raise SystemExit(_main())
