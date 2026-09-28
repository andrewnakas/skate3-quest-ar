"""Exercise the shipped payload without UI, a game process, or private assets."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile


def check_update_transaction(setup, install):
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import updater
    tx = install/'.update-transaction'
    # Model an older three-program installation upgrading to the current payload.
    old_names = ('skate3rust.exe', 'support/skate3setup.exe', 'support/skate3update.exe')
    new_names = (*old_names, 'steam-relay/skate-steam-relay.exe', 'steam-relay/steam_api64.dll')
    for directory, names, content in ((install, old_names, b'old program'),
                                      (tx/'new', new_names, b'new program')):
        for name in names:
            path = directory/name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        if directory == tx/'new':
            shutil.copy2(setup, directory/'support/skate3setup.exe')
        meta = {'files': {name: hashlib.sha256((directory/name).read_bytes()).hexdigest()
                          for name in names}}
        (directory/'release.json').write_text(json.dumps(meta), encoding='utf-8')
    preserved = ('data/installation.json', 'settings/graphics.json',
                 'support/custom-models/calibration.json')
    for name in preserved:
        (install/name).parent.mkdir(parents=True, exist_ok=True)
        (install/name).write_bytes(b'preserve')
    updater.install(install, tx)
    assert all((install/name).read_bytes() == b'preserve' for name in preserved)
    assert updater.installer.mismatches(install, meta) == []
    return install/'support/skate3setup.exe'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--setup', type=Path)
    parser.add_argument('--inside', action='store_true')
    parser.add_argument('--transaction-only', action='store_true')
    args = parser.parse_args()
    if args.transaction_only:
        with tempfile.TemporaryDirectory(prefix='character update ') as temp:
            root = Path(temp)
            setup = root/'setup.exe'
            setup.write_bytes(b'packaged importer fixture')
            check_update_transaction(setup, root/'installation')
        print('Character package update transaction passed.')
        return
    if args.inside:
        # Fresh setup needs the same feature scripts that previously existed
        # only in manually prepared customiser/native-roster worktrees.
        from tools.asset_pipeline import customiser_setup, customiser_lighting
        from tools.asset_pipeline import customisation_library, customisation_profiles, native_roster
        import hashlib
        import tkinter
        from PIL import Image
        from main import app_root
        tool = app_root()/'tools/FBX2glTF.exe'
        assert hashlib.sha256(tool.read_bytes()).hexdigest() == '8d90fb5e0a8d186a3d9a7ff8c75eaee541c3975ce4df0d80351f20092ae0877f'
        for name in ('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll'):
            assert tool.with_name(name).is_file(), name
        result = subprocess.run([str(tool), '--version'], capture_output=True, timeout=30)
        assert result.returncode == 0, result.stderr
        assert b'0.9.7' in result.stdout + result.stderr
        assert len(customiser_setup.fingerprint()) == 64
        menu = customisation_profiles.generate({'morphs': [], 'collections': []})
        assert len(menu[-1]['children'][0]['children'][0]['children']) == 37
        return
    from test_converter import fixture
    from converter import validate_output
    from PIL import Image
    setup = args.setup.resolve()
    # Child must use only its packaged Python/native dependencies, never PATH.
    env = {k: v for k, v in os.environ.items() if not k.startswith('PYTHON')}
    env['PATH'] = str(Path(os.environ['SystemRoot'])/'System32')
    with tempfile.TemporaryDirectory(prefix='character package ') as temp:
        root = Path(temp)
        install = root/'old portable copy'
        setup = check_update_transaction(setup, install)
        def run(*arguments):
            return subprocess.run([str(setup), *map(str, arguments)], cwd=root,
                                  env=env, capture_output=True, timeout=180)
        probe = run('--task', 'tools/mixamo_to_skate/check_package.py', '--inside')
        assert probe.returncode == 0, (probe.stdout, probe.stderr)
        source, reference = root/'source.glb', root/'reference.glb'
        fixture(source, True)
        fixture(reference, False)
        library, reply = root/'library', root/'reply.json'
        library.mkdir()
        selection = library/'selection.json'
        selection.write_text('{"selected": "preserve"}')
        arguments = ('--character-import', '--library-import', library,
                     '--reference', reference, '--result', reply, '--noninteractive')
        result = run(*arguments, source)
        response = json.loads(reply.read_text())
        assert result.returncode == 0 and response['status'] == 'ready', response
        entry = library/'entries'/response['id']
        validate_output(entry/'character.glb', reference)
        with Image.open(entry/'preview.png') as preview:
            preview.verify()
        before = {p.relative_to(library): p.read_bytes() for p in library.rglob('*') if p.is_file()}
        # Invoke the real bundled FBX parser on invalid data and verify failure
        # is reported without publishing a broken entry or changing selection.
        invalid = root/'invalid.fbx'
        invalid.write_bytes(b'not an FBX')
        result = run(*arguments, invalid)
        response = json.loads(reply.read_text())
        assert result.returncode == 1 and response['status'] == 'error', response
        assert 'FBX conversion failed' in response['message'], response
        assert before == {p.relative_to(library): p.read_bytes() for p in library.rglob('*') if p.is_file()}
    print('Packaged importer: manifest update, native runtime, GLB import, thumbnail, FBX failure and data preservation passed.')


if __name__ == '__main__':
    main()
