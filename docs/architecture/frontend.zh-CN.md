# 前端架构

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/App.tsx`、`src/shell/`、`src/features/`、`src/lib/`

## 1. 分层

```
shell / features/*  ──►  features/{mods, wiki, settings}  ──►  shared/  ──►  lib/
        └──────────────►  features/realms  ──┘
```

| 目录 | 是什么 | 硬约束 |
|---|---|---|
| `shell/` | 启动器主界面外壳（顶栏 / 底栏 / 抽屉 / 导入弹层 / 错误边界） | 不含页签业务 |
| `features/<域>/` | 各功能域 UI：`mods` / `wiki` / `settings` / `realms` / `setup` | 不含纯逻辑 |
| `shared/` | 跨域通用件（无业务语义）+ `ui/` 基础控件 | **不得**引用 `features/` 或 `shell/` |
| `lib/` | 纯 TS 逻辑（IPC 桥接 / 扫描 / 解析 / store / schema） | **不得** `import react` |

### 目录归属判定（新增代码放哪里）

1. 含 React 组件 / JSX 吗？没有 → `src/lib/**`；
2. 只属于某一个功能域吗？→ `src/features/<域>/**`；
3. 被两个及以上域复用、且本身不带业务语义吗？→ `src/shared/**`。

**跨域复用但带业务语义**的能力留在自己的 feature 内，不搬去 `shared/`：`features/settings`（主界面与领域共用）、`features/wiki/detail`（百科与领域信息页共用）、`features/mods/ModBrowser`（模组页与领域编辑器共用）。

### 依赖方向约束

- `features/realms/**` 可单向引用 `features/{mods,wiki,settings}` 与 `shared/`；**反向禁止**。
- 除"领域 → 被复用能力"外，同级 feature 之间不互相引用。
- `shared/` 与 `lib/` 不得反向依赖 `features/`、`shell/`。

### 跨域复用矩阵

| 被复用方 | 复用方 | 说明 |
|---|---|---|
| `features/settings/SettingsView` | `App.tsx`、`features/realms/RealmShell` | 靠 `configDir` + `scope` + `hideTitle` 跑两套数据 |
| `features/wiki/detail/*` | `features/wiki/WikiView`、`features/realms/RealmShell` | 百科详情展示层 |
| `features/mods/ModBrowser` | `features/mods/ModView`、`features/realms/editor/RealmEditor` | 同一套浏览 / 勾选组件 |
| `shared/{ConfirmDialog, WindowControls, LanguageMenuButton}` | 横跨 `shell/` 与 `features/` | 无语义通用件 |

## 2. 启动编排（`src/App.tsx`）

严格串行：

1. `detect_vcmi_paths` 解析 / 初始化目录；未就绪 → 首启向导；
2. `list_realms` + `get_active_realm`，存在活动领域则 `activate_realm`；
3. `realm_self_check`，`maps_orphan` 静默 `realm_self_heal`，其余交给对话框；
4. 异步增量构建百科缓存（不阻断）；
5. 异步静默 `check_launcher_update`。

**渲染优先级**：启动遮罩 → 首启向导 → 强制更新覆盖层 → 领域模式 → 主界面。首启向导优先于强制更新；配置就绪后强制更新会阻断领域与主界面。

## 3. 页面结构

顶栏三个入口：**领域 / 模组 / 百科**；齿轮是内容区切换（不是页签），切到设置视图。领域模式另有页面枚举 `home / info / assets / switch / editor / settings / repository / showcase`，由 `RealmShell` 分发。

## 4. 状态管理

| 机制 | 位置 | 说明 |
|---|---|---|
| `useSyncExternalStore` 外置 store | `lib/realm/store.ts`、`lib/townPreview.ts` | 模块级不可变快照 + 监听集合；变更先调后端再整表刷新 |
| 进程内缓存 | `lib/engineVersion.ts`、`lib/vcmiLanguage.ts`、`lib/hd.ts`、`lib/modcache/status.ts`、`lib/def/animationCache.ts` | 引擎版本 / 语言 / HD 倍率与缺图 / 构建状态 / DEF 两级 LRU |
| localStorage | `lib/uiPrefs.ts` | 全项目**唯一** localStorage 用途：记住"忽略官方启动器仍运行提示" |

没有引入 Redux / Zustand 等状态库；跨页共享状态一律用外置 store + 订阅。

## 5. 视觉与窗口

- 深色 Slate 基底 + 径向渐变 + 半透明边框 + 16/20px 模糊（玻璃拟态），配置在 `src/index.css` 与 `tailwind.config.js`（启用 `dark` class、自定义色板与中文字体栈）。
- 窗口最小化 / 最大化 / 关闭走 Tauri 命令（`shared/WindowControls.tsx`）；最大化状态用窗口 API 与 resize 事件双向校准。
- 图标以 `lucide-react` 为主，另有少量手写 SVG。
- 最小尺寸基准 960×640。

## 6. 已知约束

- `App.tsx` 是唯一的顶层编排点，新增启动期步骤应在此串联，避免散落到各 feature 的 `useEffect`。
- 领域模式与主界面互斥是硬约束（防两个模组管理器争写配置），不要试图让它们同时挂载。

## 7. 开放问题

- `lib/` 中 `modcache/` 与 `realm/` 已形成子域，若继续膨胀可考虑按域再拆，但目前规模尚不需要。
