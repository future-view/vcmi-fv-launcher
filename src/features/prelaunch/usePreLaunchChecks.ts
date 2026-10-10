import { useState, type ReactNode, type FC } from "react";

/**
 * 启动前检查的**通用编排**——把"启动前检测"抽象为可枚举的 step 列表。
 *
 * 设计（见 `docs/features/lang-mods` §10）：
 * - 每个检查项 = 一个 `PreLaunchStep`，新增检测类型只需追加一项，外壳与两入口（BottomBar / RealmShell）零改。
 * - `scan`（本地、纯本地、快）：返回该项的「检测结论」；`resolve`（可联网/重 IO，独立 spinner）：补全「解决方案」。
 * - 主流程（报告）不碰网络：`scan` 并行跑完即决定是否需要弹窗；`resolve` 在弹窗打开后异步跑，失败降级。
 * - 全部 step = ok → 不弹窗、直接启动（自动放行）。
 */

export type StepSeverity = "ok" | "warn" | "error";

/** 节点运行态：pending / scanning / 结果（含 resolving 进行中） */
export interface StepRuntime {
  scan?: StepScan;
  resolve?: StepResolve;
  scanning: boolean;
  resolving: boolean;
}

/** ① 本地扫描结果（报告先行）：severity 决定节点主状态与底部按钮策略 */
export interface StepScan {
  severity: StepSeverity;
  /** 结论标题（如「未启用简体中文语言包」）；步骤条节点用 `step.label`，此处用于卡片结论与无障碍 */
  title: string;
  /** 检测到了什么（本地已知，立即展示） */
  details?: ReactNode;
  /** 业务数据，由对应 Component 自行 cast 使用（如 `LaunchLangModCheck`） */
  data?: unknown;
}

/** ② 方案解析结果（可联网）：severity 通常不变，node 为方案区内容（候选列表 / 仓库不可用提示） */
export interface StepResolve {
  severity: StepSeverity;
  /** 方案区节点；null/undefined 表示无方案可补充 */
  node?: ReactNode;
  /** 业务数据，由对应 Component 自行 cast 使用（如 `ModRepoEntry[]` / "unreachable"） */
  data?: unknown;
}

/** step 卡片渲染器接收的 props（扫描/方案状态由 hook 注入；动作回调由 step 构造时闭包提供） */
export interface StepViewProps {
  scan: StepScan;
  resolve?: StepResolve;
  resolving: boolean;
}

/** 一个启动前检查项的定义 */
export interface PreLaunchStep {
  id: string;
  /** 步骤条节点标签（如「语言包」「依赖完整性」） */
  label: string;
  /** 该项卡片渲染器 */
  Component: FC<StepViewProps>;
  /** ① 本地扫描：必须快、纯本地 */
  scan: () => Promise<StepScan> | StepScan;
  /** ② 方案解析（可选，可联网）：独立 spinner，失败不影响其它 step */
  resolve?: (scan: StepScan) => Promise<StepResolve>;
}

export type PreLaunchResult = "proceed" | "dialog";

export interface UsePreLaunchChecks {
  open: boolean;
  close: () => void;
  states: Record<string, StepRuntime>;
  /** 有 step 正在下载/启用中：忙碌时禁用弹窗关闭，避免中断进行中的操作 */
  busy: boolean;
  setBusy: (b: boolean) => void;
  /** 跑全部 scan（并行）→ 决定是否弹窗；弹窗内自动跑 resolve。返回 proceed（直接启动）/ dialog（已开弹窗） */
  run: (steps: PreLaunchStep[]) => Promise<PreLaunchResult>;
}

export function usePreLaunchChecks(): UsePreLaunchChecks {
  const [open, setOpen] = useState(false);
  const [states, setStates] = useState<Record<string, StepRuntime>>({});
  const [busy, setBusy] = useState(false);

  const close = () => setOpen(false);

  const run = async (steps: PreLaunchStep[]): Promise<PreLaunchResult> => {
    // ① 并行跑所有 scan（纯本地，毫秒级）
    const entries = await Promise.all(
      steps.map(async (s) => {
        setStates((prev) => ({ ...prev, [s.id]: { ...prev[s.id], scanning: true } }));
        try {
          const scan = await Promise.resolve(s.scan());
          return [s.id, scan] as const;
        } catch {
          // 扫描失败当作 error（仍给逃生口），不阻断启动
          return [s.id, { severity: "error" as const, title: "" }] as const;
        }
      }),
    );

    const next: Record<string, StepRuntime> = {};
    for (const [id, scan] of entries) {
      next[id] = { scan, scanning: false, resolving: false };
    }
    setStates(next);

    const anyProblem = entries.some(([, scan]) => scan.severity !== "ok");
    if (!anyProblem) {
      // 全部 ok → 不弹窗、直接启动（自动放行，满足"预设没问题就直接跳过弹窗"）
      return "proceed";
    }

    // ② 有问题 → 弹窗出现，内部异步跑 resolve（补全方案，失败降级）
    setOpen(true);
    for (const s of steps) {
      if (s.resolve && next[s.id]?.scan) {
        setStates((prev) => ({ ...prev, [s.id]: { ...prev[s.id], resolving: true } }));
        try {
          const resolve = await s.resolve(next[s.id].scan!);
          setStates((prev) => ({ ...prev, [s.id]: { ...prev[s.id], resolve, resolving: false } }));
        } catch {
          setStates((prev) => ({ ...prev, [s.id]: { ...prev[s.id], resolving: false } }));
        }
      }
    }
    return "dialog";
  };

  return { open, close, states, busy, setBusy, run };
}
