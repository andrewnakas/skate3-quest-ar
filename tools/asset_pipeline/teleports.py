"""Join retail FE destinations to localized names and RX2 location matrices.

Owned content stays in private sidecars. No guessed coordinates or renamed spots.
LocationManager 828A3D50 loads data/content/global_locators; EB0009 records
contain a 64-byte matrix and a section-relative name pointer at +104.
"""
import hashlib
import json
import struct
import tempfile
from pathlib import Path

from tools.owned_game.big import BigArchive
from .dynamic_props import sections, matrix
from .environment import Collections, key_hash
from .vlt import vault


def cstring(raw, offset, limit=None):
    limit = len(raw) if limit is None else limit
    if not 0 <= offset < limit <= len(raw):
        raise ValueError('Invalid location string offset')
    return raw[offset:raw.index(b'\0', offset, limit)]


def language_table(raw):
    if len(raw) < 36 or struct.unpack_from('<I', raw)[0] != 0x39000:
        raise ValueError('Unsupported retail language table')
    count, start, strings = struct.unpack_from('<3I', raw, 8)
    start += 8
    strings += 8
    if start + count * 8 != strings or strings > len(raw):
        raise ValueError('Invalid retail language index')
    result = {}
    for index in range(count):
        key, offset = struct.unpack_from('<2I', raw, start + index * 8)
        if key in result:
            raise ValueError('Duplicate language key')
        result[key] = cstring(raw, strings + offset)
    return result


def location_records(raw):
    result = []
    for base, _, size, _, _, kind in sections(raw):
        if kind != 0xEB0009:
            continue
        count, _, _, start, end = struct.unpack_from('>5I', raw, base)
        if base + size > len(raw) or start != 32 or end != start + count * 128 or end > size:
            raise ValueError('Invalid location record table')
        for index in range(count):
            at = base + start + index * 128
            pointer = struct.unpack_from('>I', raw, at + 104)[0]
            if not end <= pointer < size:
                raise ValueError('Location name overlaps records')
            result.append({'locator': cstring(raw, base + pointer, base + size).decode('utf-8'),
                           'matrix': matrix(raw, at).tolist(), 'source_offset': at})
    return result


def convert(game_root, assets, converted):
    game_root, assets = Path(game_root), Path(assets)
    archive = BigArchive(game_root / 'data/big/miscboot.big')
    tables = {}
    for language in ('english', 'labels'):
        entry = next(e for e in archive.entries if f'/{language}/' in e.path and '_Global_' in e.path)
        tables[language] = language_table(archive.read(entry))
    labels = {value.strip().decode('ascii'): key for key, value in tables['labels'].items()}
    database = BigArchive(game_root / 'data/big/db.big')
    with tempfile.TemporaryDirectory(prefix='skate-teleports-') as tmp:
        root = Path(tmp)
        entries = [e for e in database.entries if e.path in
                   ('data/db/skatercollections.bin', 'data/db/skatercollections.vlt')]
        database.extract_entries(entries, root)
        _, binary, _ = vault(root / 'data/db/skatercollections')
    locators = {}
    for path in sorted((game_root / 'data/content/global_locators').rglob('*.rx2')):
        raw = path.read_bytes()
        stream = path.parent.name
        for record in location_records(raw):
            record.update(map=stream.removeprefix('DIST_'), source=str(path.relative_to(game_root)),
                          source_sha256=hashlib.sha256(raw).hexdigest())
            locators.setdefault((stream.casefold(), record['locator']), []).append(record)
    collections = Collections(converted)
    destinations = []
    # Own location artwork distinguishes selectable FE destinations from base
    # templates, startup checkpoints and park-layout aliases.
    for (cls, key), row in collections.rows.items():
        if cls != key_hash('fe_locations') or not any(key_hash(k) == key_hash('Hash_786A2D24F475342B') for k in row['fields']):
            continue
        fields, _ = collections.resolve(cls, key)
        label = fields[key_hash('Hash_174B301910A902C9')]['data']
        if label not in labels:
            continue  # Template placeholders have no shipped localized name.
        text = tables['english'][labels[label]]
        # 0xAB is the retail font's skate wordmark glyph. The portable UI has
        # no such font glyph; spell out the same brand instead of showing «.
        name = text.replace(b'\xab', b'skate.').decode('ascii').strip()
        locator = cstring(binary, int(fields[key_hash('location')]['data'], 16)).decode('utf-8')
        world_key = int(fields[key_hash('World')]['data'][:16], 16)
        world, _ = collections.resolve('world', world_key)
        stream = world[key_hash('WorldStream')]['data']
        if not (game_root / 'data/content' / ('world' + stream + '.big')).exists():
            continue
        matches = locators.get((stream.casefold(), locator), [])
        unique = {json.dumps(m['matrix']) for m in matches}
        if len(unique) > 1:
            raise ValueError('Ambiguous authored teleport: ' + locator)
        item = dict(id=row['key'], name=name, map=stream.removeprefix('DIST_'),
                    label_key=label, locator=locator, matrix=None)
        if matches:
            item.update(matches[0])
        else:
            item['unavailable_reason'] = 'Authored destination locator was not found in the extracted content'
        destinations.append(item)
    destinations.sort(key=lambda d: (d['map'].casefold(), d['name'].casefold()))
    output = assets / 'private/teleports.json'
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({'version': 1, 'destinations': destinations}, indent=2), encoding='utf-8')
    return destinations
