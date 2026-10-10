import { localizedName, type ModInfo } from "./modScanner";
import { isModCompatible } from "./modCompat";
import { isReservedMod } from "./modScope";
import { modBaseLanguage } from "./vcmiLanguage";
import { vcmiLangName } from "./vcmiLangs";

/**
 * 模组预设报告生成（纯函数，输出 Markdown）。
 *
 * 两种模式共用同一份取数/分组逻辑，仅渲染分支不同（单一源头，避免"界面显示 / 复制导出"
 * 两套内容漂移）：
 * - `brief`：面向玩家，回答"这个预设里有什么功能"——按类型分组的模组清单 + 一句统计；
 * - `full`：面向预设编辑人——概览统计表、含 ID/版本/备注的表格、子模组开关明细、缺失清单。
 *
 * 文案由调用方经 {@link ReportTexts} 注入（本模块不引 i18n 运行时，保持可测）；
 * 模组名沿用列表页口径（`localizedName(mod, lang)`）。
 */

export type ReportMode = "brief" | "full";

/** 报告文案（从 i18n 字典的 `preset` 段抽取） */
export interface ReportTexts {
  /** 标题基名（"模组预设启用报告"） */
  title: string;
  /** 标题拼接（`{title}` / `{name}` 占位，各语言自定标点） */
  heading: string;
  generated: string;
  engine: string;
  gameLang: string;
  presetKind: string;
  kindBuiltin: string;
  kindUser: string;
  /** `{n}` = 启用总数 */
  total: string;
  updatesHint: string;
  missingHint: string;
  overview: string;
  colItem: string;
  colCount: string;
  statEnabled: string;
  statBase: string;
  statRequired: string;
  statChosen: string;
  statSettings: string;
  statUpdates: string;
  statMissing: string;
  sectionMods: string;
  /** 分组小标题（`{type}` / `{n}` 占位） */
  groupHeading: string;
  colMod: string;
  colId: string;
  colVersion: string;
  colNote: string;
  noteBase: string;
  noteRequired: string;
  /** `{v}` = 远端版本 */
  noteUpdate: string;
  noteLangMismatch: string;
  noteIncompat: string;
  /** 精简模式的可更新后缀（"（可更新）"） */
  updateMark: string;
  settingsTitle: string;
  colParent: string;
  colSub: string;
  colState: string;
  stateOn: string;
  stateOff: string;
  missingTitle: string;
  /** 缺失模组的固定说明（"本机未安装，启动时会被 VCMI 跳过"） */
  missingNote: string;
  /** `{n}` = 缺失数量（精简模式的提示句） */
  briefMissing: string;
  /** 类型分组标签（未知类型回退原文） */
  typeLabel: (type: string) => string;
}

export interface PresetReportInput {
  /** 预设显示名（内置 → 本地化标签；用户预设 → 原名） */
  presetLabel: string;
  /** 是否内置预设 */
  builtin: boolean;
  /** 预设保存的模组 id（磁盘态，含基线/强制） */
  mods: string[];
  /** 子模组开关（VCMI 原生结构：`settings[父id][子相对键小写] = 启用`） */
  settings: Record<string, Record<string, boolean>>;
  /** 全部已装模组（含 vcmi/core；已扁平化） */
  allMods: ModInfo[];
  /** 基线（不可关闭）模组 id */
  baseMods: string[];
  /** 全局强制模组 id（launcher.json） */
  requiredMods: string[];
  /** 可更新集合：id → 远端版本 */
  updates: Record<string, string>;
  /** 引擎版本（undefined = 未知，不做兼容性标注） */
  engineVersion?: string | null;
  /** 游戏语言（VCMI 标识，用于"语言不符"判定与语言展示名） */
  vcmiLang: string;
  /** 界面语言（模组取词口径，与列表页一致） */
  lang: string;
  t: ReportTexts;
  /** 注入当前时间（测试可固定） */
  now?: Date;
}

/** 类型分组的展示顺序（其余类型按出现顺序追加；与 `mods.typeLabels` 的键对齐） */
const TYPE_ORDER = [
  "Core",
  "Town",
  "Creatures",
  "Heroes",
  "Artifacts",
  "Spells",
  "Skills",
  "Mechanics",
  "Objects",
  "Resources",
  "Interface",
  "Graphical",
  "Music",
  "Sounds",
  "Maps",
  "Campaigns",
  "Templates",
  "Translation",
  "AI",
  "Compatibility",
  "Expansion",
  "Demo",
  "Test",
  "Other",
];

/** 引擎内置模组（`modScope.RESERVED_MOD_IDS`）的展示名：它们没有 mod.json，取不到本地化名 */
const RESERVED_DISPLAY: Record<string, string> = { vcmi: "VCMI", core: "Core" };

/** 表格单元格安全化：`|` 会破坏表格结构（换全角），换行折成空格 */
function mdCell(s: string): string {
  return (s ?? "").replace(/\|/g, "｜").replace(/\s*\n\s*/g, " ").trim();
}

export function buildPresetReport(input: PresetReportInput, mode: ReportMode): string {
  const { t } = input;
  const now = input.now ?? new Date();

  const byId = new Map(input.allMods.map((m) => [m.id, m]));
  const byLower = new Map(input.allMods.map((m) => [m.id.toLowerCase(), m]));
  const find = (id: string): ModInfo | undefined => byId.get(id) ?? byLower.get(id.toLowerCase());

  const updByLower = new Map(
    Object.entries(input.updates).map(([k, v]) => [k.toLowerCase(), v] as const),
  );
  const updOf = (id: string): string | undefined => updByLower.get(id.toLowerCase());

  const baseSet = new Set(input.baseMods.map((x) => x.toLowerCase()));
  const reqSet = new Set(input.requiredMods.map((x) => x.toLowerCase()));
  const kindOf = (id: string): "base" | "required" | "chosen" => {
    const k = id.toLowerCase();
    return baseSet.has(k) ? "base" : reqSet.has(k) ? "required" : "chosen";
  };

  // 去重（大小写不敏感）并保序
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const id of input.mods) {
    const k = id.toLowerCase();
    if (seen.has(k)) continue;
    seen.add(k);
    ids.push(id);
  }

  // 引擎内置保留模组（vcmi / core，见 modScope）由引擎提供、不在 Mods 目录，
  // scan_mods 扫不到 → 若按"扫描不到 = 未安装"判定会误报缺失（实测反馈）。
  // 处理：不参与缺失判定，按 id 展示、归入「核心」组、仅按基线标注。
  const present = ids.filter((id) => !!find(id) || isReservedMod(id));
  const missing = ids.filter((id) => !find(id) && !isReservedMod(id));

  const baseCount = present.filter((id) => kindOf(id) === "base").length;
  const reqCount = present.filter((id) => kindOf(id) === "required").length;
  const chosenCount = present.length - baseCount - reqCount;
  const updCount = present.filter((id) => updOf(id) !== undefined).length;

  // 类型分组：偏好顺序优先，未收录类型按出现顺序追加
  const groups = new Map<string, string[]>();
  for (const id of present) {
    const m = find(id);
    // 无 ModInfo 的保留模组（vcmi / core）归入「核心」
    const type = m ? m.type || "Other" : "Core";
    const arr = groups.get(type);
    if (arr) arr.push(id);
    else groups.set(type, [id]);
  }
  const orderedTypes = [
    ...TYPE_ORDER.filter((tp) => groups.has(tp)),
    ...[...groups.keys()].filter((tp) => !TYPE_ORDER.includes(tp)),
  ];

  const nameOf = (id: string): string => {
    const m = find(id);
    if (m) return localizedName(m, input.lang) || m.name || id;
    // 引擎内置模组没有 mod.json，取不到本地化名：用约定的展示名（非本地化文案）
    return RESERVED_DISPLAY[id.toLowerCase()] ?? id;
  };

  /** full 模式备注列：基线/强制/可更新/不兼容/语言不符（可叠加） */
  const notesOf = (id: string): string[] => {
    const m = find(id);
    if (!m) {
      // 引擎内置（vcmi / core）：没有 ModInfo，仅按基线标注
      return kindOf(id) === "base" ? [t.noteBase] : [];
    }
    const out: string[] = [];
    const kind = kindOf(id);
    if (kind === "base") out.push(t.noteBase);
    if (kind === "required") out.push(t.noteRequired);
    const upd = updOf(id);
    if (upd) out.push(t.noteUpdate.replace("{v}", upd));
    if (!isModCompatible(input.engineVersion, m.compatibility)) out.push(t.noteIncompat);
    if (m.rawType === "Translation" && m.language && modBaseLanguage(m.language) !== input.vcmiLang) {
      out.push(t.noteLangMismatch);
    }
    return out;
  };

  /** 子模组开关明细（parent/child 名称解析：settings 键为小写相对键，需回查磁盘 id） */
  const settingRows: { parent: string; sub: string; on: boolean }[] = [];
  for (const [parentId, subs] of Object.entries(input.settings)) {
    const parent = find(parentId);
    for (const [rel, on] of Object.entries(subs)) {
      const child = byLower.get(`${parentId}.${rel}`.toLowerCase());
      settingRows.push({
        parent: parent ? nameOf(parent.id) : parentId,
        sub: child ? nameOf(child.id) : rel,
        on,
      });
    }
  }

  const L: string[] = [];
  L.push(`# ${t.heading.replace("{title}", t.title).replace("{name}", input.presetLabel)}`);
  L.push("");
  L.push(`- ${t.generated}：${now.toLocaleString()}`);
  const meta = [
    input.engineVersion ? `${t.engine}：VCMI ${input.engineVersion}` : "",
    `${t.gameLang}：${vcmiLangName(input.vcmiLang)}`,
  ].filter(Boolean);
  L.push(`- ${meta.join(" · ")}`);
  if (mode === "full") {
    // 详细模式：统计全部交给「概览」表，头部只留元信息（避免同一数字出现两遍）
    L.push(`- ${t.presetKind}：${input.builtin ? t.kindBuiltin : t.kindUser}`);
  } else {
    L.push(`- ${t.total.replace("{n}", String(present.length))}`);
    const hints: string[] = [];
    if (updCount > 0) hints.push(t.updatesHint.replace("{n}", String(updCount)));
    if (missing.length > 0) hints.push(t.missingHint.replace("{n}", String(missing.length)));
    if (hints.length) L.push(`- ${hints.join(" · ")}`);
  }
  L.push("");

  if (mode === "brief") {
    // 玩家视角：按类型分组的名称清单（含全部启用模组，保证条目数与统计一致）
    for (const type of orderedTypes) {
      const group = groups.get(type)!;
      L.push(`## ${t.groupHeading.replace("{type}", t.typeLabel(type)).replace("{n}", String(group.length))}`);
      for (const id of group) {
        L.push(`- ${nameOf(id)}${updOf(id) !== undefined ? t.updateMark : ""}`);
      }
      L.push("");
    }
    if (missing.length > 0) {
      L.push(`> ${t.briefMissing.replace("{n}", String(missing.length))}`);
      L.push("");
    }
    return trimEnd(L);
  }

  // ---- full（预设编辑人视角）----
  L.push(`## ${t.overview}`);
  L.push("");
  L.push(`| ${t.colItem} | ${t.colCount} |`);
  L.push(`| --- | --- |`);
  L.push(`| ${t.statEnabled} | ${present.length} |`);
  L.push(`| ${t.statBase} | ${baseCount} |`);
  L.push(`| ${t.statRequired} | ${reqCount} |`);
  L.push(`| ${t.statChosen} | ${chosenCount} |`);
  if (settingRows.length > 0) {
    L.push(`| ${t.statSettings} | ${settingRows.length} |`);
  }
  if (updCount > 0) {
    L.push(`| ${t.statUpdates} | ${updCount} |`);
  }
  if (missing.length > 0) {
    L.push(`| ${t.statMissing} | ${missing.length} |`);
  }
  L.push("");

  L.push(`## ${t.sectionMods}`);
  L.push("");
  for (const type of orderedTypes) {
    const group = groups.get(type)!;
    L.push(`### ${t.groupHeading.replace("{type}", t.typeLabel(type)).replace("{n}", String(group.length))}`);
    L.push(`| ${t.colMod} | ${t.colId} | ${t.colVersion} | ${t.colNote} |`);
    L.push(`| --- | --- | --- | --- |`);
    for (const id of group) {
      const m = find(id);
      const notes = notesOf(id);
      L.push(
        `| ${mdCell(nameOf(id))} | ${mdCell(m?.id ?? id)} | ${mdCell(m?.version ?? "—")} | ${mdCell(notes.join(" · ")) || "—"} |`,
      );
    }
    L.push("");
  }

  if (settingRows.length > 0) {
    L.push(`## ${t.settingsTitle}`);
    L.push("");
    L.push(`| ${t.colParent} | ${t.colSub} | ${t.colState} |`);
    L.push(`| --- | --- | --- |`);
    for (const r of settingRows) {
      L.push(
        `| ${mdCell(r.parent)} | ${mdCell(r.sub)} | ${r.on ? t.stateOn : t.stateOff} |`,
      );
    }
    L.push("");
  }

  if (missing.length > 0) {
    L.push(`## ${t.missingTitle}`);
    L.push("");
    for (const id of missing) {
      L.push(`- \`${mdCell(id)}\` — ${t.missingNote}`);
    }
    L.push("");
  }

  return trimEnd(L);
}

function trimEnd(lines: string[]): string {
  return `${lines.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd()}\n`;
}
