"""Export the two HUDs used by the engine, as one automatic setup stage.

Only runtime manifests and referenced RGBA payloads enter the installation.
Extraction intermediates stay in setup's conversion workspace and are removed
by its normal successful-install cleanup. No audio or full-disc extraction.
"""
import argparse
from pathlib import Path
import tempfile

from prepare_hud import prepare as prepare_scoring
from extract_session_marker import prepare as prepare_marker
from install_prepared_hud import install


def prepare(game, assets, work):
    # Retail texture names plus the install/generation prefix can exceed legacy
    # Windows path limits. Only compact runtime files belong in the installation.
    with tempfile.TemporaryDirectory(prefix='sk8hud-') as temporary:
        return prepare_in_workspace(game, assets, Path(temporary))


def prepare_in_workspace(game, assets, work):
    from asset_pipeline.optional_content import CONTENT_ERRORS, note
    from install_prepared_hud import runtime_files
    scoring = work / 'scoring'
    marker = work / 'session-marker'
    files = {}
    for name, source, action, is_marker in (
        ('hud', scoring, lambda: prepare_scoring(game, scoring, assets/'private/stock/skater-collections.json'), False),
        ('session-marker', marker, lambda: prepare_marker(game, marker), True)):
        availability=assets/'private'/(name+'-availability.json')
        try:
            action()
            runtime_files(source, is_marker)
        except CONTENT_ERRORS as error:
            try:
                runtime_files(assets/'private'/name,is_marker)
                retained=True
            except CONTENT_ERRORS:
                retained=False
                # Broken artwork must not be loaded as if it were complete.
                (assets/'private'/name/('hud.json' if is_marker else 'runtime/trickdisplay.json')).unlink(missing_ok=True)
            note(availability,name,error,retained)
            continue
        files.update(install(assets,None if is_marker else source,source if is_marker else None,replace=True))
        availability.unlink(missing_ok=True)
    print(f'Original runtime HUDs ready: {len(files)} verified files', flush=True)
    return files


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, required=True)
    parser.add_argument('--assets', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    args = parser.parse_args()
    prepare(args.game, args.assets, args.work)
