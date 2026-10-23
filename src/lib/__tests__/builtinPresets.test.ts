import { describe, it, expect } from "vitest";
import { mergeBuiltinPresets, type BuiltinPreset } from "@/lib/builtinPresets";

/**
 * `mergeBuiltinPresets` 把 launcher.json 的 `builtin_presets` 与源码内置合并。
 * 发行版靠它「不改源码追加语言版/新预设」，字段级合并一旦出错会静默丢模组，故回归。
 */
describe("mergeBuiltinPresets", () => {
  const base: BuiltinPreset[] = [
    {
      id: "a",
      label: { chinese: "甲" },
      mods: ["x"],
      modsByLang: { chinese: ["cx"] },
      locked: false,
    },
  ];

  it("空 / 缺省 config 时原样返回 source（同一引用）", () => {
    expect(mergeBuiltinPresets(base)).toBe(base);
    expect(mergeBuiltinPresets(base, [])).toBe(base);
  });

  it("新 id 直接追加", () => {
    const out = mergeBuiltinPresets(base, [
      { id: "b", label: { chinese: "乙" }, mods: ["y"] },
    ]);
    expect(out.map((p) => p.id)).toEqual(["a", "b"]);
    expect(out[1].mods).toEqual(["y"]);
  });

  it("同名 id 字段级合并：mods 并集去重、label 合并", () => {
    const out = mergeBuiltinPresets(base, [
      { id: "a", label: { english: "A" }, mods: ["x", "z"] },
    ]);
    expect(out).toHaveLength(1);
    expect(out[0].label).toEqual({ chinese: "甲", english: "A" });
    expect(out[0].mods).toEqual(["x", "z"]);
  });

  it("同名 id 合并：同语言 modsByLang 并集去重", () => {
    const out = mergeBuiltinPresets(base, [
      { id: "a", label: {}, mods: [], modsByLang: { chinese: ["cx", "cy"] } },
    ]);
    expect(out[0].modsByLang).toEqual({ chinese: ["cx", "cy"] });
  });

  it("同名 id 合并：新语言键的 modsByLang 加入", () => {
    const out = mergeBuiltinPresets(base, [
      { id: "a", label: {}, mods: [], modsByLang: { english: ["en1"] } },
    ]);
    expect(out[0].modsByLang).toEqual({ chinese: ["cx"], english: ["en1"] });
  });

  it("不修改入参 source（不可变）", () => {
    mergeBuiltinPresets(base, [{ id: "a", label: {}, mods: ["z"] }]);
    expect(base[0].mods).toEqual(["x"]);
  });
});
