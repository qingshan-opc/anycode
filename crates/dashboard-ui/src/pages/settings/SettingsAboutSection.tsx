import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { ExternalNavLink } from "@/components/ExternalNavLink";
import { useT } from "@/i18n/context";
import { compareSemver, fetchDesktopLatest } from "@/lib/desktopVersion";
import { isTauriDesktop } from "@/lib/desktopShell";
import {
  checkForDesktopUpdate,
  downloadAndInstallDesktopUpdate,
  relaunchDesktopApp,
} from "@/lib/desktopUpdater";
import { legalUrls, SITE_ORIGIN } from "@anycode/site-urls";

type DesktopUpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "available"; version: string }
  | { kind: "upToDate" }
  | { kind: "downloading"; percent: number | null }
  | { kind: "ready" }
  | { kind: "error"; message: string };

export function SettingsAboutSection() {
  const t = useT();
  const desktop = isTauriDesktop();

  const health = useQuery({
    queryKey: ["health"],
    queryFn: api.health,
    staleTime: 60_000,
  });

  const localVersion = health.data?.version ?? "—";
  const portalOrigin =
    health.data?.account_portal_url?.replace(/\/$/, "") || SITE_ORIGIN;

  // Browser fallback: portal manifest + external download link. In the Tauri
  // desktop shell the in-place updater flow below takes over.
  const latest = useQuery({
    queryKey: ["desktop-latest", portalOrigin],
    queryFn: () => fetchDesktopLatest(portalOrigin),
    enabled: !desktop && Boolean(health.data),
    staleTime: 5 * 60_000,
    retry: 1,
  });

  const [desk, setDesk] = useState<DesktopUpdateState>({ kind: "idle" });
  const totalRef = useRef<number | undefined>(undefined);

  const runDesktopCheck = async () => {
    setDesk({ kind: "checking" });
    const update = await checkForDesktopUpdate();
    setDesk(
      update
        ? { kind: "available", version: update.version }
        : { kind: "upToDate" },
    );
  };

  const runDesktopInstall = async () => {
    totalRef.current = undefined;
    setDesk({ kind: "downloading", percent: null });
    try {
      await downloadAndInstallDesktopUpdate((downloaded, total) => {
        if (total !== undefined) totalRef.current = total;
        const knownTotal = totalRef.current;
        setDesk({
          kind: "downloading",
          percent:
            knownTotal && knownTotal > 0
              ? Math.min(100, Math.round((downloaded / knownTotal) * 100))
              : null,
        });
      });
      setDesk({ kind: "ready" });
    } catch (err) {
      setDesk({
        kind: "error",
        message: err instanceof Error ? err.message : String(err),
      });
    }
  };

  const updateAvailable =
    !desktop &&
    latest.data?.version &&
    localVersion !== "—" &&
    compareSemver(latest.data.version, localVersion) > 0;

  const downloadUrl =
    latest.data?.latest_url ||
    latest.data?.url ||
    `${portalOrigin}/downloads/anyCode_latest_aarch64.dmg`;

  const checkBusy = desktop ? desk.kind === "checking" : latest.isFetching;

  return (
    <section className="dw-settings-section space-y-4">
      <div>
        <h2 className="text-base font-semibold m-0 text-on-surface">{t("settings.about.title")}</h2>
        <p className="text-sm text-secondary mt-1 mb-0">{t("settings.about.subtitle")}</p>
      </div>

      <div className="rounded-xl border border-outline-variant bg-surface-container-low p-4 space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div>
            <div className="text-xs font-semibold uppercase tracking-wide text-secondary">
              {t("settings.about.versionLabel")}
            </div>
            <p className="m-0 mt-1 text-sm font-code">{localVersion}</p>
          </div>
          <button
            type="button"
            className="dw-btn-secondary text-xs"
            disabled={checkBusy}
            onClick={() =>
              desktop ? void runDesktopCheck() : void latest.refetch()
            }
          >
            {checkBusy
              ? t("settings.about.checkingUpdate")
              : t("settings.about.checkUpdate")}
          </button>
        </div>

        {!desktop && latest.isError && (
          <p className="m-0 text-sm text-error">
            {(latest.error as Error).message || t("settings.about.updateCheckFailed")}
          </p>
        )}

        {desktop ? (
          <>
            {desk.kind === "available" && (
              <div className="rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 space-y-2">
                <p className="m-0 text-sm">
                  {t("settings.about.updateAvailable").replace(
                    "{version}",
                    desk.version,
                  )}
                </p>
                <button
                  type="button"
                  className="dw-btn-primary text-xs"
                  onClick={() => void runDesktopInstall()}
                >
                  {t("settings.about.installUpdate")}
                </button>
              </div>
            )}
            {desk.kind === "downloading" && (
              <p className="m-0 text-sm text-secondary">
                {desk.percent !== null
                  ? t("settings.about.updateDownloading").replace(
                      "{percent}",
                      String(desk.percent),
                    )
                  : t("settings.about.updateDownloading").replace(
                      "{percent}",
                      "…",
                    )}
              </p>
            )}
            {desk.kind === "ready" && (
              <div className="rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 space-y-2">
                <p className="m-0 text-sm">{t("settings.about.updateReady")}</p>
                <button
                  type="button"
                  className="dw-btn-primary text-xs"
                  onClick={() => void relaunchDesktopApp()}
                >
                  {t("settings.about.relaunchNow")}
                </button>
              </div>
            )}
            {desk.kind === "error" && (
              <p className="m-0 text-sm text-error">
                {t("settings.about.updateFailed").replace(
                  "{message}",
                  desk.message,
                )}
              </p>
            )}
            {desk.kind === "upToDate" && (
              <p className="m-0 text-sm text-secondary">
                {t("settings.about.upToDate")}
              </p>
            )}
          </>
        ) : updateAvailable ? (
          <div className="rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 space-y-2">
            <p className="m-0 text-sm">
              {t("settings.about.updateAvailable").replace(
                "{version}",
                latest.data!.version,
              )}
            </p>
            <a
              href={downloadUrl}
              className="dw-btn-primary text-xs no-underline inline-flex"
              target="_blank"
              rel="noreferrer"
            >
              {t("settings.about.downloadUpdate")}
            </a>
          </div>
        ) : latest.isSuccess && !latest.isFetching ? (
          <p className="m-0 text-sm text-secondary">{t("settings.about.upToDate")}</p>
        ) : null}
      </div>

      <div className="rounded-xl border border-outline-variant bg-surface-container-low p-4 space-y-3">
        <div>
          <div className="text-xs font-semibold uppercase tracking-wide text-secondary">
            {t("settings.about.algorithmLabel")}
          </div>
          <p className="m-0 mt-1 text-sm">{t("settings.about.algorithmName")}</p>
        </div>
        <div>
          <div className="text-xs font-semibold uppercase tracking-wide text-secondary">
            {t("settings.about.providerLabel")}
          </div>
          <p className="m-0 mt-1 text-sm">{t("settings.about.providerName")}</p>
        </div>
        <p className="m-0 text-sm text-secondary">{t("settings.about.aiNotice")}</p>
      </div>

      <div className="flex flex-wrap gap-2">
        <ExternalNavLink href={legalUrls.userAgreement()} className="dw-btn-secondary no-underline">
          {t("settings.about.termsLink")}
        </ExternalNavLink>
        <ExternalNavLink href={legalUrls.privacy()} className="dw-btn-secondary no-underline">
          {t("settings.about.privacyLink")}
        </ExternalNavLink>
      </div>
    </section>
  );
}
