import assert from 'node:assert/strict';
import test from 'node:test';
import { verifyBundleComponent } from './verify-bundle-component.mjs';

const source = Buffer.from('MZ-test-__TAURI_BUNDLE_TYPE_VAR_UNK-end');
const bundled = Buffer.from('MZ-test-__TAURI_BUNDLE_TYPE_VAR_NSS-end');
test('NSIS marker is normalized only in memory and both real hashes are retained', () => {
  const result = verifyBundleComponent(source, bundled, 'werc-desktop.exe');
  assert.notEqual(result.sha256, result.source_sha256);
  assert.equal(result.normalized_sha256, result.source_sha256);
  assert.equal(result.bundle_marker.changed_bytes, 3);
  assert.ok(bundled.includes(Buffer.from('_NSS')));
});
test('other byte changes, truncation, extra data and duplicate markers are rejected', () => {
  for (const changed of [Buffer.from(bundled.toString().replace('MZ', 'NZ')), bundled.subarray(0,-1), Buffer.concat([bundled,Buffer.from('x')]), Buffer.concat([bundled,bundled])]) {
    assert.throws(() => verifyBundleComponent(source, changed, 'werc-desktop.exe'));
  }
  assert.throws(() => verifyBundleComponent(source, source, 'werc-desktop.exe'));
  assert.throws(() => verifyBundleComponent(Buffer.concat([source,source]), Buffer.concat([bundled,bundled]), 'werc-desktop.exe'));
});
test('companion components receive no marker exception', () => {
  assert.throws(() => verifyBundleComponent(source,bundled,'werc-message-host.exe'));
  const result = verifyBundleComponent(source,source,'werc-message-host.exe');
  assert.equal(result.sha256,result.source_sha256);
});
