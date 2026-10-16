import type { LangDict } from "@/i18n/types";
import type { WikiEntry, WikiLinkTarget } from "@/lib/modcache";
import { fullImgUrl, pickName, type WikiCategory } from "@/lib/modcache";
import { HdIcon } from "@/shared/HdIcon";
import { useWikiPopup, usePopupTriggers } from "./WikiPopupContext";
import { SafeImg, makeRefResolver } from "./detailShared";

export function HeroDetail({
  d,
  lang,
  t,
  modId,
  cacheDir,
  modCache,
  onNavigate,
  compact,
  getEntry,
}: {
  d: Record<string, unknown>;
  lang: string;
  t: LangDict;
  modId: string;
  cacheDir?: string | null;
  modCache?: Record<string, unknown>;
  onNavigate?: (target: WikiLinkTarget) => void;
  compact?: boolean;
  getEntry?: (category: WikiCategory, id: string) => WikiEntry | undefined;
}) {
  const bio = (d.biography as Record<string, string>) ?? {};
  const spec = (d.specialty as Record<string, string>) ?? {};
  const army = (d.army as Record<string, unknown>[]) ?? [];
  const sec = (d.secondarySkills as Record<string, unknown>[]) ?? [];
  const spells = (d.spells as string[]) ?? [];

  const sharedIcons = (modCache?.sharedIcons ?? {}) as Record<string, string>;
  const pskilIcon = (key: string) => fullImgUrl(cacheDir, sharedIcons[`pskil42/${key}`]);
  const portraitUrl = d.image
    ? fullImgUrl(cacheDir, d.image as string)
    : undefined;

  /**
   * 初始部队 / 战争机器的生物解析：本 mod 生物表优先，缺失回退**全类目索引**。
   *
   * 必须是跨模组的：实测 7,397 个部队单位里 126 个是只存在于 core 的原版战争机器
   * （`ballista` / `firstAidTent` / `catapult`）。取不到记录时它们既没有图标、右键弹不出
   * 生物窗，还会因为读不到 `warMachine` 而被错分到"初始部队"格里（下方 `machines` 判据）。
   */
  const creatureRef = makeRefResolver({ category: "creatures", modId, lang, modCache, getEntry });
  const creatureById = (cid: string) => creatureRef(cid).record;
  /**
   * 初始法术解析：同样是跨模组引用（4,581/4,822 条初始法术属 core），
   * 本 mod 法术表优先 → 全类目索引兜底。名称、图标、描述都从这里取。
   */
  const spellRef = makeRefResolver({ category: "spells", modId, lang, modCache, getEntry });

  // 初始部队出现概率（VCMI gameConfig.json 的 heroes.startingStackChances，
  // CHeroOverview.cpp:135-153 按部队在 initialArmy 中的**原始索引**取，与英雄配置无关）
  const stackChances = ((modCache?.gameSettings as Record<string, unknown>)?.startingStackChances as
    number[]) ?? [100, 88, 25];

  // 初始部队：排除战争机器（VCMI CHeroOverview.cpp:148 判 `warMachine == ArtifactID::NONE`；
  // 后端 warMachine = 绑定的神器 id，空串即非战争机器）。保留原始索引以取概率。
  const troops = army
    .map((u, idx) => ({ u, idx }))
    .filter(({ u }) => {
      const cr = creatureById(String(u.creature ?? u.id ?? ""));
      if (cr) return String(cr.warMachine ?? "").length === 0;
      return !String(u.creature ?? u.id ?? "").toLowerCase().includes("warmachine");
    });
  // 战争机器：英雄初始部队里配置的战争机器（弩车/急救帐篷/弹药车等）
  const machines = army
    .map((u, idx) => ({ u, idx }))
    .filter(({ u }) => {
      const cr = creatureById(String(u.creature ?? u.id ?? ""));
      return Boolean(cr && String(cr.warMachine ?? "").length > 0);
    });
  // 投石车（首格恒为它）：本 mod 或 core 收录了 catapult 时右键可开完整生物窗
  const catapultRef = creatureRef("catapult");

  const skillById = (sid: string) =>
    ((modCache?.skills as unknown[]) ?? []).find(
      (x) => String((x as Record<string, unknown>).id) === sid,
    ) as Record<string, unknown> | undefined;

  const spellById = (sid: string) =>
    ((modCache?.spells as unknown[]) ?? []).find(
      (x) => String((x as Record<string, unknown>).id) === sid,
    ) as Record<string, unknown> | undefined;

  // VCMI 式弹窗：头像/部队行右键 = 完整窗，技能行/法术行右键 = 文本气泡
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  /** 弹完整窗（生物=CStackWindow，英雄=CHeroOverview） */
  const openEntry = (category: WikiCategory, id: string) => {
    const hit = ((modCache?.[category] as unknown[]) ?? []).find(
      (x) => String((x as Record<string, unknown>).id) === id,
    ) as Record<string, unknown> | undefined;
    if (hit) popup.showEntry(modId, category, hit);
  };
  /** 部队单元格右键：本 mod 记录优先，缺失用全局索引条目（跨模组，如 core 的 ballista） */
  const openCreature = (cid: string) => {
    const ref = creatureRef(cid);
    if (ref.record) popup.showEntry(ref.modId, "creatures", ref.record);
  };

  /**
   * 四维主属性行（攻击力/防御力/力量/知识）。
   *
   * 右键 / 长按 = 该属性的说明文本，对齐 VCMI `CHeroWindow.cpp:131-132`：
   * `area->text = LIBRARY->generaltexth->arraytxt[2 + v]`（`0..3` 依次四种属性）。
   * 文案由后端从 h3-core 的 `ARRAYTXT.TXT`（`core.arraytxt.<行号>`）解出，写在 mod 缓存的
   * 顶层 `primarySkillTexts`（游戏级数据，与 `factionNames` 同处），故本页取用零额外 IPC。
   */
  const primaryTexts = (modCache?.primarySkillTexts ?? {}) as Record<
    string,
    Record<string, string>
  >;
  const primaryDesc = (key: string) => pickName(primaryTexts[key] ?? {}, lang);
  const primaries = [
    {
      icon: pskilIcon("attack"),
      label: t.wiki.creature.attack,
      value: d.attack,
      desc: primaryDesc("attack"),
    },
    {
      icon: pskilIcon("defense"),
      label: t.wiki.creature.defense,
      value: d.defense,
      desc: primaryDesc("defense"),
    },
    {
      icon: pskilIcon("spellPower"),
      label: t.wiki.hero.spellPower,
      value: d.spellPower,
      desc: primaryDesc("spellPower"),
    },
    {
      icon: pskilIcon("knowledge"),
      label: t.wiki.hero.knowledge,
      value: d.knowledge,
      desc: primaryDesc("knowledge"),
    },
  ];

  // VCMI 等级术语（NSecondarySkill: none/basic/advanced/expert）随启动器界面语言取词（`t`）；
  // zh 统一为游戏数据（SSTRAITS 系描述）的「初级/中级/高级」写法，不再回退英文/孤立译名。
  const levelLabel = (lvl: number | string) => {
    const map: Record<string, string> = {
      "0": t.wiki.skillLevels.none,
      "1": t.wiki.skillLevels.basic,
      "2": t.wiki.skillLevels.advanced,
      "3": t.wiki.skillLevels.expert,
    };
    return map[String(lvl)] ?? String(lvl);
  };

  /**
   * 部队/战争机器单元格：右键 = CStackWindow 完整生物窗（WikiHeroContent.cpp:311, 385）。
   * `stackIndex` = 该槽在 initialArmy 中的原始索引，用于取出现概率。
   */
  const unitCell = (
    u: Record<string, unknown>,
    key: string,
    stackIndex?: number,
    withPopup = true,
  ) => {
    const cid = String(u.creature ?? u.id ?? "");
    const ref = creatureRef(cid);
    const iconUrl = ref.image ? fullImgUrl(cacheDir, ref.image) : undefined;
    const min = Number(u.min ?? 0);
    const max = Number(u.max ?? 0);
    const count = min || max ? `${min}${max !== min ? `-${max}` : ""}` : String(u.count ?? "");
    // 出现概率：VCMI 用游戏设置数组按槽位原始索引取（非英雄配置）
    const prob = stackIndex !== undefined ? Number(stackChances[stackIndex] ?? 0) : 0;
    return (
      <div
        key={key}
        className="flex w-10 flex-col items-center gap-0.5"
        {...(withPopup && ref.record ? triggers(() => openCreature(cid)) : {})}
      >
        <SafeImg
          src={iconUrl}
          alt=""
          className="h-8 w-8 rounded border border-slate-700/50 bg-slate-800/60 pixelated object-contain"
          placeholderClassName="h-8 w-8 rounded border border-slate-700/50 bg-slate-800/60"
        />
        <span className="text-[10px] text-slate-200">{count}</span>
        {prob ? <span className="text-[9px] text-slate-500">{`${prob}%`}</span> : null}
      </div>
    );
  };

  // 沿用启动器暗色主题（不复刻 VCMI 皮革背景），仅复刻布局与内容组织
  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      <h2 className="text-center text-lg font-bold text-amber-200">
        {pickName((d.name as Record<string, string>) ?? {}, lang)}
      </h2>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        {/* 左栏 */}
        <div className="space-y-3">
          <div
            className="flex gap-3 rounded-xl border border-slate-700/50 bg-slate-900/40 p-3"
            // VCMI WikiHeroContent.cpp:137：英雄头像右键 = CHeroOverview 完整英雄窗
            {...triggers(() => openEntry("heroes", String(d.id ?? "")))}
          >
            <SafeImg
              src={portraitUrl}
              alt=""
              className="h-20 w-[58px] flex-none rounded border border-slate-700/50 pixelated object-contain"
              placeholderClassName="h-20 w-[58px] flex-none rounded border border-slate-700/50 bg-slate-800/60"
            />
            <div className="min-w-0 flex-1">
              <p className="truncate text-base font-bold text-amber-200">
                {pickName((d.name as Record<string, string>) ?? {}, lang)}
              </p>
              <p className="truncate text-sm text-slate-300">
                {pickName((d.className as Record<string, string>) ?? {}, lang) || String(d.class ?? "")}
              </p>
              <p className="mt-1 text-xs text-slate-500">
                {d.female === true ? "♀" : "♂"}
                {String(d.alignment ?? "")
                  ? ` · ${t.wiki.alignment[String(d.alignment).toLowerCase() as keyof typeof t.wiki.alignment] ?? String(d.alignment)}`
                  : ""}
              </p>
            </div>
          </div>

          <div className="rounded-xl border border-slate-700/50 bg-slate-900/40 p-3">
            {/* VCMI 英雄面板主属性名为「攻击力/防御力/力量/知识」（core.priskill） */}
            <div className="mb-2 grid grid-cols-4 gap-1 text-center text-[10px] text-slate-400">
              <span>{t.wiki.creature.attack}</span>
              <span>{t.wiki.creature.defense}</span>
              <span>{t.wiki.hero.spellPower}</span>
              <span>{t.wiki.hero.knowledge}</span>
            </div>
            <div className="grid grid-cols-4 gap-1">
              {primaries.map((p, i) => (
                <div
                  key={i}
                  // 右键 / 长按 = 该属性说明（VCMI 主属性区是 `LRClickableAreaWText`，
                  // 右键文本 = `arraytxt[2 + v]`，见 `primaryTexts` 的说明）
                  {...(p.desc ? triggers((pt) => popup.showText(p.desc, pt)) : {})}
                  title={p.desc || undefined}
                  className="flex flex-col items-center gap-1 rounded border border-slate-700/50 bg-slate-800/60 p-1"
                >
                  {p.icon ? <HdIcon src={p.icon} alt="" className="h-8 w-8 object-contain" /> : <div className="h-8 w-8" />}
                  <span className="text-sm font-bold text-slate-100">{String(p.value ?? "")}</span>
                </div>
              ))}
            </div>
          </div>

          {/* VCMI CHeroOverview.cpp:102-106：传记为纯文本框，无标题 */}
          <div className="rounded-xl border border-slate-700/50 bg-slate-900/40 p-3">
            <p className="max-h-32 overflow-y-auto text-sm leading-relaxed text-slate-400">
              {pickName(bio, lang) || "—"}
            </p>
          </div>

          {pickName(spec, lang) && (
            <div className="rounded-xl border border-slate-700/50 bg-slate-900/40 p-3">
              <div className="mb-1 flex items-center gap-2">
                <SafeImg
                  src={fullImgUrl(cacheDir, d.specialtyIcon as string)}
                  alt=""
                  className="h-11 w-11 flex-none rounded border border-slate-700/50 pixelated object-contain"
                  placeholderClassName="h-11 w-11 flex-none rounded border border-slate-700/50 bg-slate-800/60"
                />
                <div className="min-w-0 flex-1">
                  <p className="text-[13px] font-medium text-amber-200">{t.wiki.hero.specialty}</p>
                  <p className="truncate text-sm text-slate-200">{pickName(spec, lang)}</p>
                </div>
              </div>
              {pickName(d.specialtyDescription as Record<string, string>, lang) && (
                <p className="text-xs leading-relaxed text-slate-500">
                  {pickName(d.specialtyDescription as Record<string, string>, lang)}
                </p>
              )}
            </div>
          )}
        </div>

        {/* 右栏 */}
        <div className="space-y-3">
          <div className="rounded-xl border border-slate-700/50 bg-slate-900/40 p-3">
            <h3 className="mb-2 text-sm font-semibold text-amber-200">{t.wiki.hero.startingArmy}</h3>
            <div className="flex flex-wrap justify-between gap-y-2">
              {/* VCMI CHeroOverview.cpp:139-143：7 个槽位背景框全部画出，空槽也可见 */}
              {Array.from({ length: 7 }).map((_, i) =>
                troops[i] ? (
                  unitCell(troops[i].u, `t-${i}`, troops[i].idx)
                ) : (
                  <div
                    key={`t-${i}`}
                    className="h-10 w-10 rounded border border-slate-700/50 bg-slate-800/30"
                  />
                ),
              )}
            </div>
          </div>

          <div className="rounded-xl border border-slate-700/50 bg-slate-900/40 p-3">
            <h3 className="mb-2 text-sm font-semibold text-amber-200">{t.wiki.hero.warMachine}</h3>
            {/* CHeroOverview.cpp:164-189：4 槽位，首格恒为投石车（与英雄配置无关），
                其余为初始部队里的战争机器（按军队顺序） */}
            <div className="flex flex-wrap gap-2">
              {Array.from({ length: 4 }).map((_, i) =>
                i === 0 ? (
                  <div
                    key="catapult"
                    className="flex w-10 flex-col items-center gap-0.5"
                    {...(catapultRef.record ? triggers(() => openCreature("catapult")) : {})}
                  >
                    <SafeImg
                      src={fullImgUrl(cacheDir, sharedIcons["warmachine/catapult"])}
                      alt=""
                      className="h-8 w-8 rounded border border-slate-700/50 pixelated object-contain"
                      placeholderClassName="h-8 w-8 rounded border border-slate-700/50 bg-slate-800/60"
                    />
                    <span className="text-[9px] text-slate-500">100%</span>
                  </div>
                ) : machines[i - 1] ? (
                  unitCell(machines[i - 1].u, `wm-${i}`, machines[i - 1].idx)
                ) : (
                  <div
                    key={`wm-${i}`}
                    className="h-10 w-10 rounded border border-slate-700/50 bg-slate-800/30"
                  />
                ),
              )}
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="rounded-xl border border-slate-700/50 bg-slate-900/40 p-2">
              <h3 className="mb-1 text-sm font-semibold text-amber-200">{t.wiki.hero.secondarySkills}</h3>
              <div className="space-y-1">
                {sec.length === 0 && <span className="text-xs text-slate-500">—</span>}
                {sec.map((s, i) => {
                  const sid = String(s.skill ?? "");
                  const skillEntry = skillById(sid);
                  const skillName =
                    pickName((skillEntry?.name as Record<string, string>) ?? {}, lang) ||
                    pickName((s.name as Record<string, string>) ?? {}, lang) ||
                    sid;
                  // 技能图标：英雄初始技能用 SECSK32 帧（后端按 VCMI WikiWindow.cpp:652 提取），
                  // mod 自定义技能用技能页图标
                  const skillImg =
                    (s.icon as string | undefined) ?? (skillEntry?.image as string | undefined);
                  // VCMI WikiHeroContent.cpp:451：技能行右键 = 该等级描述的文本气泡。
                  // 技能表 levels 的键是 basic/advanced/expert（VCMI NSecondarySkill 顺序），
                  // 而英雄配置里的 level 是数字 1/2/3 —— 需要映射
                  // （此前直接查 "1"/"2"/"3"，永远取不到，气泡回退成技能名）
                  const levelKey =
                    ["basic", "advanced", "expert"][Math.min(2, Math.max(0, Number(s.level ?? 1) - 1))];
                  const skillDesc =
                    pickName(
                      (skillEntry?.levels as Record<string, Record<string, string>> | undefined)?.[levelKey] ?? {},
                      lang,
                    ) ||
                    // 技能不在本 mod 技能表里时（实测 4,581/4,822 条初始技能是 core 技能），
                    // 用后端随英雄条目预解析的 `levelDescriptions`（[basic, advanced, expert]）
                    String((s.levelDescriptions as string[] | undefined)?.[Number(s.level ?? 1) - 1] ?? "");
                  return (
                    <div
                      key={i}
                      className="flex cursor-help items-center gap-2"
                      {...triggers((p) => popup.showText(skillDesc || skillName, p))}
                    >
                      <SafeImg
                        src={skillImg ? fullImgUrl(cacheDir, skillImg) : undefined}
                        alt=""
                        className="h-7 w-7 flex-none rounded border border-slate-700/50 pixelated object-contain"
                        placeholderClassName="h-7 w-7 flex-none rounded border border-slate-700/50 bg-slate-800/60"
                      />
                      <div className="min-w-0 flex-1 leading-tight">
                        <p className="text-[11px] text-slate-300">{levelLabel(Number(s.level ?? 1))}</p>
                        <p className="truncate text-[11px] text-slate-200">{skillName}</p>
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>

            <div className="rounded-xl border border-slate-700/50 bg-slate-900/40 p-2">
              <h3 className="mb-1 text-sm font-semibold text-amber-200">{t.wiki.hero.spells}</h3>
              <div className="space-y-1">
                {d.haveSpellBook !== true && spells.length === 0 && (
                  <span className="text-xs text-slate-500">{t.wiki.hero.noSpellBook}</span>
                )}
                {d.haveSpellBook === true && (
                  <div className="flex items-center gap-2">
                    <SafeImg
                      src={fullImgUrl(cacheDir, sharedIcons["spellbook"])}
                      alt=""
                      className="h-7 w-7 flex-none rounded border border-slate-700/50 pixelated object-contain"
                      placeholderClassName="h-7 w-7 flex-none rounded border border-slate-700/50 bg-slate-800/60"
                    />
                    <span className="text-[11px] text-slate-200">{t.wiki.hero.spellBook}</span>
                  </div>
                )}
                {spells.map((sid) => {
                  // 英雄初始法术**跨模组是常态**（实测该发行版 4,822 条初始法术里 4,581 条不在本
                  // mod 的法术表内，属 core），名称/图标/描述一律走共享解析器：
                  // 本 mod 法术表优先 → 全类目索引兜底。
                  const ref = spellRef(sid);
                  const rec = ref.record ?? {};
                  // 后端随英雄预解析的 spellDetails（含 name / index），作为兜底与优先级判据
                  const detail = ((d.spellDetails as Record<string, unknown>[]) ?? []).find(
                    (x) => String(x.id ?? "") === sid,
                  );
                  /**
                   * 图标优先级：**条目的 `image`** → `spellDetails.index` 拼 spellbon 路径。
                   *
                   * 不能反过来：mod 法术的 `index` 常为 -1（不在 H3 法术表内），而原版法术的
                   * `index` 虽有效、其记录里也有现成的 `core/images/spellbon/<idx>.webp`。
                   * 旧代码只认 index，于是 index=-1 的法术整行空白（实测「瞬间移动」无图，
                   * 同一页里没索引的反而有图）。
                   */
                  const idxFallback = Number(detail?.index ?? rec.iconIndex ?? -1);
                  const iconUrl = ref.image
                    ? fullImgUrl(cacheDir, ref.image)
                    : idxFallback >= 0
                      ? fullImgUrl(cacheDir, `core/images/spellbon/${idxFallback}.webp`)
                      : undefined;
                  // 名字：spellDetails 的名字可能是**裸 id**（该法术没有 i18n 文案），
                  // 此时让位给全局索引里的名字；两者都拿不到才显示 id。
                  const detailName = detail
                    ? pickName((detail.name as Record<string, string>) ?? {}, lang)
                    : "";
                  const spName =
                    (detailName && detailName !== sid ? detailName : "") ||
                    (ref.name !== sid ? ref.name : "") ||
                    sid;
                  const spDesc = pickName((rec.description as Record<string, string>) ?? {}, lang);
                  return (
                    <div
                      key={sid}
                      className="flex cursor-help items-center gap-2"
                      // VCMI WikiHeroContent.cpp:527-529：法术行右键 = Lv1 描述文本气泡
                      {...triggers((p) => popup.showText(spDesc || spName, p))}
                    >
                      <SafeImg
                        src={iconUrl}
                        alt=""
                        className="h-7 w-7 flex-none rounded border border-slate-700/50 pixelated object-contain"
                        placeholderClassName="h-7 w-7 flex-none rounded border border-slate-700/50 bg-slate-800/60"
                      />
                      <span className="truncate text-[11px] text-slate-200">{spName}</span>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
