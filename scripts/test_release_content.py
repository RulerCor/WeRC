import json
import tempfile
import unittest
from pathlib import Path
from release_content import verify_no_wechat_installer

ROOT = Path(__file__).resolve().parent.parent

class ReleaseContentTests(unittest.TestCase):
    def test_packaged_wechat_installer_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'other').mkdir()
            (root / 'other' / 'Weixin_4.2.0.52.exe').write_bytes(b'candidate')
            with self.assertRaises(ValueError):
                verify_no_wechat_installer(root)
            (root / 'other' / 'Weixin_4.2.0.52.exe').unlink()
            bundled = root / 'vendor-installers' / 'wechat'
            bundled.mkdir(parents=True)
            (bundled / 'verification.json').write_text('{}', encoding='utf-8')
            with self.assertRaises(ValueError):
                verify_no_wechat_installer(root)

    def test_nsis_does_not_include_vendor_installer_tree(self):
        config = json.loads((ROOT / 'apps/desktop/src-tauri/tauri.bundle.conf.json').read_text(encoding='utf-8'))
        self.assertFalse(any('vendor-installers' in k or 'vendor-installers' in v for k, v in config['bundle']['resources'].items()))

if __name__ == '__main__':
    unittest.main()
