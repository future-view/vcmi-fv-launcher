import { useState, useEffect } from "react";
import { cn } from "@/lib/utils";
import { Switch } from "@/shared/ui/switch";
import {
  Select,
  SelectGroup,
  SelectLabel,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/shared/ui/select";
import { Slider } from "@/shared/ui/slider";
import { Input } from "@/shared/ui/input";
import type { SettingItem } from "@/lib/settingsSchema";
import { useI18n } from "@/i18n";
import { useSettingText, type SettingOption } from "./text";

interface ControlProps {
  item: SettingItem;
  value: unknown;
  onChange: (v: unknown) => void;
  /** 运行时覆盖的候选（如分辨率从显示器枚举获取） */
  candidatesOverride?: SettingOption[];
  /** 跨键控件：联动键的当前值（如全屏/城镇生物互斥） */
  value2?: unknown;
  /** 跨键控件：一次写回多个键 */
  onChangeMulti?: (entries: Record<string, unknown>) => void;
  /** 依赖禁用（如全屏时禁用分辨率、垂直同步时禁用帧率） */
  disabled?: boolean;
}

/**
 * 选项 chip 组（toggleGroup / fullscreenGroup / creatureDisplay / btnList 四类控件共用）。
 * 每个候选项是独立 chip（自带边框+底色）：可点击性明确，且天然支持换行——
 * 窄卡片或长文案（俄/德语翻译）下逐个折行，不会整组溢出卡片右缘被裁切。
 * 选中态用实底蓝 + 前置圆点强化「单选」语义，与 Switch 的蓝色呼应。
 */
function SegmentedButtons({ options, value, disabled, onSelect }: {
  options: SettingOption[];
  value?: string;
  disabled?: boolean;
  onSelect: (v: string) => void;
}) {
  return (
    <div className="flex max-w-full flex-wrap gap-1.5" role="radiogroup">
      {options.map((c) => {
        const active = value === c.v;
        return (
          <button
            key={c.v}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={disabled}
            onClick={() => onSelect(c.v)}
            className={cn(
              "flex items-center gap-1.5 whitespace-nowrap rounded-md border px-2.5 py-1 text-xs transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/40",
              active
                ? "border-blue-400/50 bg-blue-500/25 font-medium text-blue-100"
                : "border-slate-700 bg-slate-800/50 text-muted hover:border-slate-600 hover:bg-slate-800 hover:text-slate-300"
            )}
          >
            {active && <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-blue-400" aria-hidden />}
            {c.text}
          </button>
        );
      })}
    </div>
  );
}

/** 按控件类型渲染对应编辑控件（受控、即时写回）；文案一律经 `./text` 从 i18n 词典取词 */
export function SettingControl({ item, value, onChange, candidatesOverride, value2, onChangeMulti, disabled }: ControlProps) {
  const { t } = useI18n();
  const text = useSettingText();

  switch (item.ctl) {
    case "switch":
      return (
        <Switch
          checked={Boolean(value)}
          disabled={disabled}
          onCheckedChange={(v) => onChange(v)}
          aria-label={text.itemLabel(item.id)}
        />
      );

    case "select": {
      // 动态候选（如 AI 模型列表）优先：`v` 即写入值，`text` 已按当前语言拼好富标签（推荐 / 未验证）
      const opts = text.optionsFor(item, candidatesOverride);
      return (
        <Select value={String(value)} disabled={disabled} onValueChange={(v) => onChange(v)}>
          <SelectTrigger className="h-8 w-44 text-sm">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {opts.map((opt) => (
              <SelectItem key={opt.v} value={opt.v}>
                {opt.text}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }

    case "slider":
    case "sliderPct": {
      const min = item.min ?? 0;
      const max = item.max ?? 100;
      const num = Number(value) || 0;
      return (
        <div className="flex w-full items-center gap-3">
          <Slider
            value={[num]}
            min={min}
            max={max}
            step={item.step ?? 1}
            disabled={disabled}
            onValueChange={(v) => onChange(v[0])}
            className="flex-1"
          />
          {/* 数值输入框（与「界面缩放」的自定义输入同款）：滑条粗调 + 键入精调。
              输入过程即时收敛到 [min, max]，避免写出越界值；sliderPct 附带 % 后缀 */}
          <div className="flex w-16 shrink-0 items-center gap-0.5">
            <Input
              type="number"
              className="h-8 min-w-0 flex-1 px-1.5 text-right font-mono text-xs"
              value={String(num)}
              min={min}
              max={max}
              step={item.step ?? 1}
              disabled={disabled}
              onChange={(e) => {
                const n = Number(e.target.value);
                if (Number.isNaN(n)) return;
                onChange(Math.min(max, Math.max(min, n)));
              }}
            />
            {item.ctl === "sliderPct" && (
              <span className="text-xs text-slate-400">%</span>
            )}
          </div>
        </div>
      );
    }

    case "number":
      return (
        <Input
          type="number"
          className="h-8 w-28 text-sm"
          value={String(value ?? "")}
          min={item.min}
          max={item.max}
          step={item.step}
          disabled={disabled}
          onChange={(e) => {
            const n = Number(e.target.value);
            if (!Number.isNaN(n)) onChange(n);
          }}
        />
      );

    case "text":
      return (
        <Input
          className="h-8 w-full max-w-sm text-sm"
          value={String(value ?? "")}
          placeholder={item.k}
          disabled={disabled}
          onChange={(e) => onChange(e.target.value)}
        />
      );

    case "btnList": {
      const options = text.optionsFor(item);
      return (
        <SegmentedButtons
          options={options}
          value={String(value)}
          disabled={disabled}
          onSelect={(v) => onChange(v)}
        />
      );
    }

    case "resolutionCombo": {
      // 组合下拉：候选 v 形如 "1280x720"，当前值 = width(value)×height(value2)
      const opts = text.optionsFor(item, candidatesOverride);
      const curKey = `${value}x${value2}`;
      const onPick = (v: string) => {
        const [w, h] = v.split("x").map((n) => Number(n));
        onChangeMulti?.({ [item.k]: Number(w), [item.altKey ?? ""]: Number(h) });
      };
      // 候选已按屏幕比分组（`group`），平铺渲染；无 group 的旧数据退化为不分组的平铺
      const groups = new Map<string, SettingOption[]>();
      for (const o of opts) {
        const g = o.group ?? "";
        const arr = groups.get(g);
        if (arr) arr.push(o);
        else groups.set(g, [o]);
      }
      const labels = [...groups.keys()];
      const flat = labels.length === 1 && labels[0] === "";
      const renderItems = (items: SettingOption[]) =>
        items.map((o) => (
          <SelectItem
            key={o.v}
            value={o.v}
            trailing={
              o.primary ? (
                <span className="ml-auto rounded bg-sky-500/20 px-1.5 py-0.5 text-[10px] leading-none text-sky-300">
                  {text.word("currentDisplay")}
                </span>
              ) : undefined
            }
          >
            {o.text}
          </SelectItem>
        ));
      return (
        <Select
          value={opts.some((o) => o.v === curKey) ? curKey : undefined}
          disabled={disabled}
          onValueChange={(v) => onPick(v)}
        >
          <SelectTrigger className="h-8 w-44 text-sm">
            <SelectValue placeholder={`${value}×${value2}`} />
          </SelectTrigger>
          <SelectContent>
            {flat ? (
              renderItems(groups.get("")!)
            ) : (
              labels.map((g) => (
                <SelectGroup key={g}>
                  <SelectLabel>{g}</SelectLabel>
                  {renderItems(groups.get(g)!)}
                </SelectGroup>
              ))
            )}
          </SelectContent>
        </Select>
      );
    }

    case "toggleGroup": {
      // 若原始值为数字，写回时转换候选字符串为数字（如 heroMoveTime: 150）
      const numeric = typeof item.val === "number";
      const options = text.optionsFor(item, candidatesOverride);
      return (
        <SegmentedButtons
          options={options}
          value={String(value)}
          disabled={disabled}
          onSelect={(v) => onChange(numeric ? Number(v) : v)}
        />
      );
    }

    case "scalingGroup": {
      // 界面缩放联动组：自动(checkbox) + 预设下拉 + 自定义输入
      // 底层单值：0=自动，否则为百分比数值
      const num = Number(value) || 0;
      const auto = num === 0;
      const presets = text.optionsFor(item, candidatesOverride).filter((c) => c.v !== "0");
      const presetValues = presets.map((c) => Number(c.v));
      const custom = !auto && !presetValues.includes(num);
      const setAuto = (on: boolean) => onChange(on ? 0 : (presetValues[0] ?? 125));
      const onPickPreset = (v: string) => onChange(Number(v));
      const onCustom = (raw: string) => {
        const n = Number(raw);
        if (!Number.isNaN(n) && n > 0) onChange(n);
      };
      return (
        <div className="flex flex-wrap items-center gap-2">
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-muted">
            <input
              type="checkbox"
              checked={auto}
              disabled={disabled}
              onChange={(e) => setAuto(e.target.checked)}
              className="h-3.5 w-3.5 accent-blue-500"
            />
            {t.settings.auto}
          </label>
          <Select
            value={!auto && !custom ? String(num) : undefined}
            disabled={disabled || auto || custom}
            onValueChange={(v) => onPickPreset(v)}
          >
            <SelectTrigger className="h-8 w-28 text-sm">
              <SelectValue placeholder={custom ? `${num}%` : t.settings.preset} />
            </SelectTrigger>
            <SelectContent>
              {presets.map((o) => (
                <SelectItem key={o.v} value={o.v}>
                  {o.text}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Input
            type="number"
            className="h-8 w-24 text-sm"
            value={auto ? "" : String(num)}
            min={50}
            max={item.max ?? 400}
            disabled={disabled || auto}
            placeholder={t.settings.customPercent}
            onChange={(e) => onCustom(e.target.value)}
          />
        </div>
      );
    }

    case "json":
      return <JsonControl item={item} value={value} onChange={onChange} />;

    case "fullscreenGroup": {
      // 窗口化(both false) / 无边框(fullscreen=true,realFullscreen=false) / 独占(both true)
      const fs = Boolean(value);
      const rfs = Boolean(value2);
      const mode = !fs ? "windowed" : rfs ? "exclusive" : "borderless";
      const modes = text.modesFor(item.id, ["windowed", "borderless", "exclusive"]);
      const apply = (m: string) => {
        const map: Record<string, [boolean, boolean]> = {
          windowed: [false, false],
          borderless: [true, false],
          exclusive: [true, true],
        };
        const [nf, nr] = map[m];
        onChangeMulti?.({ [item.k]: nf, [item.altKey ?? ""]: nr });
      };
      return (
        <SegmentedButtons options={modes} value={mode} disabled={disabled} onSelect={apply} />
      );
    }

    case "creatureDisplay": {
      // none(都不显示) / growth(增长) / available(可招募) 三态互斥
      const growth = Boolean(value2);
      const available = Boolean(value);
      const mode = growth ? "growth" : available ? "available" : "none";
      const modes = text.modesFor(item.id, ["none", "growth", "available"]);
      const apply = (m: string) => {
        const g = m === "growth";
        const a = m === "available";
        onChangeMulti?.({ [item.k]: a, [item.altKey ?? ""]: g });
      };
      return (
        <SegmentedButtons options={modes} value={mode} disabled={disabled} onSelect={apply} />
      );
    }

    default:
      return null;
  }
}

function JsonControl({ item, value, onChange }: ControlProps) {
  const { t } = useI18n();
  const [text, setText] = useState(JSON.stringify(value, null, 2));
  const [err, setErr] = useState(false);

  // 外部值变化时（如重置、切换）同步文本
  useEffect(() => {
    setText(JSON.stringify(value, null, 2));
    setErr(false);
  }, [value]);

  return (
    <div className="w-full">
      <textarea
        className={cn(
          "h-28 w-full rounded-md border bg-slate-900 p-2 font-mono text-xs text-slate-200 focus-visible:outline-none focus-visible:ring-2",
          err
            ? "border-red-500/60 focus-visible:ring-red-500/40"
            : "border-slate-700 focus-visible:ring-blue-500/40"
        )}
        value={text}
        spellCheck={false}
        onChange={(e) => {
          const nt = e.target.value;
          setText(nt);
          try {
            const parsed = JSON.parse(nt);
            setErr(false);
            onChange(parsed);
          } catch {
            setErr(true);
          }
        }}
      />
      {err && <p className="mt-1 text-xs text-red-400">{t.settings.jsonInvalid}</p>}
    </div>
  );
}
