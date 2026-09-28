"""Read TU3 APT bytecode without running it.

Operand alignment/relocation: native 82E67868, switch table 82E67914.
Function body boundaries: 82E73010 (DefineFunction2), 82E72EC0.
CONSTANTPOOL/PUSHDATA use CONST indices, not SWF's inline value encoding.
"""
from .binary import Reader, FormatError, align

BYTE = {0xA2, 0xAE, 0xAF, 0xB0, 0xB1, 0xB2, 0xB3, 0xB5, 0xB9}
SHORT = {0xA3, 0xB6}
WORD = {0x81, 0x87, 0x99, 0x9D, 0x9F, 0xB8}
STRING = {0x8B, 0x8C, 0xA1, 0xA4, 0xA5, 0xA6, 0xA7}


class Actions:
    def __init__(self, apt: bytes, const: bytes):
        self.apt = Reader(apt, 'APT actions')
        c = Reader(const, 'APT constants')
        count, start = c.u32be(24), c.u32be(28)
        c.require(start, count * 8)
        self.constants = []
        for i in range(count):
            at = start + i * 8
            kind, value = c.u32be(at), c.u32be(at + 4)
            if kind == 1:
                c.require(value, 1)
                value = c.cstring(value)
            elif kind == 6:
                value = c.f32be(at + 4)
            elif kind == 7:
                value = c.i32be(at + 4)
            self.constants.append({'kind': kind, 'value': value})

    def stream(self, offset: int, end: int | None = None, nesting: int = 0):
        if nesting > 64:
            raise FormatError('APT action nesting exceeds 64')
        r = self.apt
        end = len(r.data) if end is None else end
        r.require(offset, end - offset)
        result = []
        while offset < end:
            at, op = offset, r.u8(offset)
            offset += 1
            row = {'offset': at, 'opcode': op}
            if op == 0:
                result.append(row)
                break
            if op in BYTE:
                row['operand'] = r.u8(offset); offset += 1
            elif op in SHORT:
                row['operand'] = r.u16be(offset); offset += 2
            elif op in {0x77, 0xB4, 0xB7}:
                row['operand'] = r.i32be(offset); offset += 4
            elif op in WORD | STRING | {0x94}:
                offset = align(offset, 4)
                value = r.u32be(offset)
                row['operand'] = r.cstring(value) if op in STRING else value
                if op in {0x99, 0x9D, 0x94, 0xB8}:
                    row['target'] = offset + 4 + r.i32be(offset)
                offset += 4
            elif op in {0x88, 0x96}:
                offset = align(offset, 4)
                count, pointer = r.u32be(offset), r.u32be(offset + 4)
                r.require(pointer, count * 4)
                indices = [r.u32be(pointer + i * 4) for i in range(count)]
                if any(i >= len(self.constants) for i in indices):
                    raise FormatError(f'APT constant index outside CONST at {at:#x}')
                row['values'] = [self.constants[i] for i in indices]
                offset += 8
            elif op in {0x8E, 0x9B}:
                offset = align(offset, 4)
                header_size = 28 if op == 0x8E else 24
                r.require(offset, header_size)
                row['name'] = r.cstring(r.u32be(offset))
                count = r.u32be(offset + 4)
                pointer = r.u32be(offset + (12 if op == 0x8E else 8))
                length = r.u32be(offset + (16 if op == 0x8E else 12))
                stride = 8 if op == 0x8E else 4
                r.require(pointer, count * stride)
                row['parameters'] = [
                    {'register': r.u32be(pointer + i * 8) if op == 0x8E else 0,
                     'name': r.cstring(r.u32be(pointer + i * stride + (4 if op == 0x8E else 0)))}
                    for i in range(count)]
                row['flags'] = r.u32be(offset + 8) if op == 0x8E else 0
                offset += header_size
                row['body'] = self.stream(offset, offset + length, nesting + 1)
                offset += length
            elif op in {0x83, 0x8F}:
                # Keep unsupported compound instructions explicit, never scan
                # their payload as though it were a stream of no-operand ops.
                raise FormatError(f'APT compound opcode {op:#x} at {at:#x} needs decoding')
            row['next'] = offset
            if offset > end:
                raise FormatError(f'APT instruction at {at:#x} crosses its function body')
            result.append(row)
        return result
