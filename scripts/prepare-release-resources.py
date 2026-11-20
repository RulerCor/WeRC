"""Prepare an explicit resource tree, never copy target or development profiles wholesale."""
from pathlib import Path
import hashlib, json, shutil, zipfile, os, subprocess
from api_docs import stage_docs
from user_docs import stage_user_docs

root = Path(__file__).resolve().parent.parent
out = root / 'target/release-resources'
out.mkdir(parents=True, exist_ok=True)
stage_docs(out)

def copy(source, destination):
    source, destination = root / source, out / destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)

for path in (root / 'licenses').rglob('*'):
    if path.is_file(): copy(path, Path('licenses') / path.relative_to(root / 'licenses'))
for source, name in [
    ('LICENSE', 'WeRC-NONCOMMERCIAL.txt'),
    ('native/helper/LICENSE', 'helper-NONCOMMERCIAL.txt'),
    ('native/helper/THIRD_PARTY_NOTICES.md', 'helper-THIRD-PARTY-NOTICES.md'),
    ('native/helper/native/vendor/minhook-1.3.4/LICENSE.txt', 'MinHook-BSD.txt'),
    ('native/silk/Skype-LICENSE.txt', 'Skype-SILK.txt'),
    ('native/silk/wrapper-MIT.txt', 'silk-rs-MIT.txt'),
    ('apps/desktop/src/assets/fonts/Saira-OFL.txt', 'Saira-OFL.txt'),
    ('native/asr/whisper.cpp-1.9.5/LICENSE', 'whisper-cpp-MIT.txt'),
]: copy(source, 'licenses/' + name)
for source, name in [('docs/usage.md','usage.md'), ('docs/bot-client.md','bot-client.md'),
                     ('docs/release-notes.md','release-notes.md'), ('contracts/openapi.json','openapi.json')]:
    copy(source, name)

whisper = root / 'target/asr/whisper-cli.exe'
evidence = json.loads((root / 'target/asr/build-evidence.json').read_text(encoding='utf-8-sig'))
assert hashlib.sha256(whisper.read_bytes()).hexdigest() == evidence['sha256']
assert hashlib.sha256((root/'native/asr/windows-utf8.manifest').read_bytes()).hexdigest() == evidence['utf8_manifest_sha256']
copy(whisper, 'asr/whisper-cli.exe')
tiny = json.loads((root / 'native/asr/MODELS.json').read_text(encoding='utf-8-sig'))['models'][0]
data = (root / 'target/asr/models/ggml-tiny.bin').read_bytes()
assert len(data) == tiny['size'] and hashlib.sha256(data).hexdigest() == tiny['sha256']
copy('target/asr/models/ggml-tiny.bin', 'asr/models/ggml-tiny.bin')
copy('target/sherpa-verification/components/sherpa/sherpa-onnx-offline.exe',
     'asr/sherpa/sherpa-onnx-offline.exe')
# Other model weights are optional. Do not redistribute models with unspecified licenses.

# App-local deployment: the desktop must also work without a globally installed
# Visual C++ redistributable. Copy only the official x64 CRT redistributable set.
vswhere = Path(os.environ['ProgramFiles(x86)']) / 'Microsoft Visual Studio/Installer/vswhere.exe'
vs = Path(subprocess.check_output([str(vswhere), '-latest', '-products', '*',
    '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'], text=True).strip())
candidates = sorted((vs/'VC/Redist/MSVC').glob('14*/x64/Microsoft.VC*.CRT'))
assert candidates, 'Official x64 VC redistributable directory unavailable'
crt = candidates[-1]
for path in crt.glob('*.dll'):
    copy(path, path.name)
    copy(path, 'asr/' + path.name)
(out/'licenses/Microsoft-VisualCpp-Runtime.txt').write_text(
    'Microsoft Visual C++ Runtime Libraries\nCopyright (c) Microsoft Corporation. All rights reserved.\n'
    'Unmodified x64 runtime libraries distributed from the Visual Studio REDIST directory.\n'
    'These files are proprietary Microsoft runtime components, not covered by the application licenses.\n'
    'Redistribution terms: https://visualstudio.microsoft.com/license-terms/\n'
    'Redistributable list: https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution\n', encoding='utf8')

copy('target/release/werc-emoticon-codec-source.zip', 'open-source-components/werc-emoticon-codec-source.zip')
archive = out / 'open-source-components/mp4parse-source.zip'
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as z:
    for path in sorted((root / 'vendor/mp4parse').rglob('*')):
        if path.is_file() and 'target' not in path.relative_to(root / 'vendor/mp4parse').parts:
            z.write(path, 'mp4parse/' + path.relative_to(root / 'vendor/mp4parse').as_posix())

# Preserve original dependency notices without including dependency implementation files.
# Cargo metadata contains absolute registry/workspace paths. Always resolve it on
# this machine instead of reusing a previous machine's audit file after transfer.
metadata_bytes = subprocess.check_output([
    'cargo', 'metadata', '--locked', '--format-version', '1',
    '--filter-platform', 'x86_64-pc-windows-msvc',
    '--manifest-path', str(root / 'Cargo.toml'),
], cwd=root)
metadata = json.loads(metadata_bytes)
metadata_record = root / 'target/release-audit/cargo-metadata.json'
metadata_record.parent.mkdir(parents=True, exist_ok=True)
metadata_record.write_bytes(metadata_bytes)
inventory = []
fallback_notices = {
    'alloc-stdlib': ['dependency-fallbacks/alloc-stdlib-BSD.txt'],
    'selectors': ['mp4parse-MPL-2.0.txt'],
    'tauri-plugin': ['dependency-fallbacks/tauri-plugin-MIT.txt'],
    'webview2-com': ['dependency-fallbacks/webview2-com-MIT.txt'],
    'webview2-com-sys': ['dependency-fallbacks/webview2-com-MIT.txt'],
    'webview2-com-macros': ['dependency-fallbacks/webview2-com-macros-MIT.txt'],
}
for package in metadata['packages']:
    if package['source'] is None: continue
    directory = Path(package['manifest_path']).parent
    ident = package['name'] + '-' + package['version']
    notice_files = []
    for path in directory.iterdir():
        if path.is_file() and path.name.upper().startswith(('LICENSE', 'LICENCE', 'NOTICE', 'COPYING', 'COPYRIGHT')):
            relative = 'licenses/dependencies/' + ident + '/' + path.name
            copy(path, relative)
            notice_files.append(relative)
    if not notice_files:
        for relative in fallback_notices.get(package['name'], []):
            assert (out / 'licenses' / relative).is_file(), relative
            notice_files.append('licenses/' + relative)
    inventory.append({'name': package['name'], 'version': package['version'],
                      'license': package['license'], 'notices': notice_files,
                      'source_archive': 'https://crates.io/api/v1/crates/' + package['name'] + '/' + package['version'] + '/download',
                      'repository': package.get('repository')})
for name in ['react','react-dom','scheduler','lucide-react','@fontsource/ibm-plex-mono','@fontsource/ibm-plex-sans']:
    directory = root / 'node_modules' / name
    package = json.loads((directory / 'package.json').read_text(encoding='utf8'))
    notices = []
    for path in directory.iterdir():
        if path.is_file() and path.name.upper().startswith(('LICENSE', 'LICENCE', 'OFL', 'NOTICE', 'COPYING')):
            relative = 'licenses/dependencies/' + name.replace('/', '_') + '/' + path.name
            copy(path, relative); notices.append(relative)
    inventory.append({'name': name, 'version': package['version'], 'license': package.get('license'), 'notices': notices})
(out / 'licenses/dependency-inventory.json').write_text(json.dumps(inventory, ensure_ascii=False, indent=2), encoding='utf8')
(out / 'licenses/SOURCE-AVAILABILITY.txt').write_text(
    'Third-party source availability\n\n'
    'Dependency names, exact versions, licenses, notices and upstream source archives are listed in dependency-inventory.json.\n'
    'Unmodified Mozilla Public License dependencies (including cssparser, cssparser-macros, dtoa-short, option-ext and selectors) '
    'are available from their exact source_archive URLs in that inventory. The MPL text is included as mp4parse-MPL-2.0.txt.\n'
    'Corresponding source for the modified mp4parse component and the separate GPL emoticon decoder is included in ../open-source-components/.\n'
    'The main WeRC application source is not included.\n', encoding='utf8')
# Product documentation is shared by portable and installer resources.
stage_user_docs(out)
print('Release resources prepared; main application source and private profiles excluded.')
