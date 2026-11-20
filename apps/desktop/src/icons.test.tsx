import { describe, expect, it } from 'vitest';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { renderToStaticMarkup } from 'react-dom/server';
import App from './App';

const root = new URL('../../../', import.meta.url);

describe('用户指定的应用图标', () => {
  it('保留上传原图，透明补边后的 PNG 为正方形', () => {
    const original = readFileSync(new URL('assets/app-icon-original.jpg', root));
    expect(createHash('sha256').update(original).digest('hex')).toBe('eb8de2145bdc0c5f2752bc7e9af40ff91771cb915c8659aa93bc593c5e274f47');
    const png = readFileSync(new URL('assets/app-icon.png', root));
    expect(png.subarray(0, 8).toString('hex')).toBe('89504e470d0a1a0a');
    expect(png.readUInt32BE(16)).toBe(1050);
    expect(png.readUInt32BE(20)).toBe(1050);
  });

  it('Windows ICO 包含小图标和高分辨率尺寸', () => {
    const ico = readFileSync(new URL('apps/desktop/src-tauri/icons/icon.ico', root));
    expect(ico.readUInt16LE(0)).toBe(0);
    expect(ico.readUInt16LE(2)).toBe(1);
    const sizes = Array.from({ length: ico.readUInt16LE(4) }, (_, index) => ico[6 + index * 16] || 256);
    for (const size of [16, 32, 48, 256]) expect(sizes).toContain(size);
  });

  it('界面使用图片而非旧 W 占位符', () => {
    const markup = renderToStaticMarkup(<App/>);
    expect(markup).toContain('<img class="brand-mark"');
    expect(markup).not.toContain('<span class="brand-mark">W</span>');
  });
});
