import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "@/api/client";
import { Icon } from "@/components/Icon";
import { SectionCard } from "@/components/ui/SectionCard";
import { useT } from "@/i18n/context";

const EXAMPLE = `[
  {
    "slug": "filesystem",
    "command": "npx -y @modelcontextprotocol/server-filesystem /tmp"
  },
  {
    "slug": "api",
    "type": "http",
    "url": "https://example.com/mcp"
  }
]`;

export function McpServersPanel() {
  const t = useT();
  const queryClient = useQueryClient();
  const serversQuery = useQuery({
    queryKey: ["mcp-servers"],
    queryFn: api.mcpServers,
  });
  const [draft, setDraft] = useState<string | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);
  const [strict, setStrict] = useState<boolean | null>(null);
  const [allowlist, setAllowlist] = useState<string | null>(null);

  const save = useMutation({
    mutationFn: (payload: {
      servers: unknown[];
      governance: { strict: boolean; allowed_tools: string[] };
    }) => api.setMcpServers(payload.servers, payload.governance),
    onSuccess: () => {
      setParseError(null);
      void queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
      void queryClient.invalidateQueries({ queryKey: ["doctor"] });
    },
  });

  const servers = serversQuery.data?.servers ?? [];
  const gov = serversQuery.data?.governance;
  const text =
    draft ??
    (servers.length > 0 ? JSON.stringify(servers, null, 2) : EXAMPLE);
  const strictVal = strict ?? Boolean(gov?.strict);
  const allowlistVal =
    allowlist ?? (gov?.allowed_tools?.length ? gov.allowed_tools.join(", ") : "");

  return (
    <SectionCard title={t("settings.mcpServers.title")}>
      <p className="text-sm text-secondary m-0 mb-3">{t("settings.mcpServers.hint")}</p>
      <textarea
        className="dw-input font-code text-xs min-h-[10rem] w-full"
        value={text}
        onChange={(e) => {
          setDraft(e.target.value);
          setParseError(null);
        }}
        spellCheck={false}
      />
      <label className="inline-flex items-center gap-2 text-sm mt-3 cursor-pointer">
        <input
          type="checkbox"
          checked={strictVal}
          onChange={(e) => setStrict(e.target.checked)}
        />
        {t("settings.mcpServers.strict")}
      </label>
      <label className="flex flex-col gap-1 text-sm mt-2">
        <span className="text-secondary">{t("settings.mcpServers.allowlist")}</span>
        <input
          className="dw-input text-sm font-code"
          value={allowlistVal}
          placeholder="server:tool, other:tool"
          onChange={(e) => setAllowlist(e.target.value)}
        />
      </label>
      {gov?.env_override_note ? (
        <p className="text-xs text-secondary m-0 mt-2">{gov.env_override_note}</p>
      ) : null}
      {parseError && <p className="text-sm text-error m-0 mt-2">{parseError}</p>}
      {save.isError && (
        <p className="text-sm text-error m-0 mt-2">{t("settings.mcpServers.error")}</p>
      )}
      <div className="flex flex-wrap items-center gap-2 mt-3">
        <button
          type="button"
          className="dw-btn-primary inline-flex items-center gap-2"
          disabled={save.isPending || serversQuery.isLoading}
          onClick={() => {
            try {
              const parsed = JSON.parse(text);
              if (!Array.isArray(parsed)) {
                setParseError(t("settings.mcpServers.invalidArray"));
                return;
              }
              const tools = allowlistVal
                .split(/[,\n]/)
                .map((s) => s.trim())
                .filter(Boolean);
              save.mutate({
                servers: parsed,
                governance: { strict: strictVal, allowed_tools: tools },
              });
              setDraft(null);
              setStrict(null);
              setAllowlist(null);
            } catch {
              setParseError(t("settings.mcpServers.invalidJson"));
            }
          }}
        >
          <Icon name="save" size={16} />
          {t("settings.mcpServers.save")}
        </button>
        {save.data?.restart_hint && (
          <span className="text-xs text-secondary">{save.data.restart_hint}</span>
        )}
      </div>
    </SectionCard>
  );
}
