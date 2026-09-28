"""First-run setup window and packaged Python conversion worker."""
from pathlib import Path
import argparse,os,queue,runpy,sys,threading,traceback

ROOT=Path(getattr(sys,'_MEIPASS',Path(__file__).resolve().parents[1]))
sys.path.insert(0,str(ROOT))

def main():
    if len(sys.argv)>1 and sys.argv[1]=='--character-import':
        # Keep the importer inside the already versioned setup payload: even
        # protocol-1 updaters deliver it atomically with the game executable.
        importer=ROOT/'tools/mixamo_to_skate'
        sys.path.insert(0,str(importer))
        from main import main as import_character
        return import_character(sys.argv[2:])
    if len(sys.argv)>2 and sys.argv[1]=='--task':
        script=Path(sys.argv[2])
        if not script.is_absolute():script=ROOT/script
        script=script.resolve()
        if not script.is_relative_to((ROOT/'tools').resolve()):raise RuntimeError('Invalid conversion script')
        sys.path.insert(0,str(script.parent))
        sys.argv=[str(script),*sys.argv[3:]]
        runpy.run_path(str(script),run_name='__main__')
        return 0
    parser=argparse.ArgumentParser()
    parser.add_argument('--base',type=Path,required=True)
    parser.add_argument('--game-exe',type=Path,required=True)
    parser.add_argument('--refresh',action='store_true')
    args=parser.parse_args()
    import tkinter as tk
    from tkinter import filedialog,messagebox,ttk
    from tools.asset_pipeline.customiser_setup import install
    from tools.asset_pipeline.versions import installed, fingerprints, changed_groups
    previous=installed(args.base) if args.refresh else None
    changed=changed_groups(previous[1].get('pipelines',{}),fingerprints()) if previous else set()
    updating=previous is not None
    window=tk.Tk()
    window.title('Skate 3 Rust Engine setup')
    window.geometry('700x420');window.resizable(False,False)
    icon=ROOT/'docs/images/skating-crab.ico'
    if icon.is_file():window.iconbitmap(str(icon))
    frame=ttk.Frame(window,padding=24);frame.pack(fill='both',expand=True)
    ttk.Label(frame,text='Update game assets' if updating else 'Set up Skate 3 Rust Engine',font=('Segoe UI',20)).pack(anchor='w',pady=(0,16))
    ttk.Label(frame,text=('Asset version changes: '+(', '.join(sorted(changed)) or 'checking prepared content')+'.\nOnly changed or incomplete groups will be prepared again.\nYour previous character data remains until preparation succeeds.\nSelect your Skate 3 default.xex (or ISO) to continue.\nKeep the game data beside default.xex.') if updating else 'Select your Skate 3 Xbox 360 ISO, or default.xex inside an\nextracted game folder. Keep the game data beside default.xex.\nSetup prepares the skater, customiser, animations and disc maps.\nNo other apps need installing.\n\nISO extraction needs internet access. Allow free disk space\nand time for the first conversion.',
              font=('Segoe UI',11),justify='left').pack(anchor='w')
    status=tk.StringVar(value='Choose your game to begin.')
    ttk.Label(frame,textvariable=status,wraplength=600).pack(anchor='w',pady=(18,8))
    progress=ttk.Progressbar(frame,mode='indeterminate');progress.pack(fill='x')
    messages=queue.Queue();running=False;success=False
    def start():
        nonlocal running
        iso=filedialog.askopenfilename(parent=window,title='Select your Skate 3 default.xex or Xbox 360 ISO',
            filetypes=[('Skate 3 game','default.xex *.iso'),('Skate 3 executable','default.xex'),('Xbox 360 ISO','*.iso')])
        if not iso:return
        button.config(state='disabled');running=True;progress.start()
        def work():
            try:
                installed_root=install(Path(iso),args.base,args.game_exe,lambda text:messages.put(('progress',text)),refresh=updating)
                from tools.asset_pipeline.optional_content import summary
                warnings=summary(installed_root)
                messages.put(('done',f'Ready with {len(warnings)} unavailable components. Details: {installed_root / "setup-report.json"}' if warnings else 'Ready'))
            except Exception as error:
                args.base.mkdir(parents=True,exist_ok=True)
                (args.base/'setup-error.log').write_text(traceback.format_exc(),encoding='utf-8')
                messages.put(('error',str(error)))
        threading.Thread(target=work,daemon=True).start()
    def close():
        if running:
            messagebox.showinfo('Setup running','Wait for the current conversion to finish. Your source game files are not modified.',parent=window)
        else:window.destroy()
    buttons=ttk.Frame(frame);buttons.pack(anchor='e',pady=18)
    button=ttk.Button(buttons,text='Select ISO or default.xex',command=start);button.pack(side='left')
    def poll():
        nonlocal running,success
        while not messages.empty():
            kind,text=messages.get_nowait();status.set(text)
            if kind=='done':
                if text!='Ready':messagebox.showwarning('Setup completed with unavailable content',text,parent=window)
                running=False;success=True;progress.stop();window.destroy();return
            if kind=='error':
                running=False;progress.stop();button.config(state='normal')
                messagebox.showerror('Setup could not finish',text+'\n\nDetails: '+str(args.base/'setup-error.log'),parent=window)
        window.after(100,poll)
    window.protocol('WM_DELETE_WINDOW',close)
    window.after(100,poll);window.mainloop()
    return 0 if success else 2

if __name__=='__main__':raise SystemExit(main())
