"""Sign the WeRC setup and portable assets and write the GitHub updater manifest."""
import json
import os
from pathlib import Path
import subprocess
import sys

from release_version import VERSION, RELEASE_NAME

root = Path(__file__).resolve().parent.parent
folder = Path(sys.argv[1]).resolve()
key = os.environ.get('TAURI_SIGNING_PRIVATE_KEY_PATH')
if not key and os.environ.get('LOCALAPPDATA'):
    key = Path(os.environ['LOCALAPPDATA']) / 'WeRC-ReleaseSigning' / 'werc-updater.key'
trusted = (root / 'apps/desktop/src-tauri/updater.pub').read_text().strip()
if not key or not Path(key).is_file() or not Path(str(key) + '.pub').is_file():
    raise SystemExit('Set TAURI_SIGNING_PRIVATE_KEY_PATH to the WeRC signing key (with its .pub file).')
if Path(str(key) + '.pub').read_text().strip() != trusted:
    raise SystemExit('Signing key does not match the public key shipped by WeRC.')

base = f'https://github.com/RulerCor/WeRC/releases/download/v{VERSION}'
platforms = {}
for target, suffix in [('nsis', 'setup.exe'), ('portable', 'portable.zip')]:
    asset = folder / f'{RELEASE_NAME}-windows-x64-{suffix}'
    result = subprocess.run([
        'node', str(root / 'node_modules/@tauri-apps/cli/tauri.js'), 'signer', 'sign',
        '--private-key-path', str(key), '--app-version', VERSION, str(asset),
    ], capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(f'Signing failed for {asset.name}; check the release signing key and password.')
    signature = Path(str(asset) + '.sig').read_text(encoding='utf-8').strip()
    if not signature:
        raise SystemExit(f'Missing signature: {asset.name}')
    platforms[f'windows-x86_64-{target}'] = {'url': f'{base}/{asset.name}', 'signature': signature}

(folder / 'latest.json').write_text(json.dumps({'version': VERSION, 'platforms': platforms}, indent=2) + '\n', encoding='utf-8')
print('Upload latest.json with both release packages to the same GitHub Release.')
