"""Release file locks only for executables owned by this installation."""
import os

def close_programs(root, names):
    if os.name != 'nt': return
    import ctypes as c
    from ctypes import wintypes as w
    root=root.resolve()
    targets={os.path.normcase(str(root/name)) for name in names if name.lower().endswith('.exe')}
    k=c.WinDLL('kernel32',use_last_error=True)
    ps=c.WinDLL('psapi',use_last_error=True)
    k.OpenProcess.argtypes=[w.DWORD,w.BOOL,w.DWORD];k.OpenProcess.restype=w.HANDLE
    k.QueryFullProcessImageNameW.argtypes=[w.HANDLE,w.DWORD,w.LPWSTR,c.POINTER(w.DWORD)]
    k.TerminateProcess.argtypes=[w.HANDLE,w.UINT]
    k.WaitForSingleObject.argtypes=[w.HANDLE,w.DWORD]
    k.CloseHandle.argtypes=[w.HANDLE]
    ps.EnumProcesses.argtypes=[c.POINTER(w.DWORD),w.DWORD,c.POINTER(w.DWORD)]
    pids=(w.DWORD*65536)();used=w.DWORD()
    if not ps.EnumProcesses(pids,c.sizeof(pids),c.byref(used)):
        raise OSError('Could not check running game processes')
    for pid in pids[:used.value//c.sizeof(w.DWORD)]:
        if pid==os.getpid():continue
        # Query and terminate through the same handle, avoiding PID-reuse races.
        h=k.OpenProcess(0x1000|0x0001|0x100000,False,pid)
        if not h:continue
        try:
            text=c.create_unicode_buffer(32768);size=w.DWORD(len(text))
            if k.QueryFullProcessImageNameW(h,0,text,c.byref(size)) and os.path.normcase(text.value) in targets:
                if not k.TerminateProcess(h,0):raise OSError('Could not close '+text.value)
                if k.WaitForSingleObject(h,10000)!=0:raise OSError('Timed out closing '+text.value)
        finally:k.CloseHandle(h)
