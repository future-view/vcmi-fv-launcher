# 首启向导

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/features/setup/SetupWizard.tsx`、`src/App.tsx`、`src-tauri/src/vcmi/paths.rs`、`vcmi/setup.rs`

## 1. 触发条件

`detect_vcmi_paths()` 返回 `ready === false` 时独占渲染（此时不进入主界面，也不渲染强制更新）。

## 2. 步骤

1. **选择界面语言**；
2. **游戏目录**：自动探测，失败则手动选择；
3. **用户数据目录**：由游戏目录推导默认值（命令 `suggest_user_data_dir`），展示派生出的 `config/` 与 `Mods/` 路径供确认；
4. **保存并校验**，就绪后重跑完整启动流程。

## 3. 落盘

写入 `launcher.json`：

- `paths.game_dir`
- `paths.user_data_dir`
- `ui.lang`

## 4. 目录识别规则

优先读 `<游戏目录>/config/dirs.json` 的 5 个真实 key（支持环境变量展开，含增强版特有的 `%USERDOCUMENTS%` 映射），缺失则回退 Windows 默认用户目录；未显式指定的 4 个 key 由 `userDataPath` 派生。完整硬事实（双重挂载、junction、预设迁移等）见 [vcmi-dirs](../reference/vcmi-dirs.zh-CN.md)。VCMI **不会**自动创建 `Mods` 与 `Maps` 子目录，需要启动器自建。

## 5. 边界与容错

- `vcmi/setup.rs` 解析失败**必须报错**，禁止降级为空对象覆写（否则会静默丢掉用户已有配置）。
- `write_launcher_config` 走「宽容继承」回退，防止向导清掉便携包预置的语言 / 高清开关 / 组件地址。
- 写入 `dirs.json` 时一律用**绝对路径**（VCMI 按进程工作目录解析相对路径）。

## 6. 验证

```bash
npx tauri dev   # 真机：清空 launcher.json 的 paths 后重启，走完整向导
```

## 7. 开放问题

- 向导目前不可回退到上一步（只能重来），是否加入"上一步"待评估。
