"""Batch equivalents of rx2_parser's scalar Xbox texture decoders.

Keep the original palette rounding, swapped selector bytes, edge cropping
and zero padding. NumPy performs the per-block work outside Python loops.
"""
import numpy as np
from functools import lru_cache

@lru_cache(maxsize=16)
def _layout(units,width,pitch):
    offset=np.arange(units//width*width,dtype=np.int64)
    log=(pitch>>2)+((pitch>>1)>>(pitch>>2))
    b=offset<<log;t=((b&~4095)>>3)+((b&1792)>>2)+(b&63);m=t>>(7+log)
    aligned=(width+31)&~31
    x=(((m%(aligned>>5))<<2)+(((t>>(5+log)&2)+(b>>6))&3))<<3
    x+=((((t>>1)&~15)+(t&15))&((pitch<<3)-1))>>log
    y=(((m//(aligned>>5))<<2)+((t>>(6+log))&1)+((b&2048)>>10))<<3
    y+=(((t&(((pitch<<6)-1)&~31))+((t&15)<<1))>>(3+log))&~1
    y+=(t&16)>>4
    # Padded tile columns are outside the logical image. Flattening them
    # first aliases them onto later rows and overwrites valid blocks.
    dest=y*width+x;valid=(x<width)&(dest<units)
    # Match scalar last-write ordering even for narrow padded surfaces.
    source=np.full(units,-1,dtype=np.int64)
    np.maximum.at(source,dest[valid],offset[valid])
    return source

def _untile360(src,width_units,texel_pitch):
    units=len(src)//texel_pitch
    out=np.zeros(len(src),dtype=np.uint8)
    source=_layout(units,width_units,texel_pitch);valid=source>=0
    out[:units*texel_pitch].reshape(-1,texel_pitch)[valid]=np.frombuffer(src,dtype=np.uint8)[:units*texel_pitch].reshape(-1,texel_pitch)[source[valid]]
    return out.tobytes()

def _blocks(data,w,h,size):
    bw=(w+3)//4;bh=(h+3)//4;n=bw*bh
    a=np.zeros((n,size),dtype=np.uint8);available=min(n,len(data)//size)
    a[:available]=np.frombuffer(data,dtype=np.uint8,count=available*size).reshape(-1,size)
    return a,available,bw,bh

def _colour(b,transparent):
    c=b[:,:4].astype(np.uint32);c0=c[:,0]*256+c[:,1];c1=c[:,2]*256+c[:,3]
    def rgb(v):return np.stack((((v>>11)&31)*255//31,((v>>5)&63)*255//63,(v&31)*255//31),axis=1)
    # BC2/BC3 always use four colours. Only BC1 uses endpoint ordering
    # to select the three-colour + transparent-black palette.
    a=rgb(c0);z=rgb(c1);four=(c0>c1) | (not transparent)
    pal=np.zeros((len(b),4,4),dtype=np.uint8);pal[:,:,3]=255
    pal[:,0,:3]=a;pal[:,1,:3]=z
    pal[:,2,:3]=np.where(four[:,None],(2*a+z+1)//3,(a+z)//2)
    pal[:,3,:3]=np.where(four[:,None],(a+2*z+1)//3,0)
    if transparent:pal[~four,3,3]=0
    idx=((b[:,[5,4,7,6],None]>>np.arange(0,8,2,dtype=np.uint8))&3).reshape(-1,16)
    return pal[np.arange(len(b))[:,None],idx]

def _alpha(b):
    a=b[:,1].astype(np.uint32);z=b[:,0].astype(np.uint32);seven=a>z
    table=np.empty((len(b),8),dtype=np.uint8);table[:,0]=a;table[:,1]=z
    for i in range(6):
        low=((4-i)*a+(i+1)*z)//5 if i<4 else (0 if i==4 else 255)
        table[:,i+2]=np.where(seven,((6-i)*a+(i+1)*z)//7,low)
    bits=np.sum(b[:,[3,2,5,4,7,6]].astype(np.uint64)<<np.arange(0,48,8,dtype=np.uint64),axis=1)
    idx=(bits[:,None]>>np.arange(0,48,3,dtype=np.uint64))&7
    return table[np.arange(len(b))[:,None],idx]

def _decode(data,w,h,kind,tiled=True):
    size=8 if kind==1 else 16
    if tiled:data=_untile360(data,(w+3)//4,size)
    b,available,bw,bh=_blocks(data,w,h,size)
    if kind==2:
        px=np.zeros((len(b),16,4),dtype=np.uint8);px[:,:,0]=_alpha(b[:,:8]);px[:,:,1]=_alpha(b[:,8:]);px[:,:,3]=255
    else:
        px=_colour(b if kind==1 else b[:,8:],kind==1)
        if kind==3:
            p=np.arange(16);px[:,:,3]=((b[:,(p//2)^1]>>(4*(p&1)))&15)*17
        elif kind==5:px[:,:,3]=_alpha(b[:,:8])
    px[available:]=0
    return px.reshape(bh,bw,4,4,4).transpose(0,2,1,3,4).reshape(bh*4,bw*4,4)[:h,:w].tobytes()

def decode_dxt1(data,width,height):return _decode(data,width,height,1)
def decode_dxt3(data,width,height):return _decode(data,width,height,3)
def decode_dxt5(data,width,height,tiled=True):return _decode(data,width,height,5,tiled)
def decode_ati2(data,width,height):return _decode(data,width,height,2)
def decode_dxt1_normal(data,width,height):
    a=np.frombuffer(decode_dxt1(data,width,height),dtype=np.uint8).copy().reshape(-1,4)
    rg=a[:,:2].astype(np.float64)/255
    a[:,2]=np.rint(np.sqrt(np.maximum(1-rg[:,0]**2-rg[:,1]**2,0))*255).astype(np.uint8);a[:,3]=255
    return a.tobytes()

def _decode_raw_a8r8g8b8(data,width,height):
    a=np.zeros((width*height,4),dtype=np.uint8);n=min(len(data)//4,len(a))
    a[:n]=np.frombuffer(data,dtype=np.uint8,count=n*4).reshape(-1,4)[:,[1,2,3,0]]
    return a.tobytes()
def _decode_raw_b5g6r5(data,width,height):
    a=np.zeros((width*height,4),dtype=np.uint8);n=min(len(data)//2,len(a));v=np.frombuffer(data,dtype='>u2',count=n).astype(np.uint32)
    a[:n,0]=((v>>11)&31)*255//31;a[:n,1]=((v>>5)&63)*255//63;a[:n,2]=(v&31)*255//31;a[:n,3]=255
    return a.tobytes()
def _decode_raw_a8(data,width,height):
    a=np.zeros((width*height,4),dtype=np.uint8);n=min(len(data),len(a));a[:n]=np.frombuffer(data,dtype=np.uint8,count=n)[:,None]
    return a.tobytes()
