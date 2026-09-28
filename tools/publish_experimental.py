"""Publish one rolling prerelease, with complete build-specific asset sets."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import zipfile

from updater import REPO, PACKAGE, PREFIX, identity, program_metadata, package_name

TAG = 'experimental'


def gh(*args, payload=None):
    command = ['gh', *args]
    if payload is not None:
        command += ['--input', '-']
    result = subprocess.run(command, input=json.dumps(payload) if payload is not None else None,
                            text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or 'GitHub command failed')
    return result.stdout


def api(path, method='GET', payload=None):
    return json.loads(gh('api', f'repos/{REPO}/{path}', '--method', method, payload=payload) or 'null')


def complete_builds(assets):
    names = {a['name'] for a in assets if a.get('state') == 'uploaded'}
    result = []
    for name in names:
        match = re.fullmatch(r'release-([1-9][0-9]*)\.json', name)
        if match:
            build = int(match[1])
            package = f'skate3rust-windows-x64-build-{build}.zip'
            if {package, package + '.sha256'} <= names:
                result.append(build)
    return sorted(result, reverse=True)


def validate(directory):
    meta = json.loads((directory/'release.json').read_text(encoding='utf-8-sig'))
    identity(meta)
    program_metadata(meta)
    if meta['tag'] != TAG:
        raise ValueError('Expected experimental package')
    archive = directory/PACKAGE
    with archive.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    checksum = (directory/(PACKAGE+'.sha256')).read_text().split()
    if checksum != [digest, PACKAGE]:
        raise ValueError('Package checksum mismatch')
    with zipfile.ZipFile(archive) as z:
        if json.loads(z.read(PREFIX+'release.json').decode('utf-8-sig')) != meta:
            raise ValueError('Package identity mismatch')
        for name, expected in meta['files'].items():
            if hashlib.sha256(z.read(PREFIX+name)).hexdigest() != expected:
                raise ValueError('Program checksum mismatch')
    return meta, digest


def publish(directory):
    meta, digest = validate(directory)
    if api('git/ref/heads/main')['object']['sha'] != meta['revision']:
        print('A newer main commit exists; leaving Experimental unchanged.')
        return
    # Include drafts so interrupted first publication can resume without another release.
    releases = json.loads(gh('api', f'repos/{REPO}/releases?per_page=100', '--paginate', '--slurp'))
    release = next((r for page in releases for r in page if r['tag_name'] == TAG), None)
    if release and (release.get('immutable') or not release.get('prerelease')):
        raise ValueError('Experimental must be a mutable prerelease')
    if release and any(n >= meta['build'] for n in complete_builds(release.get('assets', []))) and not release['draft']:
        print('This build or a newer build is already published.')
        return
    if release is None:
        release = api('releases', 'POST', dict(tag_name=TAG, target_commitish=meta['revision'],
                      name='Experimental', draft=True, prerelease=True, make_latest='false'))
    package = package_name(meta)
    shutil.copyfile(directory/PACKAGE, directory/package)
    (directory/(package+'.sha256')).write_text(digest+'  '+package+'\n', encoding='ascii')
    manifest = f"release-{meta['build']}.json"
    shutil.copyfile(directory/'release.json', directory/manifest)
    # Payload/checksum first, manifest last: clients ignore an incomplete upload.
    for name in (package, package+'.sha256', manifest):
        gh('release', 'upload', TAG, str(directory/name), '--repo', REPO, '--clobber')
    current = api(f"releases/{release['id']}")
    if meta['build'] not in complete_builds(current['assets']):
        raise ValueError('Uploaded build is incomplete')
    changes = gh('api', f'repos/{REPO}/commits?sha={meta["revision"]}&per_page=10')
    commits = json.loads(changes)
    notes = (f"Rolling build **{meta['build']}**, commit `{meta['revision'][:12]}`.\n\n"
             f"[Download Windows ZIP](https://github.com/{REPO}/releases/download/{TAG}/{package})\n\n"
             "Experimental builds may contain regressions. Choose Latest in the in-game Updates window "
             "to receive these builds. Stable remains the default. Your existing maps and assets are kept.\n\n"
             "### Recent commits\n\n" + '\n'.join(
                 f"- {c['sha'][:8]} {c['commit']['message'].splitlines()[0]}" for c in commits))
    api(f"releases/{release['id']}", 'PATCH', dict(name='Experimental', body=notes,
        target_commitish=meta['revision'], draft=False, prerelease=True, make_latest='false'))
    api('git/refs/tags/'+TAG, 'PATCH', dict(sha=meta['revision'], force=True))
    # Keep the previous complete download for clients already fetching it.
    keep = set(complete_builds(current['assets'])[:2])
    for asset in current['assets']:
        match = re.fullmatch(r'(?:release-([1-9][0-9]*)\.json|skate3rust-windows-x64-build-([1-9][0-9]*)\.zip(?:\.sha256)?)', asset['name'])
        if match and int(match[1] or match[2]) not in keep:
            api(f"releases/assets/{asset['id']}", 'DELETE')
    print(f"Published Experimental build {meta['build']}")


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--directory', type=Path, required=True)
    publish(parser.parse_args().directory)
