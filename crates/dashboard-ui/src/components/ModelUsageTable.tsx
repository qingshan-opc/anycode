import type { ModelUsageRow } from "@/api/types";
import { useT } from "@/i18n/context";
import { formatMoney } from "@/lib/money";

export function ModelUsageTable({ rows }: { rows: ModelUsageRow[] }) {
  const t = useT();
  const visibleRows = rows.filter((row) => {
    const m = row.model.trim().toLowerCase();
    return m !== "mock" && !m.startsWith("mock/");
  });
  if (visibleRows.length === 0) return null;
  const showCache = visibleRows.some((row) => (row.cache_read_tokens ?? 0) > 0);

  return (
    <div className="mt-4 overflow-x-auto rounded-lg border border-outline-variant/35">
      <div className="text-xs font-medium text-secondary px-3 py-2 bg-surface-container-low border-b border-outline-variant/30">
        {t("home.modelBreakdown")}
      </div>
      <table className="w-full text-xs border-collapse">
        <thead>
          <tr className="text-left text-secondary border-b border-outline/30 bg-surface-container-lowest">
            <th className="py-2 px-3 font-medium">{t("home.modelProvider")}</th>
            <th className="py-2 px-3 font-medium">{t("home.modelName")}</th>
            <th className="py-2 px-3 font-medium text-right">{t("home.tokenCalls")}</th>
            <th className="py-2 px-3 font-medium text-right">{t("home.tokenTotal")}</th>
            {showCache && (
              <th className="py-2 px-3 font-medium text-right">{t("home.tokenCacheHitShort")}</th>
            )}
            <th className="py-2 px-3 font-medium text-right">{t("home.tokenCost")}</th>
          </tr>
        </thead>
        <tbody>
          {visibleRows.map((row) => {
            const cacheRead = row.cache_read_tokens ?? 0;
            const hitRate =
              cacheRead > 0 && row.input_tokens > 0
                ? Math.min(100, (cacheRead / row.input_tokens) * 100)
                : null;
            return (
              <tr key={`${row.provider}:${row.model}`} className="border-b border-outline/15">
                <td className="py-2 px-3 text-secondary">{row.provider}</td>
                <td className="py-2 px-3 font-mono">{row.model}</td>
                <td className="py-2 px-3 text-right tabular-nums">{row.llm_calls}</td>
                <td className="py-2 px-3 text-right tabular-nums">{formatTokens(row.total_tokens)}</td>
                {showCache && (
                  <td className="py-2 px-3 text-right tabular-nums">
                    {hitRate !== null ? `${hitRate.toFixed(0)}%` : "—"}
                  </td>
                )}
                <td className="py-2 px-3 text-right tabular-nums">
                  {formatMoney(row.estimated_cost_cny)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}
