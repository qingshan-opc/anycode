import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { settingsClient } from "@/api/client/settings";
import { SectionCard } from "@/components/ui/SectionCard";
import { useT } from "@/i18n/context";

export function SecuritySandboxPanel() {
  const t = useT();
  const queryClient = useQueryClient();
  const security = useQuery({
    queryKey: ["settings", "security"],
    queryFn: settingsClient.securitySettings,
  });

  const patch = useMutation({
    mutationFn: settingsClient.putSecuritySettings,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["settings", "security"] });
    },
  });

  const data = security.data?.security;
  const busy = security.isLoading || patch.isPending;

  return (
    <SectionCard title={t("settings.sandboxMode")}>
      <p className="text-sm text-secondary m-0 mb-3">{t("settings.sandboxModeHint")}</p>
      <dl className="grid grid-cols-[minmax(5rem,auto)_1fr] gap-x-4 gap-y-2 text-sm m-0 mb-4">
        <dt className="text-secondary font-medium m-0">{t("settings.sandboxMode")}</dt>
        <dd className="m-0">
          {data ? (data.sandbox_mode ? t("session.yes") : t("session.no")) : t("common.loading")}
        </dd>
        <dt className="text-secondary font-medium m-0">{t("settings.shippingDefault")}</dt>
        <dd className="m-0 text-secondary">{t("settings.sandboxDefaultOff")}</dd>
      </dl>
      <label className="inline-flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={Boolean(data?.sandbox_mode)}
          disabled={busy || !data}
          onChange={(e) => {
            patch.mutate({ sandbox_mode: e.target.checked });
          }}
        />
        {t("settings.sandboxModeEnable")}
      </label>
      {data && !data.bypass_allowed ? (
        <p className="text-xs text-secondary mt-3 mb-0">{t("settings.desktopBypassBlocked")}</p>
      ) : null}
      {patch.isError ? (
        <p className="text-xs text-error mt-2 mb-0">
          {patch.error instanceof Error ? patch.error.message : String(patch.error)}
        </p>
      ) : null}
    </SectionCard>
  );
}
