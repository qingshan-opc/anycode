import { useEffect, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import { api } from "../api";
import { AuthLayout } from "../components/AuthLayout";
import { useAuth } from "../hooks/useAuth";
import { useT } from "../i18n/context";
import {
  openAnycodeDeepLink,
  resolveDeviceRedirectUri,
} from "../lib/deviceLink";
import { SITE_PATHS } from "@anycode/site-urls";
import { safeAppNext } from "../lib/safeNext";

function hopLoginUrl(next: string): string {
  return `/api/auth/hop/login?next=${encodeURIComponent(next)}`;
}

export function LoginPage() {
  const nav = useNavigate();
  const loc = useLocation();
  const { setToken, authenticated } = useAuth();
  const t = useT();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [showEmailLogin, setShowEmailLogin] = useState(false);
  const [handoffLink, setHandoffLink] = useState<string | null>(null);
  const [handoffNote, setHandoffNote] = useState<string | null>(null);
  const [handoffDone, setHandoffDone] = useState(false);
  const [claimingLx, setClaimingLx] = useState(false);

  const search = new URLSearchParams(loc.search);
  const deviceCode = search.get("device_code");
  const redirectUri = search.get("redirect_uri");
  const lxToken = search.get("lx_token");
  const nextFromQuery = search.get("next");

  const goConsole = () => {
    nav("/console", { replace: true });
  };

  const wechatNext = (): string => {
    if (deviceCode) {
      return `/console/settings?code=${encodeURIComponent(deviceCode)}`;
    }
    const from = (loc.state as { from?: string } | null)?.from;
    return safeAppNext(from || nextFromQuery);
  };

  const finishDeviceHandoff = async (code: string, autoOpen: boolean) => {
    const deepLink = resolveDeviceRedirectUri(code, redirectUri);
    setHandoffLink(deepLink);
    let approved = false;
    try {
      await api.approveDeviceLink(code);
      approved = true;
      setHandoffNote(t("devices.approved"));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      // Already linked / identity gate → console settings handles the rest.
      if (/identity|实名|verify|already|approved|关联/i.test(message)) {
        nav(`/console/settings?code=${encodeURIComponent(code)}`, { replace: true });
        return;
      }
      setHandoffNote(message);
      setHandoffDone(true);
      return;
    }
    if (autoOpen) {
      // Don't replace this tab — portal should continue to the console.
      openAnycodeDeepLink(deepLink, { replacePage: false });
    }
    if (approved) {
      setHandoffDone(true);
      window.setTimeout(() => {
        goConsole();
      }, 600);
    }
  };

  useEffect(() => {
    if (!lxToken || claimingLx) return;
    setClaimingLx(true);
    setToken(lxToken);
    const dest = safeAppNext(nextFromQuery);
    // Drop lx_token from the address bar after claiming.
    nav(dest, { replace: true });
  }, [lxToken, claimingLx, setToken, nextFromQuery, nav]);

  useEffect(() => {
    if (!authenticated || !deviceCode || handoffLink) return;
    void finishDeviceHandoff(deviceCode, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one-shot handoff when session already present
  }, [authenticated, deviceCode, handoffLink]);

  useEffect(() => {
    if (!authenticated || deviceCode || lxToken) return;
    const from = (loc.state as { from?: string } | null)?.from;
    nav(safeAppNext(from || nextFromQuery), { replace: true });
  }, [authenticated, deviceCode, lxToken, loc.state, nav, nextFromQuery]);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      const res = await api.login({ email, password });
      setToken(res.token);
      if (deviceCode) {
        await finishDeviceHandoff(deviceCode, true);
        return;
      }
      const from = (loc.state as { from?: string } | null)?.from;
      nav(safeAppNext(from || nextFromQuery));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setPending(false);
    }
  };

  if (lxToken || claimingLx) {
    return (
      <AuthLayout title={t("auth.loginTitle")} subtitle={t("common.loading")}>
        <p className="muted">{t("auth.wechatFinishing")}</p>
      </AuthLayout>
    );
  }

  if (authenticated && deviceCode) {
    return (
      <AuthLayout title={t("auth.loginTitle")} subtitle={t("devices.redirecting")}>
        <div className="device-link-panel">
          <p className="muted">{handoffNote ?? t("devices.openingDesktop")}</p>
          <p className="muted">{t("devices.clickDeepLink")}</p>
          <button
            type="button"
            className="btn btn-primary btn-block"
            onClick={goConsole}
          >
            {t("devices.enterConsole")}
          </button>
          {handoffLink ? (
            <a
              className="btn btn-block"
              href={handoffLink}
              style={{ marginTop: "0.5rem" }}
              onClick={(e) => {
                e.preventDefault();
                openAnycodeDeepLink(handoffLink, { replacePage: true });
              }}
            >
              {t("devices.openDeepLink")}
            </a>
          ) : (
            <p className="muted">{t("common.loading")}</p>
          )}
          {handoffDone ? (
            <p className="muted auth-switch">{t("devices.redirectingConsole")}</p>
          ) : null}
          <p className="auth-switch muted">
            <Link to={`/console/settings?code=${encodeURIComponent(deviceCode)}`}>
              {t("console.settings")}
            </Link>
          </p>
        </div>
      </AuthLayout>
    );
  }

  if (authenticated) {
    return (
      <AuthLayout title={t("auth.loginTitle")} subtitle={t("common.loading")}>
        <p className="muted">{t("common.loading")}</p>
      </AuthLayout>
    );
  }

  const registerTo = deviceCode
    ? `${SITE_PATHS.register}?device_code=${encodeURIComponent(deviceCode)}${
        redirectUri ? `&redirect_uri=${encodeURIComponent(redirectUri)}` : ""
      }`
    : SITE_PATHS.register;

  return (
    <AuthLayout title={t("auth.loginTitle")} subtitle={t("auth.loginSubtitle")}>
      {deviceCode ? (
        <p className="auth-filing-notice" role="status">
          {t("devices.loginHandoffHint")}
        </p>
      ) : null}
      <p className="auth-consent-note muted">{t("auth.loginAlgorithmNotice")}</p>
      <a className="btn btn-primary btn-block auth-submit" href={hopLoginUrl(wechatNext())}>
        {t("auth.wechatLogin")}
      </a>
      <p className="muted auth-switch" style={{ marginTop: "1rem" }}>
        <button
          type="button"
          className="linkish"
          style={{
            background: "none",
            border: "none",
            padding: 0,
            color: "inherit",
            textDecoration: "underline",
            cursor: "pointer",
          }}
          onClick={() => setShowEmailLogin((v) => !v)}
        >
          {showEmailLogin ? t("auth.hideEmailLogin") : t("auth.emailLoginFallback")}
        </button>
      </p>
      {showEmailLogin ? (
        <form className="auth-form" onSubmit={submit} style={{ marginTop: "1rem" }}>
          <div className="field-group">
            <label className="field-label" htmlFor="login-email">
              {t("auth.email")}
            </label>
            <input
              id="login-email"
              className="auth-input"
              placeholder="you@example.com"
              type="email"
              autoComplete="username"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              required
            />
          </div>
          <div className="field-group">
            <label className="field-label" htmlFor="login-password">
              {t("auth.password")}
            </label>
            <input
              id="login-password"
              className="auth-input"
              placeholder="••••••••"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
            />
          </div>
          {error && (
            <p className="form-error" role="alert">
              {/invalid credentials/i.test(error)
                ? t("auth.invalidCredentials")
                : error}
            </p>
          )}
          <button className="btn btn-primary btn-block auth-submit" type="submit" disabled={pending}>
            {pending
              ? t("auth.signingIn")
              : deviceCode
                ? t("devices.loginAndOpenDesktop")
                : t("auth.loginTitle")}
          </button>
        </form>
      ) : null}
      {deviceCode ? (
        <div className="device-link-panel" style={{ marginTop: "1rem" }}>
          <a
            className="btn btn-block"
            href={resolveDeviceRedirectUri(deviceCode, redirectUri)}
            onClick={(e) => {
              e.preventDefault();
              openAnycodeDeepLink(resolveDeviceRedirectUri(deviceCode, redirectUri), {
                replacePage: true,
              });
            }}
          >
            {t("devices.openDeepLink")}
          </a>
          <p className="muted" style={{ marginTop: "0.5rem" }}>
            {t("devices.openDeepLinkBeforeLogin")}
          </p>
        </div>
      ) : null}
      <p className="muted auth-switch">
        {t("auth.noAccount")}{" "}
        <Link to={registerTo}>{t("auth.registerLink")}</Link>
      </p>
    </AuthLayout>
  );
}
