# 游戏文件导入

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src-tauri/src/vcmi/game_import.rs`、`vcmi/mod_install.rs`、`commands/game_import.rs`、`commands/mod_install.rs`、`src/shell/ImportFilesDialog.tsx`

## 1. 目标与范围

**做**：让用户把散落的地图、存档、模组压缩包、HD 素材拖进启动器，由启动器判断类型并放到正确位置。

**不做**：修改文件内容；替代领域地图管理（见 [realms](./realms.zh-CN.md)）。

## 2. 导入分类（`vcmi/game_import.rs`）

| 输入 | 去向 |
|---|---|
| 地图（.h3m 等） | `Maps/` |
| 存档 | `Saves/` |
| 含 `mod.json` 的 zip | `Mods/<modId>` |
| json | 配置目录 |
| HD 目录 / HD 版 exe | 生成 `Mods/hd-edition`（供 HD Edition 探测） |

- **冲突策略**：`Skip` / `Overwrite` / `Rename`。
- 目录递归限深 8、上限 2000 项，防止误拖整个磁盘。
- 命令 `import_game_files(paths, conflict, dry_run, overrides)`：`dry_run=true` 只预检，`false` 执行，返回 `added / replaced / skipped / failed / unsupported / modsInstalled`。

## 3. 本地模组安装（`vcmi/mod_install.rs`）

| 命令 | 行为 |
|---|---|
| `preview_mod_zip` | **不写盘**：识别 mod 根、读元数据，返回 `modId / name / version / modType / entryCount / target / exists` |
| `install_local_mod` | 装到 `userModsDir/<modId>`；`overwrite=false` 且已存在则报错 |

实现要点：

- 先解到临时目录 `.fv-install-<id>/` 再就位，失败不留半成品；
- 只解 mod 根内的条目（支持"包根即 mod"的形态）；
- 覆盖时把旧目录改名保留，失败可回退；
- mod id 去尾随版本号（避免 `mymod-1.2` 这种目录名）；
- 写 `.fv-install.json` 台账，供 [mods-repository](./mods-repository.zh-CN.md) 的版本比较与删除使用。

## 4. 界面四态（`shell/ImportFilesDialog.tsx`）

`phase = "select" | "executing" | "done"`，用户可感知四态：

1. **空态**：缺省描述 + 拖放区；
2. **有文件态**：自动预检并给出类型徽标，每项带「改判 ▾」让用户纠正误判；
3. **执行中**：禁止关闭；
4. **结果态**：就地切换为汇总（added / replaced / skipped / failed）。

## 5. 边界与容错

- 预检先行：先 `dry_run` 再执行，用户确认后才落盘。
- 不支持的类型计入 `unsupported` 而不是报错中断。
- 带密码的 `Content.zip` 由百科侧处理（回退色块），导入侧对加密包直接判为失败并计数。

## 6. 验证

```bash
npx tauri dev   # 真机：拖入 h3m / 存档 / 模组 zip / HD 目录，观察预检与结果态
npm run verify:maps  # 地图相关校验脚本
```

## 7. 开放问题

- HD 目录导入生成 `Mods/hd-edition` 的判定依赖目录特征，遇到改版 HD 包可能需要扩充特征。
