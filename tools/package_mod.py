"""Build a validated mod ZIP. Run from any directory; Rust performs authoritative checks."""
import argparse
import os
from pathlib import Path
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--target-dir', type=Path, help='Optional Cargo build cache')
    args = parser.parse_args()
    source, output = args.source.resolve(), args.output.resolve()
    if not source.is_dir() or not (source / 'mod.json').is_file():
        parser.error('Source must be a folder with mod.json at its root')
    if output.suffix.lower() != '.zip' or output.is_relative_to(source):
        parser.error('Output must be a .zip outside the source folder')
    command = ['cargo', 'run', '--quiet', '--locked', '-p', 'skate-mods', '--example', 'check_mod']
    if args.target_dir:
        command += ['--target-dir', str(args.target_dir.resolve())]
    subprocess.run(command + ['--', str(source)], cwd=ROOT, check=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix='.package-', suffix='.tmp', dir=output.parent)
    os.close(descriptor)
    try:
        with zipfile.ZipFile(temporary, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
            for path in sorted(source.rglob('*')):
                if path.is_symlink():
                    raise ValueError(f'Symlinks are not allowed: {path}')
                if path.is_file():
                    # Stable metadata keeps committed packages unchanged on a clean rebuild.
                    entry = zipfile.ZipInfo(path.relative_to(source).as_posix(), (1980, 1, 1, 0, 0, 0))
                    entry.create_system = 3
                    entry.external_attr = 0o100644 << 16
                    entry.compress_type = zipfile.ZIP_DEFLATED
                    archive.writestr(entry, path.read_bytes(), compresslevel=6)
        subprocess.run(command + ['--', temporary], cwd=ROOT, check=True)
        os.replace(temporary, output)
        print(f'Ready: {output} ({output.stat().st_size:,} bytes)')
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)

if __name__ == '__main__':
    main()
