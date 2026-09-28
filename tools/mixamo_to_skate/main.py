"""Windows drag-and-drop / file-dialog entry point, also usable from the CLI."""
import argparse
import json
import os
import sys
from pathlib import Path

from converter import convert_file, create_profile, sha
from glb import ConversionError, require


def app_root():
    # Release setup carries source + native resources under its extraction root.
    # A standalone frozen converter retains its legacy adjacent resource layout.
    bundled = Path(__file__).resolve().parent
    if (bundled/'tools/FBX2glTF.exe').is_file():
        return bundled
    return Path(sys.executable).parent if getattr(sys,'frozen',False) else bundled


def find_reference(expected_hash=None):
    base = Path(os.environ.get('LOCALAPPDATA',Path.home()/'AppData/Local'))/'Skate3RustEngine/installations'
    candidates = sorted(base.glob('*/assets/private/skater.glb'))
    if expected_hash:
        candidates = [p for p in candidates if sha(p)==expected_hash]
        if candidates:
            return candidates[0]
    if len(candidates)==1:
        return candidates[0]
    if candidates and len({sha(p) for p in candidates})==1:
        return candidates[0]
    return None


def main(argv=None):
    parser = argparse.ArgumentParser(description='Convert a Mixamo humanoid to the installed Skate render rig.')
    parser.add_argument('files',nargs='*',type=Path)
    parser.add_argument('--reference',type=Path,help='Owned stock assets/private/skater.glb')
    parser.add_argument('--profile',type=Path,help='Optional local paired-stock calibration JSON')
    parser.add_argument('--output',type=Path,help='New output folder (single input only)')
    parser.add_argument('--fbx-tool',type=Path)
    parser.add_argument('--no-board',action='store_true',help='Output character only, for future modular import')
    parser.add_argument('--calibrate',action='store_true',help='Create profile from a paired stock Mixamo GLB')
    parser.add_argument('--noninteractive',action='store_true')
    parser.add_argument('--library-import',type=Path,help='Import into the game character library')
    parser.add_argument('--result',type=Path,help='Game import result file')
    args = parser.parse_args(argv)
    if args.library_import:
        from library_import import run
        return run(args,app_root())
    gui = not args.noninteractive and (not args.files or getattr(sys,'frozen',False))
    root = None
    try:
        if gui:
            import tkinter as tk
            from tkinter import filedialog, messagebox
            root = tk.Tk()
            root.withdraw()
            if not args.files:
                args.files = [Path(f) for f in filedialog.askopenfilenames(title='Choose Mixamo FBX characters',
                    filetypes=[('Mixamo characters','*.fbx *.glb')])]
                if not args.files:
                    return 0
        profile_path = args.profile or app_root()/'calibration.json'
        profile = json.loads(profile_path.read_text(encoding='utf-8')) if profile_path.is_file() else None
        reference = args.reference or find_reference(profile.get('reference_sha256') if profile else None)
        if reference is None and gui:
            selected = filedialog.askopenfilename(title='Choose your installed stock skater.glb',
                                                 filetypes=[('Stock GLB','*.glb')])
            reference = Path(selected) if selected else None
        require(reference is not None,'Stock reference not found. Use --reference path/to/skater.glb')
        require(args.files,'No input selected')
        require(not args.output or len(args.files)==1,'--output supports one input only')
        if args.calibrate:
            require(len(args.files)==1 and args.files[0].suffix.lower()=='.glb','Calibration requires one normalized Mixamo GLB')
            require(not profile_path.exists(),'Calibration exists; choose another --profile path')
            profile_path.write_text(json.dumps(create_profile(args.files[0],reference),indent=2),encoding='utf-8')
            print('Calibration saved:',profile_path)
            return 0
        tool = args.fbx_tool or app_root()/'tools/FBX2glTF.exe'
        outputs = []
        for source in args.files:
            destination = args.output
            if destination is None:
                destination = source.with_name(source.stem+'-skate')
                counter = 2
                while destination.exists():
                    destination = source.with_name(source.stem+f'-skate-{counter}')
                    counter += 1
            print('Converting:',source,flush=True)
            result = convert_file(source,reference,destination,tool,profile,not args.no_board)
            outputs.append(str(result))
            print('Converted and validated:',result,flush=True)
        if gui:
            messagebox.showinfo('Conversion complete','Saved:\n\n'+'\n'.join(outputs)+
                '\n\nThe game was not modified. These files still need visual/gameplay review.')
        return 0
    except Exception as error:
        print('Conversion failed:',str(error),file=sys.stderr)
        if gui and root is not None:
            messagebox.showerror('Conversion failed',str(error)+'\n\nExisting outputs and game files were not replaced.')
        return 1
    finally:
        if root is not None:
            root.destroy()


if __name__=='__main__':
    raise SystemExit(main())
