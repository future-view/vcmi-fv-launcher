import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

/**
 * ESLint 扁平配置（eslint.config.js，ESLint 9+/10）。
 *
 * 目标不是「一把梭全开」，而是**把 docs/ide/rules/02-coding-conventions.mdc 里
 * 有实际约束力的条目落成可执行规则**，同时不制造噪音：
 *
 * - `no-restricted-imports`（error）：禁止 `../` 跨目录相对导入，强制 `@/` 别名
 *   （约定「跨目录一律用 @/，禁止 ../ 连写」）。这是最容易被顺手破坏的一条。
 * - `react-hooks/rules-of-hooks`（error）：Hooks 调用顺序错误是真 bug（不是风格）。
 * - `react-hooks/exhaustive-deps`（warn）：依赖数组遗漏多为隐患，先告警不阻断。
 * - `no-explicit-any`（warn）：约定「避免 any」，告警提醒但不阻断存量。
 *
 * 刻意**不**启用 `js.configs.recommended`：其 `no-empty` 会把项目满地的
 * `catch {}` 容错分支全标红，与「容错铁律」冲突；`no-unused-vars` 也与
 * tsconfig 的 `noUnusedLocals:false` 语义不一致。只保留有业务价值的规则。
 *
 * 只 lint 前端 `src/**`；Rust、构建产物、脚本、参考目录不在范围。
 */
export default tseslint.config(
  {
    ignores: [
      "dist/**",
      "node_modules/**",
      ".codebuddy/**",
      "src-tauri/**",
      "references/**",
      "releases/**",
      "scripts/**",
      "docs/**",
      "public/**",
      "**/*.config.js",
      "**/*.config.ts",
      "vitest.config.ts",
    ],
  },
  { linterOptions: { reportUnusedDisableDirectives: "off" } },
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      globals: { ...globals.browser },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/no-explicit-any": "warn",
      "@typescript-eslint/no-unused-vars": "off",
      "@typescript-eslint/no-empty-object-type": "off",
      "no-console": "off",
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["../*", "../**"],
              message:
                "跨目录请用 @/ 别名（docs/ide/rules/02-coding-conventions.mdc），禁止 ../ 连写。",
            },
          ],
        },
      ],
    },
  },
);
