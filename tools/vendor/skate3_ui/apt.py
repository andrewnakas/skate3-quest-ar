from __future__ import annotations

from pathlib import Path

from .binary import FormatError, Reader

CHARACTER_TYPES = {
    1: "shape",
    2: "text",
    3: "font",
    4: "button",
    5: "sprite",
    6: "sound",
    7: "bitmap",
    8: "morph",
    9: "animation",
    10: "static_text",
    11: "none",
    12: "video",
    13: "command",
    14: "command_list",
    15: "level",
    16: "custom_control",
    17: "image",
    18: "plugin",
    19: "bitmap_slice",
    20: "avatar",
}

CONTROL_TYPES = {
    1: "do_action",
    2: "frame_label",
    3: "place_object2",
    4: "remove_object2",
    5: "background_color",
    6: "start_sound",
    7: "start_sound_stream",
    8: "do_init_action",
    9: "place_object3",
    10: "place_object4",
    11: "remove_object3",
}


class AptInspector:
    def __init__(self, apt_path: Path, const_path: Path):
        self.apt_path = Path(apt_path)
        self.const_path = Path(const_path)
        self.apt = Reader(self.apt_path.read_bytes(), str(self.apt_path))
        self.const = Reader(self.const_path.read_bytes(), str(self.const_path))
        self.const.require(0, 32)
        self.apt.require(0, 16)

    def character(self, offset: int, character_id: int) -> dict | None:
        if not offset:
            return None
        self.apt.require(offset, 16)
        type_id = self.apt.u32be(offset)
        body = offset + 16
        result = {
            "id": character_id,
            "offset": offset,
            "type": type_id,
            "type_name": CHARACTER_TYPES.get(type_id, f"unknown_{type_id}"),
        }
        if type_id in (1, 2, 10):
            self.apt.require(body, 16)
            result["bounds"] = [self.apt.f32be(body + i * 4) for i in range(4)]
        if type_id == 2:
            self.apt.require(body, 52)
            color = self.apt.u32be(body + 24)
            result["text"] = {
                "font_id": self.apt.u32be(body + 16),
                "alignment": self.apt.u32be(body + 20),
                "color_argb": f"#{color:08x}",
                "font_height": self.apt.f32be(body + 28),
                "read_only": bool(self.apt.u32be(body + 32)),
                "multiline": bool(self.apt.u32be(body + 36)),
                "word_wrap": bool(self.apt.u32be(body + 40)),
                "initial_text": self.apt.cstring(self.apt.u32be(body + 44)),
                "variable": self.apt.cstring(self.apt.u32be(body + 48)),
            }
        elif type_id == 3:
            self.apt.require(body, 12)
            glyph_count = self.apt.u32be(body + 4)
            glyphs_offset = self.apt.u32be(body + 8)
            if glyph_count > 100_000:
                raise FormatError("unreasonable APT font glyph count")
            glyphs = []
            if glyph_count:
                self.apt.require(glyphs_offset, glyph_count * 4)
                glyphs = [
                    self.apt.u32be(glyphs_offset + index * 4)
                    for index in range(glyph_count)
                ]
            result["font"] = {
                "name": self.apt.cstring(self.apt.u32be(body)),
                "glyph_character_ids": glyphs,
            }
        elif type_id == 10:
            self.apt.require(body, 48)
            result["matrix"] = [
                self.apt.f32be(body + 16 + i * 4) for i in range(6)
            ]
            record_count = self.apt.u32be(body + 40)
            records_offset = self.apt.u32be(body + 44)
            if record_count > 100_000:
                raise FormatError("unreasonable APT static-text record count")
            self.apt.require(records_offset, record_count * 56)
            records = []
            for record_index in range(record_count):
                record_offset = records_offset + record_index * 56
                glyph_count = self.apt.u32be(record_offset + 48)
                glyphs_offset = self.apt.u32be(record_offset + 52)
                if glyph_count > 1_000_000:
                    raise FormatError("unreasonable APT static-text glyph count")
                self.apt.require(glyphs_offset, glyph_count * 4)
                glyphs = []
                for glyph_index in range(glyph_count):
                    glyph_offset = glyphs_offset + glyph_index * 4
                    glyphs.append(
                        {
                            "index": self.apt.u16be(glyph_offset),
                            "advance": int.from_bytes(
                                self.apt.bytes(glyph_offset + 2, 2),
                                "big",
                                signed=True,
                            ),
                        }
                    )
                records.append(
                    {
                        "offset": record_offset,
                        "font_id": self.apt.u32be(record_offset),
                        "color_transform": [
                            self.apt.f32be(record_offset + 4 + i * 4)
                            for i in range(8)
                        ],
                        "x_offset": self.apt.f32be(record_offset + 36),
                        "y_offset": self.apt.f32be(record_offset + 40),
                        "scale": self.apt.f32be(record_offset + 44),
                        "glyphs": glyphs,
                    }
                )
            result["static_text"] = {"records": records}
        elif type_id in (5, 9):
            self.apt.require(body, 12)
            movie = {
                "frame_count": self.apt.u32be(body),
                "frames_offset": self.apt.u32be(body + 4),
                "labels_offset": self.apt.u32be(body + 8),
            }
            if movie["frame_count"] > 100_000:
                raise FormatError("unreasonable APT frame count")
            if type_id == 9:
                self.apt.require(body, 52)
                movie.update(
                    {
                        "character_count": self.apt.u32be(body + 12),
                        "characters_offset": self.apt.u32be(body + 16),
                        "width": self.apt.u32be(body + 20),
                        "height": self.apt.u32be(body + 24),
                        "milliseconds_per_frame": self.apt.u32be(body + 28),
                        "import_count": self.apt.u32be(body + 32),
                        "imports_offset": self.apt.u32be(body + 36),
                        "export_count": self.apt.u32be(body + 40),
                        "exports_offset": self.apt.u32be(body + 44),
                        "current_constant_index": self.apt.u32be(body + 48),
                    }
                )
            result["movie"] = movie
        elif type_id == 7:
            self.apt.require(body, 4)
            result["bitmap"] = {"texture_id": self.apt.u32be(body)}
        return result

    def control(self, offset: int) -> dict:
        self.apt.require(offset, 4)
        type_id = self.apt.u32be(offset)
        result = {
            "offset": offset,
            "type": type_id,
            "type_name": CONTROL_TYPES.get(type_id, f"unknown_{type_id}"),
        }
        if type_id in (3, 9):
            size = 72 if type_id == 9 else 64
            self.apt.require(offset, size)
            name_pointer = self.apt.u32be(offset + 52)
            result.update(
                {
                    "flags": self.apt.u32be(offset + 4),
                    "depth": self.apt.i32be(offset + 8),
                    "character_id": self.apt.i32be(offset + 12),
                    "matrix": [
                        self.apt.f32be(offset + 16 + i * 4) for i in range(6)
                    ],
                    "color_transform": list(self.apt.bytes(offset + 40, 8)),
                    "ratio": self.apt.f32be(offset + 48),
                    "name": self.apt.cstring(name_pointer),
                    "clip_depth": self.apt.i32be(offset + 56),
                    "actions_offset": self.apt.u32be(offset + 60),
                }
            )
            if type_id == 9:
                result["blend_mode"] = self.apt.i32be(offset + 64)
                result["filter_pointer"] = self.apt.u32be(offset + 68)
        elif type_id == 2:
            self.apt.require(offset, 8)
            result["label"] = self.apt.cstring(self.apt.u32be(offset + 4))
        elif type_id == 5:
            self.apt.require(offset, 8)
            result["color"] = f"#{self.apt.u32be(offset + 4):08x}"
        elif type_id == 1:
            self.apt.require(offset, 8)
            result["actions_offset"] = self.apt.u32be(offset + 4)
            action_offset = result["actions_offset"]
            if action_offset:
                self.apt.require(action_offset, 1)
                result["first_action_opcode"] = self.apt.u8(action_offset)
        elif type_id == 8:
            self.apt.require(offset, 12)
            result["sprite_id"] = self.apt.u32be(offset + 4)
            result["actions_offset"] = self.apt.u32be(offset + 8)
            action_offset = result["actions_offset"]
            if action_offset:
                self.apt.require(action_offset, 1)
                result["first_action_opcode"] = self.apt.u8(action_offset)
        elif type_id in (4, 11):
            self.apt.require(offset, 8)
            result["depth"] = self.apt.i32be(offset + 4)
        elif type_id == 6:
            self.apt.require(offset, 8)
            result["sound_id"] = self.apt.u32be(offset + 4)
        return result

    def frames(self, movie: dict) -> list[dict]:
        count = movie["frame_count"]
        frames_offset = movie["frames_offset"]
        self.apt.require(frames_offset, count * 8)
        frames = []
        for frame_index in range(count):
            frame = frames_offset + frame_index * 8
            control_count = self.apt.u32be(frame)
            controls_offset = self.apt.u32be(frame + 4)
            if control_count > 100_000:
                raise FormatError(f"frame {frame_index}: unreasonable control count")
            self.apt.require(controls_offset, control_count * 4)
            controls = []
            for control_index in range(control_count):
                pointer = self.apt.u32be(controls_offset + control_index * 4)
                if pointer:
                    controls.append(self.control(pointer))
            frames.append({"index": frame_index, "controls": controls})
        return frames

    def inspect(self) -> dict:
        header = {
            "magic_hex": self.const.bytes(0, 20).hex(),
            "main_character_offset": self.const.u32be(20),
            "constant_count": self.const.u32be(24),
            "constants_offset": self.const.u32be(28),
        }
        root = self.character(header["main_character_offset"], 0)
        imports = []
        exports = []
        characters = []
        if root and root["type"] in (5, 9):
            movie = root["movie"]
            import_count = movie.get("import_count", 0)
            imports_offset = movie.get("imports_offset", 0)
            if import_count > 100_000:
                raise FormatError("unreasonable APT import count")
            if import_count:
                self.apt.require(imports_offset, import_count * 16)
                for index in range(import_count):
                    record = imports_offset + index * 16
                    imports.append(
                        {
                            "index": index,
                            "offset": record,
                            "file": self.apt.cstring(self.apt.u32be(record)),
                            "name": self.apt.cstring(self.apt.u32be(record + 4)),
                            "character_id": self.apt.u32be(record + 8),
                        }
                    )
            export_count = movie.get("export_count", 0)
            exports_offset = movie.get("exports_offset", 0)
            if export_count > 100_000:
                raise FormatError("unreasonable APT export count")
            if export_count:
                self.apt.require(exports_offset, export_count * 8)
                for index in range(export_count):
                    record = exports_offset + index * 8
                    exports.append(
                        {
                            "index": index,
                            "offset": record,
                            "name": self.apt.cstring(self.apt.u32be(record)),
                            "character_id": self.apt.u32be(record + 4),
                        }
                    )
            count = movie.get("character_count", 0)
            table = movie.get("characters_offset", 0)
            if count > 100_000:
                raise FormatError("unreasonable APT character count")
            if count:
                self.apt.require(table, count * 4)
                for character_id in range(count):
                    parsed = self.character(
                        self.apt.u32be(table + character_id * 4), character_id
                    )
                    if parsed:
                        if "movie" in parsed:
                            parsed["frames"] = self.frames(parsed["movie"])
                        characters.append(parsed)
            root["frames"] = self.frames(movie)
        return {
            "format": "skate3-apt",
            "apt_source": str(self.apt_path),
            "const_source": str(self.const_path),
            "const_header": header,
            "root": root,
            "characters": characters,
            "imports": imports,
            "exports": exports,
        }


def inspect_apt(apt_path: Path, const_path: Path) -> dict:
    return AptInspector(apt_path, const_path).inspect()
