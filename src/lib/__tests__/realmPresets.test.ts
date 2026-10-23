import { describe, it, expect } from "vitest";
import {
  adoptRefs,
  addPreset,
  deletePreset,
  enableDep,
  purgeRoots,
  renamePreset,
  type PresetMap,
} from "@/features/realms/editor/realmPresets";
import { REALM_PRESET_ID } from "@/features/realms/editor/editorShared";

/**
 * 领域「模组组合」的纯变换。
 *
 * 这几条规则是**静默失败**的重灾区：删掉最后一个组合 = 所有模组都不生效、
 * `__realm__` 被改名 = 导出找不到契约 id、引用归属算错 = 勾了的模组进不了激活集，
 * 都不报错。故把边界逐条钉住。
 */
describe("realmPresets", () => {
  const base: PresetMap = { [REALM_PRESET_ID]: ["a"], extra: ["b"] };

  describe("addPreset", () => {
    it("建成空组合且不动入参", () => {
      const out = addPreset(base, "new")!;
      expect(out.new).toEqual([]);
      expect(base.new).toBeUndefined();
      expect(out).not.toBe(base);
    });

    it("空名 / 纯空格 / 撞名 → null", () => {
      expect(addPreset(base, "")).toBeNull();
      expect(addPreset(base, "   ")).toBeNull();
      expect(addPreset(base, "extra")).toBeNull();
    });
  });

  describe("deletePreset", () => {
    it("删掉后给出剩余的第一个作为新激活项", () => {
      const r = deletePreset(base, "extra")!;
      expect(Object.keys(r.presets)).toEqual([REALM_PRESET_ID]);
      expect(r.active).toBe(REALM_PRESET_ID);
    });

    it("只剩一个时拒绝删除（否则空启用集 = 所有模组失效）", () => {
      expect(deletePreset({ [REALM_PRESET_ID]: [] }, REALM_PRESET_ID)).toBeNull();
    });
  });

  describe("renamePreset", () => {
    it("键整体迁移并保持其余顺序", () => {
      const out = renamePreset(base, "extra", "renamed")!;
      expect(Object.keys(out)).toEqual([REALM_PRESET_ID, "renamed"]);
      expect(out.renamed).toEqual(["b"]);
    });

    it("契约 id 不可改名", () => {
      expect(renamePreset(base, REALM_PRESET_ID, "other")).toBeNull();
    });

    it("新名为空 / 与原值相同 / 撞名 → null", () => {
      expect(renamePreset(base, "extra", "  ")).toBeNull();
      expect(renamePreset(base, "extra", "extra")).toBeNull();
      expect(renamePreset(base, "extra", REALM_PRESET_ID)).toBeNull();
    });
  });

  describe("purgeRoots", () => {
    const rootOf = (id: string) => (id.includes(".") ? id.split(".")[0] : id);

    it("子模组按根模组一并清理", () => {
      const ps: PresetMap = { p: ["P", "P.S1", "Q"], q: ["P.S2"] };
      const out = purgeRoots(ps, ["p"], rootOf);
      expect(out.p).toEqual(["Q"]);
      expect(out.q).toEqual([]);
    });

    it("大小写不敏感", () => {
      const out = purgeRoots({ p: ["Soldier"] }, ["SOLDIER"], rootOf);
      expect(out.p).toEqual([]);
    });

    it("空清单直接返回原引用（不触发重渲染）", () => {
      expect(purgeRoots(base, [], rootOf)).toBe(base);
    });
  });

  describe("adoptRefs", () => {
    it("缺省合并进 __realm__ 并去重", () => {
      const r = adoptRefs({ [REALM_PRESET_ID]: ["a"] }, ["a", "b"]);
      expect(r.name).toBe(REALM_PRESET_ID);
      expect(r.presets[REALM_PRESET_ID]).toEqual(["a", "b"]);
    });

    it("create 撞名自动加后缀，且不覆盖已有内容", () => {
      const r = adoptRefs({ mine: ["x"], "mine-2": [] }, ["y"], "mine");
      expect(r.name).toBe("mine-3");
      expect(r.presets["mine-3"]).toEqual(["y"]);
      expect(r.presets.mine).toEqual(["x"]);
    });
  });

  describe("enableDep", () => {
    it("补入所有指定组合", () => {
      const out = enableDep({ a: [], b: [] }, ["a", "b"], "Dep");
      expect(out.a).toEqual(["Dep"]);
      expect(out.b).toEqual(["Dep"]);
    });

    it("已存在（大小写不敏感）时不重复补入，且返回原引用", () => {
      const ps = { a: ["dep"] };
      expect(enableDep(ps, ["a"], "Dep")).toBe(ps);
    });

    it("组合不存在时跳过（不凭空造组合）", () => {
      const out = enableDep(base, ["missing"], "Dep");
      expect(out).toBe(base);
      expect(out.missing).toBeUndefined();
    });
  });
});
