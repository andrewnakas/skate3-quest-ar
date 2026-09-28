"""Generate three original test triangles, without reading any game assets.

Run from the repository root. The fixture covers float, 16-bit offset and
32-bit integer vertex encodings; its IDs and edge flags are test values.
"""
import struct
from pathlib import Path
records=[]
for mode in range(3):
 c=bytearray(96);struct.pack_into('>HHH',c,0,1,11,4);struct.pack_into('>H',c,8,96);c[10]=3;c[12]=mode
 points=[(mode*40,0,0),(mode*40,0,4),(mode*40+4,0,0)]
 if mode==0:
  for i,p in enumerate(points):struct.pack_into('>4f',c,16+i*16,*(v*.25 for v in p),0)
 elif mode==1:
  struct.pack_into('>3i',c,16,mode*40,0,0)
  for i,p in enumerate(points):struct.pack_into('>3h',c,28+i*6,p[0]-mode*40,p[1],p[2])
 else:
  for i,p in enumerate(points):struct.pack_into('>3i',c,16+i*12,*p)
 c[80:91]=bytes([0xe1,0,1,2,0x20,0x42,0x9a,0x34,0x12,0x21,0x43])
 m=bytearray(160);struct.pack_into('>3f',m,0,mode*10,0,0);struct.pack_into('>3f',m,16,mode*10+1,0,1)
 struct.pack_into('>I',m,40,1);struct.pack_into('>II',m,48,96,144);struct.pack_into('>fHBBI',m,56,.25,0x10,2,2,1);struct.pack_into('>I',m,80,256);struct.pack_into('>I',m,144,160);m+=c
 name=f'compression-{mode}'.encode();records.append(struct.pack('<I',len(name))+name+struct.pack('<I',len(m))+m)
 p=Path('crates/skate-data/tests/fixtures/retail-collision.rwcmset');p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b'RWCMSET1'+struct.pack('<I',3)+b''.join(records))
