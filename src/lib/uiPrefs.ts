/**
 * 界面层的本地偏好（`localStorage`）。
 *
 * 只放**与便携包无关**的界面开关：这类偏好丢了也不影响使用（最多重新弹一次提示）。
 * 需要跟随便携目录走的偏好一律走 `launcher.json`（见 `setRealmPublishTarget`）——
 * WebView 的 `localStorage` 绑定系统用户数据目录，不随便携包走。
 *
 * 读写一律 try/catch：隐私模式 / 禁用存储时当作「未设置」，绝不阻断主流程。
 */

/** 「官方启动器仍在运行」提示已被忽略（§4.4 分级守卫的软提示） */
const SKIP_LAUNCHER_NOTICE_KEY = "vcmi-fv.ui.skipLauncherNotice";

function readFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function writeFlag(key: string, value: boolean): void {
  try {
    if (value) {
      localStorage.setItem(key, "1");
    } else {
      localStorage.removeItem(key);
    }
  } catch {
    // 写不进去就放弃，下次照常提示
  }
}

/** 是否已勾选「忽略提醒」：只开着官方启动器切换领域时不再弹提示。 */
export function isLauncherNoticeMuted(): boolean {
  return readFlag(SKIP_LAUNCHER_NOTICE_KEY);
}

/** 设置「忽略提醒」（true = 后续不再提示）。 */
export function setLauncherNoticeMuted(muted: boolean): void {
  writeFlag(SKIP_LAUNCHER_NOTICE_KEY, muted);
}

/** 百科中栏排序偏好（按类目各记一组主/次排序，见 `features/wiki/sort.ts`）。 */
const WIKI_SORT_KEY = "vcmi-fv.ui.wikiSort";

/** 形状宽松：只保证是个对象，字段合法性由调用方的 `normalizeSort` 裁剪。 */
export type WikiSortPrefs = Record<string, { primary?: string; secondary?: string }>;

export function readWikiSort(): WikiSortPrefs {
  try {
    const raw = localStorage.getItem(WIKI_SORT_KEY);
    if (!raw) return {};
    const v: unknown = JSON.parse(raw);
    return v && typeof v === "object" && !Array.isArray(v) ? (v as WikiSortPrefs) : {};
  } catch {
    return {};
  }
}

export function writeWikiSort(prefs: WikiSortPrefs): void {
  try {
    localStorage.setItem(WIKI_SORT_KEY, JSON.stringify(prefs));
  } catch {
    // 写不进去就放弃，下次用默认排序
  }
}
