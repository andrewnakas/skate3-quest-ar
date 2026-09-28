"""Resolve authored Marquee resources without substituting other models or LODs."""
from pathlib import PurePosixPath
import xml.etree.ElementTree as ET


class MissingMarqueeAsset(FileNotFoundError):
    pass


class Resources:
    def __init__(self, archive):
        self.archive = archive
        self.entries = {e.path.replace('\\', '/').lower(): e for e in archive.entries}
        self.by_name = {}
        for name, entry in self.entries.items():
            self.by_name.setdefault(PurePosixPath(name).name, []).append(entry)

    def resolve(self, path):
        key = path.replace('\\', '/').lower()
        if key in self.entries:
            return self.entries[key]
        # arenaid is the file identity. Only accept a unique identical filename,
        # never a similarly named character or a lower-detail LOD.
        leaf = PurePosixPath(key).name
        matches = self.by_name.get(leaf, []) if leaf.startswith('0x') and leaf.endswith('.rx2') else []
        if len(matches) == 1:
            return matches[0]
        if matches:
            raise ValueError('Ambiguous Marquee asset identity: '+path)
        raise MissingMarqueeAsset('Owned marquee.big is missing '+path)

    def read(self, path):
        return self.archive.read(self.resolve(path))

    def recipe(self, name):
        root = ET.fromstring(self.read('data/content/recipe/marquee/'+name+'.xml'))
        for component in root.findall('comp'):
            mods = component.findall('mod')
            if len(mods) != 1:
                raise ValueError('Ambiguous native component '+component.attrib['n'])
            lod = next(l for l in mods[0].findall('lod') if l.get('idx') == '0')
            self.resolve(f"data/content/marquee/model/{name}/{component.attrib['n']}/{lod.attrib['arenaid']}.rx2")
        for material in root.findall('mat'):
            for texture in material.findall('sp'):
                tid = texture.attrib['id'].lower().removeprefix('0x')
                self.resolve('data/content/marquee/texture/0x'+tid+'.rx2')
        return root
