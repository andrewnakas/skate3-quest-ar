"""Release updater protocol 1. No game assets, shell commands or archive extractall."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import subprocess
import threading
import time
import urllib.request
import urllib.error
import zipfile
import update_install as installer
from update_processes import close_programs

REPO = 'SK8-ENGINE/skate-3-rust-engine'
API = f'https://api.github.com/repos/{REPO}/releases'
PACKAGE = 'skate3rust-windows-x64.zip'
FILES = ('skate3rust.exe', 'support/skate3setup.exe', 'support/skate3update.exe',
         'steam-relay/skate-steam-relay.exe', 'steam-relay/steam_api64.dll', 'release.json')

PREFIX = 'skate3rust-windows-x64/'


class DownloadRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not newurl.startswith('https://'):
            raise ValueError('Insecure redirect')
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        # Private asset requests redirect to signed CDN URLs. Never forward a PAT.
        if redirected is not None:
            redirected.remove_header('Authorization')
        return redirected


def asset_url(asset):
    return asset['url'] if os.environ.get('SKATE_UPDATE_GITHUB_TOKEN') else asset['browser_download_url']


def atomic(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_suffix('.tmp')
    with temp.open('w', encoding='utf-8') as output:
        json.dump(value, output)
        output.flush()
        os.fsync(output.fileno())
    os.replace(temp, path)


def read_json(path, default=None):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except (OSError, ValueError):
        return default


def identity(m):
    if not isinstance(m, dict):
        raise ValueError('Invalid release metadata')
    if (m.get('schema') != 1 or m.get('target') != 'windows-x64'
            or m.get('repository') != REPO or not isinstance(m.get('build'), int)
            or m['build'] <= 0 or not isinstance(m.get('tag'), str)
            or not isinstance(m.get('revision'), str)
            or not re.fullmatch(r'[0-9a-f]{40}', m['revision'])):
        raise ValueError('Incompatible release metadata')
    return m['build']


def program_metadata(m):
    identity(m)
    installer.manifest_files(m)


def safe_local_paths(root):
    # Never follow a user junction/symlink into asset stores or another install.
    paths = [root / name for name in FILES]
    tx = root / '.update-transaction'
    paths += [tx / side / name for side in ('old', 'new') for name in FILES]
    for path in paths:
        while path != root:
            if path.is_symlink() or path.is_junction():
                raise ValueError('Updater program directories cannot contain links or junctions')
            path = path.parent


def fetch(url, cancel, limit, progress=lambda value: None):
    if cancel.is_set():
        raise InterruptedError('Cancelled')
    # URLs originate only from the fixed repository API, never notes/manifest.
    if not (url.startswith(API) or url.startswith(f'https://github.com/{REPO}/releases/download/')):
        raise ValueError('Unexpected download URL')
    headers = {'User-Agent': 'Skate3RustEngine-Updater/1', 'X-GitHub-Api-Version': '2022-11-28'}
    if url.startswith(API):
        headers['Accept'] = 'application/octet-stream' if '/assets/' in url else 'application/vnd.github+json'
        token = os.environ.get('SKATE_UPDATE_GITHUB_TOKEN')
        if token:
            headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(url, headers=headers)
    deadline = time.monotonic() + 300
    result = bytearray()
    try:
        response = urllib.request.build_opener(DownloadRedirect()).open(req, timeout=15)
    except urllib.error.HTTPError as error:
        if error.code in (403, 429):
            raise ValueError('GitHub rate limit or access restriction; try again later') from error
        if error.code == 404:
            raise ValueError('Releases are not publicly available, or private-release access is missing') from error
        raise
    with response:
        if not response.url.startswith('https://'):
            raise ValueError('Insecure redirect')
        while True:
            if cancel.is_set():
                raise InterruptedError('Cancelled')
            if time.monotonic() > deadline:
                raise TimeoutError('Download timed out')
            block = response.read(256 * 1024)
            if not block:
                return bytes(result)
            result.extend(block)
            if len(result) > limit:
                raise ValueError('Download exceeds size limit')
            progress(f'Downloaded {len(result) // (1024 * 1024)} MB')


def eligible(release, channel):
    return (not release.get('draft') and bool(release.get('published_at'))
            and (channel == 'Latest' or not release.get('prerelease')))


def package_name(meta):
    # Rolling asset names are derived from a validated integer, never a path
    # supplied by release notes or an arbitrary manifest field.
    return f'skate3rust-windows-x64-build-{identity(meta)}.zip' if meta['tag'] == 'experimental' else PACKAGE


def release_candidates(assets, rolling):
    if not rolling:
        return [('release.json', PACKAGE)]
    builds = sorted((int(m[1]) for name in assets
                     if (m := re.fullmatch(r'release-([1-9][0-9]*)\.json', name))), reverse=True)
    return [(f'release-{n}.json', f'skate3rust-windows-x64-build-{n}.zip') for n in builds]


def discover(current, channel, cancel, repair=False):
    current_build = identity(current)
    candidates = []
    deadline = time.monotonic() + 120
    # Exhaust pagination; never silently claim current after a truncated scan.
    for page in range(1, 101):
        if time.monotonic() > deadline:
            raise TimeoutError('Release check timed out')
        releases = json.loads(fetch(f'{API}?per_page=100&page={page}', cancel, 8 * 1024 * 1024))
        for release in releases:
            if time.monotonic() > deadline:
                raise TimeoutError('Release check timed out')
            if not eligible(release, channel):
                continue
            assets = {a['name']: a for a in release.get('assets', []) if a.get('state') == 'uploaded'}
            for manifest_name, package in release_candidates(assets, release.get('tag_name') == 'experimental'):
                if not {package, package + '.sha256', manifest_name} <= assets.keys():
                    continue
                try:
                    meta = json.loads(fetch(asset_url(assets[manifest_name]), cancel, 65536))
                    build = identity(meta)
                    program_metadata(meta)
                    if (meta['tag'] != release['tag_name'] or build < current_build or (build == current_build and not repair)
                            or package_name(meta) != package):
                        continue
                except (ValueError, KeyError):
                    continue
                candidates.append((build, release['id'], release, assets, meta))
        if len(releases) < 100:
            break
    else:
        raise ValueError('Too many release pages; check again later')
    return max(candidates, key=lambda item: item[:2]) if candidates else None


def stage(candidate, directory, cancel, progress):
    _, _, release, assets, meta = candidate
    package = package_name(meta)
    checksum = fetch(asset_url(assets[package + '.sha256']), cancel, 1024).decode('ascii').split()
    if len(checksum) != 2 or checksum[1] != package or not re.fullmatch('[0-9a-fA-F]{64}', checksum[0]):
        raise ValueError('Invalid release checksum')
    archive = fetch(asset_url(assets[package]), cancel, 1024 * 1024 * 1024, progress)
    digest = hashlib.sha256(archive).hexdigest()
    if digest != checksum[0].lower():
        raise ValueError('Package checksum mismatch')
    api_digest = assets[package].get('digest')
    if api_digest and api_digest != 'sha256:' + digest:
        raise ValueError('GitHub asset digest mismatch')
    import io
    with zipfile.ZipFile(io.BytesIO(archive)) as z:
        names = set()
        total = 0
        for info in z.infolist():
            name = info.filename
            parts = name.rstrip('/').split('/')
            if (not name.startswith(PREFIX) or '\\' in name or ':' in name
                    or any(p in ('', '.', '..') for p in parts)
                    or name.lower() in names or (info.external_attr >> 16) & 0o170000 == 0o120000):
                raise ValueError('Unsafe archive path')
            names.add(name.lower())
            total += info.file_size
            if total > 2 * 1024 * 1024 * 1024:
                raise ValueError('Expanded package too large')
        for name in [*installer.manifest_files(meta), "release.json"]:
            installer.safe_paths(directory.parent, [name])
            if cancel.is_set():
                raise InterruptedError('Cancelled')
            payload = z.read(PREFIX + name)
            if name != 'release.json':
                if hashlib.sha256(payload).hexdigest() != meta['files'][name]:
                    raise ValueError('Program checksum mismatch')
            elif json.loads(payload.decode('utf-8-sig')) != meta:
                raise ValueError('Package identity mismatch')
            dest = directory / 'new' / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(payload)


def retry(operation):
    deadline = time.monotonic() + 45
    while True:
        try:
            return operation()
        except OSError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(.25)


def rollback(root, tx):
    installer.rollback(root, tx)


def install(root, tx):
    installer.install(root, tx)


def main(request=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--request', type=Path, required=True)
    args = parser.parse_args()
    request = request or read_json(args.request)
    args.request.unlink(missing_ok=True)
    root = Path(request['root']).resolve()
    safe_local_paths(root)
    tx = root / '.update-transaction'
    # OS releases the lock after crashes; concurrent game instances cannot install.
    lock = (root / '.update.lock').open('a+b')
    lock.write(b'0')
    lock.flush()
    lock.seek(0)
    if os.name == 'nt':
        import msvcrt
        try:
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        except OSError:
            return
    if request.get('recover'):
        time.sleep(3)
        try:
            rollback(root, tx)
        except Exception as error:
            import tkinter.messagebox
            tkinter.messagebox.showerror('Update recovery', 'Could not restore the previous program. Close other game instances and try again. Backups remain in .update-transaction/old.\n' + str(error))
            return
        lock.close()
        subprocess.Popen([str(root / 'skate3rust.exe'), *request['args']], cwd=request['cwd'])
        return
    import tkinter as tk
    from tkinter import ttk
    win = tk.Tk()
    win.title('Skate 3 Rust Engine — Updates')
    win.geometry('660x520')
    automatic = request['automatic']
    if automatic:
        win.withdraw()
    settings_path = Path(os.environ.get('LOCALAPPDATA', str(Path.home()))) / 'Skate3RustEngine/settings/updates.json'
    settings = read_json(settings_path, {})
    channel = tk.StringVar(value=settings.get('channel') if settings.get('channel') in ('Stable', 'Latest') else 'Stable')
    status = tk.StringVar(value='Ready to check')
    ttk.Label(win, text='Updates — Update downloads, then closes and restarts the game.').pack(pady=8)
    choice = ttk.Combobox(win, textvariable=channel, values=('Stable', 'Latest'), state='readonly')
    choice.pack()
    ttk.Label(win, text='Stable: published releases. Latest: rolling Experimental and newer releases.').pack()
    notes = tk.Text(win, wrap='word', height=19)
    notes.pack(fill='both', expand=True, padx=12, pady=8)
    notes.configure(state='disabled')
    ttk.Label(win, textvariable=status, wraplength=620).pack()
    events = queue.Queue()
    cancel = threading.Event()
    candidate = None
    busy = False
    generation = 0
    closing = False
    repair_files = []
    repair_after_update = False

    def work(kind, operation):
        nonlocal busy
        busy = True
        update.configure(state='disabled')
        check.configure(state='disabled')
        choice.configure(state='disabled')
        token = generation
        def run():
            try:
                events.put((token, kind, operation()))
            except Exception as e:
                events.put((token, 'error', str(e)))
        threading.Thread(target=run, daemon=True).start()

    def check_now():
        nonlocal candidate, generation
        generation += 1
        candidate = None
        notes.configure(state='normal')
        notes.delete('1.0', 'end')
        notes.configure(state='disabled')
        cancel.clear()
        settings['channel'] = channel.get()
        settings[channel.get()] = time.time()
        try:
            atomic(settings_path, settings)
        except OSError:
            pass
        status.set('Checking GitHub…')
        selected = channel.get()
        def perform_check():
            nonlocal repair_files, repair_after_update
            current = read_json(root / 'release.json', {})
            program_metadata(current)
            with (root / 'skate3rust.exe').open('rb') as executable:
                digest = hashlib.file_digest(executable, 'sha256').hexdigest()
            if (current['revision'] != request['revision'] or str(current['build']) != request['build']
                    or digest != current['files']['skate3rust.exe']):
                raise ValueError('This executable does not match its release metadata')
            repair_files = installer.mismatches(root, current)
            previous = read_json(tx/'old/release.json', {})
            repair_after_update = (bool(repair_files) and isinstance(previous.get('build'), int)
                                   and previous['build'] < current['build'])
            return discover(current, selected, cancel, repair=bool(repair_files))
        work('checked', perform_check)

    def do_update():
        nonlocal automatic
        automatic = False
        if candidate is None or busy:
            return
        cancel.clear()
        def prepare():
            if (tx / 'journal.json').exists():
                raise ValueError('An interrupted update needs recovery. Restart the game first.')
            tx.mkdir(exist_ok=True)
            stage(candidate, tx, cancel, lambda text: events.put((generation, 'progress', text)))
        status.set('Downloading and verifying. Cancel keeps the game running.')
        work('staged', prepare)

    def close():
        nonlocal closing
        if busy:
            cancel.set()
            closing = True
            status.set('Cancelling…')
        else:
            win.destroy()

    check = ttk.Button(win, text='Check now', command=check_now)
    check.pack(side='left', padx=12, pady=12)
    update = ttk.Button(win, text='Update', command=do_update, state='disabled')
    update.pack(side='left', padx=12)
    back = ttk.Button(win, text='Cancel', command=close)
    back.pack(side='right', padx=12)
    win.protocol('WM_DELETE_WINDOW', close)
    choice.bind('<<ComboboxSelected>>', lambda _: check_now())

    def poll():
        nonlocal candidate, busy, automatic
        try:
            while True:
                token, kind, value = events.get_nowait()
                if token != generation:
                    continue
                if kind == 'progress':
                    status.set(value)
                    continue
                busy = False
                if closing:
                    win.destroy()
                    return
                check.configure(state='normal')
                choice.configure(state='readonly')
                if kind == 'error':
                    atomic(root / '.update-error.json', {'time': time.time(), 'error': value})
                    back.configure(state='normal')
                    win.protocol('WM_DELETE_WINDOW', close)
                    status.set('Update unavailable: ' + value[:250] + '. You can keep playing and try again later.')
                    if automatic:
                        win.destroy()
                        return
                elif kind == 'checked':
                    candidate = value
                    if candidate:
                        automatic = False
                        win.deiconify()
                        notes.configure(state='normal')
                        notes.delete('1.0', 'end')
                        notes.insert('end', candidate[4]['tag'] + '\n\n' + (candidate[2].get('body') or 'No release notes.')[:50000])
                        notes.configure(state='disabled')
                        update.configure(state='normal', text='Repair' if repair_files else 'Update')
                        status.set(('Repair needed: ' + ', '.join(repair_files) + '. Download will repair the installation.') if repair_files else 'New release available. Cancel keeps your current version.')
                        if repair_after_update and candidate[0] == identity(read_json(root/'release.json', {})):
                            # Finish the update the user already accepted in the old helper.
                            do_update()
                    elif automatic:
                        win.destroy()
                        return
                    else:
                        status.set('Repair needed, but this build is not available in the selected channel. Select its channel or a newer release.' if repair_files else 'No newer compatible release in this channel.')
                elif kind == 'staged':
                    if cancel.is_set():
                        continue
                    status.set('Closing game and installing… Please keep this window open.')
                    back.configure(state='disabled')
                    win.protocol('WM_DELETE_WINDOW', lambda: None)
                    def finish():
                        Path(request['signal']).write_text('ready', encoding='ascii')
                        # Allow normal shutdown, then release stale processes owned by this copy.
                        time.sleep(3)
                        close_programs(root, set(installer.manifest_files(read_json(root/'release.json', {})))
                                       | set(installer.manifest_files(read_json(tx/'new/release.json', {}))))
                        try:
                            install(root, tx)
                        except Exception:
                            if not (tx / 'journal.json').exists():
                                subprocess.Popen([str(root / 'skate3rust.exe'), *request['args']], cwd=request['cwd'])
                            raise
                        lock.close()
                        subprocess.Popen([str(root / 'skate3rust.exe'), *request['args']], cwd=request['cwd'])
                    work('installed', finish)
                elif kind == 'installed':
                    win.destroy()
                    return
        except queue.Empty:
            pass
        win.after(100, poll)

    needs_repair = bool(installer.mismatches(root, read_json(root/'release.json', {})))
    if needs_repair:
        automatic = False
        win.deiconify()
    if not automatic or needs_repair or time.time() - settings.get(channel.get(), 0) >= 21600:
        check_now()
    else:
        win.destroy()
        return
    win.after(100, poll)
    win.mainloop()


if __name__ == '__main__':
    # Suppress PyInstaller's generic exception/crash dialog for routine updater
    # failures, including unwritable portable directories before UI creation.
    import sys
    request = read_json(Path(sys.argv[2]), {}) if len(sys.argv) == 3 and sys.argv[1] == '--request' else {}
    try:
        main(request)
    except Exception as error:
        if request and not request.get('automatic', True):
            try:
                import tkinter.messagebox
                tkinter.messagebox.showerror('Updates unavailable', str(error)[:300] + '\nYou can keep playing. Try again later.')
            except Exception:
                pass
