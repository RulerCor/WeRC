"""Stage/inspect once, then publish only after the final executable smoke passes."""
from pathlib import Path
import hashlib, json, os, shutil, subprocess, sys, zipfile
from release_content import verify_no_wechat_installer
from api_docs import verify_docs
from user_docs import verify_user_docs
from release_version import VERSION, RELEASE_NAME

root = Path(__file__).resolve().parent.parent
audit = root / f'target/release-audit/{RELEASE_NAME}'
stage = root / f'target/release-stage/{RELEASE_NAME}'
extracted = root / f'target/release-installer-extracted/{RELEASE_NAME}'
names = ['werc-desktop.exe', 'werc-native-worker.exe', 'werc-message-host.exe',
         'werc-message-bridge.dll', 'werc-voice-codec.exe', 'werc-emoticon-codec.exe',
         'werc-sqlcipher-worker.exe', 'werc-mcp-stdio.exe']
installer = root / f'target/release/bundle/nsis/WeRC_{VERSION}_x64-setup.exe'

def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024*1024), b''): h.update(block)
    return h.hexdigest()

def json_file(path): return json.loads(path.read_text(encoding='utf-8-sig'))

if sys.argv[1] == 'stage':
    audit.mkdir(parents=True, exist_ok=True)
    verify_docs(root / 'target/release-resources')
    verify_user_docs(root / 'target/release-resources')
    if stage.exists():
        assert sha(stage/'werc-desktop.exe') == sha(root/'target/release/werc-desktop.exe'), 'Existing stage is a different build'
    shutil.copytree(root / 'target/release-resources', stage, dirs_exist_ok=True,
                    ignore=lambda directory, names: {'vendor-installers'} if Path(directory) == root / 'target/release-resources' and 'vendor-installers' in names else set())
    verify_no_wechat_installer(stage)
    for name in names: shutil.copy2(root / 'target/release' / name, stage / name)
    extracted.mkdir(parents=True, exist_ok=True)
    seven = shutil.which('7z.exe')
    if not seven:
        program_files = os.environ.get('ProgramFiles')
        candidate = Path(program_files) / '7-Zip/7z.exe' if program_files else None
        if candidate is None or not candidate.is_file():
            raise SystemExit('7-Zip is required; install it or put 7z.exe on PATH.')
        seven = str(candidate)
    with (audit / 'installer-extract.log').open('w', encoding='utf8') as log:
        subprocess.run([seven, 'x', str(installer), '-o'+str(extracted), '-y'], check=True, stdout=log, stderr=subprocess.STDOUT)
    files = [p for p in extracted.rglob('*') if p.is_file()]
    main_files = [p for p in files if p.name == 'werc-desktop.exe']
    assert len(main_files) == 1
    resource_root = main_files[0].parent
    verify_no_wechat_installer(extracted)
    components = []
    for source in [p for p in stage.rglob('*') if p.is_file() and p.name not in ['manifest.json','WebView2RuntimeInstaller.exe']]:
        relative = source.relative_to(stage)
        matches = [p for p in files if p == resource_root / relative]
        assert len(matches) == 1, f'Missing/duplicate installer resource: {relative}'
        if source.name == 'werc-desktop.exe' and sha(source) == sha(matches[0]):
            components.append({'name':source.name,'sha256':sha(source),'byte_identical':True})
        elif source.name == 'werc-desktop.exe':
            result = subprocess.run(['node',str(root/'scripts/verify-bundle-component.mjs'),str(source),str(matches[0]),source.name],check=True,capture_output=True,text=True)
            components.append(json.loads(result.stdout))
        else: assert sha(source) == sha(matches[0]), f'Installer mismatch: {relative}'
    webview = [p for p in files if p.name.startswith('MicrosoftEdgeWebView2RuntimeInstaller') and p.suffix == '.exe']
    assert len(webview) == 1 and webview[0].stat().st_size > 50*1024*1024
    shutil.copy2(webview[0], stage / 'WebView2RuntimeInstaller.exe')
    (audit/'installer-content.json').write_text(json.dumps({'passed':True,'files_checked':len(list(stage.rglob('*'))),
        'components':components,'offline_webview_bytes':webview[0].stat().st_size,
        'offline_webview_sha256':sha(webview[0]),'installed_on_another_computer':False},indent=2),encoding='utf8')
    print('Stage and independently extracted installer contents match, including offline WebView2.')
elif sys.argv[1] == 'publish':
    verify_no_wechat_installer(stage)
    verify_docs(stage)
    verify_user_docs(stage)
    for result in ['runtime-portable.json','runtime-installer.json','installer-content.json','mcp-stdio-result.json']:
        evidence = json_file(audit/result)
        assert evidence['passed'], f'Final check did not pass: {result}'
        if result.startswith('runtime-'):
            binary = stage/'werc-desktop.exe' if result == 'runtime-portable.json' else extracted/'werc-desktop.exe'
            assert evidence['version'] == VERSION and evidence['desktop_sha256'] == sha(binary), 'Runtime evidence is for a different build'
        if result == 'mcp-stdio-result.json':
            assert evidence['sha256'] == sha(stage/'werc-mcp-stdio.exe'), 'STDIO evidence is for a different build'
    all_files = sorted(p for p in stage.rglob('*') if p.is_file() and p != stage/'manifest.json')
    for path in all_files:
        relative = path.relative_to(stage)
        assert path.suffix.lower() not in ['.pdb','.map','.rs','.tsx','.ts','.cpp','.c','.h','.py'], relative
        assert 'node_modules' not in relative.parts and 'state.sqlite3' not in path.name
        assert not any(part.lower() in ['reviewdata','webview','owned-profiles'] for part in relative.parts), relative
        assert path.name != 'desktop-lock.bin', 'An enabled application password must never ship'
        if path.suffix.lower() in ['.md','.txt','.json','.html']:
            content = path.read_text(encoding='utf-8-sig', errors='replace')
            assert str(root).lower() not in content.lower(), f'Developer path leaked: {relative}'
    manifest = {'product':'WeRC','version':VERSION,'platform':'windows-x86_64',
        'application_source_included':False,'source_maps_included':False,
        'third_party_corresponding_source':'open-source-components/',
        'files':[{'path':p.relative_to(stage).as_posix(),'bytes':p.stat().st_size,'sha256':sha(p)} for p in all_files]}
    (stage/'manifest.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf8')
    destination = root.parent.parent/'deliverables'/RELEASE_NAME
    assert not destination.exists(), 'Release destination already exists; choose a new version'
    destination.mkdir(parents=True)
    portable = destination/f'{RELEASE_NAME}-windows-x64-portable.zip'
    pending_portable = portable.with_suffix('.zip.partial')
    # Some unmodified vendor runtime files carry pre-1980 timestamps. ZIP must
    # clamp those timestamps while preserving the original file bytes.
    with zipfile.ZipFile(pending_portable,'w',zipfile.ZIP_DEFLATED,compresslevel=6,strict_timestamps=False) as z:
        for p in sorted(stage.rglob('*')):
            if p.is_file():z.write(p,RELEASE_NAME+'/'+p.relative_to(stage).as_posix())
    with zipfile.ZipFile(pending_portable) as z: assert z.testzip() is None
    pending_portable.replace(portable)
    setup = destination/f'{RELEASE_NAME}-windows-x64-setup.exe'
    shutil.copy2(installer,setup)
    for source,name in [('release-notes.md','版本说明.md'),('usage.md','使用说明.md')]:
        shutil.copy2(stage/source,destination/name)
    checksums = [{'file':p.name,'bytes':p.stat().st_size,'sha256':sha(p)} for p in [portable,setup]]
    (destination/'SHA256SUMS.txt').write_text('\n'.join(p['sha256']+'  '+p['file'] for p in checksums)+'\n',encoding='ascii')
    (audit/'published.json').write_text(json.dumps({'passed':True,'artifacts':checksums,'manual_distribution':True},indent=2),encoding='utf8')
    print(json.dumps(checksums,indent=2))
else: raise SystemExit('Use stage or publish')
