import type { LangDict } from "@/i18n/types";
import type { H3DataKind } from "@/lib/tauri";

/**
 * 初始化向导的共享类型与纯函数。
 *
 * 原来这些散在 `SetupWizard.tsx` 顶部；向导本体与拆分出的 `parts/*` 都要用，
 * 故单独成模块（**无 React**，可被任何子组件引用）。
 */

/** 用户数据复制进度（`onUserDataCopyProgress` 载荷的前端展开态） */
export interface CopyProgressState {
  /** 已收到首帧进度；此前显示"正在准备复制…"（后端在整树预扫体量） */
  started: boolean;
  /** 复制阶段：Data 文件 / 音轨 / 地图（各自独立分母，标题必须区分） */
  stage?: "data" | "mp3" | "maps";
  doneFiles: number;
  doneBytes: number;
  totalFiles: number;
  totalBytes: number;
}

/** 字节数 → 人类可读（1024 进制，保留一位小数；不足 1KB 直接给字节） */
export function fmtBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), units.length - 1);
  const v = n / Math.pow(1024, i);
  return `${i === 0 ? v : v.toFixed(1)} ${units[i]}`;
}

/**
 * 不支持的数据模态 → 用户可读原因（VCMI 只接受《死亡阴影》/《完全版》或 demo）。
 *
 * Why 单独抽出来：状态行与「点击复制时的预检」必须说同一句话，
 * 否则两处文案会漂移（此前把 AB 写成"需要末日之刃"，用户会照着去装一个同样跑不起来的版本）。
 */
export function h3UnsupportedText(kind: H3DataKind | null | undefined, t: LangDict): string | null {
  switch (kind) {
    case "hdEdition":
      return t.setup.h3HdEdition;
    case "abOnly":
      return t.setup.h3AbOnly;
    case "roeOnly":
      return t.setup.h3RoeOnly;
    default:
      return null;
  }
}

/**
 * 复制源专用：把"选错目录（没有 `.lod`/`.pak`）"也算作阻断原因。
 * 目标目录的 `empty` 不能套这句话（那是"未检测到数据文件"，语义不同）。
 */
export function h3SourceBlockText(kind: H3DataKind | null | undefined, t: LangDict): string | null {
  if (kind === "empty") return t.setup.h3SourceEmpty;
  return h3UnsupportedText(kind, t);
}
