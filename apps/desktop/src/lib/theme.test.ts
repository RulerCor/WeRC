import { afterEach, describe, expect, it, vi } from 'vitest';
import { applyTheme, readTheme } from './theme';

afterEach(() => vi.unstubAllGlobals());

describe('主题存储降级', () => {
  it('未提供 localStorage 时回退浅色', () => {
    vi.stubGlobal('localStorage', undefined);
    expect(readTheme()).toBe('light');
  });
  it('只接受已知主题值', () => {
    vi.stubGlobal('localStorage', { getItem: () => 'invalid-value' });
    expect(readTheme()).toBe('light');
    vi.stubGlobal('localStorage', { getItem: () => 'dark' });
    expect(readTheme()).toBe('dark');
  });
  it('存储拒绝写入时仍更新窗口主题', () => {
    const dataset = { theme: 'light' };
    vi.stubGlobal('document', { documentElement: { dataset } });
    vi.stubGlobal('localStorage', { setItem: () => { throw new Error('storage blocked'); } });
    expect(() => applyTheme('dark')).not.toThrow();
    expect(dataset.theme).toBe('dark');
  });
});
