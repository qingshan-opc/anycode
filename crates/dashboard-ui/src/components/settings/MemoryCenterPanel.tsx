import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import { SectionCard } from "@/components/ui/SectionCard";
import { useT } from "@/i18n/context";

type MemoryCenterSnapshot = {
  backend?: string;
  sync_mode?: string;
  preferences?: unknown[];
  project_facts?: unknown[];
  feedback?: unknown[];
  pending_episodes?: number;
  dream_preview?: {
    promotions?: number;
    conflicts?: unknown[];
    skipped_secrets?: number;
    duplicates_merged?: number;
  };
  lightrag?: { url?: string; reachable?: boolean };
  e2ee?: { mode_label?: string; sync_enabled?: boolean };
};

const MEMORY_BACKENDS = ["file", "hybrid", "pipeline", "lightrag"] as const;

function Pill({
  label,
  tone = "neutral",
}: {
  label: string;
  tone?: "ok" | "warn" | "neutral";
}) {
  const cls =
    tone === "ok"
      ? "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300"
      : tone === "warn"
        ? "bg-amber-500/15 text-amber-800 dark:text-amber-200"
        : "bg-black/5 text-secondary dark:bg-white/10";
  return (
    <span className={`inline-flex items-center rounded px-2 py-0.5 text-xs font-medium ${cls}`}>
      {label}
    </span>
  );
}

export function MemoryCenterPanel() {
  const t = useT();
  const qc = useQueryClient();
  const center = useQuery({
    queryKey: ["memory-center"],
    queryFn: () => api.memoryCenter(),
    staleTime: 15_000,
  });

  const patchBackend = useMutation({
    mutationFn: (backend: string) => api.patchMemoryBackend(backend),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ["memory-center"] });
    },
  });

  const data = center.data as MemoryCenterSnapshot | undefined;
  const currentBackend = data?.backend ?? "file";
  const prefs = data?.preferences?.length ?? 0;
  const facts = data?.project_facts?.length ?? 0;
  const feedback = data?.feedback?.length ?? 0;
  const episodes = data?.pending_episodes ?? 0;
  const conflicts = data?.dream_preview?.conflicts?.length ?? 0;

  return (
    <SectionCard title={t("settings.memoryCenter")}>
      <p className="text-sm text-secondary m-0 mb-3">{t("settings.memoryCenterHint")}</p>
      {center.isLoading && <p className="text-sm text-secondary m-0">…</p>}
      {center.error && (
        <p className="text-sm text-error m-0 mb-2">
          {(center.error as Error).message || t("settings.memoryCenterError")}
        </p>
      )}
      {data && (
        <>
          <label className="flex flex-col gap-1 text-sm mb-3 max-w-xs">
            <span className="text-secondary font-medium">{t("settings.memoryBackend")}</span>
            <select
              className="dw-input text-sm"
              value={MEMORY_BACKENDS.includes(currentBackend as (typeof MEMORY_BACKENDS)[number])
                ? currentBackend
                : "file"}
              disabled={patchBackend.isPending}
              onChange={(e) => patchBackend.mutate(e.target.value)}
            >
              {MEMORY_BACKENDS.map((b) => (
                <option key={b} value={b}>
                  {b}
                </option>
              ))}
            </select>
          </label>
          {patchBackend.data?.restart_hint && (
            <p className="text-xs text-secondary m-0 mb-2">{patchBackend.data.restart_hint}</p>
          )}
          {patchBackend.data?.error && (
            <p className="text-sm text-error m-0 mb-2">{patchBackend.data.error}</p>
          )}
          <div className="flex flex-wrap gap-2 mb-3">
            <Pill label={`backend: ${data.backend ?? "?"}`} />
            <Pill
              label={`sync: ${data.sync_mode ?? data.e2ee?.mode_label ?? "local_only"}`}
              tone={data.e2ee?.sync_enabled ? "ok" : "neutral"}
            />
            <Pill
              label={
                data.lightrag?.reachable
                  ? t("settings.memoryCenterLightragUp")
                  : t("settings.memoryCenterLightragDown")
              }
              tone={data.lightrag?.reachable ? "ok" : "warn"}
            />
            <Pill label={`prefs ${prefs}`} />
            <Pill label={`facts ${facts}`} />
            <Pill label={`feedback ${feedback}`} />
            <Pill label={`episodes ${episodes}`} />
            {conflicts > 0 && <Pill label={`conflicts ${conflicts}`} tone="warn" />}
          </div>
          {data.lightrag?.url && (
            <p className="text-xs text-secondary m-0 mb-2 font-code break-all">
              LightRAG: {data.lightrag.url}
            </p>
          )}
          <details className="text-xs">
            <summary className="cursor-pointer text-secondary mb-1">
              {t("settings.memoryCenterRaw")}
            </summary>
            <pre className="m-0 p-2 overflow-auto max-h-64 rounded bg-black/5 dark:bg-white/5 text-[11px] leading-snug">
              {JSON.stringify(data, null, 2)}
            </pre>
          </details>
        </>
      )}
      <button
        type="button"
        className="dw-btn-secondary text-sm mt-3"
        disabled={center.isFetching}
        onClick={() => void center.refetch()}
      >
        {center.isFetching ? "…" : t("settings.memoryCenterRefresh")}
      </button>
    </SectionCard>
  );
}
