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
  type H3DataKind,
  type H3DataStatus,
  type VcmiDirs,
} from "@/lib/tauri";
import { DEFAULT_GITHUB_MIRROR_PREFIX, githubMirrorRule } from "@/lib/mirrorRules";
import { backendErrorCode, backendErrorText } from "@/lib/backendError";
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
import { H3DataGate } from "@/features/setup/parts/H3DataGate";
import { OptionalContent } from "@/features/setup/parts/OptionalContent";
import { PathField } from "@/features/setup/parts/PathField";
import { fmtBytes, h3SourceBlockText, type CopyProgressState } from "@/features/setup/setupShared";

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
  /**
   * 同步来源两层模型：探测源 `probeSource`（随游戏目录的 dirs.json 解析结果更新——
   * 增强版指向 `My Games\vcmi_enhancement` 等；此前不同步更新，来源行会错显
   * `My Games\vcmi`）+ 手动指定源 `customSource`（第 ② 步「选择…」切换，
   * 覆盖只在此处发生）。生效源 = `customSource || probeSource`。
   */
  const [probeSource, setProbeSource] = useState("");
  const [probeSourceExists, setProbeSourceExists] = useState(false);
  const [customSource, setCustomSource] = useState("");
  /** 选源/同步的框内红字：A 类路径关系校验（即时）与 B 类后端结构闸门（点同步时） */
  const [syncPickError, setSyncPickError] = useState("");
  /** 生效源被后端判定结构不合法 ⇒ 禁用「同步数据」，直到换源 / 恢复默认 */
  const [sourceInvalid, setSourceInvalid] = useState(false);
  /** 「选择…」对话框打开期间防重入 */
  const [choosingSource, setChoosingSource] = useState(false);
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
  /**
   * 复制**源**的数据模态（`null` = 未检/无法识别）。
   * ⚠️ 与 `h3` 不同：`h3` 描述"当前游戏目录 + 用户数据目录"，
   * 这里描述用户挑的那个待复制目录——版本不对时必须在复制前拦住。
   */
  const [h3SourceKind, setH3SourceKind] = useState<H3DataKind | null>(null);
  const [h3SourceChecking, setH3SourceChecking] = useState(false);
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
  /** 已在盘上（游戏目录 / 用户数据目录）存在的可选内容：显示「已就绪」并收起安装控件 */
  const [hdPresent, setHdPresent] = useState(false);
  const [chrPresent, setChrPresent] = useState(false);
  /** 当前正在安装的可选内容项（`null`=空闲）：**按项显示**「安装中」，不再两项一起变**/
  const [installingExtra, setInstallingExtra] = useState<"hd" | "chr" | null>(null);
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
  /**
   * 生效同步源：手动指定优先，否则用探测源。
   * 探测源在 detect 回调 / 选游戏目录 / 自动探测三处更新（见下）。
   */
  const effectiveSource = customSource || probeSource;
  /** 手动源来自目录选择器（必然存在）；探测源的存在性以 detect / suggest 结果为准 */
  const sourceExists = customSource ? true : probeSourceExists;
  /** 是否展示「GitHub 镜像」建议：探测与语言无关，但建议只给有需要的语言 */
  const mirrorHintAllowed = uiLangFeature(lang, "githubMirrorHint");
  /** 同步进行中：锁定一切（路径 / 开关 / 上一步 / 下一步） */
  const syncing = syncState === "running";
  /** 已同步：锁定路径与开关，防止复制到旧路径留下孤儿目录 */
  const syncLocked = syncState === "done";

  /** 原版数据闸门：**未确认即视为未通过**（缺 LOD 的后果是 VCMI 无法启动） */
  const h3Ready = h3Probed && !!h3?.found;

  /** 可选内容是否已就绪：本次装好（`*Installed`）或盘上已存在（`*Present`，第 ③ 步体检得到） */
  const hdReady = hdInstalled === true || hdPresent;
  const chrReady = chrInstalled === true || chrPresent;

  /** 第 ④ 步汇总里的「高清版素材」是否为已启用：已就绪，或探测到既有 hd-edition 且开关保持打开 */
  const hdActive = hdReady || (hdDetected && hdEnable);

  useEffect(() => {
    detectVcmiPaths().then((d) => {
      if (d) {
        setDetected(d);
        // 同步探测源：dirs.json 解析出的用户目录优先（增强版 → vcmi_enhancement），
        // 无 dirs.json 时回退官方默认目录（My Games\vcmi）
        const ps = (d.userDir ?? "").trim();
        setProbeSource(ps || d.systemUserDataDir || "");
        setProbeSourceExists(ps ? !!d.userDirExists : !!d.systemUserDataDirExists);
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
        stage: p.stage,
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
   * 同步区**可见性联动**：仅当目标目录 ≠ 生效源时才有意义 ——
   * 开了专属目录（数据从零开始）、手动选了非默认目标，或手动切换了来源。
   * 目标 = 生效源（数据本来就在那儿）时隐藏，并给一句说明，
   * 避免"开关开了却什么都没有"的困惑。
   */
  const showSync = !!gameDir.trim() && normPath(userDataDir) !== normPath(effectiveSource);

  // 同步区隐藏时，未完成的同步状态与手动来源一并作废（目标已等于来源）
  useEffect(() => {
    if (!showSync) {
      resetSync();
      setCustomSource("");
      setSourceInvalid(false);
      setSyncPickError("");
    }
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
      // 已有 hd-edition / chronicles ⇒ 显示「已就绪」，不再让用户重复安装
      // （此前只看本次会话是否点过安装，已存在也显示空开关，用户实测反馈）
      setHdPresent(health.optionalMods?.hdEdition === true);
      setChrPresent(health.optionalMods?.chronicles === true);
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

  /**
   * 复制**源**体检：只判用户挑的那个目录，不掺已保存的游戏/用户目录。
   *
   * 用户可能挑三种东西（游戏根 / `Data` / 散装 `.lod` 目录），后端 `resolve_source`
   * 已做归一，这里传两次同一路径即可（两个根都指向它，`detect` 不会回退到保存的配置）。
   */
  const inspectH3Source = async (path: string): Promise<H3DataKind | null> => {
    const p = path.trim();
    if (!p) {
      setH3SourceKind(null);
      return null;
    }
    setH3SourceChecking(true);
    try {
      const health = await gamedataHealthCheck(p, p, false);
      const kind = health?.h3Data?.kind ?? null;
      setH3SourceKind(kind);
      return kind;
    } catch {
      // 探测失败不阻断（后端复制时还有一次闸门），只是拿不到提前提示
      setH3SourceKind(null);
      return null;
    } finally {
      setH3SourceChecking(false);
    }
  };

  // 源路径一改，旧的模态结论立即作废（避免"换了目录还显示上一个目录的报错"）
  useEffect(() => {
    setH3SourceKind(null);
  }, [h3DataDir]);

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
    setCustomSource("");
    setSourceInvalid(false);
    setSyncPickError("");
    const suggested = await suggestUserDataDir(path);
    if (suggested) {
      setUserDataDir(suggested.userDataDir);
      // 探测源跟随游戏目录：dirs.json 可能把用户目录指向别处（增强版 →
      // vcmi_enhancement）。不同步更新就会拿 My Games\vcmi 当来源，
      // 而该目录里并没有这个发行版的数据（实测截图 bug）。
      setProbeSource(suggested.userDataDir || "");
      setProbeSourceExists(!!suggested.exists);
      if (!suggested.exists) setError(t.setup.missingUserData);
    }
  };

  const handleChooseUser = async () => {
    const path = await pickDirectory(t.setup.userDataDir);
    if (!path) return;
    setUserDataDir(path);
    setError("");
  };

  /**
   * 选源即时校验（**纯路径关系，不碰 IO**；后端 fs 探测能力前端没有）。
   *
   * - 源 = 目标：不能写入来源行——`showSync` 会因"目标=来源"隐藏同步区，
   *   红字将无处显示（UI 状态矛盾）；
   * - 目标在源内部：后端 `migrate_user_data_dir` 直接拒绝
   *   （`setup_userdata_invalid`，复制会自我递归），提前拦一道。
   */
  const pickSourceError = (src: string): string => {
    const s = normPath(src);
    const d = normPath(userDataDir);
    if (!s || !d) return "";
    if (s === d) return t.setup.copySourceSameTarget;
    if (d.startsWith(`${s}\\`)) return t.setup.copySourceContainsTarget;
    return "";
  };

  /**
   * 「选择…」：手动指定同步来源（发行版把游戏与配置分成不同压缩包时，
   * 配置目录在任意位置，探测源猜不到）。**只换源、不开跑**——
   * 导入的唯一触发入口是「同步数据」按钮；选错目录在执行前即可更正。
   *
   * A 类硬非法 ⇒ 不写入来源行（保持上一个生效值）+ 红字一次性提示；
   * 通过 ⇒ 设为手动源，并清掉旧同步状态（旧结论对新源无效）。
   */
  const handleChooseSource = async () => {
    if (choosingSource) return;
    setChoosingSource(true);
    try {
      const path = await pickDirectory(t.setup.copyConfigTitle);
      if (!path) return;
      const err = pickSourceError(path);
      if (err) {
        setSyncPickError(err);
        return;
      }
      setCustomSource(path);
      setSourceInvalid(false);
      setSyncPickError("");
      resetSync();
    } finally {
      setChoosingSource(false);
    }
  };

  /** 「恢复默认」：放弃手动来源回到探测源。显式出口——不做静默回退 */
  const handleResetSource = () => {
    setCustomSource("");
    setSourceInvalid(false);
    setSyncPickError("");
    resetSync();
  };

  /** 切换「启动器专属用户数据目录」：开 → 填 exe 同目录 userdata；关 → 回到解析出的用户目录 */
  const toggleCustomUserData = (on: boolean) => {
    setCustomUserData(on);
    resetSync();
    setCustomSource("");
    setSourceInvalid(false);
    setSyncPickError("");
    if (on) {
      if (portableDir) setUserDataDir(portableDir);
    } else {
      // 关开关回探测源（customSource 此刻正被清空，不能用 effectiveSource 的旧闭包值）
      setUserDataDir(probeSource || userDataDir);
    }
  };

  /** 自动探测：探测到有效目录组合即可填充表单，否则禁用 */
  const autoFill = () => {
    if (!detected?.detected) return;
    setGameDir(detected.gameDir || "");
    setUserDataDir(detected.userDir || "");
    setCustomUserData(false);
    resetSync();
    // 探测源一并恢复为 detect 结果（用户可能先手动选过游戏目录改过它）
    const ps = (detected.userDir ?? "").trim();
    setProbeSource(ps || detected.systemUserDataDir || "");
    setProbeSourceExists(ps ? !!detected.userDirExists : !!detected.systemUserDataDirExists);
    setCustomSource("");
    setSourceInvalid(false);
    setSyncPickError("");
    setError("");
  };

  /** 显式同步：只迁移数据、不写任何配置（保存按钮不再隐式复制） */
  const handleSync = async () => {
    const src = effectiveSource.trim();
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
      // B 类：后端结构闸门拒绝（源不是 VCMI 用户数据目录）⇒ 框内红字 +
      // 「同步数据」禁用，来源路径保留供重选 / 恢复默认；其余错误走全局 error
      if (backendErrorCode(e) === "setup_userdata_source_invalid") {
        setSourceInvalid(true);
        setSyncPickError(backendErrorText(e, t) || t.setup.error);
      } else {
        setError(backendErrorText(e) || t.setup.error);
      }
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
      // 版本预检：RoE-only / AB-only / HD Edition 搬过去 VCMI 也起不来
      // （运行期判 ROE → fatal），直接在源上拒绝，不白搬几百 MB。
      // 后端 `copyH3Data` 内还有同一道闸门（防绕过）。
      const kind = h3SourceKind ?? (await inspectH3Source(from));
      const unsupported = h3SourceBlockText(kind, t);
      if (unsupported) {
        setError(unsupported);
        return;
      }
      // 第二参 = Mp3/Maps 的归属根：与 .lod 同一个 radio 目标
      await copyH3Data(gd, targetRoot, from, to, h3CopyMp3, h3CopyMaps);
      setProgress(IDLE_PROGRESS);
      await checkH3Data(gd, ud, false);
    } catch (e: any) {
      setProgress(IDLE_PROGRESS);
      setError(backendErrorText(e) || t.setup.error);
    }
  };

  /**
   * 第 ③ 步：选择 HD 版安装目录（必须包含 `HOMM3 2.0.exe`）。
   *
   * ⚠️ 只给**目录**一个入口：后端 `classify_input` 也认该 exe 本体，但原生对话框
   * 无法在同一会话混选文件与目录，向导里再加一个按钮会挤占本就紧张的横向空间
   * （俄/德语更长）。exe 本体入口只存在于主界面导入弹层（`shell/ImportFilesDialog`）。
   */
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
    setInstallingExtra(kind);
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
      setInstallingExtra(null);
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
    // 选完立刻判版本：不对时用户当场就知道"该装什么"，而不是点了复制才被拒
    void inspectH3Source(path);
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

            {/* 数据同步：显式按钮（保存不再隐式复制）；三种状态在同一框内原地切换。
                目标 = 生效源（数据本来就在那儿）时隐藏，给一句说明。 */}
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
                  <>
                    {/* 已同步：来源行只读保留——数据来自哪个目录，事后仍可核对 */}
                    <p className="flex items-baseline gap-1.5 text-xs text-slate-500">
                      <span className="shrink-0">
                        {customSource ? t.setup.syncSourceLabelManual : t.setup.syncSourceLabel}
                      </span>
                      <span className="min-w-0 truncate" title={effectiveSource}>
                        {effectiveSource || "—"}
                      </span>
                    </p>
                    <p className="mt-1.5 break-words text-xs text-emerald-400">
                      {t.setup.syncDone.replace("{arg}", fmtBytes(syncDone.bytes))}
                      {" · "}
                      {t.setup.copyFiles.replace("{arg}", String(syncDone.files))}
                    </p>
                  </>
                ) : (
                  <>
                    {/* 区域说明：说清"复制什么、到哪、不复制会怎样"——缺了它，
                        用户只看到来源行与按钮，看不懂这个区域的功能（实测截图反馈）。
                        同步仅在首启提供，"从空目录开始"的后果必须写明。 */}
                    <p className="mb-2 break-words text-xs text-slate-500">
                      {t.setup.syncAreaHint}
                    </p>
                    {/* 来源行：前缀 + 路径（truncate + title）+ 选择…；
                        手动指定态附「恢复默认」。「选择…」只换源、不开跑——
                        导入的唯一触发入口是下方的「同步数据」。 */}
                    <div className="flex items-center gap-2">
                      <span className="shrink-0 text-xs text-slate-500">
                        {customSource ? t.setup.syncSourceLabelManual : t.setup.syncSourceLabel}
                      </span>
                      <span
                        className="min-w-0 flex-1 truncate text-sm text-slate-200"
                        title={effectiveSource}
                      >
                        {effectiveSource || "—"}
                      </span>
                      {customSource && (
                        <button
                          type="button"
                          onClick={handleResetSource}
                          className="shrink-0 text-xs text-sky-400 hover:text-sky-300 hover:underline"
                        >
                          {t.setup.copySourceReset}
                        </button>
                      )}
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => void handleChooseSource()}
                        disabled={choosingSource}
                        className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
                      >
                        {t.setup.choose}
                      </Button>
                    </div>
                    {/* 唯一触发入口：以生效源（探测源或手动源）开跑 */}
                    <div className="mt-2 flex items-center gap-3">
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => void handleSync()}
                        disabled={!sourceExists || sourceInvalid}
                        className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
                      >
                        {t.setup.syncData}
                      </Button>
                      <span className="min-w-0 flex-1 break-words text-xs text-slate-500">
                        {!sourceExists ? t.setup.copySourceMissing : syncNote}
                      </span>
                    </div>
                    {/* 框内红字：A 类选源即时拒绝 / B 类后端结构闸门拒绝（路径均保留） */}
                    {syncPickError && (
                      <p className="mt-1.5 break-words text-xs text-red-400">{syncPickError}</p>
                    )}
                  </>
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
            sourceKind={h3SourceKind}
            sourceChecking={h3SourceChecking}
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
            hdReady={hdReady}
            onChooseHdDir={handleChooseHdDir}
            onInstallHd={() => void handleInstallHd()}
            chrInstallOn={chrInstallOn}
            setChrInstallOn={setChrInstallOn}
            chrInstallFile={chrInstallFile}
            chrInstalled={chrInstalled}
            chrReady={chrReady}
            onChooseChrFile={handleChooseChrFile}
            onInstallChr={() => void handleInstallChr()}
            optTarget={optTarget}
            setOptTarget={setOptTarget}
            gameDir={gameDir.trim()}
            userDataDir={userDataDir.trim()}
            installing={installingExtra}
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

            {/* 可选内容汇总：无论装不装，都显示高清版 / 历代记的当前选择 */}
            <div className="mb-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3 text-xs text-slate-400">
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
                <span className={chrReady ? "text-emerald-400" : "text-slate-500"}>
                  {chrReady ? t.setup.optEnabled : t.setup.optDisabled}
                </span>
              </div>
            </div>

            {/* ⚠️ 顺序：先「已定稿的信息」，再「可选建议」。建议类面板（HD 生成 / GitHub 镜像）
                统一放最后，避免一进第 ④ 步就被两块彩色面板抢占视线（用户实测反馈）。 */}
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
                disabled={saving || syncing || installingExtra !== null}
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
                  installingExtra !== null || // 安装进行中临时锁导航（同上）
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
