import { useEffect, useState } from "react";
import { useI18n, UI_LANG_OPTIONS } from "@/i18n";
import { uiLangFeature } from "@/i18n/langs";
import {
  cancelUserDataCopy,
  copyH3Data,
  copyUserData,
  detectVcmiPaths,
  gamedataHealthCheck,
  getRealDirs,
  hdEditionStatus,
  importGameFiles,
  onUserDataCopyProgress,
  pickDirectory,
  pickFile,
  probeUpdateManifest,
  readLauncherConfig,
  saveHdEditionUse,
  saveHdEnhance,
  saveNetworkMirrorRules,
  suggestUserDataDir,
  writeLauncherConfig,
  type H3DataStatus,
  type VcmiDirs,
} from "@/lib/tauri";
import { DEFAULT_GITHUB_MIRROR_PREFIX, githubMirrorRule } from "@/lib/mirrorRules";
import { backendErrorText } from "@/lib/backendError";
import { logWarn } from "@/lib/log";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/ui/select";

export interface SetupWizardProps {
  onReady: () => void;
}

/**
 * 固定四步向导（步骤条恒定四项，**没有时隐时现的步骤**）：
 * ① 游戏目录与界面语言 → ② 用户数据目录（含既有数据同步）→
 * ③ 游戏数据文件（检测 + 导入，硬闸门）→ ④ 汇总确认（派生信息 + 保存）。
 *
 * 「保存并进入启动器」只在最后一步出现；中途的前进由各步自身条件把关
 * （③ 的「下一步」在数据未就绪前禁用）。
 */
type Step = 1 | 2 | 3 | 4;

/** 同步状态机：`idle`（可同步）→ `running`（锁定）→ `done`（已同步，锁定路径与开关） */
type SyncState = "idle" | "running" | "done";

interface CopyProgressState {
  /** 已收到首帧进度；此前显示"正在准备复制…"（后端在整树预扫体量） */
  started: boolean;
  doneFiles: number;
  doneBytes: number;
  totalFiles: number;
  totalBytes: number;
}

const IDLE_PROGRESS: CopyProgressState = {
  started: false,
  doneFiles: 0,
  doneBytes: 0,
  totalFiles: 0,
  totalBytes: 0,
};

/**
 * 弹窗尺寸：固定高度 + 加宽（`max-w-2xl`）。
 * 加宽是为了让第 ③/④ 步的四项步骤条在**长语言**（俄/德）下仍能单行显示，
 * 同时给路径字段更多空间；高度取最长语言在最满一步的值，切步骤不跳动。
 */
const DIALOG_HEIGHT = "h-[min(520px,88vh)]";
const DIALOG_WIDTH = "max-w-2xl";

/** 路径等值比较（Windows 大小写不敏感 + 去尾部分隔符） */
function normPath(p: string): string {
  return p.trim().replace(/[\\/]+$/, "").toLowerCase();
}

/** 字节数 → 人类可读（1024 进制，保留一位小数；不足 1KB 直接给字节） */
function fmtBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), units.length - 1);
  const v = n / Math.pow(1024, i);
  return `${i === 0 ? v : v.toFixed(1)} ${units[i]}`;
}

export function SetupWizard({ onReady }: SetupWizardProps) {
  const { t, lang, vcmiLang, setVcmiLang } = useI18n();
  const [detected, setDetected] = useState<VcmiDirs | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const [step, setStep] = useState<Step>(1);
  const [gameDir, setGameDir] = useState("");
  const [userDataDir, setUserDataDir] = useState("");
  // 自定义用户数据目录（隔离多个发行版）与其同步动作
  const [customUserData, setCustomUserData] = useState(false);
  const [syncState, setSyncState] = useState<SyncState>("idle");
  const [syncNote, setSyncNote] = useState("");
  const [syncDone, setSyncDone] = useState<{ files: number; bytes: number } | null>(null);
  const [progress, setProgress] = useState<CopyProgressState>(IDLE_PROGRESS);
  // 网络：清单不可达（仅 GitHub 时才建议开镜像；汇总步展示）
  const [netUnreachable, setNetUnreachable] = useState(false);
  const [mirrorOn, setMirrorOn] = useState(true);
  // 第 ④ 步：探测到既有 hd-edition 时就地展示「启用生成高清素材」建议，随保存一起生效
  const [hdDetected, setHdDetected] = useState(false);
  const [hdEnable, setHdEnable] = useState(true);
  // 第 ③ 步：原版数据文件（固定步骤的硬闸门）
  const [h3, setH3] = useState<H3DataStatus | null>(null);
  const [h3Checking, setH3Checking] = useState(false);
  /** 是否已成功探测过一次（`false` = 还没探测 / 探测失败，均按"未确认"处理） */
  const [h3Probed, setH3Probed] = useState(false);
  const [h3DataDir, setH3DataDir] = useState("");
  const [h3Candidates, setH3Candidates] = useState<string[]>([]);
  // 第 ③ 步复制范围：音轨默认带（官方同口径），地图可选（不复制也会自动建 Maps 目录）
  const [h3CopyMp3, setH3CopyMp3] = useState(true);
  const [h3CopyMaps, setH3CopyMaps] = useState(false);
  /**
   * `.lod` 复制目标：默认**游戏目录 Data**（领域隔离下的唯一安全选择——领域只隔离
   * 用户数据目录，游戏目录被所有领域共享；放进用户数据目录会导致切换领域后数据不可见、
   * 对应领域直接起不来）。仍保留用户数据目录选项（如游戏目录只读），选中时给警示。
   */
  const [h3Target, setH3Target] = useState<"game" | "user">("game");
  // 可选内容（第 ③ 步配置 + 点击安装、第 ④ 步汇总）：高清版素材 / 历代记战役
  const [hdInstallDir, setHdInstallDir] = useState("");
  const [hdInstallOn, setHdInstallOn] = useState(false);
  /** HD 安装结果：`null`=未安装，`true`=成功，`false`=失败（保留开关状态供重试） */
  const [hdInstalled, setHdInstalled] = useState<boolean | null>(null);
  const [chrInstallFile, setChrInstallFile] = useState("");
  const [chrInstallOn, setChrInstallOn] = useState(false);
  const [chrInstalled, setChrInstalled] = useState<boolean | null>(null);
  const [installingExtra, setInstallingExtra] = useState(false);
  /**
   * 可选内容安装目标：默认**游戏目录**（领域共享；两项产物都在 `<目标>/Mods/`：
   * HD → `hd-edition`，历代记 → `chronicles` + 8 个篇章子模组）。
   * 选中用户数据目录时同 LOD radio 的领域警示。
   */
  const [optTarget, setOptTarget] = useState<"game" | "user">("game");
  /** 非阻断提示（历代记导入失败的具体原因），琥珀色，不阻止进入启动器 */
  const [notice, setNotice] = useState("");

  const portableDir = detected?.portableUserDataDir ?? "";
  const portableExists = !!detected?.portableUserDataDirExists;
  const systemDir = detected?.systemUserDataDir ?? "";
  const systemExists = !!detected?.systemUserDataDirExists;
  /**
   * 同步源：优先用**解析出的**用户目录（`dirs.json` 可能指向别处，如增强版），
   * 否则退回官方默认目录。切回默认目录时也以它回填表单。
   */
  const sourceDir = detected?.userDir?.trim() || systemDir;
  const sourceExists = sourceDir === systemDir ? systemExists : !!detected?.userDirExists;
  /** 是否展示「GitHub 镜像」建议：探测与语言无关，但建议只给有需要的语言 */
  const mirrorHintAllowed = uiLangFeature(lang, "githubMirrorHint");
  /** 同步进行中：锁定一切（路径 / 开关 / 上一步 / 下一步） */
  const syncing = syncState === "running";
  /** 已同步：锁定路径与开关，防止复制到旧路径留下孤儿目录 */
  const syncLocked = syncState === "done";

  /** 原版数据闸门：**未确认即视为未通过**（缺 LOD 的后果是 VCMI 无法启动） */
  const h3Ready = h3Probed && !!h3?.found;

  /** 第 ④ 步汇总里的「高清版素材」是否为已启用：本次装好，或探测到既有 hd-edition 且开关保持打开 */
  const hdActive = hdInstalled === true || (hdDetected && hdEnable);

  useEffect(() => {
    detectVcmiPaths().then((d) => {
      if (d) {
        setDetected(d);
        // 自动探测默认触发：探测到有效路径即回填表单（仅填充，不产生任何磁盘副作用），
        // 等待用户走完向导点"保存并进入启动器"后才落盘 launcher.json / 提取城镇图
        if (d.detected) {
          setGameDir(d.gameDir || "");
          setUserDataDir(d.userDir || "");
          setError("");
        }
      }
    });
    // 清单可达性探测：后台异步跑，不阻塞填表；失败且是 GitHub 地址时才给建议
    let alive = true;
    probeUpdateManifest().then((p) => {
      if (alive && p && !p.ok && p.github) setNetUnreachable(true);
    });
    return () => {
      alive = false;
    };
  }, []);

  // 迁移进度订阅（后端 200ms 一帧 + 末帧）
  useEffect(() => {
    let unsub: (() => void) | undefined;
    let alive = true;
    onUserDataCopyProgress((p) =>
      setProgress({
        started: true,
        doneFiles: p.doneFiles,
        doneBytes: p.doneBytes,
        totalFiles: p.totalFiles,
        totalBytes: p.totalBytes,
      }),
    ).then((fn) => {
      if (alive) {
        unsub = fn;
      } else {
        fn(); // 组件已卸载：立即退订，避免泄漏
      }
    });
    return () => {
      alive = false;
      unsub?.();
    };
  }, []);

  /** 同步结果作废（游戏目录变更 / 关闭自定义目录 / 重新自动探测） */
  const resetSync = () => {
    setSyncState("idle");
    setSyncDone(null);
    setSyncNote("");
    setProgress(IDLE_PROGRESS);
  };

  /**
   * 同步区**可见性联动**：仅当目标目录 ≠ 同步源时才有意义 ——
   * 开了专属目录（数据从零开始），或手动选了非默认目录。
   * 维持默认目录（= 源本身）时隐藏，并给一句说明，避免"开关开了却什么都没有"的困惑。
   */
  const showSync = !!gameDir.trim() && normPath(userDataDir) !== normPath(sourceDir);

  // 同步区隐藏时，未完成的同步状态一并作废（目标已切回默认目录）
  useEffect(() => {
    if (!showSync) resetSync();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showSync]);

  /**
   * 原版数据探测（第 ③ 步硬闸门的唯一判据）。
   *
   * - **在第 ③ 步才探**：此时用户数据目录已在第 ② 步定稿，判定根不会再变；
   * - **fail-closed**：命令失败 / 后端未返回 `h3Data` ⇒ `h3Probed=false` ⇒ 不放行；
   * - `withCandidates`：注册表查询（`reg query` ×4，约百毫秒）只在**确认缺失**时才补一次，
   *   避免拖慢常态下的状态显示。
   */
  const checkH3Data = async (
    gd: string,
    ud: string,
    withCandidates = false,
  ): Promise<H3DataStatus | null> => {
    if (!gd.trim() || !ud.trim()) return null;
    setH3Checking(true);
    try {
      const health = await gamedataHealthCheck(gd, ud, withCandidates);
      if (!health) {
        setH3Probed(false);
        return null;
      }
      const st = health.h3Data ?? null;
      setH3(st); // ⚠️ 必须回写状态：只置 h3Probed 会让面板永远显示空缺失清单（曾踩过）
      setH3Probed(!!st);
      setH3Candidates(health.dataInstallCandidates ?? []);
      if (st && !st.found) {
        logWarn(
          "setup",
          `原版数据缺失：${st.missing.join(", ")}（roe=${st.roe} sod=${st.sod} demo=${st.demo}）`,
        );
        if (!withCandidates) {
          // 仅在缺失路径上补一次注册表候选（预填数据目录），常态不付这个开销
          void gamedataHealthCheck(gd, ud, true).then((full) => {
            if (full) setH3Candidates(full.dataInstallCandidates ?? []);
          });
        }
      }
      return st;
    } catch (e) {
      console.warn("[setup] 原版数据探测失败", e);
      logWarn("setup", `原版数据探测失败：${String(e)}`);
      setH3Probed(false);
      return null;
    } finally {
      setH3Checking(false);
    }
  };

  // 进入第 ③ 步 / 用户数据目录变化 / 同步结束 ⇒ 重探（②→③ 是唯一入口，判定根已定稿）
  useEffect(() => {
    if (step !== 3) return;
    void checkH3Data(gameDir.trim(), userDataDir.trim());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, userDataDir, syncState]);

  // 进入第 ④ 步 ⇒ 按**用户刚选的目录**探测 hd-edition（此时 launcher.json 还没写）
  useEffect(() => {
    if (step !== 4) return;
    let alive = true;
    void (async () => {
      const hd = await hdEditionStatus({
        gameDir: gameDir.trim(),
        userDataDir: userDataDir.trim(),
      });
      if (alive) setHdDetected(!!hd?.found);
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, gameDir, userDataDir]);

  /** 选择游戏目录：校验是否含 VCMI_client.exe，并自动推导用户数据目录 */
  const handleChooseGame = async () => {
    const path = await pickDirectory(t.setup.gameDir);
    if (!path) return;
    setGameDir(path);
    setError("");
    // 换游戏目录 ⇒ 同步源可能变了，已同步结果作废
    resetSync();
    const suggested = await suggestUserDataDir(path);
    if (suggested) {
      setUserDataDir(suggested.userDataDir);
      if (!suggested.exists) setError(t.setup.missingUserData);
    }
  };

  const handleChooseUser = async () => {
    const path = await pickDirectory(t.setup.userDataDir);
    if (!path) return;
    setUserDataDir(path);
    setError("");
  };

  /** 切换「启动器专属用户数据目录」：开 → 填 exe 同目录 userdata；关 → 回到解析出的用户目录 */
  const toggleCustomUserData = (on: boolean) => {
    setCustomUserData(on);
    resetSync();
    if (on) {
      if (portableDir) setUserDataDir(portableDir);
    } else {
      setUserDataDir(sourceDir || userDataDir);
    }
  };

  /** 自动探测：探测到有效目录组合即可填充表单，否则禁用 */
  const autoFill = () => {
    if (!detected?.detected) return;
    setGameDir(detected.gameDir || "");
    setUserDataDir(detected.userDir || "");
    setCustomUserData(false);
    resetSync();
    setError("");
  };

  /** 显式同步：只迁移数据、不写任何配置（保存按钮不再隐式复制） */
  const handleSync = async () => {
    const src = sourceDir.trim();
    const dst = userDataDir.trim();
    if (!src || !dst) {
      setError(t.setup.error);
      return;
    }
    setError("");
    setSyncNote("");
    setSyncState("running");
    setProgress(IDLE_PROGRESS);
    try {
      const r = await copyUserData(gameDir.trim(), src, dst);
      setProgress(IDLE_PROGRESS);
      if (r.copied) {
        setSyncDone({ files: r.files, bytes: r.bytes });
        setSyncState("done");
      } else {
        // 目标已存在 / 源不存在：后端静默跳过，这里给出说明（不是错误）
        setSyncState("idle");
        setSyncNote(
          portableExists && dst === portableDir
            ? t.setup.copyTargetExists
            : t.setup.copySourceMissing,
        );
      }
    } catch (e: any) {
      setProgress(IDLE_PROGRESS);
      setSyncState("idle");
      setError(backendErrorText(e) || t.setup.error);
    }
  };

  /** 跳过复制：置后端哨兵，复制循环下一个检查点退出 */
  const handleSkipSync = async () => {
    try {
      await cancelUserDataCopy();
    } catch {
      // 取消失败不阻断：复制可能已接近结束
    }
  };

  /** 写入镜像规则（保留既有规则，同前缀不重复添加）；失败不影响进入启动器 */
  const applyMirrorRule = async () => {
    try {
      const cfg = await readLauncherConfig();
      const prev = cfg?.mirrorRules ?? [];
      if (prev.some((r) => r.mode === "prefix" && r.prefix === DEFAULT_GITHUB_MIRROR_PREFIX)) {
        return;
      }
      await saveNetworkMirrorRules([
        ...prev,
        githubMirrorRule(DEFAULT_GITHUB_MIRROR_PREFIX, t.setup.netMirrorEnable),
      ]);
    } catch {
      // 镜像只是便利设施：写失败不阻断初始化
    }
  };

  const finish = async () => {
    const dirs = await getRealDirs();
    if (!dirs?.ready) {
      setError(t.setup.error);
      return;
    }
    onReady();
  };

  const handleSave = async () => {
    const gd = gameDir.trim();
    const ud = userDataDir.trim();
    if (!gd || !ud) {
      setError(t.setup.error);
      return;
    }
    // 双保险：③ 的「下一步」禁用是主路径，这里防状态竞态（探测刚失败）误放行
    if (!h3Ready) {
      const st = await checkH3Data(gd, ud);
      if (!st?.found) {
        setStep(3);
        setError(t.setup.h3GateHint);
        return;
      }
    }
    setSaving(true);
    setError("");
    try {
      // 语言随配置一并写入：向导阶段 launcher.json 尚不存在，无法走 set_ui_lang；
      // 这样便携包首次保存后语言即被固化，后续启动以它为唯一权威。
      // ⚠️ 不再隐式复制：数据同步是第 ② 步的显式动作（单一职责、进度可见可跳过）。
      await writeLauncherConfig({
        gameDir: gd,
        userDataDir: ud,
        uiLang: vcmiLang,
        redirectUserData: customUserData,
      });
      // 可选内容已在第 ③ 步**点击安装**时落盘（显式 `userDataDir`），这里不再重复安装
      if (mirrorOn && netUnreachable) await applyMirrorRule();
      // 第 ④ 步探测到的既有 hd-edition：勾选则开启「生成高清素材」+ 素材来源（auto）。
      // 这两项写入依赖 launcher.json 已存在，故必须排在 writeLauncherConfig 之后。
      if (hdDetected && hdEnable) {
        await saveHdEnhance(true);
        await saveHdEditionUse("auto");
      }
      await finish();
    } catch (e: any) {
      setError(backendErrorText(e) || t.setup.error);
    } finally {
      setSaving(false);
    }
  };

  /**
   * 第 ③ 步：复制外部数据目录，随后重新检测。
   * ⚠️ radio 决定**全部**数据的归属（`.lod` / `Mp3` / `Maps` 同一个根）——
   * 此前只把 `.lod` 发往 radio 目标、Mp3/Maps 恒发用户数据目录，用户实测报过。
   */
  const handleCopyH3 = async () => {
    const gd = gameDir.trim();
    const ud = userDataDir.trim();
    const from = h3DataDir.trim();
    const targetRoot = h3Target === "game" ? gd : ud;
    const to = `${targetRoot}\\Data`;
    if (!gd || !ud || !from) {
      setError(t.setup.error);
      return;
    }
    setError("");
    setProgress(IDLE_PROGRESS);
    try {
      // 第二参 = Mp3/Maps 的归属根：与 .lod 同一个 radio 目标
      await copyH3Data(gd, targetRoot, from, to, h3CopyMp3, h3CopyMaps);
      setProgress(IDLE_PROGRESS);
      await checkH3Data(gd, ud, false);
    } catch (e: any) {
      setProgress(IDLE_PROGRESS);
      setError(backendErrorText(e) || t.setup.error);
    }
  };

  /** 第 ③ 步：选择 HD 版安装目录（含 HOMM3 2.0.exe 的目录，或该 exe 本体） */
  const handleChooseHdDir = async () => {
    const path = await pickDirectory(t.setup.hdInstall);
    if (!path) return;
    setHdInstallDir(path);
    setError("");
  };

  /** 第 ③ 步：选择历代记离线安装器（.exe，GOG 离线包） */
  const handleChooseChrFile = async () => {
    const path = await pickFile(t.setup.chrInstall, ["exe"]);
    if (!path) return;
    setChrInstallFile(path);
    setError("");
  };

  /**
   * 点击「安装」：走 `import_game_files`（HD → `<目标>/Mods/hd-edition`；
   * 历代记 → 内建解包 + 生成 `<目标>/Mods/chronicles*`，临时解包目录也在 `<目标>/tmp`）。
   *
   * `userDataDir` 显式传入：向导阶段 launcher.json 还没写，导入命令不能只依赖
   * `State<AppPaths>`（否则会按空路径落盘）。
   */
  const runInstall = async (
    src: string,
    kind: "hd" | "chr",
  ): Promise<{ ok: boolean; msg?: string }> => {
    // 安装目标根：radio（默认游戏目录）。两项产物都落在 <目标>/Mods/，
    // 历代记的临时解包目录同样跟随该目标（<目标>/tmp/chronicles-extract，结束后删除）。
    // 命令参数名叫 userDataDir（历史命名），语义是"导入环境的用户数据根"。
    const targetRoot = (optTarget === "game" ? gameDir.trim() : userDataDir.trim()).trim();
    if (!targetRoot || !src.trim()) return { ok: false, msg: t.setup.error };
    setInstallingExtra(true);
    setError("");
    try {
      const report = await importGameFiles({
        paths: [src.trim()],
        conflict: "overwrite",
        dryRun: false,
        gameDir: gameDir.trim(),
        userDataDir: targetRoot,
      });
      const item = report.items[0];
      if (item && item.status === "failed") {
        return { ok: false, msg: item.reason || item.path };
      }
      return { ok: true };
    } catch (e: any) {
      return { ok: false, msg: backendErrorText(e) || t.setup.error };
    } finally {
      setInstallingExtra(false);
    }
  };

  const handleInstallHd = async () => {
    const r = await runInstall(hdInstallDir, "hd");
    setHdInstalled(r.ok);
    if (r.msg) setError(r.msg);
  };

  const handleInstallChr = async () => {
    const r = await runInstall(chrInstallFile, "chr");
    setChrInstalled(r.ok);
    if (r.msg) setNotice(r.msg); // 失败原因（安装包不完整 / 非 Inno 包）用琥珀字提示，不阻断
  };

  /** 第 ③ 步：重新检测（用户自己手动放好文件后） */
  const handleRedetectH3 = async () => {
    await checkH3Data(gameDir.trim(), userDataDir.trim(), true);
  };

  /** 第 ③ 步：选择外部数据目录（Data 目录或其游戏根目录） */
  const handleChooseH3Dir = async () => {
    const path = await pickDirectory(t.setup.h3DataDir);
    if (!path) return;
    setH3DataDir(path);
    setError("");
  };

  // 第 ③ 步数据面板展开时：用注册表候选预填数据目录（用户可改）
  useEffect(() => {
    if (step === 3 && !h3DataDir.trim() && h3Candidates.length > 0) {
      setH3DataDir(h3Candidates[0]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, h3Candidates]);

  const pct =
    progress.totalBytes > 0
      ? Math.min(100, Math.round((progress.doneBytes / progress.totalBytes) * 100))
      : 0;

  const stepLabels = [
    t.setup.gameDir,
    t.setup.userDataDir,
    t.setup.h3Title,
    t.setup.step4Summary,
  ];

  return (
    // 卡片本体：定位 / 居中 / 标题栏（含窗口三键）由 `App.tsx` 的首启布局负责。
    // ⚠️ 这里**不能**再用 fixed 全屏遮罩——会把 InitTopBar 一起盖掉（曾回归过一次）。
    <div
      className={`flex ${DIALOG_HEIGHT} w-full ${DIALOG_WIDTH} flex-col rounded-2xl border border-slate-800 bg-slate-900/90 p-6 shadow-2xl backdrop-blur-xl`}
    >
      {/* ── 顶部标题区：标题 + 副标题 + 步骤条（四项固定不动） ── */}
      <header className="shrink-0">
        <h1 className="text-2xl font-bold text-sky-400">{t.setup.title}</h1>
        <p className="mt-1 text-sm text-slate-400">{t.setup.subtitle}</p>
        <div className="mt-4 flex flex-nowrap items-center gap-1.5 text-[11px]">
          {stepLabels.map((label, i) => (
            <span
              key={label}
              title={label}
              className={`flex min-w-0 items-center gap-1 rounded-full px-2 py-0.5 ${
                i + 1 === step ? "bg-sky-600/20 text-sky-300" : "text-slate-500"
              }`}
            >
              <span className="font-mono">{i + 1}</span>
              {/* 长语言（俄/德）下标签截断，hover 看全称；保证四步永远单行 */}
              <span className="max-w-[8.5rem] truncate">{label}</span>
            </span>
          ))}
          <span className="ml-auto shrink-0 text-slate-500">
            {t.setup.step.replace("{arg}", String(step))}
          </span>
        </div>
      </header>

      {/* ── 中部内容区：唯一随步骤切换的区域 ── */}
      <div className="min-h-0 flex-1 overflow-y-auto pt-4">
        {step === 1 ? (
          /* ① 游戏目录与界面语言（只放必填项，不带额外说明行） */
          <>
            <div className="mb-4 flex items-center justify-between gap-3">
              <label className="text-sm font-medium text-slate-300">{t.setup.language}</label>
              <Select value={vcmiLang} onValueChange={(v) => setVcmiLang(v)}>
                <SelectTrigger className="w-44 shrink-0 bg-slate-950/60 text-slate-200">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent className="z-[200] border-slate-700 bg-slate-900 text-slate-200">
                  {UI_LANG_OPTIONS.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <PathField
              label={t.setup.gameDir}
              hint={t.setup.gameDirHint}
              choose={t.setup.choose}
              value={gameDir}
              onChoose={handleChooseGame}
            />
          </>
        ) : step === 2 ? (
          /* ② 用户数据目录（专属开关 + 路径 + 既有数据同步） */
          <>
            <label className="mb-3 flex items-start justify-between gap-3">
              <span className="min-w-0 flex-1">
                <span className="block break-words text-sm font-medium text-slate-300">
                  {t.setup.customUserData}
                </span>
                <span className="mt-0.5 block break-words text-xs text-slate-500">
                  {t.setup.customUserDataHint}
                </span>
              </span>
              <Switch
                checked={customUserData}
                onCheckedChange={toggleCustomUserData}
                disabled={syncing || syncLocked}
              />
            </label>

            <PathField
              label={customUserData ? t.setup.customUserDataDir : t.setup.userDataDir}
              hint={customUserData ? t.setup.customUserDataDirHint : t.setup.userDataDirHint}
              choose={t.setup.choose}
              value={userDataDir}
              onChoose={handleChooseUser}
              disabled={syncing || syncLocked}
            />

            {/* 数据同步：显式按钮（保存不再隐式复制）；仅当目标 ≠ 同步源时显示，
                三种状态在同一框内原地切换。目标即来源时给一句说明，避免"开关开了却什么都没有"。 */}
            {showSync ? (
              <div className="mb-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3">
                {syncing ? (
                  <>
                    <div className="text-sm text-slate-200">
                      {progress.started
                        ? t.setup.copyProgress.replace("{arg}", `${pct}%`)
                        : t.setup.copyPreparing}
                    </div>
                    {progress.started && (
                      <>
                        <div className="mt-2 h-1.5 w-full overflow-hidden rounded bg-slate-800">
                          <div
                            className="h-full bg-sky-500 transition-[width] duration-200"
                            style={{ width: `${pct}%` }}
                          />
                        </div>
                        <div className="mt-2 break-words text-xs text-slate-400">
                          {t.setup.copyBytes.replace(
                            "{arg}",
                            `${fmtBytes(progress.doneBytes)} / ${fmtBytes(progress.totalBytes)}`,
                          )}
                          {" · "}
                          {t.setup.copyFiles.replace(
                            "{arg}",
                            `${progress.doneFiles} / ${progress.totalFiles}`,
                          )}
                        </div>
                      </>
                      )}
                    <p className="mt-2 break-words text-xs text-slate-500">
                      {t.setup.copyHintNoClose}
                    </p>
                    <div className="mt-2 flex justify-end">
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => void handleSkipSync()}
                        className="border-slate-700 text-slate-300 hover:bg-slate-800"
                      >
                        {t.setup.copySkip}
                      </Button>
                    </div>
                  </>
                ) : syncDone ? (
                  <p className="break-words text-xs text-emerald-400">
                    {t.setup.syncDone.replace("{arg}", fmtBytes(syncDone.bytes))}
                    {" · "}
                    {t.setup.copyFiles.replace("{arg}", String(syncDone.files))}
                  </p>
                ) : (
                  <div className="flex items-center gap-3">
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => void handleSync()}
                      disabled={!sourceExists}
                      className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
                    >
                      {t.setup.syncData}
                    </Button>
                    <span className="min-w-0 flex-1 break-words text-xs text-slate-500">
                      {!sourceExists
                        ? t.setup.copySourceMissing
                        : syncNote || t.setup.syncSource.replace("{arg}", sourceDir || "—")}
                    </span>
                  </div>
                )}
              </div>
            ) : (
              <p className="mb-3 break-words text-xs text-slate-500">{t.setup.syncNoNeed}</p>
            )}
          </>
        ) : step === 3 ? (
          /* ③ 游戏数据文件（固定步骤：检测 + 导入，硬闸门）+ 可选内容 */
          <>
          <H3DataGate
            h3={h3}
            h3Checking={h3Checking}
            h3Probed={h3Probed}
            h3DataDir={h3DataDir}
            setH3DataDir={setH3DataDir}
            h3Candidates={h3Candidates}
            copyMp3={h3CopyMp3}
            setCopyMp3={setH3CopyMp3}
            copyMaps={h3CopyMaps}
            setCopyMaps={setH3CopyMaps}
            lodTarget={h3Target}
            setLodTarget={setH3Target}
            gameDir={gameDir.trim()}
            userDataDir={userDataDir.trim()}
            onChooseDir={handleChooseH3Dir}
            onCopy={() => void handleCopyH3()}
            onRedetect={() => void handleRedetectH3()}
            copying={progress.started}
            pct={pct}
            progress={progress}
          />

          {/* 可选内容（默认收起，勾上开关才展开，避免信息过载） */}
          <OptionalContent
            hdInstallOn={hdInstallOn}
            setHdInstallOn={setHdInstallOn}
            hdInstallDir={hdInstallDir}
            hdInstalled={hdInstalled}
            onChooseHdDir={handleChooseHdDir}
            onInstallHd={() => void handleInstallHd()}
            chrInstallOn={chrInstallOn}
            setChrInstallOn={setChrInstallOn}
            chrInstallFile={chrInstallFile}
            chrInstalled={chrInstalled}
            onChooseChrFile={handleChooseChrFile}
            onInstallChr={() => void handleInstallChr()}
            optTarget={optTarget}
            setOptTarget={setOptTarget}
            gameDir={gameDir.trim()}
            userDataDir={userDataDir.trim()}
            disabled={installingExtra}
          />
          </>
        ) : (
          /* ④ 确认并保存：只读信息 + 可选内容汇总 + 保存入口 */
          <>
            <div className="mb-3">
              <div className="mb-1 flex items-baseline justify-between gap-2">
                <label className="text-sm font-medium text-slate-300">{t.setup.gameDir}</label>
                <span className="min-w-0 break-words font-mono text-xs text-slate-400">
                  {gameDir || "—"}
                </span>
              </div>
              <div className="flex items-baseline justify-between gap-2">
                <label className="text-sm font-medium text-slate-300">
                  {customUserData ? t.setup.customUserDataDir : t.setup.userDataDir}
                </label>
                <span className="min-w-0 break-words font-mono text-xs text-slate-400">
                  {userDataDir || "—"}
                </span>
              </div>
            </div>

            {/* 派生目录：卡片两行（单行「·」分隔的长路径可读性差） */}
            <div className="mb-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3 text-xs text-slate-400">
              <div className="mb-1 font-medium text-slate-300">{t.setup.derived}</div>
              <div className="break-words">
                {t.setup.userConfigDir}:{" "}
                <span className="font-mono text-slate-500">
                  {userDataDir ? `${userDataDir}\\config` : "—"}
                </span>
              </div>
              <div className="break-words">
                {t.setup.userModsDir}:{" "}
                <span className="font-mono text-slate-500">
                  {userDataDir ? `${userDataDir}\\Mods` : "—"}
                </span>
              </div>
            </div>

            {/* 检测到既有 hd-edition：建议就地开关，随「保存并进入启动器」一起写入 */}
            {hdDetected && (
              <div className="mb-3 rounded-xl border border-sky-800/60 bg-sky-950/30 p-3">
                <div className="text-sm font-medium text-sky-300">{t.setup.hdDetected}</div>
                <p className="mt-1 break-words text-xs text-slate-400">{t.setup.hdDetectedHint}</p>
                <label className="mt-2 flex items-start justify-between gap-3">
                  <span className="min-w-0 flex-1 break-words text-sm text-slate-200">
                    {t.setup.hdEnable}
                  </span>
                  <Switch checked={hdEnable} onCheckedChange={setHdEnable} />
                </label>
              </div>
            )}

            {/* 清单不可达 → 建议开启 GitHub 镜像（仅部分界面语言展示） */}
            {netUnreachable && mirrorHintAllowed && (
              <div className="rounded-xl border border-amber-800/60 bg-amber-950/20 p-3">
                <label className="flex items-start justify-between gap-3">
                  <span className="min-w-0 flex-1">
                    <span className="block break-words text-sm font-medium text-amber-300">
                      {t.setup.netUnreachable}
                    </span>
                    <span className="mt-0.5 block break-words text-xs text-slate-400">
                      {t.setup.netMirrorSuggest}
                    </span>
                  </span>
                  <Switch checked={mirrorOn} onCheckedChange={setMirrorOn} />
                </label>
                <p className="mt-2 break-words text-xs text-slate-500">
                  {t.setup.netMirrorNote}
                </p>
              </div>
            )}

            {/* 可选内容汇总：无论装不装，都显示高清版 / 历代记的当前选择 */}
            <div className="mt-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3 text-xs text-slate-400">
              <div className="mb-1 font-medium text-slate-300">{t.setup.optionalContent}</div>
              {/* 只显示启用状态，不显示来源路径（路径长且无决策价值，用户实测反馈） */}
              <div className="break-words">
                {t.setup.hdInstall}:{" "}
                <span className={hdActive ? "text-emerald-400" : "text-slate-500"}>
                  {hdActive ? t.setup.optEnabled : t.setup.optDisabled}
                </span>
              </div>
              <div className="break-words">
                {t.setup.chrInstall}:{" "}
                <span className={chrInstalled === true ? "text-emerald-400" : "text-slate-500"}>
                  {chrInstalled === true ? t.setup.optEnabled : t.setup.optDisabled}
                </span>
              </div>
            </div>
          </>
          )}

          {notice && <p className="mt-3 break-words text-xs text-amber-400">{notice}</p>}
          {error && <p className="mt-3 break-words text-sm text-red-400">{error}</p>}
      </div>

      {/* ── 底部功能区：保存按钮只在最后一步（④）出现 ── */}
      <footer className="mt-4 flex shrink-0 items-center justify-between gap-3 border-t border-slate-800 pt-4">
        <>
            {step === 1 ? (
              <Button
                type="button"
                variant="outline"
                onClick={autoFill}
                disabled={!detected?.detected}
                className="border-slate-700 text-slate-300 hover:bg-slate-800"
              >
                {t.setup.autoDetect}
              </Button>
            ) : (
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setError("");
                  setStep((s) => (s - 1) as Step);
                }}
                // 安装进行中（HD 拷贝 / 历代记解包）临时锁导航：避免没处理完就切页
                disabled={saving || syncing || installingExtra}
                className="border-slate-700 text-slate-300 hover:bg-slate-800"
              >
                {t.setup.prev}
              </Button>
            )}
            {step === 4 ? (
              <Button
                type="button"
                onClick={handleSave}
                disabled={saving}
                className="bg-sky-600 px-6 text-white hover:bg-sky-500"
              >
                {saving ? t.common.loading : t.setup.save}
              </Button>
            ) : (
              <Button
                type="button"
                onClick={() => {
                  setError("");
                  setStep((s) => (s + 1) as Step);
                }}
                disabled={
                  saving ||
                  syncing ||
                  installingExtra || // 安装进行中临时锁导航（同上）
                  (step === 1 && !gameDir.trim()) ||
                  (step === 2 && !userDataDir.trim()) ||
                  // ③ 是硬闸门：原版数据未确认（缺失 / 探测失败）时不能前进
                  (step === 3 && !h3Ready)
                }
                title={step === 3 && !h3Ready ? t.setup.h3GateHint : undefined}
                className="bg-sky-600 px-6 text-white hover:bg-sky-500 disabled:opacity-50"
              >
                {t.setup.next}
              </Button>
            )}
        </>
      </footer>
    </div>
  );
}

/**
 * 可选内容（第 ③ 步底部）：高清版素材 / 历代记战役。
 * 两个开关**默认关闭**，勾上后才展开对应目录/文件选择——避免第 ③ 步信息过载。
 * 安装是**点击即装**（保存前显式传 `gameDir` / `userDataDir`），成功后才锁定该行。
 */
function OptionalContent({
  hdInstallOn,
  setHdInstallOn,
  hdInstallDir,
  hdInstalled,
  onChooseHdDir,
  onInstallHd,
  chrInstallOn,
  setChrInstallOn,
  chrInstallFile,
  chrInstalled,
  onChooseChrFile,
  onInstallChr,
  optTarget,
  setOptTarget,
  gameDir,
  userDataDir,
  disabled,
}: {
  hdInstallOn: boolean;
  setHdInstallOn: (v: boolean) => void;
  hdInstallDir: string;
  hdInstalled: boolean | null;
  onChooseHdDir: () => void | Promise<unknown>;
  onInstallHd: () => void | Promise<unknown>;
  chrInstallOn: boolean;
  setChrInstallOn: (v: boolean) => void;
  chrInstallFile: string;
  chrInstalled: boolean | null;
  onChooseChrFile: () => void | Promise<unknown>;
  onInstallChr: () => void | Promise<unknown>;
  /** 安装目标根：游戏目录（默认，领域共享）/ 用户数据目录 */
  optTarget: "game" | "user";
  setOptTarget: (v: "game" | "user") => void;
  gameDir: string;
  userDataDir: string;
  disabled?: boolean;
}) {
  const { t } = useI18n();
  const targetRoot = optTarget === "game" ? gameDir : userDataDir;
  return (
    <div className="mt-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3">
      {/* 安装目标：默认游戏目录（领域共享）；用户数据目录随领域隔离，选中给警示 */}
      <div className="mb-2 flex flex-wrap items-center gap-x-4 gap-y-1">
        <span className="text-xs text-slate-400">{t.setup.optTarget}</span>
        <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
          <input
            type="radio"
            name="opt-target"
            checked={optTarget === "game"}
            onChange={() => setOptTarget("game")}
            disabled={disabled}
            className="h-3.5 w-3.5 accent-sky-500"
          />
          {t.setup.h3TargetGame}
        </label>
        <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
          <input
            type="radio"
            name="opt-target"
            checked={optTarget === "user"}
            onChange={() => setOptTarget("user")}
            disabled={disabled}
            className="h-3.5 w-3.5 accent-sky-500"
          />
          {t.setup.h3TargetUser}
        </label>
        <span className="min-w-0 break-words font-mono text-xs text-slate-500">
          {targetRoot || "—"}
        </span>
      </div>
      {optTarget === "user" && (
        <p className="mb-2 break-words text-xs text-amber-400">{t.setup.h3TargetUserWarn}</p>
      )}

      <label className="flex items-start justify-between gap-3">
        <span className="min-w-0 flex-1">
          <span className="block break-words text-sm font-medium text-slate-300">
            {t.setup.hdInstall}
          </span>
          {/* 已装好就不再占两行说明（下方有绿色「已安装」结论） */}
          {hdInstallOn && hdInstalled !== true && (
            <span className="mt-0.5 block break-words text-xs text-slate-500">
              {t.setup.hdInstallHint}
            </span>
          )}
        </span>
        <Switch
          checked={hdInstallOn}
          onCheckedChange={setHdInstallOn}
          disabled={disabled}
        />
      </label>
      {hdInstallOn && (
        <div className="mt-2 flex gap-2">
          <Input
            value={hdInstallDir}
            readOnly
            placeholder={t.setup.hdInstallPlaceholder}
            // 已安装 ⇒ 整行禁用，避免二次操作（用户实测反馈）
            disabled={disabled || hdInstalled === true}
            className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
          />
          <Button
            type="button"
            variant="outline"
            onClick={onChooseHdDir}
            disabled={disabled || hdInstalled === true}
            className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.choose}
          </Button>
          <Button
            type="button"
            onClick={onInstallHd}
            disabled={disabled || hdInstalled === true || !hdInstallDir.trim()}
            className="shrink-0 bg-sky-600 text-white hover:bg-sky-500"
          >
            {disabled ? t.setup.installing : t.setup.install}
          </Button>
        </div>
      )}
      {hdInstalled === true && (
        <p className="mt-1 break-words text-xs text-emerald-400">{t.setup.hdInstalled}</p>
      )}
      {hdInstalled === false && (
        <p className="mt-1 break-words text-xs text-red-400">{t.setup.installFailed}</p>
      )}

      <label className="mt-3 flex items-start justify-between gap-3">
        <span className="min-w-0 flex-1">
          <span className="block break-words text-sm font-medium text-slate-300">
            {t.setup.chrInstall}
          </span>
          {/* 已装好就不再占两行说明（下方有绿色「已生成」结论） */}
          {chrInstallOn && chrInstalled !== true && (
            <span className="mt-0.5 block break-words text-xs text-slate-500">
              {t.setup.chrInstallHint}
            </span>
          )}
        </span>
        <Switch checked={chrInstallOn} onCheckedChange={setChrInstallOn} disabled={disabled} />
      </label>
      {chrInstallOn && (
        <div className="mt-2 flex gap-2">
          <Input
            value={chrInstallFile}
            readOnly
            placeholder={t.setup.chrInstallPlaceholder}
            disabled={disabled || chrInstalled === true}
            className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
          />
          <Button
            type="button"
            variant="outline"
            onClick={onChooseChrFile}
            disabled={disabled || chrInstalled === true}
            className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.choose}
          </Button>
          <Button
            type="button"
            onClick={onInstallChr}
            disabled={disabled || chrInstalled === true || !chrInstallFile.trim()}
            className="shrink-0 bg-sky-600 text-white hover:bg-sky-500"
          >
            {disabled ? t.setup.installing : t.setup.install}
          </Button>
        </div>
      )}
      {chrInstalled === true && (
        <p className="mt-1 break-words text-xs text-emerald-400">{t.setup.chrInstalled}</p>
      )}
      {chrInstalled === false && (
        <p className="mt-1 break-words text-xs text-amber-400">{t.setup.chrInstallFailed}</p>
      )}
    </div>
  );
}

/**
 * 原版数据内联闸门（第 ③ 步本体，**固定步骤**）：
 * - 就绪：一行绿色状态（含命中根：游戏目录 / 用户数据目录）；
 * - 缺失 / 探测失败：就地展开数据目录选择 + 复制 + 重试；
 * - 检测中：占位"检测中…"，避免状态行忽隐忽现造成布局跳动。
 */
function H3DataGate({
  h3,
  h3Checking,
  h3Probed,
  h3DataDir,
  setH3DataDir,
  h3Candidates,
  copyMp3,
  setCopyMp3,
  copyMaps,
  setCopyMaps,
  lodTarget,
  setLodTarget,
  gameDir,
  userDataDir,
  onChooseDir,
  onCopy,
  onRedetect,
  copying,
  pct,
  progress,
}: {
  h3: H3DataStatus | null;
  h3Checking: boolean;
  h3Probed: boolean;
  h3DataDir: string;
  setH3DataDir: (v: string) => void;
  h3Candidates: string[];
  copyMp3: boolean;
  setCopyMp3: (v: boolean) => void;
  copyMaps: boolean;
  setCopyMaps: (v: boolean) => void;
  /** `.lod` 复制目标：游戏目录 Data（默认，领域安全）/ 用户数据目录 Data */
  lodTarget: "game" | "user";
  setLodTarget: (v: "game" | "user") => void;
  gameDir: string;
  userDataDir: string;
  onChooseDir: () => void | Promise<unknown>;
  onCopy: () => void | Promise<unknown>;
  onRedetect: () => void | Promise<unknown>;
  copying: boolean;
  pct: number;
  progress: CopyProgressState;
}) {
  const { t } = useI18n();

  // 检测中（尚未有结果）：占位，不展开操作区
  if (h3Checking && !h3Probed) {
    return (
      <p className="break-words text-xs text-slate-500">
        <span className="mr-1.5 text-slate-400">○</span>
        {t.setup.h3Title}: {t.setup.h3Checking}
      </p>
    );
  }

  // 就绪：一行绿色状态（说明命中在哪个根）
  if (h3Probed && h3?.found) {
    return (
      <p className="break-words text-xs text-emerald-400">
        <span className="mr-1.5">●</span>
        {t.setup.h3FoundIn.replace("{arg}", h3.inGame ? t.setup.gameDir : t.setup.userDataDir)}
      </p>
    );
  }

  // 探测失败：禁用的原因 + 重试
  if (!h3Probed) {
    return (
      <div className="rounded-xl border border-slate-700 bg-slate-950/40 p-3">
        <p className="break-words text-xs text-amber-300">
          <span className="mr-1.5">●</span>
          {t.setup.h3Title}: {t.setup.h3ProbeFailed}
        </p>
        <div className="mt-2">
          <Button
            type="button"
            variant="outline"
            onClick={onRedetect}
            disabled={h3Checking}
            className="border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.h3Retry}
          </Button>
        </div>
      </div>
    );
  }

  // 缺失：就地展开（目标固定游戏目录 Data，与用户数据目录选择无关）
  return (
    <div className="rounded-xl border border-amber-800/60 bg-amber-950/20 p-3">
      <p className="break-words text-xs text-amber-200">
        <span className="mr-1.5">●</span>
        {h3?.roe && !h3.sod && !h3.demo
          ? t.setup.h3RoeOnly
          : t.setup.h3Missing.replace("{arg}", h3?.missing.join("、") || "—")}
      </p>
      <p className="mt-1 break-words text-xs text-slate-400">{t.setup.h3MissingHint}</p>

      <div className="mt-2">
        <div className="mb-1 flex items-baseline justify-between gap-2">
          <label className="text-sm font-medium text-slate-300">{t.setup.h3DataDir}</label>
          <span className="min-w-0 break-words text-xs text-slate-500">
            {t.setup.h3DataDirHint}
          </span>
        </div>
        <div className="flex gap-2">
          <Input
            value={h3DataDir}
            onChange={(e) => setH3DataDir(e.target.value)}
            placeholder={h3Candidates[0] ?? ""}
            className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
          />
          <Button
            type="button"
            variant="outline"
            onClick={onChooseDir}
            className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.choose}
          </Button>
        </div>
        {/* 复制目标：默认游戏目录（领域共享）；用户数据目录仅在不适用领域隔离时选 */}
        <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1">
          <span className="text-xs text-slate-400">{t.setup.h3CopyTarget}</span>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="radio"
              name="h3-lod-target"
              checked={lodTarget === "game"}
              onChange={() => setLodTarget("game")}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3TargetGame}
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="radio"
              name="h3-lod-target"
              checked={lodTarget === "user"}
              onChange={() => setLodTarget("user")}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3TargetUser}
          </label>
        </div>
        {lodTarget === "user" && (
          <p className="mt-1 break-words text-xs text-amber-400">{t.setup.h3TargetUserWarn}</p>
        )}
        <p className="mt-1 break-words text-xs text-slate-500">
          {t.setup.h3Target.replace(
            "{arg}",
            lodTarget === "game" ? `${gameDir}\\Data` : `${userDataDir}\\Data`,
          )}
        </p>

        {/* Data 内的 .lod/.snd/.vid 全搬（官方同口径，含 VIDEO.VID）；Mp3/Maps 由下面两个开关控制 */}
        <p className="mt-1 break-words text-xs text-slate-500">{t.setup.h3CopyScope}</p>
        <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1">
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="checkbox"
              checked={copyMp3}
              onChange={(e) => setCopyMp3(e.target.checked)}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3CopyMp3}
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="checkbox"
              checked={copyMaps}
              onChange={(e) => setCopyMaps(e.target.checked)}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3CopyMaps}
          </label>
        </div>
        <p className="mt-1 break-words text-xs text-slate-500">{t.setup.h3CopyHint}</p>

        {h3Candidates.length > 0 && (
          <p className="mt-1 break-words text-xs text-slate-500">
            {t.setup.h3Candidates.replace("{arg}", h3Candidates.join("；"))}
          </p>
        )}
      </div>

      {copying ? (
        <div className="mt-2">
          <div className="text-xs text-slate-200">
            {t.setup.h3Copying.replace("{arg}", `${pct}%`)}
          </div>
          <div className="mt-1.5 h-1.5 w-full overflow-hidden rounded bg-slate-800">
            <div
              className="h-full bg-sky-500 transition-[width] duration-200"
              style={{ width: `${pct}%` }}
            />
          </div>
          <div className="mt-1.5 break-words text-xs text-slate-400">
            {t.setup.copyBytes.replace(
              "{arg}",
              `${fmtBytes(progress.doneBytes)} / ${fmtBytes(progress.totalBytes)}`,
            )}
            {" · "}
            {t.setup.copyFiles.replace("{arg}", `${progress.doneFiles} / ${progress.totalFiles}`)}
          </div>
        </div>
      ) : (
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <Button
            type="button"
            onClick={onCopy}
            disabled={!h3DataDir.trim()}
            className="bg-sky-600 text-white hover:bg-sky-500"
          >
            {t.setup.h3Copy}
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={onRedetect}
            disabled={h3Checking}
            className="border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.h3Redetect}
          </Button>
        </div>
      )}
    </div>
  );
}

function PathField({
  label,
  hint,
  choose,
  value,
  onChoose,
  disabled,
  error,
}: {
  label: string;
  hint: string;
  choose: string;
  value: string;
  onChoose: () => void | Promise<unknown>;
  disabled?: boolean;
  error?: string;
}) {
  return (
    <div className="mb-3">
      <div className="mb-1 flex items-baseline justify-between gap-2">
        <label className="text-sm font-medium text-slate-300">{label}</label>
        <span className="min-w-0 break-words text-xs text-slate-500">{hint}</span>
      </div>
      <div className="flex gap-2">
        <Input
          value={value}
          readOnly
          className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
        />
        <Button
          type="button"
          variant="outline"
          onClick={onChoose}
          disabled={disabled}
          className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
        >
          {choose}
        </Button>
      </div>
      {error && <p className="mt-1 text-xs text-red-400">{error}</p>}
    </div>
  );
}
