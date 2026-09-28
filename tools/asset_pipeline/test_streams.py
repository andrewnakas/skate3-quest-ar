import struct
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'vendor/university/tools/vanilla_map_extraction/tools'))
from skate3_streams import AssetRecord, StreamFormatError, read_sfil


class StreamTests(unittest.TestCase):
    def fixture(self, tail=b'\0'*128):
        raw=bytearray(384)
        raw[:4]=b'SFIL';struct.pack_into('>I',raw,16,128)
        struct.pack_into('>Q3I',raw,128,123,32,128,256)
        raw[256:288]=bytes(range(32))
        record=AssetRecord(123,0,32,128,128,0,4,b'')
        return bytes(raw)+tail,record

    def test_raw_assets_padding_and_shared_index(self):
        class NoSuffixCopy(bytes):
            def __getitem__(self,key):
                if isinstance(key,slice) and key.start and key.stop is None:
                    raise AssertionError('Copied the entire remaining stream')
                return super().__getitem__(key)
        raw,record=self.fixture()
        with patch.object(Path,'read_bytes',return_value=NoSuffixCopy(raw)):
            normal=read_sfil('fixture.xsf',[record])
            shared=read_sfil('fixture.xsf',[record],record_index={123:record})
        self.assertEqual(normal,shared)
        self.assertEqual(normal[0].data,bytes(range(32)))
        self.assertEqual(normal[0].source_offset,128)

    def test_truncated_nonzero_tail_is_still_rejected(self):
        raw,record=self.fixture(b'\0\1')
        with patch.object(Path,'read_bytes',return_value=raw):
            with self.assertRaisesRegex(StreamFormatError,'truncated'):
                read_sfil('fixture.xsf',[record])

    def test_missing_and_unknown_records_are_still_rejected(self):
        raw,record=self.fixture()
        missing=AssetRecord(456,0,32,128,128,0,4,b'')
        with patch.object(Path,'read_bytes',return_value=raw):
            with self.assertRaisesRegex(StreamFormatError,'missing'):
                read_sfil('fixture.xsf',[record,missing])
            self.assertEqual(len(read_sfil('fixture.xsf',[record,missing],require_all_records=False)),1)
            with self.assertRaisesRegex(StreamFormatError,'absent'):
                read_sfil('fixture.xsf',[missing])


if __name__=='__main__':unittest.main()
