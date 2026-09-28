"""Manifest-driven program updates. User data is never release-owned."""
import hashlib
import json
import os
import re
import shutil
import time

PROTECTED = {'data', 'assets', 'maps', 'logs', 'saves', 'settings', 'mods', 'custom-models', 'screenshots'}
REQUIRED = {'skate3rust.exe', 'support/skate3setup.exe', 'support/skate3update.exe'}

def safe_name(name):
    if not isinstance(name, str) or not name or '\\' in name or ':' in name or name.startswith('/'):
        raise ValueError('Unsafe program path')
    parts = name.split('/')
    if any(not p or p in ('.', '..') or p[-1] in ' .' or any(ord(c)<32 for c in p)
           or p.split('.')[0].upper() in {'CON','PRN','AUX','NUL',*(f'COM{i}' for i in range(10)),*(f'LPT{i}' for i in range(10))} for p in parts):
        raise ValueError('Unsafe program path')
    if parts[0].lower() in PROTECTED or parts[0].startswith('.'):
        raise ValueError('Release cannot manage user data')
    return name

def manifest_files(meta):
    files = meta.get('files')
    if not isinstance(files, dict) or not REQUIRED <= files.keys() or len(files)>20000:
        raise ValueError('Missing program hashes')
    seen = set()
    for name, digest in files.items():
        safe_name(name)
        if name.lower()=='release.json' or name.lower() in seen or not isinstance(digest,str) or not re.fullmatch('[0-9a-f]{64}',digest):
            raise ValueError('Invalid program manifest')
        seen.add(name.lower())
    for name in seen:
        if any('/'.join(name.split('/')[:i]) in seen for i in range(1,len(name.split('/')))):
            raise ValueError('Program file/directory conflict')
    return files

def safe_paths(root, names):
    for name in names:
        safe_name(name)
        for path in (root/name, root/'.update-transaction/new'/name, root/'.update-transaction/old'/name):
            while path != root:
                if path.is_symlink() or path.is_junction():
                    raise ValueError('Updater paths cannot contain links or junctions')
                path=path.parent

def mismatches(root, meta):
    files=manifest_files(meta)
    safe_paths(root, files)
    bad=[]
    for name,digest in files.items():
        try:
            with (root/name).open('rb') as f: actual=hashlib.file_digest(f,'sha256').hexdigest()
        except OSError: actual=None
        if actual!=digest: bad.append(name)
    return bad

def read(path, default=None):
    try: return json.loads(path.read_text(encoding='utf-8-sig'))
    except (OSError,ValueError): return default

def atomic(path, value):
    path.parent.mkdir(parents=True,exist_ok=True)
    temp=path.with_suffix('.tmp')
    with temp.open('w',encoding='utf-8') as f:
        json.dump(value,f);f.flush();os.fsync(f.fileno())
    os.replace(temp,path)

def retry(op):
    deadline=time.monotonic()+45
    while True:
        try:return op()
        except OSError:
            if time.monotonic()>=deadline:raise
            time.sleep(.25)

def rollback(root,tx):
    journal=read(tx/'journal.json')
    if journal is None:return
    if journal.get('protocol')==1:
        # Earlier updaters only backed up this fixed list.
        names=['skate3rust.exe','support/skate3setup.exe','support/skate3update.exe',
               'steam-relay/skate-steam-relay.exe','steam-relay/steam_api64.dll','release.json']
        present=[n for n in names if (tx/'old'/n).is_file()]
    elif journal.get('protocol')==2:
        names=journal['names'];present=journal['present']
        if not isinstance(names,list) or not isinstance(present,list) or len(names)>40001 or not set(present)<=set(names):
            raise ValueError('Invalid recovery journal')
    else:raise ValueError('Unknown update journal')
    safe_paths(root,names)
    for name in reversed(names):
        dest=root/name
        if name in present:
            dest.parent.mkdir(parents=True,exist_ok=True)
            retry(lambda: shutil.copy2(tx/'old'/name,dest))
        else:retry(lambda: dest.unlink(missing_ok=True))
    (tx/'journal.json').unlink()

def install(root,tx):
    if (tx/'journal.json').exists():raise ValueError('Recover interrupted update first')
    meta=read(tx/'new/release.json',{})
    new=manifest_files(meta)
    old_meta=read(root/'release.json',{})
    old=manifest_files(old_meta)
    names=sorted(set(new)|set(old))+['release.json']
    safe_paths(root,names)
    if mismatches(tx/'new',meta):raise ValueError('Staged program checksum mismatch')
    present=[]
    # Keep the journal absent until every backup has completed.
    for name in names:
        src=root/name
        if src.exists():
            if not src.is_file():raise ValueError('Program path is not a file: '+name)
            backup=tx/'old'/name;backup.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(src,backup);present.append(name)
    atomic(tx/'journal.json',{'protocol':2,'names':names,'present':present})
    try:
        for name in names[:-1]:
            dest=root/name
            if name in new:
                dest.parent.mkdir(parents=True,exist_ok=True)
                retry(lambda: os.replace(tx/'new'/name,dest))
            else:retry(lambda: dest.unlink(missing_ok=True))
        if mismatches(root,meta):raise ValueError('Installed program checksum mismatch')
        # Publish version only after the entire program verifies.
        retry(lambda: os.replace(tx/'new/release.json',root/'release.json'))
        (tx/'journal.json').unlink()
    except Exception:
        rollback(root,tx)
        raise
