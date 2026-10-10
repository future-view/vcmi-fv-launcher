import { useEffect, useMemo, useState } from "react";
import { useI18n, UI_LANG_OPTIONS } from "@/i18n";
import {
  detectVcmiPaths,
  getRealDirs,
  pickDirectory,
  suggestUserDataDir,
  writeLauncherConfig,
  type VcmiDirs,
} from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/ui/select";

export interface SetupWizardProps {
  onReady: () => void;
}

interface Paths {
  gameDir: string;
  userDataDir: string;
}

export function SetupWizard({ onReady }: SetupWizardProps) {
  const { t, vcmiLang, setVcmiLang } = useI18n();
  const [detected, setDetected] = useState<VcmiDirs | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const [form, setForm] = useState<Paths>({ gameDir: "", userDataDir: "" });

  useEffect(() => {
    detectVcmiPaths().then((d) => {
      if (d) {
        setDetected(d);
        // 自动探测默认触发：探测到有效路径即回填表单（仅填充，不产生任何磁盘副作用），
        // 等待用户点击"保存并进入启动器"后才落盘 launcher.json / 提取城镇图
        if (d.detected) {
          setForm({
            gameDir: d.gameDir || "",
            userDataDir: d.userDir || "",
          });
          setError("");
        }
      }
    });
  }, []);

  /** 选择游戏目录：校验是否含 VCMI_client.exe，并自动推导用户数据目录 */
  const handleChooseGame = async () => {
    const path = await pickDirectory(t.setup.gameDir);
    if (!path) return;
    setForm((f) => ({ ...f, gameDir: path }));
    setError("");
    // 由后端校验 + 自动推导用户数据目录
    const suggested = await suggestUserDataDir(path);
    if (suggested) {
      setForm({ gameDir: path, userDataDir: suggested.userDataDir });
      // 若用户目录不存在，提示但仍允许手动指定
      if (!suggested.exists) {
        setError(t.setup.missingUserData);
      }
    }
  };

  const handleChooseUser = async () => {
    const path = await pickDirectory(t.setup.userDataDir);
    if (!path) return;
    setForm((f) => ({ ...f, userDataDir: path }));
    setError("");
  };

  const derived = useMemo(() => {
    const user = form.userDataDir.trim();
    return {
      userConfigDir: user ? `${user}\\config` : "",
      userModsDir: user ? `${user}\\Mods` : "",
    };
  }, [form.userDataDir]);

  /** 自动探测：探测到有效目录组合即可填充表单，否则禁用 */
  const autoFill = () => {
    if (!detected?.detected) return;
    setForm({
      gameDir: detected.gameDir || "",
      userDataDir: detected.userDir || "",
    });
    setError("");
  };

  const handleSave = async () => {
    const gameDir = form.gameDir.trim();
    const userDataDir = form.userDataDir.trim();
    if (!gameDir || !userDataDir) {
      setError(t.setup.error);
      return;
    }
    setSaving(true);
    try {
      // 语言随配置一并写入：向导阶段 launcher.json 尚不存在，无法走 set_ui_lang；
      // 这样便携包首次保存后语言即被固化，后续启动以它为唯一权威。
      await writeLauncherConfig({ gameDir, userDataDir, uiLang: vcmiLang });
      const dirs = await getRealDirs();
      if (!dirs?.ready) {
        setError(t.setup.error);
        return;
      }
      onReady();
    } catch (e: any) {
      setError(backendErrorText(e) || t.setup.error);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex flex-col items-center justify-center bg-slate-950 p-6 text-slate-100">
      <div className="w-full max-w-xl rounded-2xl border border-slate-800 bg-slate-900/90 p-8 shadow-2xl backdrop-blur-xl">
        <h1 className="mb-1 text-2xl font-bold text-sky-400">{t.setup.title}</h1>
        <p className="mb-6 text-sm text-slate-400">{t.setup.subtitle}</p>

        {/* 界面语言：优先匹配系统语言，否则保底英文；可在下拉中切换 */}
        <div className="mb-5 flex items-center justify-between gap-3">
          <div>
            <label className="block text-sm font-medium text-slate-300">
              {t.setup.language}
            </label>
            <p className="mt-0.5 text-xs text-slate-500">{t.setup.languageHint}</p>
          </div>
          <Select value={vcmiLang} onValueChange={(v) => setVcmiLang(v)}>
            <SelectTrigger className="w-44 bg-slate-950/60 text-slate-200">
              <SelectValue />
            </SelectTrigger>
            <SelectContent className="z-[200] border-slate-700 bg-slate-900 text-slate-200">
              {UI_LANG_OPTIONS.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        <PathField
          label={t.setup.gameDir}
          hint={t.setup.gameDirHint}
          choose={t.setup.choose}
          value={form.gameDir}
          onChoose={handleChooseGame}
        />
        <PathField
          label={t.setup.userDataDir}
          hint={t.setup.userDataDirHint}
          choose={t.setup.choose}
          value={form.userDataDir}
          onChoose={handleChooseUser}
        />

        {/* 自动推导的派生目录（只读展示，无需输入） */}
        <div className="mb-5 rounded-xl border border-slate-800 bg-slate-950/40 p-3 text-xs text-slate-400">
          <div className="mb-1 font-medium text-slate-300">{t.setup.derived}</div>
          <div>
            {t.setup.userConfigDir}:{" "}
            <span className="font-mono text-slate-500">{derived.userConfigDir || "—"}</span>
          </div>
          <div>
            {t.setup.userModsDir}:{" "}
            <span className="font-mono text-slate-500">{derived.userModsDir || "—"}</span>
          </div>
        </div>

        {error && <p className="mt-2 text-sm text-red-400">{error}</p>}

        <div className="mt-6 flex items-center justify-between gap-3">
          <Button
            variant="outline"
            onClick={autoFill}
            disabled={!detected?.detected}
            className="border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.autoDetect}
          </Button>
          <Button
            onClick={handleSave}
            disabled={saving}
            className="bg-sky-600 px-6 text-white hover:bg-sky-500"
          >
            {saving ? t.common.loading : t.setup.save}
          </Button>
        </div>
      </div>
    </div>
  );
}

function PathField({
  label,
  hint,
  choose,
  value,
  onChoose,
  error,
}: {
  label: string;
  hint: string;
  choose: string;
  value: string;
  onChoose: () => void | Promise<unknown>;
  error?: string;
}) {
  return (
    <div className="mb-4">
      <label className="mb-1 block text-sm font-medium text-slate-300">{label}</label>
      <p className="mb-2 text-xs text-slate-500">{hint}</p>
      <div className="flex gap-2">
        <Input
          value={value}
          readOnly
          className="flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
        />
        <Button
          type="button"
          variant="outline"
          onClick={onChoose}
          className="border-slate-700 text-slate-300 hover:bg-slate-800"
        >
          {choose}
        </Button>
      </div>
      {error && <p className="mt-1 text-xs text-red-400">{error}</p>}
    </div>
  );
}
