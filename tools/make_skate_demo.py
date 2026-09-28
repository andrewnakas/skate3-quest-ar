"""Write a small, original v8 map for validating the adapter (no private assets)."""
import pathlib
import struct

out = bytearray(b"SKATE08\0")
def u(*values):
    out.extend(struct.pack("<" + "I" * len(values), *values))
def f(*values):
    out.extend(struct.pack("<" + "f" * len(values), *values))
def s(value):
    data = value.encode("utf-8")
    u(len(data))
    out.extend(data)

u(0x12345678)
s("SKATE Format Demo")
f(0, 0, 0, 0)
f(.09,.34,.72, .58,.78,.98, .18,.25,.34, 0,12,.62, 17,0)
f(.045,.10,.26, 1,.32,.10, .05,.035,.06)
f(.007,.015,.045, .045,.085,.17, .008,.014,.032)
f(1,.92,.78, .42,.56,.92, 1.25,.18,.32,.11, 1,1,1)
u(1, 2, 4, 6, 2, 0, 0, 0, 0)
s("Checker floor")
u(1)
f(.6, .1, 1,1,1, .85,0)
u(1,2)
f(1)
u(0,0,0,0)
f(.5)
u(3,1,0)
s("Albedo checker")
u(2,2,1,16)
out.extend(bytes([220,220,220,255, 50,90,150,255, 50,90,150,255, 220,220,220,255]))
s("Indirect constant")
u(1,1,0,4)
out.extend(bytes([64,64,64,255]))
points = [(-30,0,-30),(-30,0,30),(30,0,30),(30,0,-30)]
for i, p in enumerate(points):
    f(*p, 0,1,0, (0 if i<2 else 30), (30 if i in (1,2) else 0), .5,.5)
    u(1)
indices = [0,1,2,0,2,3]
u(*indices)
for i in range(0,6,3):
    for index in indices[i:i+3]:
        f(*points[index])
    u(i//3+1,1)
target = pathlib.Path(__file__).resolve().parents[1] / "maps" / "format-demo.skate"
target.parent.mkdir(exist_ok=True)
target.write_bytes(out)
print(f"Wrote {target} ({len(out)} bytes)")
