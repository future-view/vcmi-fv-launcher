import { useI18n } from "@/i18n";
import brandIcon from "@/assets/brand-icon.png";

/**
 * 应用品牌块（logo + 名称）：标题栏的**左半部分**。
 *
 * 抽成共享组件的原因：「初始化向导」与主界面（模组/百科/设置共用 `TopBar`）要求
 * 标题栏视觉统一 —— 品牌块只此一处定义，避免两份 markup 各自漂移。
 * 拖拽区与窗口控件由各标题栏自行组合（主界面带页签，首启只有品牌 + 三键）。
 */
export function AppBrand() {
  const { t } = useI18n();
  return (
    <>
      <div
        className="h-[34px] w-[34px] overflow-hidden rounded-[10px] shadow-lg shadow-blue-600/30"
        onContextMenu={(e) => e.preventDefault()}
      >
        <img
          src={brandIcon}
          alt="VCMI"
          className="h-full w-full object-cover"
          draggable={false}
        />
      </div>
      <span className="text-lg font-bold tracking-wide">{t.app.brand}</span>
    </>
  );
}
