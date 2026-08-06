import { useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { api, type PaymentOrder } from "../api";
import { ConsolePage } from "../components/ConsolePage";
import { WeChatPayModal } from "../components/WeChatPayModal";
import { formatMessage, useT } from "../i18n/context";

type Member = { id: string; email: string; display_name: string; role: string };
type Peer = {
  user_id: string;
  display_name: string;
  email: string;
  device_name: string;
  version: string;
  online: boolean;
  last_seen: string;
};
type TeamStatus = {
  gate: "setup_required" | "invite_required" | "ready";
  organization_name: string;
  member_count: number;
  team_setup: boolean;
  pending_invites: number;
};
type Invite = {
  id: string;
  kind: string;
  email: string | null;
  status: string;
  expires_at: string;
  created_at: string;
};
type SeatInfo = {
  used: number;
  limit: number;
  extraSeats: number;
  extraUntil: string | null;
};

const SEAT_ADDON_UNIT_YUAN = 300;

function buildInviteUrl(acceptPath: string): string {
  return `${window.location.origin}${acceptPath}`;
}

export function TeamPage() {
  const t = useT();
  const [searchParams, setSearchParams] = useSearchParams();
  const inviteToken = searchParams.get("invite")?.trim() ?? "";

  const [members, setMembers] = useState<Member[]>([]);
  const [peers, setPeers] = useState<Peer[]>([]);
  const [team, setTeam] = useState<TeamStatus | null>(null);
  const [invites, setInvites] = useState<Invite[]>([]);
  const [seats, setSeats] = useState<SeatInfo | null>(null);
  const [seatQuantity, setSeatQuantity] = useState(1);
  const [seatOrder, setSeatOrder] = useState<PaymentOrder | null>(null);
  const [seatBusy, setSeatBusy] = useState(false);
  const [teamName, setTeamName] = useState("");
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteLink, setInviteLink] = useState<string | null>(null);
  const [linkBusy, setLinkBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const teamReady = team?.gate === "ready";
  const seatLimitHit = error?.includes("seat limit") ?? false;

  const load = () => {
    setError(null);
    void Promise.all([
      api.orgMembers(),
      api.teamPeers(),
      api.teamStatus(),
      api.listOrgInvites(),
      api.bundle(),
    ])
      .then(([m, p, ts, inv, b]) => {
        setMembers(m.members ?? []);
        setPeers(p.peers ?? []);
        setTeam(ts.team);
        setInvites(inv.invites ?? []);
        const ent = b.account.entitlements;
        setSeats({
          used: ent.seat_used ?? ts.team.member_count,
          limit: ent.seat_limit_effective ?? ent.seat_limit ?? 10,
          extraSeats: ent.extra_seats ?? 0,
          extraUntil: ent.extra_seats_until ?? null,
        });
        if (!teamName.trim() && ts.team.organization_name) {
          setTeamName(ts.team.organization_name);
        }
      })
      .catch((err) => {
        setMembers([]);
        setPeers([]);
        setTeam(null);
        setInvites([]);
        setSeats(null);
        setError(err instanceof Error ? err.message : String(err));
      });
  };

  useEffect(() => {
    load();
    const timer = window.setInterval(load, 15_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!inviteToken) return;
    setNotice(null);
    void api
      .acceptOrgInvite(inviteToken)
      .then(() => {
        setNotice(t("console.teamAcceptDone"));
        searchParams.delete("invite");
        setSearchParams(searchParams, { replace: true });
        load();
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
  }, [inviteToken, searchParams, setSearchParams, t]);

  const onlineHint = useMemo(() => {
    if (!team) return null;
    if (team.gate === "setup_required") return t("console.teamOnlineGateSetup");
    if (team.gate === "invite_required") return t("console.teamOnlineGateInvite");
    if (peers.length === 0) return t("console.teamOnlineEmpty");
    return null;
  }, [team, peers.length, t]);

  const submitSetup = async () => {
    setError(null);
    setNotice(null);
    try {
      const res = await api.teamSetup(teamName);
      setTeam(res.team);
      setNotice(t("console.teamSetupDone"));
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const submitInviteLink = async () => {
    setError(null);
    setNotice(null);
    setLinkBusy(true);
    try {
      const res = await api.createOrgInviteLink();
      setInviteLink(buildInviteUrl(res.accept_path));
      setNotice(t("console.teamInviteLinkReady"));
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLinkBusy(false);
    }
  };

  const submitInvite = async () => {
    setError(null);
    setNotice(null);
    try {
      const res = await api.createOrgInvite(inviteEmail);
      setInviteLink(buildInviteUrl(res.accept_path));
      load();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const copyInviteLink = async () => {
    if (!inviteLink) return;
    try {
      await navigator.clipboard.writeText(inviteLink);
      setNotice(t("console.teamInviteLinkCopied"));
    } catch {
      setNotice(t("console.teamInviteLinkCopyFailed"));
    }
  };

  const inviteLabel = (inv: Invite) =>
    inv.kind === "link" ? t("console.teamInviteLinkKind") : (inv.email ?? "");

  const submitSeatPurchase = async () => {
    setError(null);
    setNotice(null);
    setSeatBusy(true);
    try {
      const res = await api.checkoutSeatAddon(seatQuantity);
      if (res.provider === "wechat" && res.order) {
        setSeatOrder(res.order);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSeatBusy(false);
    }
  };

  return (
    <ConsolePage title={t("console.team")} description={t("console.teamSubtitle")}>
      {error ? <p className="console-error">{error}</p> : null}
      {seatLimitHit ? <p className="console-meta">{t("console.teamSeatsFull")}</p> : null}
      {notice ? <p className="form-note">{notice}</p> : null}

      {inviteToken ? (
        <section className="console-section">
          <h2 className="console-section__title">{t("console.teamAcceptTitle")}</h2>
          <p className="console-meta">{t("console.teamInviteDesc")}</p>
        </section>
      ) : null}

      {team && !team.team_setup ? (
        <section className="console-section">
          <h2 className="console-section__title">{t("console.teamSetupTitle")}</h2>
          <p className="console-meta">{t("console.teamSetupDesc")}</p>
          <div className="console-form-row">
            <label className="console-form-row__label" htmlFor="team-name">
              {t("console.teamSetupName")}
            </label>
            <input
              id="team-name"
              className="auth-input"
              value={teamName}
              onChange={(e) => setTeamName(e.target.value)}
            />
            <button className="btn btn-primary" type="button" onClick={() => void submitSetup()}>
              {t("console.teamSetupSubmit")}
            </button>
          </div>
        </section>
      ) : null}

      {team && team.team_setup ? (
        <section className="console-section">
          <h2 className="console-section__title">{t("console.teamSeatsTitle")}</h2>
          {seats ? (
            <p className="console-meta">
              {formatMessage(t("console.teamSeatsUsage"), {
                used: String(seats.used),
                limit: String(seats.limit),
              })}
              {seats.extraSeats > 0 && seats.extraUntil
                ? ` · ${formatMessage(t("console.teamSeatsExtraUntil"), {
                    count: String(seats.extraSeats),
                    until: seats.extraUntil,
                  })}`
                : ""}
            </p>
          ) : null}
          <p className="console-meta">{t("console.teamSeatsPriceHint")}</p>
          <div className="console-form-row">
            <label className="console-form-row__label" htmlFor="seat-quantity">
              {t("console.teamSeatsQuantity")}
            </label>
            <input
              id="seat-quantity"
              className="auth-input"
              type="number"
              min={1}
              max={50}
              value={seatQuantity}
              onChange={(e) =>
                setSeatQuantity(Math.max(1, Math.min(50, Number(e.target.value) || 1)))
              }
            />
            <span className="console-meta">
              {formatMessage(t("console.teamSeatsTotal"), {
                amount: String(seatQuantity * SEAT_ADDON_UNIT_YUAN),
              })}
            </span>
            <button
              className="btn btn-secondary"
              type="button"
              disabled={seatBusy}
              onClick={() => void submitSeatPurchase()}
            >
              {seatBusy ? t("common.loading") : t("console.teamSeatsBuy")}
            </button>
          </div>
        </section>
      ) : null}

      {team && team.team_setup ? (
        <section className="console-section">
          <h2 className="console-section__title">{t("console.teamInviteTitle")}</h2>
          <p className="console-meta">{t("console.teamInviteLinkDesc")}</p>
          <div className="console-form-row">
            <button
              className="btn btn-primary"
              type="button"
              disabled={linkBusy}
              onClick={() => void submitInviteLink()}
            >
              {linkBusy ? t("common.loading") : t("console.teamInviteLinkCreate")}
            </button>
          </div>
          {inviteLink ? (
            <div className="console-form-row" style={{ alignItems: "flex-start", gap: "0.75rem" }}>
              <div style={{ flex: 1, minWidth: 0 }}>
                <p className="console-meta">{t("console.teamInviteLink")}</p>
                <code style={{ wordBreak: "break-all", display: "block" }}>{inviteLink}</code>
              </div>
              <button className="btn btn-secondary" type="button" onClick={() => void copyInviteLink()}>
                {t("console.teamInviteLinkCopy")}
              </button>
            </div>
          ) : null}

          <details className="console-meta" style={{ marginTop: "1rem" }}>
            <summary>{t("console.teamInviteEmailOptional")}</summary>
            <div className="console-form-row" style={{ marginTop: "0.75rem" }}>
              <label className="console-form-row__label" htmlFor="invite-email">
                {t("console.teamInviteEmail")}
              </label>
              <input
                id="invite-email"
                className="auth-input"
                type="email"
                value={inviteEmail}
                onChange={(e) => setInviteEmail(e.target.value)}
              />
              <button className="btn btn-secondary" type="button" onClick={() => void submitInvite()}>
                {t("console.teamInviteSubmit")}
              </button>
            </div>
          </details>

          {invites.length > 0 ? (
            <>
              <p className="console-meta">{t("console.teamInvitePending")}</p>
              <ul className="console-list">
                {invites.map((inv) => (
                  <li key={inv.id} className="console-list__item">
                    <strong>{inviteLabel(inv)}</strong>
                    <span className="console-meta">{new Date(inv.expires_at).toLocaleString()}</span>
                  </li>
                ))}
              </ul>
            </>
          ) : null}
        </section>
      ) : null}

      <section className="console-section">
        <h2 className="console-section__title">{t("console.teamOnline")}</h2>
        {!teamReady || peers.length === 0 ? (
          <p className="console-meta">{onlineHint}</p>
        ) : (
          <ul className="console-list">
            {peers.map((p) => (
              <li key={p.user_id + p.device_name} className="console-list__item">
                <strong>{p.display_name || p.device_name}</strong>
                <span className="console-meta">{p.email}</span>
                <span className="console-meta">
                  v{p.version} · {new Date(p.last_seen).toLocaleString()}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="console-section">
        <h2 className="console-section__title">{t("console.teamMembers")}</h2>
        <ul className="console-list">
          {members.map((m) => (
            <li key={m.id} className="console-list__item">
              <strong>{m.display_name || m.email}</strong>
              <span className="console-meta">{m.email}</span>
              <span className="console-meta">{m.role}</span>
            </li>
          ))}
        </ul>
      </section>

      {teamReady ? <p className="console-meta">{t("console.teamHandoffHint")}</p> : null}

      {seatOrder ? (
        <WeChatPayModal
          order={seatOrder}
          onClose={() => setSeatOrder(null)}
          onPaid={() => {
            setSeatOrder(null);
            setNotice(t("plans.wechatPaid"));
            load();
          }}
        />
      ) : null}
    </ConsolePage>
  );
}
