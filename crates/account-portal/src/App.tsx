import { Navigate, Route, Routes, useLocation, useSearchParams } from "react-router-dom";
import { ConsoleLayout } from "./components/ConsoleLayout";
import { SiteFooter } from "./components/SiteFooter";
import { TopNav } from "./components/TopNav";
import { AuthProvider, useAuth } from "./hooks/useAuth";
import { I18nProvider } from "./i18n/context";
import { ApiKeysPage } from "./pages/ApiKeysPage";
import { BillingPage } from "./pages/BillingPage";
import { CaseDetailPage } from "./pages/CaseDetailPage";
import { ChangelogPage } from "./pages/ChangelogPage";
import { DesignLabPage } from "./pages/DesignLabPage";
import { DownloadsPage } from "./pages/DownloadsPage";
import { FeaturesPage } from "./pages/FeaturesPage";
import { HomePage } from "./pages/HomePage";
import { HomePageNxBackup } from "./pages/HomePageNxBackup";
import { HomePageOrbit } from "./pages/HomePageOrbit";
import { LoginPage } from "./pages/LoginPage";
import { MarketingPlansPage } from "./pages/MarketingPlansPage";
import { OverviewPage } from "./pages/OverviewPage";
import { PlansPage } from "./pages/PlansPage";
import { ProductPage } from "./pages/ProductPage";
import { RegisterPage } from "./pages/RegisterPage";
import { SettingsPage } from "./pages/SettingsPage";
import { TeamPage } from "./pages/TeamPage";
import { JoinTeamPage } from "./pages/JoinTeamPage";
import { UsagePage } from "./pages/UsagePage";
import { AlgorithmDisclosurePage } from "./pages/legal/AlgorithmDisclosurePage";
import { PrivacyPolicyPage } from "./pages/legal/PrivacyPolicyPage";
import { UserAgreementPage } from "./pages/legal/UserAgreementPage";
import { DocsRoutes } from "./pages/DocsRoutes";
import { SITE_PATHS } from "@anycode/site-urls";
import { MobileApp, RequireMobileAuth } from "./pages/mobile/MobileApp";

function RequireAuth({ children }: { children: React.ReactNode }) {
  const { authenticated, validating } = useAuth();
  const loc = useLocation();
  if (validating) {
    return <p className="muted console-meta">Loading…</p>;
  }
  if (!authenticated) {
    const from = loc.pathname + loc.search;
    const next =
      from.startsWith("/console") && !from.startsWith("//") ? from : "/console";
    // WeChat SSO hop — do not land on email form or 818 console.
    if (typeof window !== "undefined") {
      window.location.replace(
        `/api/auth/hop/login?next=${encodeURIComponent(next)}`,
      );
    }
    return <p className="muted console-meta">Redirecting…</p>;
  }
  return <>{children}</>;
}

function LegacyDeviceLinkRedirect() {
  const [params] = useSearchParams();
  const code = params.get("code");
  const suffix = code ? `?code=${encodeURIComponent(code)}` : "";
  return <Navigate to={`/console/settings${suffix}`} replace />;
}

function AppRoutes() {
  const { pathname } = useLocation();
  const isHome =
    pathname === "/" ||
    pathname === "/home-nx" ||
    pathname === "/home-classic";
  const isConsole = pathname.startsWith("/console");
  const isDocs = pathname.startsWith("/docs");
  const isDesignLab = import.meta.env.DEV && pathname === "/__design-prototype";
  const isMarketing = !isConsole;
  const isLegacyHome = pathname === "/home-nx" || pathname === "/home-classic";
  const hideSiteFooter = pathname === SITE_PATHS.downloads;
  const isMobile = pathname === SITE_PATHS.mobile || pathname.startsWith(`${SITE_PATHS.mobile}/`);
  const isOrbitSite = !isDesignLab && !isLegacyHome && !isMobile;

  return (
    <div
      className={`app app--nx${isHome ? " app--home" : ""}${isOrbitSite ? " app--orbit-site" : ""}${isDocs ? " app--docs" : ""}${isDesignLab ? " app--design-lab" : ""}${isMobile ? " app--mobile" : ""}`}
    >
      {!isDesignLab && !isMobile ? <TopNav /> : null}
      <main
        className={`main${isHome ? " main--landing" : ""}${isConsole ? " main--console" : ""}${isDocs ? " main--docs" : ""}${isMobile ? " main--mobile" : ""}${isMarketing && !isHome && !isDocs && !isDesignLab && !isMobile ? " main--site" : ""}`}
      >
        <Routes>
          <Route path="/" element={<HomePageOrbit />} />
          {import.meta.env.DEV ? (
            <Route path="/__design-prototype" element={<DesignLabPage />} />
          ) : null}
          <Route path="/home-nx" element={<HomePageNxBackup />} />
          <Route path={SITE_PATHS.features} element={<FeaturesPage />} />
          <Route path={SITE_PATHS.product} element={<ProductPage />} />
          <Route path={SITE_PATHS.plans} element={<MarketingPlansPage />} />
          <Route path={SITE_PATHS.downloads} element={<DownloadsPage />} />
          <Route path={SITE_PATHS.changelog} element={<ChangelogPage />} />
          <Route path="/cases/:caseId" element={<CaseDetailPage />} />
          <Route path="/home-classic" element={<HomePage />} />
          <Route
            path={`${SITE_PATHS.mobile}/*`}
            element={
              <RequireMobileAuth>
                <MobileApp />
              </RequireMobileAuth>
            }
          />
          <Route
            path={SITE_PATHS.mobile}
            element={
              <RequireMobileAuth>
                <MobileApp />
              </RequireMobileAuth>
            }
          />
          <Route path={SITE_PATHS.login} element={<LoginPage />} />
          <Route path={SITE_PATHS.register} element={<RegisterPage />} />
          <Route path="/join" element={<JoinTeamPage />} />
          <Route path={SITE_PATHS.legalAlgorithmDisclosure} element={<AlgorithmDisclosurePage />} />
          <Route path={SITE_PATHS.legalUserAgreement} element={<UserAgreementPage />} />
          <Route path={SITE_PATHS.legalPrivacy} element={<PrivacyPolicyPage />} />
          <Route path="/docs/*" element={<DocsRoutes />} />

          <Route
            path="/console"
            element={
              <RequireAuth>
                <ConsoleLayout />
              </RequireAuth>
            }
          >
            <Route index element={<OverviewPage />} />
            <Route path="plans" element={<PlansPage />} />
            <Route path="usage" element={<UsagePage />} />
            <Route path="billing" element={<BillingPage />} />
            <Route path="api" element={<ApiKeysPage />} />
            <Route path="team" element={<TeamPage />} />
            <Route path="settings" element={<SettingsPage />} />
          </Route>

          <Route path="/models" element={<Navigate to="/console/usage" replace />} />
          <Route path="/billing" element={<Navigate to="/console/billing" replace />} />
          <Route path="/devices" element={<Navigate to="/console/settings" replace />} />
          <Route path="/devices/link" element={<LegacyDeviceLinkRedirect />} />
        </Routes>
      </main>
      {isMarketing && !isHome && !isDesignLab && !hideSiteFooter && !isMobile ? (
        <SiteFooter />
      ) : null}
    </div>
  );
}

export function App() {
  return (
    <I18nProvider>
      <AuthProvider>
        <AppRoutes />
      </AuthProvider>
    </I18nProvider>
  );
}
