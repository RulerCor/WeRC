import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    // 只测试工作源码，不能把target/packages中的旧源码快照重复计入。
    include: ['apps/desktop/src/**/*.test.{ts,tsx}'],
  },
});
