"""Release identity shared by packaging and user documentation."""
from pathlib import Path
import json
ROOT = Path(__file__).resolve().parent.parent
VERSION = json.loads((ROOT / 'package.json').read_text(encoding='utf-8'))['version']
assert json.loads((ROOT / 'apps/desktop/src-tauri/tauri.conf.json').read_text(encoding='utf-8'))['version'] == VERSION
RELEASE_NAME = f'WeRC-v{VERSION}'
