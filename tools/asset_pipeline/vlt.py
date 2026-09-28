"""Read Skate 3's big-endian, 64-bit AttribSys schema and collections.

Layout reference: NFSTools/VaultLib (MIT), ModernBase structures. Data comes
only from the user's BIN/VLT pair. Unknown names retain their numeric hash.
"""
from pathlib import Path
import hashlib
import json
import re
import struct

MASK = (1 << 64) - 1

def mix(a, b, c):
    for who, shift in [(0,43),(1,-9),(2,8),(0,38),(1,-23),(2,5),
                       (0,35),(1,-49),(2,11),(0,12),(1,-18),(2,22)]:
        if who == 0: a = ((a-b-c) ^ (c >> shift)) & MASK
        elif who == 1: b = ((b-c-a) ^ (a << -shift)) & MASK
        else: c = ((c-a-b) ^ (b >> shift)) & MASK
    return a,b,c

def hash64(text):
    if not text: return 0
    data = text.encode('ascii')
    a = b = 0xABCDEF0011223344
    c = 0x9E3779B97F4A7C13
    pos = 0
    while len(data)-pos >= 24:
        x,y,z=struct.unpack_from('<QQQ',data,pos)
        a,b,c=mix((a+x)&MASK,(b+y)&MASK,(c+z)&MASK)
        pos += 24
    tail=data[pos:]
    a=(a+int.from_bytes(tail[:8],'little'))&MASK
    b=(b+int.from_bytes(tail[8:16],'little'))&MASK
    c=(c+len(data)+(int.from_bytes(tail[16:],'little')<<8))&MASK
    return mix(a,b,c)[2]

def take(data, at, size):
    if at < 0 or size < 0 or at+size>len(data):
        raise ValueError(f'Truncated AttribSys record at {at:#x}, size {size}')
    return data[at:at+size]

def unpack(fmt,data,at):
    return struct.unpack('>'+fmt,take(data,at,struct.calcsize('>'+fmt)))

def array_items(data, pos, size, alignment):
    """ModernBase array: four u16 header lanes, then aligned elements.

    Alignment in the schema is log2(bytes), not the array header's mask.
    Retain each element separately: padding is not part of its value.
    """
    capacity, count, stride, _ = unpack('4H', data, pos)
    if count > capacity or stride != size or not stride or alignment > 16:
        raise ValueError(f'Invalid VLT array at {pos:#x}')
    boundary = 1 << alignment
    cursor = pos + 8
    items = []
    for index in range(capacity):
        cursor = (cursor + boundary - 1) & -boundary
        raw = take(data, cursor, stride)
        if index < count:
            items.append(raw.hex().upper())
        cursor += stride
    return {'capacity': capacity, 'element_size': stride,
            'alignment': boundary, 'items': items}


def array_text_items(binary, items):
    values = []
    for raw in items:
        ptr, = unpack('I', bytes.fromhex(raw), 0)
        if not ptr:
            values.append('')
            continue
        end = binary.find(b'\0', ptr)
        if end < 0:
            raise ValueError(f'Unterminated VLT array text at {ptr:#x}')
        values.append(take(binary, ptr, end-ptr).decode('utf-8'))
    return values


def chunks(data):
    at=0
    while at<len(data):
        tag,size=unpack('II',data,at)
        if size<8: raise ValueError('Invalid AttribSys chunk length')
        take(data,at,size)
        yield tag,at+8,size-8
        at+=size
        if tag==0x456e6443: break

def vault(stem):
    v=bytearray(stem.with_suffix('.vlt').read_bytes())
    b=bytearray(stem.with_suffix('.bin').read_bytes())
    exports=[]
    for tag,at,size in chunks(v):
        if tag==0x5074724e:
            target=b
            for p in range(at,at+size,16):
                offset,kind,index,dest=unpack('IHHQ',v,p)
                if kind==0: break
                if kind==2: target=v if index==0 else b
                elif kind in (1,3):
                    take(target,offset,4)
                    struct.pack_into('>I',target,offset,0 if kind==1 else dest)
                elif kind!=4: raise ValueError(f'Unknown VLT pointer kind {kind}')
        elif tag==0x4578704e:
            count,=unpack('Q',v,at)
            if count>(size-8)//24: raise ValueError('Invalid VLT export count')
            exports=[unpack('QQII',v,at+8+i*24) for i in range(count)]
    return v,b,exports

def convert(schema_stem, collections_stem, names):
    sv,sb,se=vault(schema_stem)
    cv,cb,ce=vault(collections_stem)
    strings=set(names)
    for data in (sb,cb):
        strings.update(s.decode('ascii') for s in re.findall(rb'[A-Za-z_][A-Za-z0-9_:./ -]{1,180}\x00',bytes(data)) for s in [s[:-1]])
    lookup={hash64(s):s for s in strings}
    def name(key): return lookup.get(key,f'Hash_{key:016X}') if key else ''
    classes={}
    for _,kind,size,at in se:
        if kind!=0x2A7895AC4A876152: continue
        key,reserve,count,defs,static_size,static,layout=unpack('QIIIIII',sv,at)
        fields={}
        for i in range(count):
            fkey,typ,offset,n,maximum,flags,alignment=unpack('QQHHHBB',sb,defs+i*24)
            fields[fkey]=(typ,offset,n,maximum,flags,alignment)
        classes[key]=(fields,static)
    rows=[]
    source_hash=hashlib.sha256(collections_stem.with_suffix('.vlt').read_bytes()).hexdigest()
    for _,kind,size,at in ce:
        if kind!=0xAD303B8F42B3307E: continue
        key,cls,parent,reserve,pad,count,ntypes,typeslen,layout,pad2=unpack('QQQIIIHHII',cv,at)
        fields,static=classes[cls]
        out={}
        def value(fkey,data,pos,inline=False):
            typ,offset,n,maximum,flags,alignment=fields[fkey]
            t=name(typ)
            length=4 if inline else n
            if t=='EA::Reflection::Text' and not flags&1:
                ptr,=unpack('I',data,pos)
                if ptr:
                    end=cb.find(b'\x00',ptr)
                    if end<0: raise ValueError(f'Unterminated VLT text at {pos:x}: ptr={ptr:x} field={name(fkey)} size={n}')
                    raw=take(cb,ptr,end-ptr).decode('utf-8')
                else: raw=''
                if raw.isascii(): lookup[hash64(raw)]=raw
                out[name(fkey)]={'type':t,'data':raw}
            else:
                raw=take(data,pos,length)
                out[name(fkey)]={'type':t,'data':raw.hex().upper()}
                if flags&1:
                    # Keep the legacy data field unchanged for existing
                    # gameplay consumers. New readers use the full array.
                    out[name(fkey)]['array']=array_items(data,pos,n,alignment)
                    if t=='EA::Reflection::Text':
                        array=out[name(fkey)]['array']
                        array['text_items']=array_text_items(cb,array['items'])
        for fk,f in fields.items():
            if f[4]&2 and layout: value(fk,cb,layout+f[1])
        entries=at+48+typeslen*8
        for i in range(count):
            pos=entries+i*16
            fk,ptr,ti,node_flags,entry_flags=unpack('QI HBB',cv,pos)
            if fk not in fields: raise ValueError(f'Unknown schema field {fk:x}')
            f=fields[fk]
            if f[2]<=4 and not f[4]&1: value(fk,cv,pos+8,True)
            else: value(fk,cb,ptr)
        rows.append({'class':name(cls),'key':key,'parent':parent,'fields':out,
                     'source':'skatercollections.vlt','sha256':source_hash})
    for row in rows:
        row['key']=name(row['key'])
        row['parent']=name(row['parent'])
    return {'version':1,'collections':rows}
