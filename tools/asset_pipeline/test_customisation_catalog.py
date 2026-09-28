"""Synthetic CAC records only; no game content is needed or stored here."""
from pathlib import Path
import struct
import tempfile
import unittest

from tools.asset_pipeline.customisation_catalog import (
    MODEL_PREFIX, TEXTURE_PREFIX, parse_catalog, prepare,
    selected_paths, write_private,
)
from tools.asset_pipeline.customisation_native import MORPH_FIELDS, decode_morphs


M1 = "0000000000000001"
M2 = "0000000000000002"
MODEL = "0000000000000020"
ASSET = "0000000000000030"
TEX1 = "0000000000000040"
TEX2 = "0000000000000050"


def fixture():
    xml = f'''<compositeasset n="cas_db" type="CreateACharacter" schemaversion="6.0">
    <mat id="0x{M1}" type="test_shader">
      <ud n="cas"><cas.Gender value="female"/><cas.RequiresRemovalOfComponents value="Hat,Hair"/></ud>
      <sp chn="diffuse" id="0x{TEX1}" uv="1" sampler="authored"/>
      <scalar value="0.375"/>
    </mat>
    <mat id="0x{M2}" type="test_shader_second"><sp chn="alpha" id="0x{TEX2}"/></mat>
    <comp n="Head"><mod id="0x{ASSET}" n="synthetic_head">
      <lod idx="0" id="0x0000000000000060" arenaid="0x{MODEL}">
        <matinst><matvar id="0x{M1}" n="first"/></matinst>
        <matinst><matvar id="0x{M2}" n="second"/></matinst>
      </lod>
    </mod></comp></compositeasset>'''.encode()
    paths = {f"{MODEL_PREFIX}/Head/0x{MODEL}.rx2",
             f"{TEXTURE_PREFIX}/0x{TEX1}.rx2", f"{TEXTURE_PREFIX}/0x{TEX2}.rx2"}
    selection = [{"slot": "Head", "asset_id": ASSET, "lod": 0, "material_ids": [M1, M2]}]
    return xml, paths, selection


class CatalogTests(unittest.TestCase):
    def test_preserves_material_instance_order_and_authored_shader_inputs(self):
        xml, paths, selection = fixture()
        catalog = parse_catalog(xml, paths)
        lod = catalog["components"][0]["models"][0]["lods"][0]
        self.assertEqual(lod["model_id"], MODEL)  # arenaid names the RX2, not id
        self.assertEqual([group[0]["id"] for group in lod["material_instances"]], [M1, M2])
        definition = catalog["materials"][M1]["authored"]["children"]
        self.assertEqual(definition[1]["attributes"]["sampler"], "authored")
        self.assertEqual(definition[1]["attributes"]["uv"], "1")
        self.assertEqual(definition[2]["attributes"]["value"], "0.375")
        self.assertEqual(catalog["materials"][M1]["flags"]["cas.RequiresRemovalOfComponents"], "Hat,Hair")
        self.assertEqual(set(selected_paths(catalog, selection)), paths)

    def test_rejects_incomplete_or_reordered_material_selection(self):
        xml, paths, selection = fixture()
        catalog = parse_catalog(xml, paths)
        for choices in ([M1], [M2, M1], [M1, "000000000000ffff"]):
            selection[0]["material_ids"] = choices
            with self.assertRaises(ValueError):
                selected_paths(catalog, selection)

    def test_missing_archived_texture_is_reported_and_cannot_be_extracted(self):
        xml, paths, selection = fixture()
        paths.remove(f"{TEXTURE_PREFIX}/0x{TEX2}.rx2")
        catalog = parse_catalog(xml, paths)
        self.assertFalse(catalog["materials"][M2]["textures"][0]["in_archive"])
        with self.assertRaisesRegex(ValueError, "Selected texture is absent"):
            selected_paths(catalog, selection)

    def test_rejects_broken_material_references_and_path_slots(self):
        xml, paths, _ = fixture()
        broken = xml.replace(f'<matvar id="0x{M1}"'.encode(), b'<matvar id="0x000000000000ffff"')
        with self.assertRaisesRegex(ValueError, "Missing material definition"):
            parse_catalog(broken, paths)
        with self.assertRaisesRegex(ValueError, "Invalid or duplicate component"):
            parse_catalog(xml.replace(b'comp n="Head"', b'comp n="../Head"'), paths)

    def test_never_overwrites_staged_content_and_reuses_identical_files(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / "private" / "resource"
            self.assertEqual(write_private(target, b"first"), "written")
            self.assertEqual(write_private(target, b"first"), "reused")
            with self.assertRaisesRegex(ValueError, "Refusing to replace"):
                write_private(target, b"second")
            self.assertEqual(target.read_bytes(), b"first")
            self.assertEqual([p.name for p in target.parent.iterdir()], ["resource"])

    def test_rejects_staging_inside_or_above_owned_source_before_opening_archive(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root) / "owned"
            source.mkdir()
            for destination in (source, source / "overlay", Path(root)):
                with self.assertRaisesRegex(ValueError, "separate from the owned source"):
                    prepare(source, destination)


def morph_rows():
    rows = []
    for index in range(19):
        values = {"target": {"type": "EA::Reflection::Text", "data": f"target_{index}"},
                  "target_order": {"type": "EA::Reflection::Int32", "data": struct.pack(">i", index).hex()},
                  "ui_zone": {"type": "zone", "data": struct.pack(">I", 18 - index).hex()}}
        for branch in ("a", "b"):
            for name, number in (("min", 0.0), ("max", 0.5), ("default", 0.25)):
                values[f"{name}_{branch}"] = {"type": "EA::Reflection::Float", "data": struct.pack(">f", number).hex()}
        rows.append({"class": "cac_morph_params", "key": f"field_{index}", "fields": {
            f"Hash_{MORPH_FIELDS[name]:016X}": value for name, value in values.items()}})
    return rows


class NativeMetadataTests(unittest.TestCase):
    def test_target_order_and_ui_zone_remain_independent(self):
        decoded = decode_morphs(list(reversed(morph_rows())))
        self.assertEqual(decoded[3]["target_order"], 3)
        self.assertEqual(decoded[3]["ui_zone"], 15)
        self.assertEqual(decoded[3]["branch_a"], {"min": 0.0, "max": 0.5, "default": 0.25})

    def test_rejects_missing_targets_and_nonfinite_ranges(self):
        with self.assertRaisesRegex(ValueError, "incomplete or duplicated"):
            decode_morphs(morph_rows()[:-1])
        rows = morph_rows()
        rows[3]["fields"][f"Hash_{MORPH_FIELDS['max_a']:016X}"]["data"] = "7fc00000"
        with self.assertRaisesRegex(ValueError, "Non-finite"):
            decode_morphs(rows)


if __name__ == "__main__":
    unittest.main()
