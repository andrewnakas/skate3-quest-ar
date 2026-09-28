"""Small, strict GLB reader/writer for the offline character converter."""
import base64
import copy
import json
import struct
from pathlib import Path

import numpy as np


class ConversionError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise ConversionError(message)


def transform(node):
    if 'matrix' in node:
        result = np.asarray(node['matrix'], dtype=float).reshape(4, 4).T
    else:
        x, y, z, w = node.get('rotation', [0, 0, 0, 1])
        q = np.asarray([x, y, z, w], dtype=float)
        require(abs(np.linalg.norm(q) - 1) < 1e-3, 'Invalid node quaternion')
        result = np.eye(4)
        result[:3, :3] = [
            [1-2*y*y-2*z*z, 2*x*y-2*z*w, 2*x*z+2*y*w],
            [2*x*y+2*z*w, 1-2*x*x-2*z*z, 2*y*z-2*x*w],
            [2*x*z-2*y*w, 2*y*z+2*x*w, 1-2*x*x-2*y*y]]
        result[:3, :3] *= node.get('scale', [1, 1, 1])
        result[:3, 3] = node.get('translation', [0, 0, 0])
    require(np.isfinite(result).all() and abs(np.linalg.det(result)) > 1e-12,
            'Non-finite or singular node transform')
    return result


def clean_node_transform(node):
    """Round tiny authored matrix shear to a valid glTF TRS representation."""
    matrix = transform(node)
    u, _, vh = np.linalg.svd(matrix[:3,:3])
    rotation = u@vh
    require(np.linalg.det(rotation)>0, 'Mirrored stock node unsupported')
    scale = np.diag(rotation.T@matrix[:3,:3])
    require(np.allclose(rotation*scale,matrix[:3,:3],atol=2e-5), 'Stock node has significant shear')
    # Stable matrix-to-quaternion, including 180-degree rotations.
    trace = np.trace(rotation)
    if trace > 0:
        s = np.sqrt(trace+1)*2
        q = [(rotation[2,1]-rotation[1,2])/s,(rotation[0,2]-rotation[2,0])/s,
             (rotation[1,0]-rotation[0,1])/s,s/4]
    else:
        i = int(np.argmax(np.diag(rotation)));j=(i+1)%3;k=(i+2)%3
        s = np.sqrt(1+rotation[i,i]-rotation[j,j]-rotation[k,k])*2
        q = [0.,0.,0.,0.]
        q[i]=s/4;q[j]=(rotation[j,i]+rotation[i,j])/s;q[k]=(rotation[k,i]+rotation[i,k])/s
        q[3]=(rotation[k,j]-rotation[j,k])/s
    q = np.asarray(q)/np.linalg.norm(q)
    return {'translation':matrix[:3,3].tolist(),'rotation':q.tolist(),'scale':scale.tolist()}


class Document:
    def __init__(self, path):
        self.path = Path(path)
        require(self.path.stat().st_size <= 512 * 1024**2, 'GLB exceeds 512 MiB limit')
        self.raw = self.path.read_bytes()
        require(len(self.raw) >= 28, 'Truncated GLB')
        magic, version, total = struct.unpack_from('<III', self.raw)
        require(magic == 0x46546c67 and version == 2 and total == len(self.raw),
                'Invalid GLB header')
        chunks = {}
        offset = 12
        while offset < total:
            require(offset + 8 <= total, 'Truncated chunk header')
            length, kind = struct.unpack_from('<I4s', self.raw, offset)
            require(offset + 8 + length <= total and kind not in chunks, 'Invalid GLB chunk')
            chunks[kind] = self.raw[offset+8:offset+8+length]
            offset += 8 + length
        self.doc = json.loads(chunks[b'JSON'])
        self.data = chunks.get(b'BIN\0', b'')
        require(len(self.doc.get('buffers', [])) == 1 and
                'uri' not in self.doc['buffers'][0], 'Only embedded GLB buffers are supported')
        require(not self.doc.get('extensionsRequired'), 'Required glTF extensions are unsupported')
        self.parents = {}
        nodes = self.doc['nodes']
        require(len(nodes) <= 4096, 'Too many scene nodes')
        for i, node in enumerate(nodes):
            for child in node.get('children', []):
                require(0 <= child < len(nodes) and child not in self.parents,
                        'Invalid or multiply parented node')
                self.parents[child] = i
        self.worlds = {}
        def visit(i, active):
            require(i not in active, 'Skeleton/scene contains a cycle')
            if i not in self.worlds:
                parent = self.parents.get(i)
                self.worlds[i] = ((visit(parent, active | {i}) if parent is not None else np.eye(4))
                                  @ transform(nodes[i]))
            return self.worlds[i]
        for i in range(len(nodes)):
            visit(i, set())

    def view(self, i):
        v = self.doc['bufferViews'][i]
        require(v.get('buffer', 0) == 0, 'External buffer unsupported')
        start, length = v.get('byteOffset', 0), v['byteLength']
        require(start >= 0 and length >= 0 and start+length <= len(self.data), 'Buffer view out of bounds')
        return self.data[start:start+length]

    def accessor(self, i):
        a = self.doc['accessors'][i]
        require('sparse' not in a, 'Sparse accessors need an explicit conversion first')
        types = {5120:'i1', 5121:'u1', 5122:'<i2', 5123:'<u2', 5125:'<u4', 5126:'<f4'}
        sizes = {'SCALAR':1, 'VEC2':2, 'VEC3':3, 'VEC4':4, 'MAT4':16}
        dtype, width = np.dtype(types[a['componentType']]), sizes[a['type']]
        require(0 < a['count'] <= 6_000_000, 'Accessor count out of supported range')
        v = self.doc['bufferViews'][a['bufferView']]
        data = self.view(a['bufferView'])
        start, stride = a.get('byteOffset', 0), v.get('byteStride', dtype.itemsize*width)
        require(start >= 0 and stride >= dtype.itemsize*width and
                start+(a['count']-1)*stride+dtype.itemsize*width <= len(data), 'Accessor out of bounds')
        values = np.ndarray((a['count'], width), dtype=dtype, buffer=data,
                            offset=start, strides=(stride, dtype.itemsize)).copy()
        if a.get('normalized'):
            require(dtype.kind in 'iu', 'Invalid normalized accessor')
            values = np.maximum(values.astype(float)/np.iinfo(dtype).max, -1)
        require(np.isfinite(values).all(), 'Non-finite accessor')
        return values

    def image(self, i):
        image = self.doc['images'][i]
        if 'bufferView' in image:
            return self.view(image['bufferView']), image['mimeType']
        uri = image.get('uri', '')
        require(uri.startswith('data:') and ';base64,' in uri, 'Missing embedded texture; export FBX with media')
        header, payload = uri.split(',', 1)
        return base64.b64decode(payload, validate=True), header[5:].split(';')[0]


class Writer:
    def __init__(self):
        self.data = bytearray()
        self.doc = {'asset': {'version':'2.0', 'generator':'Mixamo to Skate 1'},
                    'bufferViews':[], 'accessors':[], 'materials':[], 'textures':[], 'images':[]}

    def view(self, data):
        self.data.extend(b'\0' * (-len(self.data) % 4))
        offset = len(self.data)
        self.data.extend(data)
        self.doc['bufferViews'].append({'buffer':0, 'byteOffset':offset, 'byteLength':len(data)})
        return len(self.doc['bufferViews'])-1

    def accessor(self, values, kind, component=5126):
        values = np.asarray(values, dtype={5126:'<f4', 5123:'<u2', 5125:'<u4'}[component])
        require(np.isfinite(values).all(), 'Non-finite output')
        a = {'bufferView':self.view(values.tobytes()), 'componentType':component,
             'count':len(values), 'type':kind}
        if kind == 'VEC3':
            a.update(min=values.min(axis=0).tolist(), max=values.max(axis=0).tolist())
        self.doc['accessors'].append(a)
        return len(self.doc['accessors'])-1

    def materials_from(self, source):
        offset = len(self.doc['materials'])
        texture_offset = len(self.doc['textures'])
        for image_index in range(len(source.doc.get('images', []))):
            data, mime = source.image(image_index)
            require(mime in ('image/png', 'image/jpeg'), 'Only PNG/JPEG textures supported')
            self.doc['images'].append({'bufferView':self.view(data), 'mimeType':mime})
        image_offset = len(self.doc['images']) - len(source.doc.get('images', []))
        sampler_offset = len(self.doc.get('samplers', []))
        self.doc.setdefault('samplers', []).extend(copy.deepcopy(source.doc.get('samplers', [])))
        for t in source.doc.get('textures', []):
            require('source' in t and not t.get('extensions'), 'Unsupported texture extension')
            t = copy.deepcopy(t)
            t['source'] += image_offset
            if 'sampler' in t:
                t['sampler'] += sampler_offset
            self.doc['textures'].append(t)
        def fix(obj):
            if isinstance(obj, dict):
                for key, value in obj.items():
                    if key.endswith('Texture') and isinstance(value, dict):
                        value['index'] += texture_offset
                    else:
                        fix(value)
            elif isinstance(obj, list):
                for value in obj:
                    fix(value)
        for m in source.doc.get('materials', []):
            m = copy.deepcopy(m)
            require(not m.get('extensions'), 'Material extensions require explicit conversion')
            m.pop('extras', None)
            fix(m)
            self.doc['materials'].append(m)
        return offset

    def save(self, path):
        for mesh in self.doc.get('meshes',[]):
            for p in mesh['primitives']:
                for index in p['attributes'].values():
                    self.doc['bufferViews'][self.doc['accessors'][index]['bufferView']]['target']=34962
                if 'indices' in p:
                    self.doc['bufferViews'][self.doc['accessors'][p['indices']]['bufferView']]['target']=34963
        for key in ('materials', 'textures', 'images', 'samplers'):
            if not self.doc.get(key):
                self.doc.pop(key, None)
        self.doc['buffers'] = [{'byteLength':len(self.data)}]
        data = bytes(self.data) + b'\0'*(-len(self.data)%4)
        encoded = json.dumps(self.doc, separators=(',', ':'), allow_nan=False).encode()
        encoded += b' '*(-len(encoded)%4)
        Path(path).write_bytes(struct.pack('<III', 0x46546c67, 2, 28+len(encoded)+len(data)) +
                              struct.pack('<I4s', len(encoded), b'JSON') + encoded +
                              struct.pack('<I4s', len(data), b'BIN\0') + data)
