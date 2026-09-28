"""Isolate optional content failures while keeping validated gameplay mandatory."""
import json
import traceback
import re
import struct
from xml.etree.ElementTree import ParseError
from .setup_state import atomic_json

# Do not downgrade disk-full/permission errors, MemoryError, cancellation or
# arbitrary programming exceptions into an incomplete but successful install.
CONTENT_ERRORS = (FileNotFoundError, KeyError, ValueError, RuntimeError, StopIteration, EOFError, struct.error, ParseError)


def note(path, component, error, retained=False, report=print):
    path.parent.mkdir(parents=True, exist_ok=True)
    value = {'version': 1, 'component': component,
             'status': 'retained' if retained else 'unavailable',
             'error': str(error), 'traceback': ''.join(traceback.format_exception(type(error), error, error.__traceback__))}
    atomic_json(path, value)
    report(f'{component}: {"keeping previous content" if retained else "unavailable"}: {error}')
    return value


def summary(stage):
    warnings = []
    private = stage/'assets/private'
    custom = private/'customisation'
    current = {}
    try:current = json.loads((custom/'current.json').read_text())
    except (OSError, ValueError):pass
    identity = current.get('set', '') if isinstance(current, dict) else ''
    active = custom/'sets'/identity if re.fullmatch('[0-9a-f]{32}', identity) else None
    for path in private.rglob('*-availability.json'):
        if path.is_relative_to(custom/'sets') and (active is None or not path.is_relative_to(active)):
            continue
        try:
            item = json.loads(path.read_text())
            if isinstance(item, dict) and item.get('status') in ('retained', 'unavailable'):
                warnings.append(item)
        except (OSError, ValueError):continue
    if active:
        for filename, key, component in [('library-v3.json', 'errors', 'Customiser items'),
                                        ('native-roster/complete.json', 'unavailable', 'Pro characters')]:
            try:items = json.loads((active/filename).read_text()).get(key, [])
            except (OSError, ValueError):continue
            if items:warnings.append(dict(component=component, status='unavailable', items=items))
    atomic_json(stage/'setup-report.json', {'version': 1, 'warnings': warnings})
    return warnings
