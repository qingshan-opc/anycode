import type { WorkbenchTab } from "@/api/types/workbench";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";
import { WORKBENCH_PANELS_ORDERED } from "./registry";

type Props = {
  activeTab: WorkbenchTab;
  onSelectTab: (tab: WorkbenchTab) => void;
  disabled?: boolean;
  /** Unread counts per tab (rendered as a badge dot); omitted/zero = hidden. */
  badges?: Partial<Record<WorkbenchTab, number>>;
};

export function WorkbenchActivityRail({ activeTab, onSelectTab, disabled, badges }: Props) {
  const t = useT();

  return (
    <aside className="conv-workbench-rail flex flex-col items-center py-2 gap-1 shrink-0 w-12 border-l border-outline-variant bg-surface-container-low">
      {WORKBENCH_PANELS_ORDERED.map((tab) => {
        const active = activeTab === tab.id;
        const badge = badges?.[tab.id] ?? 0;
        return (
          <button
            key={tab.id}
            type="button"
            title={t(tab.titleKey)}
            disabled={disabled}
            className={`relative p-2 rounded-lg border-0 cursor-pointer transition-colors ${
              active
                ? "bg-primary/15 text-primary"
                : "bg-transparent text-secondary hover:bg-surface-container-high hover:text-on-surface"
            } ${disabled ? "opacity-40 cursor-not-allowed" : ""}`}
            onClick={() => onSelectTab(tab.id)}
          >
            <Icon name={tab.icon} size={20} />
            {badge > 0 && (
              <span
                className="conv-workbench-badge absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] px-0.5 rounded-full bg-primary text-on-primary text-[9px] leading-[14px] text-center font-semibold"
                aria-hidden
              >
                {badge > 99 ? "99+" : badge}
              </span>
            )}
          </button>
        );
      })}
    </aside>
  );
}
