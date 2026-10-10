# 替换图标

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src-tauri/tauri.conf.json`、`src-tauri/icons/`、`src/assets/brand-icon.png`

## 1. 需要替换的文件

`tauri.conf.json` 的 `bundle.icon` 引用四个文件，需同步替换 `src-tauri/icons/` 下的同名文件：

| 文件 | 用途 |
|---|---|
| `icons/32x32.png` | 任务栏小图标 |
| `icons/128x128.png` | 窗口图标 |
| `icons/128x128@2x.png` | 高 DPI 图标 |
| `icons/icon.ico` | Windows exe 图标 |

`icons/icon.png` 是源图；目录下另有 android / ios / Microsoft Store 的尺寸套件，若要覆盖全平台一并替换。

**前端品牌图标**另有一份：`src/assets/brand-icon.png`（界面内的品牌位），不在 `bundle.icon` 体系内，别漏。

## 2. 替换后

重新打包，`icon.ico` 才会写进 exe：

```bash
npx tauri build
```

## 3. 注意事项

- Windows 会缓存 exe 图标，替换后若任务栏 / 资源管理器仍显示旧图标，是缓存不是没生效。
- `icon.ico` 需包含多尺寸（16/32/48/256），否则缩放时会出现锯齿。
