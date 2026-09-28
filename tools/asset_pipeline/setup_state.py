"""Portable setup locking, source validation and immutable output receipts."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path


def source_directory(selected, require_core=True):
    selected = Path(selected).resolve()
    if selected.is_file() and selected.name.lower() == 'default.xex':
        selected = selected.parent
    elif not selected.is_dir():
        raise RuntimeError('Select your Skate 3 default.xex or Xbox 360 ISO')
    required = ('default.xex',)
    if require_core:
        required += ('data/big/miscload.big', 'data/big/miscboot.big',
                     'data/big/db.big', 'data/content/createacharacter.big')
    for name in required:
        if not (selected/name).is_file():
            raise RuntimeError('Keep the Skate 3 game content beside default.xex; missing '+name)
    return selected


@contextmanager
def setup_lock(base):
    base.mkdir(parents=True, exist_ok=True)
    # Kernel locks are released on process death. Keep the lock file in place:
    # unlinking it allows two processes to lock different file identities.
    with (base/'setup.lock').open('a+b') as stream:
        stream.seek(0, 2)
        if not stream.tell():
            stream.write(b'0'); stream.flush()
        stream.seek(0)
        try:
            if os.name == 'nt':
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            raise RuntimeError('Setup is already running for this copy') from error
        try:
            yield
        finally:
            stream.seek(0)
            if os.name == 'nt':
                msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(stream, fcntl.LOCK_UN)


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def receipt(root, paths):
    # Windows short names and directory junctions must use the same spelling
    # on both sides of relative_to; also reject outputs outside the root.
    root = root.resolve()
    paths = (p.resolve() for p in paths)
    return {p.relative_to(root).as_posix(): {'size': p.stat().st_size, 'sha256': sha(p)}
            for p in sorted(set(paths)) if p.is_file()}


def valid_receipt(root, files):
    if not isinstance(files, dict) or not files:
        return False
    try:
        for name, item in files.items():
            path = (root/name).resolve()
            if not path.is_relative_to(root.resolve()) or not path.is_file():
                return False
            if path.stat().st_size != item['size'] or sha(path) != item['sha256']:
                return False
        return True
    except (OSError, KeyError, TypeError, ValueError):
        return False


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix+'.tmp')
    temporary.write_text(json.dumps(value, separators=(',', ':')), encoding='utf-8')
    temporary.replace(path)
