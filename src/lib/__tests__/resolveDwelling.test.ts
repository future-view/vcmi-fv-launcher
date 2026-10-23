import { describe, it, expect } from "vitest";
import { resolveDwellingId, resolveDwellingIds } from "@/lib/realm/showcase";

/**
 * `resolveDwellingId` 按**名称优先**解析兵种招募建筑（播放器与编辑器共用）。
 * 名称匹配分支用到 `pickName` —— 曾在 strict 改造中修正其可空参数，故回归保护。
 */
describe("resolveDwellingId", () => {
  it("按多语言建筑名匹配（名称优先于 id）", () => {
    const buildings = [
      { id: "castle.dwelling1", name: { chinese: "枪兵营", english: "Pikeman Dwelling" } },
    ];
    const creature = { id: "pikeman", dwellingNames: { chinese: ["枪兵营"] } };
    expect(resolveDwellingId(buildings, creature, "chinese")).toBe("castle.dwelling1");
  });

  it("建筑名为纯字符串时也能匹配（第三方模组偶见）", () => {
    const buildings = [{ id: "b1", name: "枪兵营" }];
    const creature = { id: "pikeman", dwellingNames: { chinese: ["枪兵营"] } };
    expect(resolveDwellingId(buildings, creature, "chinese")).toBe("b1");
  });

  it("名称不匹配且无等级 → null", () => {
    const buildings = [{ id: "b1", name: { english: "X" } }];
    expect(resolveDwellingId(buildings, { id: "c" }, "chinese")).toBeNull();
  });

  it("lang 缺省时不崩溃（跳过名称匹配，无等级则 null）", () => {
    const buildings = [{ id: "b1", name: { english: "Pikeman Dwelling" } }];
    const creature = { id: "pikeman", dwellingNames: { english: ["Pikeman Dwelling"] } };
    expect(resolveDwellingId(buildings, creature, undefined)).toBeNull();
  });
});

describe("resolveDwellingIds", () => {
  it("去重且保序", () => {
    const buildings = [
      { id: "b1", name: { english: "A" } },
      { id: "b2", name: { english: "B" } },
    ];
    const creatures = [
      { id: "c1", dwellingNames: { english: ["A"] } },
      { id: "c2", dwellingNames: { english: ["B"] } },
      { id: "c3", dwellingNames: { english: ["A"] } },
    ];
    expect(resolveDwellingIds(buildings, creatures, "english")).toEqual(["b1", "b2"]);
  });
});
