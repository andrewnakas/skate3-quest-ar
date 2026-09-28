"""Export the physics records already decoded by skate-data's ABIN reader."""
from pathlib import Path
import hashlib,json,struct
from .vlt import take,unpack

def fast_name(data,at,words):
    alphabet='0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ_'
    result=''
    for word in unpack('I'*words,data,at):
        divisor=38**5
        for _ in range(6):
            digit,word=divmod(word,divisor)
            if not digit:
                if word: raise ValueError('Invalid FastString padding')
                break
            if digit>len(alphabet): raise ValueError('Invalid FastString digit')
            result+=alphabet[digit-1]
            divisor//=38
    return result

def convert(path):
    data=path.read_bytes()
    end,version=unpack('II',data,0)
    if version!=1 or end>len(data):raise ValueError('Invalid animation bank')
    skeletons=[];at=48
    while at<end:
        size,kind=unpack('II',data,at)
        if size<48 or size%16 or at+size>end:raise ValueError('Invalid ABIN record')
        if kind==5:
            payload=(at+55)&~15
            count,=unpack('I',data,payload)
            start=payload+16
            take(data,start,count*112)
            bones=[]
            for i in range(count):
                pos=start+i*112
                # Bone words+23..27 contain FastString30 (the same record
                # consumed by native skeleton construction).
                bones.append({'name':fast_name(data,pos+92,5), 'source_offset':pos,
                              'words':list(unpack('I'*28,data,pos))})
            skeletons.append({'name':fast_name(data,at+16,6),'source_offset':at,'bones':bones})
        at+=size
    if not skeletons:raise ValueError('No physics skeletons in animation bank')
    return {'version':1,'source_sha256':hashlib.sha256(data).hexdigest(),'skeletons':skeletons}
