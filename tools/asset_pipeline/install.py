"""Local owned-disc installation. No game content is downloaded or packaged."""
from pathlib import Path
import hashlib,json,os,shutil,subprocess,sys,time,urllib.request,uuid,zipfile
from concurrent.futures import ThreadPoolExecutor,as_completed
from tools.owned_game.big import BigArchive

TOOLS=Path(__file__).resolve().parents[1]

def map_workers():
    count=min(3,max(1,(os.cpu_count() or 1)//2))
    if os.name=='nt':
        import ctypes
        class Memory(ctypes.Structure):
            _fields_=[('length',ctypes.c_ulong),('load',ctypes.c_ulong)]+[(name,ctypes.c_ulonglong) for name in
                ('total','available','page_total','page_available','virtual_total','virtual_available','extended')]
        memory=Memory();memory.length=ctypes.sizeof(memory)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(memory)):
            # Reserve memory for the desktop; each map and its loader can
            # briefly hold several copies of geometry and textures.
            count=min(count,max(1,(memory.available-2*1024**3)//(3*1024**3)))
    return count
XISO_URL='https://github.com/XboxDev/extract-xiso/releases/download/build-202505152050/extract-xiso-Win64_Release.zip'
XISO_SHA='fec88d03c7efd6205ab09be4abba70c0afd0eb27a5709f0a6235b828ba5ac11e'

def digest(path):
    with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def remove_intermediate(path,root):
    target=path.resolve();root=root.resolve()
    if target==root or not target.is_relative_to(root):
        raise RuntimeError('Refusing to remove a path outside conversion workspace')
    shutil.rmtree(target)

def download(url,expected,cache,report):
    cache.mkdir(parents=True,exist_ok=True)
    archive=cache/url.rsplit('/',1)[1]
    if not archive.is_file() or digest(archive)!=expected:
        report('Downloading '+archive.name)
        temp=archive.with_suffix('.part')
        request=urllib.request.Request(url,headers={'User-Agent':'Mozilla/5.0 Skate3RustEngine-Setup/1.0'})
        with urllib.request.urlopen(request,timeout=60) as response,temp.open('wb') as output:
            shutil.copyfileobj(response,output,1024*1024)
        if digest(temp)!=expected:raise RuntimeError('Download checksum mismatch: '+archive.name)
        temp.replace(archive)
    return archive

def unpack_zip(archive,destination):
    destination=destination.resolve()
    with zipfile.ZipFile(archive) as z:
        for info in z.infolist():
            target=(destination/info.filename).resolve()
            if not target.is_relative_to(destination):raise RuntimeError('Unsafe tool archive path')
            if (info.external_attr>>16)&0o170000==0o120000:raise RuntimeError('Tool archive contains a symbolic link')
        z.extractall(destination)

def dependency(cache,name,url,sha,report):
    folder=cache/name
    marker=folder/'.complete'
    if not marker.is_file():
        unpack_zip(download(url,sha,cache,report),folder)
        marker.write_text(sha)
    executable=next(folder.rglob(name+'.exe'),None)
    if executable is None:raise RuntimeError('Missing downloaded tool: '+name)
    return executable

def run(args,log,report):
    kwargs={'creationflags':subprocess.CREATE_NO_WINDOW} if os.name=='nt' else {}
    external=os.name=='nt' and getattr(sys,'frozen',False) and Path(args[0]).resolve()!=Path(sys.executable).resolve()
    if external:
        # External tools and the game must load their own libraries, not
        # the setup bundle's DLL directory inherited by child processes.
        import ctypes
        ctypes.windll.kernel32.SetDllDirectoryW(None)
        env=os.environ.copy()
        bundle=Path(sys._MEIPASS).resolve()
        env['PATH']=os.pathsep.join(p for p in env.get('PATH','').split(os.pathsep)
                                   if p and not Path(p).resolve().is_relative_to(bundle))
        kwargs['env']=env
    try:
        child=subprocess.Popen([str(a) for a in args],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,
                               text=True,encoding='utf-8',errors='replace',**kwargs)
    finally:
        if external:ctypes.windll.kernel32.SetDllDirectoryW(sys._MEIPASS)
    with child as process:
        for line in process.stdout:
            log.write(line);log.flush()
        if process.wait():raise RuntimeError('Conversion failed. See '+str(log.name))

def task(script,*args):
    if getattr(sys,'frozen',False):return [sys.executable,'--task',str(script),*map(str,args)]
    return [sys.executable,str(script),*map(str,args)]

def extract(archive,destination,entries=None):
    data=BigArchive(archive)
    data.extract_entries(data.entries if entries is None else [e for e in data.entries if entries(e)],destination)
    return data

def convert_map(archive,work,maps,stage,game_exe,log,report):
    timings={};started=time.perf_counter()
    def finished(phase):
        nonlocal started
        now=time.perf_counter();timings[phase]=round(now-started,3);started=now
        report(f'{archive.stem}: {phase} {timings[phase]:.3f}s')
    map_tools=TOOLS/'vendor/university/tools/vanilla_map_extraction/tools'
    sys.path.insert(0,str(map_tools))
    from prepare_hawaiian_dream import prepare
    from prepare_university import EXCLUDED_NORMAL_TEXTURE_IDS
    from build_retail_collision_archive import build_archive
    from .map_writer import write as write_map, SpawnSelector
    district=archive.stem.removeprefix('world')
    label=district.removeprefix('DIST_')
    district_work=work/district
    extract(archive,district_work/'raw')
    finished('extract')
    stream=district_work/'raw/data/content/world/stream'/district
    if not stream.is_dir():raise RuntimeError('Missing district stream '+str(stream))
    spawn=SpawnSelector(district)
    manifest_path=prepare(stream_directory=stream,output_root=district_work/'intermediate',
        utt_root=TOOLS/'vendor/utt',district_name=district,map_name=label,
        package_name='Skate 3 owned disc',cache_format='skate3-rust-map-v1',
        # Smaller parks keep their textures in Pres rather than a Tex stream.
        texture_stream_names=('Tex',) if any(stream.glob('cTex_*.xsf')) else (),
        excluded_normal_texture_ids=EXCLUDED_NORMAL_TEXTURE_IDS,raw_texture_cache=True,
        collision_consumer=spawn.consider,
        # Model/texture RX2 copies are unused by the direct writer and were
        # deleted after conversion. Keep simulation and irradiance sources.
        write_render_sources=False)
    finished('prepare')
    collision=district_work/'collision.rwcmset'
    build_archive(manifest_path,collision)
    finished('collision_archive')
    final=maps/(label+'.skate')
    write_map(manifest_path,final,collision,report,prepared_spawn=spawn.result(label))
    finished('write_map')
    from .dynamic_props import export as write_props
    caches=list((work/'dmo/cache').glob('DMO_*'))
    from .optional_content import CONTENT_ERRORS, note
    props=stage/'assets/private/native-props'/(label+'.skate')
    try:
        if not caches:raise RuntimeError('Movable-object source catalog is unavailable')
        placed, unresolved=write_props(manifest_path,caches,props,catalog_path=work/'dmo/catalog.json')
        (stage/'assets/private/native-props'/(label+'-availability.json')).unlink(missing_ok=True)
    except CONTENT_ERRORS as error:
        if props.is_dir():remove_intermediate(props,stage)
        note(stage/'assets/private/native-props'/(label+'-availability.json'),label+' movable props',error,report=report)
        placed,unresolved=0,0
    finished('props')
    report(f'{label}: placed {placed} authored DMO instances, {unresolved} unresolved templates')
    report('Checking converted map: '+label)
    run([game_exe,'--assets',stage/'assets','--map',final,'--check-assets'],log,report)
    finished('validate')
    entry={'name':label,'path':'maps/'+final.name,'sha256':digest(final)}
    remove_intermediate(district_work,work)
    finished('hash_and_cleanup')
    entry['phase_seconds']=timings
    return entry


def install(iso,base,game_exe,report,game_root=None,refresh=False,finalize=None,audio_image=None):
    from .setup_state import setup_lock
    with setup_lock(base):
        return _install(iso,base,game_exe,report,game_root,refresh,finalize,audio_image)


def _install(iso,base,game_exe,report,game_root=None,refresh=False,finalize=None,audio_image=None):
    from .versions import fingerprints, changed_groups, installed, GROUPS
    from .group_receipts import damaged, record
    from .setup_state import atomic_json
    from . import asset_exports as exports
    target_versions=fingerprints()
    previous=installed(base) if refresh else None
    groups=changed_groups(previous[1].get('pipelines',{}),target_versions) if previous else set(GROUPS)
    source=str((iso if iso is not None else game_root).resolve())
    if game_root is None and iso is not None:
        selected=iso.resolve()
        if selected.is_dir():game_root=selected
        elif selected.suffix.lower()=='.xex':
            if selected.name.lower()!='default.xex' or not selected.is_file():
                raise RuntimeError('Select default.xex inside your extracted Skate 3 game folder')
            game_root=selected.parent
    base=base.resolve();base.mkdir(parents=True,exist_ok=True)
    from .setup_state import source_directory
    if game_root is not None:
        game_root=source_directory(game_root, require_core=not previous)
        if previous and previous[1].get('source_hash') not in (None,digest(game_root/'default.xex')):
            raise RuntimeError('Select the same Xbox game edition used to set up this copy')
    if previous:groups.update(damaged(*previous, exclude=groups))
    def outputs(stage):
        saved=previous[1].get('outputs',{}) if previous else {}
        return {g:saved[g] if g not in groups and g in saved else record(stage,g) for g in GROUPS}
    if not groups:
        with (base/'refresh-validation.log').open('w',encoding='utf-8') as log:
            run([game_exe,'--assets',previous[0]/'assets','--test-world','--check-assets'],log,report)
        if finalize:finalize(previous[0])
        from .optional_content import summary
        summary(previous[0])
        atomic_json(base/'installation.json', {**previous[1], 'pipelines':target_versions,
                    'outputs':outputs(previous[0]), 'source':source})
        report('Game assets are current')
        return previous[0]
    install_id=uuid.uuid4().hex
    stage=base/'installations'/install_id
    stage.mkdir(parents=True)
    if previous:
        report('Preparing an asset update; keeping the previous installation until it succeeds')
        immutable_maps = ({(previous[0]/item['path']).resolve() for item in
                           json.loads((previous[0]/'maps.json').read_text())}
                          if 'maps' not in groups else set())
        from .customiser_cache import SOURCES as character_stages
        sets=previous[0]/'assets/private/customisation/sets'
        immutable_sets=[p.resolve() for p in sets.glob('*') if p.is_dir()
                        and all((p/(name+'-complete.json')).is_file() for name in character_stages)]
        for entry in previous[0].iterdir():
            if entry.name in {'conversion','setup.log'} or entry.name.endswith('-conversion.log'):continue
            # Unchanged maps and immutable character generations share storage.
            # Mutable user data and rebuilt outputs get independent files.
            def copy_map(src,dst):
                if Path(src).resolve() in immutable_maps:
                    try:os.link(src,dst)
                    except OSError:shutil.copy2(src,dst)
                else:shutil.copy2(src,dst)
                return dst
            def copy_asset(src,dst):
                if any(Path(src).resolve().is_relative_to(root) for root in immutable_sets):
                    try:os.link(src,dst)
                    except OSError:shutil.copy2(src,dst)
                else:shutil.copy2(src,dst)
                return dst
            def ignore_rebuilt_assets(directory, names):
                # These raw inputs are owned by the converters. A rebuilding
                # group must extract into an empty cache, not overwrite files
                # copied from the previous installation. Leave that live copy
                # and all user settings/custom models untouched.
                source_directory = Path(directory).resolve()
                private_source = (previous[0]/'assets/private').resolve()
                if 'core' in groups and source_directory == private_source:
                    return {'stock'} & set(names)
                if 'character' in groups and source_directory == private_source/'stock/data/content':
                    return {'createacharacter'} & set(names)
                return set()
            if entry.is_dir():
                shutil.copytree(entry,stage/entry.name,
                    ignore=ignore_rebuilt_assets if entry.name=='assets' else None,
                    copy_function=copy_map if entry.name=='maps' and 'maps' not in groups
                    else copy_asset if entry.name=='assets' else shutil.copy2)
            else:shutil.copy2(entry,stage/entry.name)
    private=stage/'assets/private';private.mkdir(parents=True,exist_ok=True)
    maps=stage/'maps';maps.mkdir(exist_ok=True)
    work=stage/'conversion';work.mkdir()
    with (stage/'setup.log').open('w',encoding='utf-8') as log:
        if game_root is None:
            iso=iso.resolve()
            if not iso.is_file() or iso.suffix.lower()!='.iso':raise RuntimeError('Select an Xbox 360 Skate 3 ISO')
            extractor=dependency(base/'tools','extract-xiso',XISO_URL,XISO_SHA,report)
            game_root=work/'disc'
            report('Extracting your ISO')
            run([extractor,'-x',iso,'-d',game_root],log,report)
        else:game_root=game_root.resolve()
        required_files=['default.xex']
        if 'core' in groups:required_files += ['data/big/miscload.big','data/big/miscboot.big','data/big/db.big']
        if 'character' in groups:required_files += ['data/content/createacharacter.big']
        for required in required_files:
            if not (game_root/required).is_file():raise RuntimeError('This is not a supported Skate 3 disc: missing '+required)
        source_hash=digest(game_root/'default.xex')
        if previous and previous[1].get('source_hash',source_hash)!=source_hash:
            raise RuntimeError('Select the same Xbox game edition used to set up this copy')
        stock=private/'stock'
        if 'core' in groups:
            # Keep the established exporter call shape when no private runtime image was supplied;
            # setup tests and third-party exporters can continue to implement the original contract.
            if audio_image is None:
                converted=exports.core(game_root,stage,work,report,log)
            else:
                converted=exports.core(game_root,stage,work,report,log,audio_image=audio_image)
        else:
            converted=json.loads((stock/'skater-collections.json').read_text(encoding='utf-8'))
        if 'hud' in groups:
            exports.hud(game_root,stage,work,report,log)
        if 'character' in groups:
            exports.character(game_root,stage,work,report,log,converted)
        if 'environment' in groups:
            exports.environment(game_root,stage,work,report,log,converted)
        if 'maps' in groups:
            report('Preparing authored movable-object models')
            from .dynamic_props import prepare_catalog
            from .optional_content import CONTENT_ERRORS, note
            try:
                prepare_catalog(game_root,work/'dmo')
                (private/'native-props/props-availability.json').unlink(missing_ok=True)
            except CONTENT_ERRORS as error:
                if (work/'dmo').exists():remove_intermediate(work/'dmo',work)
                note(private/'native-props/props-availability.json','Movable props',error,report=report)
        report('Validating skater, input and animation data')
        run([game_exe,'--assets',stage/'assets','--test-world','--check-assets'],log,report)
        if 'maps' in groups:
            archives=list((game_root/'data/content').glob('worldDIST_*.big'))
            archives.sort(key=lambda p:(p.stem!='worldDIST_University',p.name.lower()))
            workers=map_workers()
            report(f'Converting {len(archives)} maps with {workers} workers')
            def map_job(archive):
                result=work/(archive.stem+'.json')
                with (stage/(archive.stem+'-conversion.log')).open('w',encoding='utf-8') as map_log:
                    run(task(TOOLS/'asset_pipeline/map_job.py','--archive',archive,'--stage',stage,
                             '--game-exe',game_exe,'--result',result),map_log,report)
                return json.loads(result.read_text(encoding='utf-8'))
            completed={}
            with ThreadPoolExecutor(max_workers=workers) as pool:
                # Start the expensive districts together so one does not
                # remain queued behind a string of small parks.
                futures={pool.submit(map_job,a):a for a in sorted(archives,key=lambda p:-p.stat().st_size)}
                for future in as_completed(futures):
                    archive=futures[future]
                    try:completed[archive.name]=future.result()
                    except CONTENT_ERRORS as error:
                        label=archive.stem.removeprefix('worldDIST_')
                        (maps/(label+'.skate')).unlink(missing_ok=True)
                        props=private/'native-props'/(label+'.skate')
                        if props.is_dir():remove_intermediate(props,private)
                        note(private/'map-status'/(label+'-availability.json'),label,error,report=report)
                        continue
                    (private/'map-status'/(completed[archive.name]['name']+'-availability.json')).unlink(missing_ok=True)
                    report(f"Converted {len(completed)}/{len(archives)} maps: {completed[archive.name]['name']}")
            catalog=[completed[a.name] for a in archives if a.name in completed]
            if previous:
                # An absent/failed source district may still have a usable old
                # converted copy. Validate its bytes AND load with this engine.
                for old in json.loads((previous[0]/'maps.json').read_text()):
                    if any(item['path']==old['path'] for item in catalog):continue
                    src=(previous[0]/old['path']).resolve()
                    if not src.is_relative_to((previous[0]/'maps').resolve()):raise ValueError('Invalid old map path')
                    if not src.is_file() or digest(src)!=old.get('sha256'):continue
                    try:run([game_exe,'--assets',stage/'assets','--map',src,'--check-assets'],log,report)
                    except CONTENT_ERRORS:continue
                    target=stage/old['path'];target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,target)
                    catalog.append(old)
                    note(private/'map-status'/(old['name']+'-availability.json'),old['name'],
                         RuntimeError('New source map unavailable; previous map passed validation'),retained=True,report=report)
            # Exclude invalid old retail maps from the runtime directory scan.
            valid_paths={item['path'] for item in catalog}
            if previous:
                for old in json.loads((previous[0]/'maps.json').read_text()):
                    if old['path'] not in valid_paths:(stage/old['path']).unlink(missing_ok=True)
            if not catalog:raise RuntimeError('No playable map could be prepared or recovered. Restore at least one worldDIST_*.big archive beside default.xex and retry; the previous installation has been kept.')
        report('Validating installed runtime inputs')
        run([game_exe,'--assets',stage/'assets','--test-world','--check-assets'],log,report)
        settings=stage/'settings';settings.mkdir(exist_ok=True)
        if not previous:
            (settings/'default-map.json').write_text(json.dumps(next((m['path'] for m in catalog if m['name']=='University'),catalog[0]['path'])),encoding='utf-8')
        if 'maps' in groups:
            (stage/'maps.json').write_text(json.dumps(catalog,indent=2),encoding='utf-8')
            selected=settings/'default-map.json'
            if selected.is_file() and not (stage/json.loads(selected.read_text())).is_file():
                selected.write_text(json.dumps(catalog[0]['path']))
        remove_intermediate(work,stage)
        if finalize:finalize(stage)
        from .optional_content import summary
        warnings=summary(stage)
        # Publish after core validation and all optional outcomes have been recorded.
        marker=base/'installation.json.new'
        marker.write_text(json.dumps({'version':1,'directory':'installations/'+install_id,'source':source,'source_hash':source_hash,'pipelines':target_versions,'outputs':outputs(stage)}),encoding='utf-8')
        marker.replace(base/'installation.json')
        report(f'Setup complete ({len(warnings)} unavailable/retained components; see setup-report.json)' if warnings else 'Setup complete')
        return stage
