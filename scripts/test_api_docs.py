"""Offline docs must cover the contract and survive packaging without a server."""
import json
import tempfile
import unittest
from html.parser import HTMLParser
from pathlib import Path
from api_docs import FILENAME, ROOT, SOURCES, operations, render, stage_docs, verify_docs


class ReferenceParser(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids, self.links, self.endpoints, self.remote = set(), [], [], []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'id' in attrs:
            self.ids.add(attrs['id'])
        if tag == 'a':
            self.links.append(attrs['href'])
        if tag == 'article':
            self.endpoints.append(attrs['data-operation'])
        if tag in ('script', 'link', 'img', 'iframe') and (attrs.get('src') or attrs.get('href')):
            self.remote.append(attrs)


class ApiDocsTests(unittest.TestCase):
    def test_all_operations_and_schema_links_are_offline_and_complete(self):
        spec = json.loads(SOURCES['openapi.json'].read_bytes())
        page = render(SOURCES['openapi.json'].read_bytes(), SOURCES['bot-client.md'].read_bytes())
        parser = ReferenceParser()
        parser.feed(page)
        self.assertEqual(parser.endpoints, [method.upper() + ' ' + path for path, method, _ in operations(spec)])
        self.assertGreaterEqual(len(parser.endpoints), 106)
        self.assertFalse(parser.remote)
        for link in parser.links:
            if link.startswith('#'):
                self.assertIn(link[1:], parser.ids)
            else:
                self.assertIn(link, SOURCES)

    def test_package_gate_rejects_missing_or_stale_docs_and_contract(self):
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder)
            stage_docs(out)
            verify_docs(out)
            (out / FILENAME).unlink()
            with self.assertRaises(FileNotFoundError):
                verify_docs(out)
            stage_docs(out)
            (out / FILENAME).write_text('old reference', encoding='utf-8')
            with self.assertRaisesRegex(AssertionError, 'stale offline'):
                verify_docs(out)
            stage_docs(out)
            (out / 'openapi.json').write_text('{}', encoding='utf-8')
            with self.assertRaisesRegex(AssertionError, 'Stale packaged OpenAPI'):
                verify_docs(out)
            stage_docs(out)
            (out / 'bot-client.md').write_text('old examples', encoding='utf-8')
            with self.assertRaisesRegex(AssertionError, 'Stale packaged Bot guide'):
                verify_docs(out)

    def test_descriptions_and_guide_cannot_inject_markup(self):
        spec = json.loads(SOURCES['openapi.json'].read_bytes())
        path, method, _ = operations(spec)[0]
        spec['paths'][path][method]['description'] = '<img src=x onerror=alert(1)>'
        page = render(json.dumps(spec).encode(), b'</pre><script>alert(1)</script>')
        self.assertIn('&lt;img src=x onerror=alert(1)&gt;', page)
        self.assertIn('&lt;/pre&gt;&lt;script&gt;alert(1)&lt;/script&gt;', page)

    def test_windows_installer_has_the_same_offline_reference(self):
        config = json.loads((ROOT / 'apps/desktop/src-tauri/tauri.bundle.conf.json').read_text(encoding='utf-8'))
        resources = config['bundle']['resources']
        for name in (FILENAME, *SOURCES):
            self.assertEqual(resources['../../../target/release-resources/' + name], name)


if __name__ == '__main__':
    unittest.main()
