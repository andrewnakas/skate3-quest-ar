"""Synthetic update transactions; no game, Steam or user installation."""
import hashlib,json,tempfile,unittest
from pathlib import Path
from unittest.mock import patch
import update_install as i

BASE={n:b'old' for n in i.REQUIRED}
def layout(root,files,build=1):
    for name,data in files.items():
        p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data)
    meta={'files':{n:hashlib.sha256(b).hexdigest() for n,b in files.items()},'build':build}
    i.atomic(root/'release.json',meta)
    return meta

class Transactions(unittest.TestCase):
    def test_add_replace_remove_and_preserve_unowned_content(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);tx=root/'.update-transaction'
            layout(root,{**BASE,'obsolete/tool.dll':b'old'})
            new={n:b'new' for n in BASE};new['new-tool/bin/helper.exe']=b'helper'
            meta=layout(tx/'new',new,2)
            for name in ['data/maps/owned.skate','mods/my-mod.zip','personal.txt']:
                p=root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b'user')
            i.install(root,tx)
            self.assertEqual(i.mismatches(root,meta),[])
            self.assertFalse((root/'obsolete/tool.dll').exists())
            self.assertEqual(i.read(root/'release.json')['build'],2)
            for name in ['data/maps/owned.skate','mods/my-mod.zip','personal.txt']:
                self.assertEqual((root/name).read_bytes(),b'user')

    def test_failure_restores_deleted_files_and_removes_new_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);tx=root/'.update-transaction'
            old=layout(root,{**BASE,'a-obsolete.dll':b'old'})
            layout(tx/'new',{**{n:b'new' for n in BASE},'b-new.dll':b'new'},2)
            replace=i.os.replace
            def fail(src,dest):
                if Path(src)==tx/'new/support/skate3setup.exe':raise OSError('locked')
                return replace(src,dest)
            with patch.object(i.os,'replace',fail),patch.object(i,'retry',lambda op:op()),self.assertRaises(OSError):
                i.install(root,tx)
            self.assertEqual(i.mismatches(root,old),[])
            self.assertFalse((root/'b-new.dll').exists())
            self.assertEqual(i.read(root/'release.json'),old)
            self.assertFalse((tx/'journal.json').exists())

    def test_missing_component_is_repaired(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);tx=root/'.update-transaction'
            meta=layout(root,{**BASE,'steam-relay/helper.exe':b'right'})
            (root/'steam-relay/helper.exe').unlink()
            self.assertEqual(i.mismatches(root,meta),['steam-relay/helper.exe'])
            layout(tx/'new',{**BASE,'steam-relay/helper.exe':b'right'})
            i.install(root,tx)
            self.assertEqual(i.mismatches(root,meta),[])

    def test_recover_interrupted_transaction(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);tx=root/'.update-transaction'
            old=layout(root,BASE)
            for name in [*BASE,'release.json']:
                p=tx/'old'/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes((root/name).read_bytes())
            (root/'added.dll').write_bytes(b'new');(root/'skate3rust.exe').write_bytes(b'new')
            i.atomic(tx/'journal.json',{'protocol':2,'names':[*BASE,'added.dll','release.json'],'present':[*BASE,'release.json']})
            i.rollback(root,tx)
            self.assertEqual(i.mismatches(root,old),[]);self.assertFalse((root/'added.dll').exists())

    def test_legacy_recovery(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);tx=root/'.update-transaction'
            layout(root,BASE);layout(tx/'old',BASE)
            (root/'skate3rust.exe').write_bytes(b'partial')
            i.atomic(tx/'journal.json',{'protocol':1});i.rollback(root,tx)
            self.assertEqual((root/'skate3rust.exe').read_bytes(),b'old')

    def test_corrupt_staging_does_not_modify_install(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);tx=root/'.update-transaction';old=layout(root,BASE)
            layout(tx/'new',BASE);(tx/'new/skate3rust.exe').write_bytes(b'bad')
            with self.assertRaises(ValueError):i.install(root,tx)
            self.assertEqual(i.mismatches(root,old),[])
            self.assertFalse((tx/'journal.json').exists())

    def test_manifest_rejects_user_data_and_windows_path_aliases(self):
        for name in ['../outside','C:/oops','data/map.skate','mods/user.zip','a/../b','support/NUL.dll','a./b','AUX','/absolute','a\\b','release.json']:
            with self.subTest(name=name),self.assertRaises(ValueError):
                i.manifest_files({'files':{**{n:'a'*64 for n in BASE},name:'b'*64}})

if __name__=='__main__':unittest.main()
