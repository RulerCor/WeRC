import { statSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, join, resolve } from 'node:path';

// CARGO_HOME may contain only the dependency cache. Rust proxies can remain on
// PATH, or belong to a standalone toolchain installation.
export function rustToolPath(name, { env = process.env, home = homedir() } = {}) {
  const executable = `${name}.exe`;
  const pathKey = Object.keys(env).find(key => key.toLowerCase() === 'path');
  const candidates = [
    ...(env.CARGO_HOME ? [join(env.CARGO_HOME, 'bin', executable)] : []),
    ...(env[pathKey] ?? '').split(delimiter).filter(Boolean).map(directory =>
      join(directory.replace(/^"(.*)"$/, '$1'), executable)),
    join(home, '.cargo', 'bin', executable),
  ];
  for (const candidate of candidates) {
    try {
      if (statSync(candidate).isFile()) return resolve(candidate);
    } catch (error) {
      if (!['ENOENT', 'ENOTDIR'].includes(error.code)) throw error;
    }
  }
  throw new Error(`未找到 ${executable}；请安装项目指定的 Rust 工具链并将其 bin 目录加入 PATH。`);
}
