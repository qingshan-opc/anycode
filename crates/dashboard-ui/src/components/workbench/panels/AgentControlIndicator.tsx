import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";

type Props = {
  /** User clicked the pill — hand browser control back to the user. */
  onTakeControl: () => void;
};

/**
 * Floating pill shown while the agent holds the browser lock. Renders as an
 * overlay at the bottom-center of the browser viewport; in CEF-embed mode the
 * host surface is shrunk to make room for it (React cannot sit above the
 * native view), in screenshot mode it floats directly over the stage.
 */
export function AgentControlIndicator({ onTakeControl }: Props) {
  const t = useT();
  return (
    <button
      type="button"
      onClick={onTakeControl}
      className="absolute bottom-3 left-1/2 -translate-x-1/2 z-20 inline-flex items-center gap-2 rounded-full border border-primary/40 bg-surface-container-high/95 px-3.5 py-1.5 text-xs text-on-surface shadow-lg backdrop-blur-sm transition-colors hover:bg-surface-container-high"
      aria-label={`${t("workbench.browserLockAgent")} · ${t("workbench.browserUnlock")}`}
    >
      <span className="w-2 h-2 rounded-full bg-primary animate-pulse shrink-0" />
      <Icon name="smart_toy" size={14} className="text-primary shrink-0" />
      <span className="whitespace-nowrap">{t("workbench.browserLockAgent")}</span>
      <span className="whitespace-nowrap text-primary font-semibold">
        {t("workbench.browserUnlock")}
      </span>
    </button>
  );
}
