import { type InstalledMod, type RealmInstalledMod } from "@/lib/tauri";
import type { Realm } from "@/lib/realm/types";
import type { ModInfo } from "@/lib/modScanner";

/** 领域激活时自动应用的模组预设 id（设计 §3.3：固定 `__realm__`）。 */
export const REALM_PRESET_ID = "__realm__";

export interface EditorProps {
  /**
   * 正在编辑的领域。
   * 新建流程已改为「先定 ID 再编辑」：`CreateRealmDialog` 提交后即 `create_realm`
   * 建立领域目录与 `realm.json`，因此编辑器拿到的**恒为已存在的领域**，
   * 地图上传 / 素材导入 / 整合模组 / 导出等依赖领域目录的能力全程可用。
   */
  editing: Realm;
  onCancel: () => void;
  onSaved: () => void;
  showToast: (type: "success" | "error" | "info", msg: string) => void;
  /**
   * 把编辑器内部的返回处理（`handleBack`，含未保存改动的确认弹窗）注册给宿主
   * （`RealmShell` 的 `editorBackRef`）：编辑器的「返回 + 标题」已上提为覆盖层
   * 左上角悬浮页头（§9.2.4，v1.16），由宿主页头触发本回调。传 `null` 表示注销。
   */
  registerBack: (fn: (() => void) | null) => void;
}

/** 把领域「已安装模组」精简条目映射为 `ModListItem` 所需的 `ModInfo`（§9.6.1：复用 Mods 页面内容区布局）。 */
export function installedToModInfo(m: InstalledMod): ModInfo {
  return {
    id: m.id,
    // 磁盘原名优先（`id` 是小写归一后的 VCMI modId，不能当目录名用；旧后端无该字段时兜底）
    diskName: m.diskName || m.id,
    parentId: m.parentId ?? m.id,
    name: m.name,
    localizedName: m.name,
    desc: m.desc ?? "",
    type: m.type ?? "Other",
    version: m.version ?? m.meta?.version ?? undefined,
    author: m.author ?? m.meta?.author ?? undefined,
    // 磁盘绝对路径：优先用后端回传值（子模组的 `<父>/Mods/<子>` 布局只有它准）
    modPath: m.modPath ?? "",
    hasContentZip: false,
    contentZipEncrypted: false,
    isSubmod: m.isSubmod ?? false,
    keepDisabled: m.keepDisabled,
    depends: m.depends ?? [],
    softDepends: m.softDepends ?? [],
    conflicts: m.conflicts ?? [],
    // 引擎兼容区间 / 语言 / modType 原文：领域模组列表与 Mods 页面共用同一套
    // 守卫与角标（版本不兼容、翻译语言不符），缺了这些字段领域页就会静默失去提示。
    compatibility: m.compatibility ?? null,
    language: m.language ?? undefined,
    rawType: m.rawType ?? null,
    subs: [],
    factions: m.factions,
    i18n: m.i18n,
  };
}

/**
 * 把领域「已实装」（`list_realm_mods`）条目映射为 `ModInfo`：
 * 预设视图的领域范围列表与行详情抽屉（`ModRowDrawer`）共用。
 * 数据来自后端目录扫描，因此携带 i18n / rawType / version 等全部元信息；
 * `compatibility` / `language` 后端未透传（恒定 null，不产生误导角标）。
 */
export function realmModToModInfo(m: RealmInstalledMod): ModInfo {
  return {
    id: m.id,
    diskName: m.diskName || m.id,
    parentId: m.id.includes(".") ? m.id.split(".")[0] : m.id,
    name: m.name ?? m.id,
    localizedName: m.name ?? m.id,
    desc: m.desc ?? "",
    type: m.type ?? "Other",
    version: m.version ?? undefined,
    author: m.author ?? undefined,
    modPath: "",
    hasContentZip: false,
    contentZipEncrypted: false,
    isSubmod: m.isSubmod ?? false,
    keepDisabled: m.keepDisabled,
    depends: m.depends ?? [],
    softDepends: m.softDepends ?? [],
    conflicts: m.conflicts ?? [],
    compatibility: null,
    language: undefined,
    rawType: m.rawType ?? null,
    subs: [],
    factions: m.factions,
    i18n: m.i18n,
  };
}
