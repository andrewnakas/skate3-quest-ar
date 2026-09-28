"""Optional bundled native RefPack backend; source-only tools retain Python."""
import ctypes
from pathlib import Path

_library=None
for path in (Path(__file__).with_name('refpack.dll'),Path(__file__).resolve().parents[2]/'target/native/refpack.dll'):
    if path.is_file():
        _library=ctypes.CDLL(str(path))
        _library.skate_refpack.argtypes=[ctypes.c_char_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t,ctypes.c_size_t,ctypes.c_bool]
        _library.skate_refpack.restype=ctypes.c_int
        break

def decode(data,size,start,early=False):
    if _library is None:return None
    output=ctypes.create_string_buffer(size)
    if _library.skate_refpack(data,len(data),output,size,start,early):
        raise ValueError('Malformed RefPack command or declared output size')
    return output.raw
