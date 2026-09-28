from __future__ import annotations

from copy import deepcopy

from .binary import FormatError


PLACE_MOVE = 0x01
PLACE_HAS_CHARACTER = 0x02
PLACE_HAS_MATRIX = 0x04
PLACE_HAS_COLOR_TRANSFORM = 0x08
PLACE_HAS_RATIO = 0x10
PLACE_HAS_NAME = 0x20
PLACE_HAS_CLIP_DEPTH = 0x40
PLACE_HAS_CLIP_ACTIONS = 0x80


def frame_labels(character: dict) -> dict[str, int]:
    labels = {}
    for frame in character.get("frames", []):
        for control in frame["controls"]:
            if control["type_name"] == "frame_label":
                label = control.get("label", "")
                if not label:
                    continue
                if label in labels:
                    raise FormatError(
                        f"character {character['id']}: duplicate frame label {label!r}"
                    )
                labels[label] = frame["index"]
    return labels


def first_stop_frame(character: dict, start_frame: int) -> int:
    for frame in character.get("frames", [])[start_frame:]:
        if any(
            control["type_name"] == "do_action"
            and control.get("first_action_opcode") == 0x07
            for control in frame["controls"]
        ):
            return frame["index"]
    raise FormatError(
        f"character {character['id']}: no stop action at/after frame {start_frame}"
    )


def playback_frames(character: dict, label: str) -> list[int]:
    labels = frame_labels(character)
    if label not in labels:
        raise FormatError(
            f"character {character['id']}: frame label {label!r} is absent"
        )
    frames = character.get("frames", [])
    if not frames:
        raise FormatError(f"character {character['id']}: timeline has no frames")

    start = labels[label]
    result = []
    frame_index = start
    for _ in range(len(frames)):
        result.append(frame_index)
        frame = frames[frame_index]
        if any(
            control["type_name"] == "do_action"
            and control.get("first_action_opcode") == 0x07
            for control in frame["controls"]
        ):
            return result
        frame_index = (frame_index + 1) % len(frames)
    raise FormatError(
        f"character {character['id']}: playback from {label!r} never stops"
    )


def playback_frame(character: dict, label: str, *, play: bool) -> int:
    labels = frame_labels(character)
    if label not in labels:
        raise FormatError(
            f"character {character['id']}: frame label {label!r} is absent"
        )
    start = labels[label]
    return playback_frames(character, label)[-1] if play else start


def resolve_display_list(character: dict, through_frame: int) -> dict:
    frames = character.get("frames", [])
    if through_frame < 0 or through_frame >= len(frames):
        raise FormatError(
            f"character {character['id']}: frame {through_frame} is out of range"
        )

    objects: dict[int, dict] = {}
    unresolved = []
    for frame in frames[: through_frame + 1]:
        frame_index = frame["index"]
        for control in frame["controls"]:
            kind = control["type_name"]
            if kind in ("remove_object2", "remove_object3"):
                objects.pop(control["depth"], None)
                continue
            if kind not in ("place_object2", "place_object3"):
                continue

            flags = control["flags"]
            depth = control["depth"]
            moving = bool(flags & PLACE_MOVE)
            if moving:
                if depth not in objects:
                    unresolved.append(
                        {
                            "frame": frame_index,
                            "control_offset": control["offset"],
                            "reason": "move references an empty depth",
                            "depth": depth,
                        }
                    )
                    current = {"depth": depth, "properties": {}, "provenance": {}}
                    objects[depth] = current
                else:
                    current = objects[depth]
            else:
                current = {"depth": depth, "properties": {}, "provenance": {}}
                objects[depth] = current

            source = {
                "frame": frame_index,
                "control_offset": control["offset"],
            }

            def assign(name: str, value: object) -> None:
                current["properties"][name] = deepcopy(value)
                current["provenance"][name] = source

            if flags & PLACE_HAS_CHARACTER:
                assign("character_id", control["character_id"])
            if flags & PLACE_HAS_MATRIX:
                assign("matrix", control["matrix"])
            if flags & PLACE_HAS_COLOR_TRANSFORM:
                assign("color_transform_argb", control["color_transform"])
            if flags & PLACE_HAS_RATIO:
                assign("ratio", control["ratio"])
            if flags & PLACE_HAS_NAME:
                assign("name", control["name"])
            if flags & PLACE_HAS_CLIP_DEPTH:
                assign("clip_depth", control["clip_depth"])
            if control["type"] == 9:
                assign("blend_mode", control.get("blend_mode", -1))
                assign("filter_pointer", control.get("filter_pointer", 0))

    for depth, item in objects.items():
        missing = [
            name
            for name in ("character_id", "matrix")
            if name not in item["properties"]
        ]
        if missing:
            unresolved.append(
                {
                    "depth": depth,
                    "reason": f"display object lacks {', '.join(missing)}",
                }
            )

    return {
        "character_id": character["id"],
        "through_frame": through_frame,
        "objects": [objects[depth] for depth in sorted(objects)],
        "unresolved": unresolved,
    }
