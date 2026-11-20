import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import {tmpdir} from './development-paths.mjs';
import { delimiter, join } from 'node:path';
import { rustToolPath } from './rust-tool-path.mjs';

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'werc-rust-path-'));
  t.after(() => rmSync(root, { recursive: true }));
  const tool = (directory, name) => {
    mkdirSync(directory, { recursive: true });
    const path = join(directory, `${name}.exe`);
    writeFileSync(path, 'owned non-executable path fixture');
    return path;
  };
  return { root, tool, home: join(root, '空白用户') };
}

test('cache-only CARGO_HOME resolves proxies from mixed-case Windows Path', t => {
  const { root, tool, home } = fixture(t);
  const bin = join(root, '工具 链');
  const cargo = tool(bin, 'cargo');
  const rustc = tool(bin, 'rustc');
  const env = { CARGO_HOME: join(root, '仅缓存'), Path: `"${bin}"` };
  assert.equal(rustToolPath('cargo', { env, home }), cargo);
  assert.equal(rustToolPath('rustc', { env, home }), rustc);
});

test('explicit Cargo proxy wins, missing rustc independently falls back to PATH', t => {
  const { root, tool, home } = fixture(t);
  const cargoHome = join(root, '用户 工具');
  const cargo = tool(join(cargoHome, 'bin'), 'cargo');
  const bin = join(root, '系统工具');
  tool(bin, 'cargo');
  const rustc = tool(bin, 'rustc');
  const env = { CARGO_HOME: cargoHome, PATH: [join(root, 'absent'), bin].join(delimiter) };
  assert.equal(rustToolPath('cargo', { env, home }), cargo);
  assert.equal(rustToolPath('rustc', { env, home }), rustc);
});

test('default user installation is a fallback when PATH is absent', t => {
  const { tool, home } = fixture(t);
  const cargo = tool(join(home, '.cargo', 'bin'), 'cargo');
  assert.equal(rustToolPath('cargo', { env: {}, home }), cargo);
});

test('missing tools report setup guidance without selecting a directory', t => {
  const { root, home } = fixture(t);
  mkdirSync(join(root, 'cargo.exe'));
  assert.throws(() => rustToolPath('cargo', { env: { PATH: root }, home }), /未找到 cargo.exe/);
});
