import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { LogoMarkIcon } from "../../components/LogoMarkIcon";
import { useAuth } from "../../hooks/useAuth";
import { formatMessage, useLocale, useT } from "../../i18n/context";
import {
  cloudEventsToLines,
  relativeTime,
  remoteChatApi,
  type CloudRemoteConversation,
  type CloudRemoteEvent,
  type RemoteMineDevice,
} from "../../lib/remoteChat";
import "./mobile.css";

const HOME_DEVICE_KEY = "anycode-mobile-home-device";

function readStoredDevice(): string | null {
  try {
    return localStorage.getItem(HOME_DEVICE_KEY);
  } catch {
    return null;
  }
}

function writeStoredDevice(id: string) {
  try {
    localStorage.setItem(HOME_DEVICE_KEY, id);
  } catch {
    /* ignore */
  }
}

function hopToMobile() {
  window.location.replace(`/api/auth/hop/login?next=${encodeURIComponent("/m")}`);
}

export function RequireMobileAuth({ children }: { children: React.ReactNode }) {
  const { authenticated, validating } = useAuth();
  const t = useT();

  useEffect(() => {
    if (!validating && !authenticated) hopToMobile();
  }, [authenticated, validating]);

  if (validating || !authenticated) {
    return (
      <div className="ac-mobile">
        <div className="ac-mobile__top">
          <div className="ac-mobile__brand">
            <LogoMarkIcon size={28} />
            <div>
              <h1>{t("mobile.title")}</h1>
              <p>{t("mobile.signingIn")}</p>
            </div>
          </div>
        </div>
      </div>
    );
  }
  return <>{children}</>;
}

export function MobileApp() {
  const t = useT();
  const locale = useLocale();
  const { logout } = useAuth();
  const [devices, setDevices] = useState<RemoteMineDevice[]>([]);
  const [homeDeviceId, setHomeDeviceId] = useState<string | null>(readStoredDevice);
  const [conversations, setConversations] = useState<CloudRemoteConversation[]>([]);
  const [selectedConvId, setSelectedConvId] = useState<string | null>(null);
  const [events, setEvents] = useState<CloudRemoteEvent[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [composingNew, setComposingNew] = useState(false);
  const threadRef = useRef<HTMLDivElement>(null);

  const selectedDevice = devices.find((d) => d.id === homeDeviceId) ?? null;
  const selectedConversation =
    conversations.find((c) => c.id === selectedConvId) ?? null;
  const lines = useMemo(() => cloudEventsToLines(events), [events]);
  const running = selectedConversation?.status === "running" || sending;
  const online = selectedDevice?.online ?? false;

  const selectDevice = useCallback((id: string) => {
    setHomeDeviceId(id);
    writeStoredDevice(id);
    setSelectedConvId(null);
    setEvents([]);
    setComposingNew(false);
  }, []);

  const refreshDevices = useCallback(async () => {
    const data = await remoteChatApi.listMineDevices();
    setDevices(data.devices);
    setHomeDeviceId((current) => {
      if (current && data.devices.some((d) => d.id === current)) return current;
      const next = data.devices.find((d) => d.online)?.id ?? data.devices[0]?.id ?? null;
      if (next) writeStoredDevice(next);
      return next;
    });
    return data.devices;
  }, []);

  const refreshConversations = useCallback(async (deviceId: string) => {
    const data = await remoteChatApi.listConversations(deviceId);
    setConversations(data.conversations);
    return data.conversations;
  }, []);

  const refreshThread = useCallback(async (conversationId: string) => {
    const data = await remoteChatApi.getConversation(conversationId, 0);
    setEvents(data.events);
    setConversations((prev) => {
      const next = prev.filter((c) => c.id !== data.conversation.id);
      return [data.conversation, ...next];
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const list = await refreshDevices();
        if (cancelled) return;
        const deviceId =
          (homeDeviceId && list.some((d) => d.id === homeDeviceId) && homeDeviceId) ||
          list.find((d) => d.online)?.id ||
          list[0]?.id ||
          null;
        if (deviceId) await refreshConversations(deviceId);
        if (!cancelled) setError(null);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 8_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [homeDeviceId, refreshConversations, refreshDevices]);

  useEffect(() => {
    if (!selectedConvId) {
      setEvents([]);
      return;
    }
    let cancelled = false;
    const tick = async () => {
      try {
        await refreshThread(selectedConvId);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      }
    };
    void tick();
    const runningNow = selectedConversation?.status === "running";
    const id = window.setInterval(() => void tick(), runningNow ? 1_200 : 4_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [refreshThread, selectedConvId, selectedConversation?.status]);

  useEffect(() => {
    const el = threadRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [lines, running]);

  async function onSend() {
    const prompt = draft.trim();
    if (!prompt || !homeDeviceId || sending || !online) return;
    setSending(true);
    setError(null);
    try {
      const data = await remoteChatApi.postPrompt({
        home_device_id: homeDeviceId,
        prompt,
        conversation_id: selectedConvId ?? undefined,
        title: selectedConvId ? undefined : prompt,
      });
      setSelectedConvId(data.conversation.id);
      setComposingNew(false);
      setDraft("");
      await refreshConversations(homeDeviceId);
      await refreshThread(data.conversation.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSending(false);
    }
  }

  async function onCancel() {
    if (!selectedConvId) return;
    try {
      await remoteChatApi.postCancel(selectedConvId);
      await refreshThread(selectedConvId);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  const showThread = Boolean(selectedConvId) || composingNew;

  return (
    <div className="ac-mobile">
      <header className="ac-mobile__top">
        {showThread ? (
          <button
            type="button"
            className="ac-mobile__ghost"
            onClick={() => {
              setSelectedConvId(null);
              setComposingNew(false);
              setDraft("");
            }}
          >
            {t("mobile.back")}
          </button>
        ) : null}
        <div className="ac-mobile__brand">
          <LogoMarkIcon size={28} />
          <div>
            <h1>
              {selectedConversation?.title ||
                (selectedDevice
                  ? formatMessage(t("mobile.commanding"), {
                      device: selectedDevice.device_name || selectedDevice.platform,
                    })
                  : t("mobile.title"))}
            </h1>
            <p>{selectedDevice ? selectedDevice.device_name : t("mobile.subtitle")}</p>
          </div>
        </div>
        <button
          type="button"
          className="ac-mobile__ghost"
          onClick={() => {
            logout();
            hopToMobile();
          }}
        >
          {t("mobile.signOut")}
        </button>
      </header>

      {error ? <p className="ac-mobile__error">{error}</p> : null}

      <div className="ac-mobile__body">
        {!homeDeviceId || devices.length === 0 ? (
          <div className="ac-mobile__scroll ac-mobile__pad">
            {loading ? (
              <p className="ac-mobile__status">{t("common.loading")}</p>
            ) : (
              <div className="ac-mobile__empty">
                <p className="ac-mobile__hint">{t("mobile.noDevices")}</p>
                <p className="ac-mobile__hint">{t("mobile.noDevicesHint")}</p>
              </div>
            )}
          </div>
        ) : showThread ? (
          <>
            <div ref={threadRef} className="ac-mobile__thread">
              {lines.length === 0 ? (
                <div className="ac-mobile__empty">
                  <p className="ac-mobile__hint">
                    {online ? t("mobile.empty") : t("mobile.offlineHint")}
                  </p>
                </div>
              ) : (
                lines.map((line) => (
                  <div key={line.id} className={`ac-mobile__msg ac-mobile__msg--${line.role}`}>
                    <small>
                      {line.role === "user"
                        ? t("mobile.you")
                        : line.role === "tool"
                          ? t("mobile.tool")
                          : line.role === "error"
                            ? t("mobile.error")
                            : t("mobile.assistant")}
                    </small>
                    <p>{line.text}</p>
                  </div>
                ))
              )}
            </div>
            <div className="ac-mobile__composer">
              <textarea
                rows={2}
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                disabled={!online || sending}
                placeholder={online ? t("mobile.placeholder") : t("mobile.offlineHint")}
              />
              {running && selectedConversation ? (
                <button type="button" className="ac-mobile__stop" onClick={() => void onCancel()}>
                  {t("mobile.stop")}
                </button>
              ) : (
                <button
                  type="button"
                  className="ac-mobile__send"
                  disabled={!online || sending || !draft.trim()}
                  onClick={() => void onSend()}
                >
                  {t("mobile.send")}
                </button>
              )}
            </div>
          </>
        ) : (
          <>
            <div className="ac-mobile__picker">
              <label htmlFor="ac-mobile-device">{t("mobile.devices")}</label>
              <select
                id="ac-mobile-device"
                value={homeDeviceId}
                onChange={(e) => selectDevice(e.target.value)}
              >
                {devices.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.device_name || d.platform} ·{" "}
                    {d.online ? t("mobile.online") : t("mobile.offline")}
                  </option>
                ))}
              </select>
            </div>
            <button
              type="button"
              className="ac-mobile__new"
              onClick={() => {
                setSelectedConvId(null);
                setEvents([]);
                setDraft("");
                setComposingNew(true);
              }}
            >
              {t("mobile.newChat")}
            </button>
            <div className="ac-mobile__scroll">
              {conversations.length === 0 ? (
                <div className="ac-mobile__pad">
                  <p className="ac-mobile__hint">
                    {online ? t("mobile.empty") : t("mobile.offlineHint")}
                  </p>
                </div>
              ) : (
                conversations.map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className={`ac-mobile__row${c.id === selectedConvId ? " ac-mobile__row--on" : ""}`}
                    onClick={() => setSelectedConvId(c.id)}
                  >
                    <span>
                      <strong>{c.title || c.id}</strong>
                      {c.project_name ? <em>{c.project_name}</em> : null}
                    </span>
                    <em>{relativeTime(c.updated_at, locale)}</em>
                  </button>
                ))
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
