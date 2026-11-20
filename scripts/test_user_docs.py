import tempfile
import unittest
from pathlib import Path
from release_version import RELEASE_NAME
from user_docs import INTERNAL_REPORTS, PACKAGE_README, ROOT, stage_package_docs, stage_user_docs, verify_user_docs


class UserDocsTests(unittest.TestCase):
    def test_package_staging_removes_only_internal_report_copies(self):
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            package = Path(folder)
            for name in INTERNAL_REPORTS:
                (package / name).write_text('internal audit', encoding='utf-8')
            retained = package / RELEASE_NAME / 'licenses' / 'LICENSE.txt'
            retained.parent.mkdir(parents=True)
            retained.write_text('license must remain', encoding='utf-8')
            stage_package_docs(package)
            verify_user_docs(package / RELEASE_NAME)
            self.assertTrue(all(not (package / name).exists() for name in INTERNAL_REPORTS))
            self.assertEqual(retained.read_text(), 'license must remain')
            self.assertEqual((package / 'README.txt').read_text(encoding='utf-8'), PACKAGE_README)

    def test_manuals_have_no_developer_paths_or_internal_evidence_links(self):
        for path in [ROOT / 'docs' / name for name in ('usage.md', 'release-notes.md', 'bot-client.md')]:
            content = path.read_text(encoding='utf-8')
            self.assertNotIn(str(ROOT), content)
            self.assertNotIn('target/server-verification', content)
            self.assertNotIn('server-first-run-goals', content)
            self.assertNotIn('真机证据', content)
            self.assertNotIn('静态校验', content)

    def test_resource_docs_cannot_silently_become_stale(self):
        with tempfile.TemporaryDirectory(dir=ROOT / 'target') as folder:
            stage_user_docs(folder)
            verify_user_docs(folder)
            (Path(folder) / 'release-notes.md').write_text('old internal report', encoding='utf-8')
            with self.assertRaises(AssertionError):
                verify_user_docs(folder)


if __name__ == '__main__':
    unittest.main()
