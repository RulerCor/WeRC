import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const digest = bytes => createHash('sha256').update(bytes).digest('hex');
export function verifyBundleComponent(source, bundled, name) {
  const sourceHash = digest(source);
  const bundledHash = digest(bundled);
  if (name !== 'werc-desktop.exe') {
    assert.ok(source.equals(bundled), `Installer component mismatch: ${name}`);
    return { name, sha256: bundledHash, source_sha256: sourceHash };
  }
  const prefix = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_');
  const original = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_UNK');
  const nsis = Buffer.from('__TAURI_BUNDLE_TYPE_VAR_NSS');
  const offset = source.indexOf(prefix);
  assert.ok(offset >= 0 && source.indexOf(prefix, offset + 1) === -1, 'Source bundle marker must be unique');
  assert.equal(bundled.indexOf(prefix), offset, 'Bundle marker moved');
  assert.equal(bundled.indexOf(prefix, offset + 1), -1, 'Duplicate bundle marker');
  assert.ok(source.subarray(offset, offset + original.length).equals(original), 'Unexpected source bundle type');
  assert.ok(bundled.subarray(offset, offset + nsis.length).equals(nsis), 'Expected NSIS bundle type');
  const normalized = Buffer.from(bundled);
  original.copy(normalized, offset);
  assert.ok(source.equals(normalized), 'Installer bytes differ outside the exact Tauri bundle marker');
  return { name, sha256: bundledHash, source_sha256: sourceHash,
    normalized_sha256: digest(normalized), bundle_marker: { offset, from: 'UNK', to: 'NSS', changed_bytes: 3 } };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [source, bundled, name] = process.argv.slice(2);
  console.log(JSON.stringify(verifyBundleComponent(readFileSync(source), readFileSync(bundled), name)));
}
