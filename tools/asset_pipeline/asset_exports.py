"""Owned-disc exports, separate from setup transaction orchestration."""
import json
import shutil
from pathlib import Path
from . import install as engine
from .vlt import convert as convert_vlt
from .physics_skeleton import convert as convert_skeleton

def core(game_root, stage, work, report, log, converted=None, audio_image=None):
    private=stage/"assets/private"
    stock=private/"stock"
    report('Extracting animation banks, graphs and gameplay inputs')
    engine.extract(game_root/'data/big/miscload.big',stock)
    engine.extract(game_root/'data/big/miscboot.big',stock,lambda e:e.path.lower()=='data/config/input.cfg')
    # Some disc banks are loose files rather than members of miscload.
    loose=game_root/'data/anim'
    if loose.is_dir():shutil.copytree(loose,stock/'data/anim',dirs_exist_ok=True)
    # Player/board sound banks are owned-disc inputs. Keep them inside the private prepared
    # installation so a normal launch does not depend on a developer-specific extraction path.
    # They remain ignored local assets and are never included in source control or release media.
    # MixMapSK8.mxb is the authored mixer every player-sound gain, pitch and pan word comes from.
    audio=game_root/'data/audio'
    for name in ('audiofiles.big','wheels.big','grains.big','MixMapSK8.mxb'):
        source=audio/name
        if source.is_file():
            target=stock/'data/audio'/name
            target.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(source,target)
    # The evaluator also needs the owner's TU3 runtime image. It is decrypted code, not a
    # distributable game asset, so accept it only as an explicit local setup input and keep it
    # alongside the owned audio archives. Missing images leave ordinary setup usable; the sound
    # runtime will report that exact capability as unavailable rather than substitute raw samples.
    if audio_image is not None:
        image_root=Path(audio_image).resolve()
        regions=sorted(image_root.glob('g_*.bin')) if image_root.is_dir() else []
        if not regions:
            raise RuntimeError('Audio runtime image has no g_*.bin regions: '+str(image_root))
        target_root=stock/'audio-runtime-image'
        target_root.mkdir(parents=True,exist_ok=True)
        for source in regions:
            shutil.copy2(source,target_root/source.name)
    report('Converting physics and difficulty settings')
    database=work/'database'
    engine.extract(game_root/'data/big/db.big',database,lambda e:Path(e.path).name.lower() in {
        'skaterschema.bin','skaterschema.vlt','skatercollections.bin','skatercollections.vlt'})
    names=(engine.TOOLS/'asset_pipeline/names.txt').read_text(encoding='utf-8').splitlines()
    converted=convert_vlt(database/'data/db/skaterschema',database/'data/db/skatercollections',names)
    (stock/'skater-collections.json').write_text(json.dumps(converted),encoding='utf-8')
    skeleton=convert_skeleton(stock/'data/anim/OnBoard.abin')
    (stock/'physics-skeletons.json').write_text(json.dumps(skeleton),encoding='utf-8')
    return converted


def hud(game_root, stage, work, report, log, converted=None):
    private=stage/"assets/private"
    stock=private/"stock"
    report('Preparing original scoring and session-marker HUD assets')
    engine.run(engine.task(engine.TOOLS/'prepare_runtime_huds.py', '--game', game_root,
             '--assets', stage/'assets', '--work', work/'hud'), log, report)


def character(game_root, stage, work, report, log, converted=None):
    private=stage/"assets/private"
    stock=private/"stock"
    report('Preparing the skater model and textures')
    manifest=json.loads((engine.TOOLS/'default_skater_retail_manifest.json').read_text())
    needed=set()
    for c in manifest['components']:
        needed.add(f"data/content/createacharacter/model/cas_db/{c['slot']}/0x{c['model_id']}.rx2".lower())
        needed.update(f'data/content/createacharacter/texture/0x{x}.rx2'.lower() for x in c['textures'].values())
    engine.extract(game_root/'data/content/createacharacter.big',stock,lambda e:e.path.lower() in needed)
    character=work/'character'
    engine.run(engine.task(engine.TOOLS/'extract_default_skater.py','--owned-data-root',stock,'--work-root',character,
             '--private-root',private/'default_skater','--utt-root',engine.TOOLS/'vendor/utt'),log,report)
    report('Building the skater model and rig')
    from .character_glb import convert as write_character
    write_character(character/'selected/models',private,manifest)
    from .character_lighting import convert as write_character_lighting
    write_character_lighting(character/'selected/models',private,converted)
    game_manifest={'version':1,'character_scene':'private/skater.glb','initial_animation':'R_IDLE_HCOM_000',
                   'action_graph':'private/stock/data/state/ActionGraph_OnBoard.stategraph',
                   'motion_graph':'private/stock/data/state/MotionGraph_OnBoard.stategraph'}
    (private/'game.json').write_text(json.dumps(game_manifest),encoding='utf-8')


def environment(game_root, stage, work, report, log, converted=None):
    private=stage/"assets/private"
    stock=private/"stock"
    report('Preparing retail sky domes')
    from .sky import convert as write_skies
    def attempt(name, action):
        from .optional_content import CONTENT_ERRORS, note
        target=work/('environment-'+name)/'assets'
        target.mkdir(parents=True,exist_ok=True)
        availability=private/'environment-status'/(name+'-availability.json')
        try:action(target)
        except CONTENT_ERRORS as error:
            note(availability,'Environment '+name,error,report=report)
            return
        shutil.copytree(target,stage/'assets',dirs_exist_ok=True)
        availability.unlink(missing_ok=True)
    attempt('skies',lambda assets:write_skies(game_root,assets,converted))
    from .render_parameters import convert as write_render_parameters
    attempt('parameters',lambda assets:write_render_parameters(assets,converted))
    report('Extracting original travel destinations and location names')
    from .teleports import convert as write_teleports
    attempt('teleports',lambda assets:write_teleports(game_root,assets,converted))
    report('Preparing global foliage backdrops')
    from .backdrop import convert as write_backdrops
    attempt('backdrops',lambda assets:write_backdrops(game_root,assets,converted))
