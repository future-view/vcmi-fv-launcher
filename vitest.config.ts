import { defineConfig } from "vitest/config";
import path from "path";

/**
 * 单元测试配置（独立于 vite.config.ts）。
 *
 * 只覆盖 `src/lib/**` 的纯逻辑（无 React / 无 Tauri），用 node 环境即可 —— 与项目
 * 「逻辑层可被 Node 脚本单独验证」的分层原则一致。刻意不复用 vite.config.ts：
 * 那里的 mock-cache dev server 中间件与测试无关，隔离可避免副作用。
 *
 * `@/` 别名与 tsconfig/vite 保持一致，测试文件可直接 `@/lib/xxx` 引入被测模块。
 */
export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
    globals: false,
  },
});
