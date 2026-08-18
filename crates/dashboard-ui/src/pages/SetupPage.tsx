import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { settingsClient } from "@/api/client/settings";
import { setupClient } from "@/api/client/setup";
import type { QuickAuthPreset } from "@/api/types/setup";
import { useT } from "@/i18n/context";

type WizardStep = "welcome" | "model" | "done";

/**
 * 首次上手引导（/setup）：欢迎 → 模型（BYOK 保存+连通性测试）→ 完成。
 * 后端 `crates/setup/` + `/api/setup/*` 早已就绪，本页是其前端入口；
 * 完成时先 `workspace/ensure` 再 `complete`（写入 `setup_completed_at`）。
 * i18n 复用既有 `setup.*` 向导键与 `settings.model.*` 表单标签。
 */
export function SetupPage() {
  const t = useT();
  const navigate = useNavigate();
  const status = useQuery({
    queryKey: ["setup", "status"],
    queryFn: setupClient.setupStatus,
  });
  const presets = useQuery({
    queryKey: ["setup", "quick-auth"],
    queryFn: setupClient.setupQuickAuth,
  });

  const steps = status.data?.setup.steps ?? [];
  const modelAlreadyConfigured = useMemo(
    () => steps.some((s) => s.id === "llm" && s.complete),
    [steps],
  );

  const [step, setStep] = useState<WizardStep>("welcome");
  const [presetId, setPresetId] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [model, setModel] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");
  const [testState, setTestState] = useState<"idle" | "testing" | "ok" | "fail">(
    "idle",
  );
  const [testError, setTestError] = useState("");
  const [finishError, setFinishError] = useState("");
  const [finishing, setFinishing] = useState(false);

  const presetList: QuickAuthPreset[] = presets.data?.presets ?? [];
  const selectedPreset = presetList.find((p) => p.id === presetId);

  const onPresetChange = (id: string) => {
    setPresetId(id);
    const p = presetList.find((x) => x.id === id);
    if (p) {
      setBaseUrl(p.base_url);
      setModel(p.default_model);
    }
    setTestState("idle");
    setSaveError("");
    setTestError("");
  };

  const saveAndTest = async () => {
    if (!selectedPreset || !apiKey.trim()) return;
    setSaving(true);
    setSaveError("");
    setTestError("");
    setTestState("idle");
    try {
      await settingsClient.putLlmConfig({
        provider: selectedPreset.provider,
        plan: selectedPreset.plan,
        base_url: baseUrl.trim() || selectedPreset.base_url,
        model: model.trim() || selectedPreset.default_model,
        api_key: apiKey.trim(),
      });
    } catch (e) {
      setSaving(false);
      setSaveError(e instanceof Error ? e.message : String(e));
      return;
    }
    setSaving(false);
    setTestState("testing");
    try {
      const result = await settingsClient.testLlm("chat");
      if (result.ok) {
        setTestState("ok");
      } else {
        setTestState("fail");
        setTestError(result.error ?? result.message ?? "");
      }
    } catch (e) {
      setTestState("fail");
      setTestError(e instanceof Error ? e.message : String(e));
    }
  };

  const finish = async () => {
    setFinishing(true);
    setFinishError("");
    try {
      await setupClient.setupEnsureWorkspace();
      await setupClient.setupComplete({ scan_projects: true });
      void navigate({ to: "/conversations", replace: true });
    } catch (e) {
      setFinishing(false);
      setFinishError(e instanceof Error ? e.message : String(e));
    }
  };

  const canSave =
    !saving &&
    testState !== "testing" &&
    !!selectedPreset &&
    apiKey.trim().length > 0;
  const canNextFromModel = modelAlreadyConfigured || testState === "ok";

  return (
    <div className="dw-cloud-gate">
      <div className="dw-cloud-gate__glow" aria-hidden />
      <main className="dw-cloud-gate__stage max-w-[560px]">
        <p className="dw-cloud-gate__eyebrow">{t("setup.title")}</p>
        <ol className="flex gap-3 my-4 list-none p-0 text-sm" aria-label={t("setup.progressAria")}>
          <li className={step === "welcome" ? "font-semibold" : "text-secondary"}>
            {step !== "welcome" ? "✓ " : "1. "}
            {t("setup.steps.welcome")}
          </li>
          <li className={step === "model" ? "font-semibold" : "text-secondary"}>
            {step === "done" || canNextFromModel ? "✓ " : "2. "}
            {t("setup.steps.model")}
          </li>
          <li className={step === "done" ? "font-semibold" : "text-secondary"}>
            {"3. "}
            {t("setup.steps.done")}
          </li>
        </ol>

        {step === "welcome" ? (
          <section className="grid gap-3">
            <h1 className="dw-cloud-gate__title">{t("setup.welcome.title")}</h1>
            <p className="dw-cloud-gate__lede">{t("setup.welcome.body")}</p>
            <ul className="text-sm text-secondary grid gap-1 my-2">
              <li>{t("setup.welcome.pointLocal")}</li>
              <li>{t("setup.welcome.pointKey")}</li>
              {status.data?.setup.platform ? (
                <li>
                  {t("setup.welcome.pointPlatform").replace(
                    "{platform}",
                    status.data.setup.platform,
                  )}
                </li>
              ) : null}
            </ul>
            <div className="flex gap-2 mt-2">
              <button
                type="button"
                className="dw-btn-primary"
                onClick={() => setStep("model")}
              >
                {t("setup.start")}
              </button>
              <button
                type="button"
                className="dw-btn-ghost"
                onClick={() => void finish()}
              >
                {t("setup.skipContinue")}
              </button>
            </div>
          </section>
        ) : null}

        {step === "model" ? (
          <section className="grid gap-3">
            <h1 className="dw-cloud-gate__title">{t("setup.model.title")}</h1>
            <p className="dw-cloud-gate__lede">{t("setup.model.hint")}</p>
            {modelAlreadyConfigured ? (
              <p className="dw-cloud-gate__status" role="note">
                {t("setup.test.ok")}
              </p>
            ) : null}
            <label className="grid gap-1 text-sm">
              <span>{t("setup.model.quickPresets")}</span>
              <select
                className="dw-input w-full"
                value={presetId}
                onChange={(e) => onPresetChange(e.target.value)}
              >
                <option value="">{t("settings.model.providerPlaceholder")}</option>
                {presetList.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="grid gap-1 text-sm">
              <span>{t("settings.model.apiKey")}</span>
              <input
                className="dw-input w-full font-code"
                type="password"
                value={apiKey}
                onChange={(e) => {
                  setApiKey(e.target.value);
                  setTestState("idle");
                }}
                placeholder={t("setup.model.apiKeyPlaceholder")}
                autoComplete="off"
              />
            </label>
            <label className="grid gap-1 text-sm">
              <span>{t("settings.model.baseUrl")}</span>
              <input
                className="dw-input w-full font-code"
                value={baseUrl}
                onChange={(e) => {
                  setBaseUrl(e.target.value);
                  setTestState("idle");
                }}
              />
            </label>
            <label className="grid gap-1 text-sm">
              <span>{t("settings.model.model")}</span>
              <input
                className="dw-input w-full font-code"
                value={model}
                onChange={(e) => {
                  setModel(e.target.value);
                  setTestState("idle");
                }}
              />
            </label>

            {saveError ? (
              <p className="dw-cloud-gate__error" role="alert">
                {saveError}
              </p>
            ) : null}
            {testState === "fail" ? (
              <p className="dw-cloud-gate__error" role="alert">
                {t("setup.test.fail")}
                {testError ? `: ${testError}` : ""}
              </p>
            ) : null}
            {testState === "ok" ? (
              <p className="dw-cloud-gate__status" role="status">
                {t("setup.test.ok")}
              </p>
            ) : null}

            <div className="flex gap-2 mt-2">
              <button
                type="button"
                className="dw-btn-ghost"
                onClick={() => setStep("welcome")}
              >
                {t("common.back")}
              </button>
              <button
                type="button"
                className="dw-btn-secondary"
                disabled={!canSave}
                onClick={() => void saveAndTest()}
              >
                {saving || testState === "testing"
                  ? t("setup.test.running")
                  : t("setup.test.run")}
              </button>
              <button
                type="button"
                className="dw-btn-primary"
                disabled={!canNextFromModel}
                onClick={() => setStep("done")}
              >
                {t("common.next")}
              </button>
            </div>
            {!canNextFromModel ? (
              <button
                type="button"
                className="dw-btn-ghost text-sm justify-self-start"
                onClick={() => setStep("done")}
              >
                {t("setup.skipContinue")}
              </button>
            ) : null}
          </section>
        ) : null}

        {step === "done" ? (
          <section className="grid gap-3">
            <h1 className="dw-cloud-gate__title">{t("setup.done.title")}</h1>
            <p className="dw-cloud-gate__lede">{t("setup.done.body")}</p>
            {finishError ? (
              <p className="dw-cloud-gate__error" role="alert">
                {finishError}
              </p>
            ) : null}
            <div className="flex gap-2 mt-2">
              <button
                type="button"
                className="dw-btn-primary"
                disabled={finishing}
                onClick={() => void finish()}
              >
                {t("setup.done.start")}
              </button>
              <button
                type="button"
                className="dw-btn-ghost"
                onClick={() => setStep("model")}
              >
                {t("common.back")}
              </button>
            </div>
          </section>
        ) : null}
      </main>
    </div>
  );
}
