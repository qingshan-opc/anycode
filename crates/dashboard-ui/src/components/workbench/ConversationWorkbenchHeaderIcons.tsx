import type { WorkbenchTab } from "@/api/types/workbench";
import type { ConversationTabId } from "./hooks/useWorkbenchSidebarState";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";
import { WORKBENCH_PANELS_ORDERED } from "./registry";

type Props = {
  activeTab: WorkbenchTab;
  expanded: boolean;
  onSelectTab: (tab: WorkbenchTab) => void;
  disabled?: boolean;
  /** Unread counts per tab (rendered as a badge dot); omitted/zero = hidden. */
  badges?: Partial<Record<WorkbenchTab, number>>;
  /** Panels living as conversation-area tabs (their active signal differs). */
  tabbedPanels?: WorkbenchTab[];
  conversationTab?: ConversationTabId;
};

export function ConversationWorkbenchHeaderIcons({
  activeTab,
  expanded,
  onSelectTab,
  disabled,
  badges,
  tabbedPanels,
  conversationTab,
}: Props) {
  const t = useT();

  return (
    <div className="conv-workbench-header-icons" role="toolbar" aria-label={t("workbench.title")}>
      {WORKBENCH_PANELS_ORDERED.map((tab) => {
        const tabbed = tabbedPanels?.includes(tab.id) ?? false;
        const active = tabbed ? conversationTab === tab.id : expanded && activeTab === tab.id;
        const badge = badges?.[tab.id] ?? 0;
        return (
          <button
            key={tab.id}
            type="button"
            title={t(tab.titleKey)}
            disabled={disabled}
            aria-pressed={active}
            className={`conv-workbench-header-icons__btn relative${active ? " conv-workbench-header-icons__btn--active" : ""}`}
            onClick={() => onSelectTab(tab.id)}
          >
            <Icon name={tab.icon} size={18} />
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
    </div>
  );
}
