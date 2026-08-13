import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { api } from "@/api/client";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";
import type { GitChangeKind, GitFileChange, GitLogEntry } from "@/api/types/workbench";

type Props = {
  projectId: string;
  /** Rendered after Commit & Push on the same pill row (e.g. turn phase). */
  trailing?: ReactNode;
};

const KIND_BADGE: Record<GitChangeKind, { cls: string; key: string }> = {
  modified: { cls: "conv-git-bar__file-badge--modified", key: "git.modified" },
  added: { cls: "conv-git-bar__file-badge--added", key: "git.added" },
  deleted: { cls: "conv-git-bar__file-badge--deleted", key: "git.deleted" },
  renamed: { cls: "conv-git-bar__file-badge--renamed", key: "git.renamed" },
  untracked: { cls: "conv-git-bar__file-badge--untracked", key: "git.untracked" },
  type_changed: { cls: "conv-git-bar__file-badge--modified", key: "git.typeChanged" },
};

/** Render unified diff text with per-line colouring. */
function DiffBody({ diff }: { diff: string }) {
  const lines = useMemo(() => diff.split("\n"), [diff]);
  return (
    <pre className="conv-git-bar__diff-pre">
      {lines.map((line, i) => {
        let cls = "conv-diff-line";
        if (line.startsWith("+") && !line.startsWith("+++")) cls += " conv-diff-line--add";
        else if (line.startsWith("-") && !line.startsWith("---")) cls += " conv-diff-line--del";
        else if (line.startsWith("@@")) cls += " conv-diff-line--hunk";
        else if (line.startsWith("diff ") || line.startsWith("index ") || line.startsWith("---") || line.startsWith("+++") || line.startsWith("new file") || line.startsWith("deleted file")) cls += " conv-diff-line--meta";
        return (
          <span key={i} className={cls}>
            {line || " "}
          </span>
        );
      })}
    </pre>
  );
}

/** Branch switcher + commit history dropdown behind the branch pill. */
function BranchMenu({
  projectId,
  currentBranch,
  onSwitched,
  onShowCommit,
}: {
  projectId: string;
  currentBranch: string | null;
  onSwitched: () => void;
  onShowCommit: (c: GitLogEntry) => void;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [view, setView] = useState<"branches" | "commits">("branches");
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState(false);
  const [dirty, setDirty] = useState<{ branch: string; files: string[] } | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const branchesQuery = useQuery({
    queryKey: ["project-git-branches", projectId],
    queryFn: () => api.projectGitBranches(projectId),
    enabled: open,
  });

  const logQuery = useQuery({
    queryKey: ["project-git-log", projectId],
    queryFn: () => api.projectGitLog(projectId, 30, 0),
    enabled: open && view === "commits",
  });

  const refresh = () => {
    void branchesQuery.refetch();
    void logQuery.refetch();
    onSwitched();
  };

  const doCheckout = async (branch: string, force: boolean) => {
    setBusy(true);
    setActionError(null);
    try {
      const res = await api.projectGitCheckout(projectId, branch, force);
      if ("error" in res && res.error === "dirty_tree") {
        setDirty({ branch, files: res.files });
        return;
      }
      setDirty(null);
      refresh();
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const doCreate = async () => {
    const name = newName.trim();
    if (!name) return;
    setBusy(true);
    setActionError(null);
    try {
      await api.projectGitCreateBranch(projectId, name, true);
      setNewName("");
      refresh();
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const branches = branchesQuery.data?.branches ?? [];
  const local = branches.filter((b) => !b.remote);
  const remote = branches.filter((b) => b.remote);
  const commits = logQuery.data?.commits ?? [];

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        className="conv-git-bar__pill conv-git-bar__pill--action"
        title={t("git.branches")}
        onClick={() => {
          setOpen((v) => !v);
          setDirty(null);
          setActionError(null);
        }}
        aria-expanded={open}
        aria-haspopup="menu"
      >
        <Icon name="account_tree" size={14} />
        <span className="conv-git-bar__label font-mono">
          {currentBranch ?? t("git.detached")}
        </span>
        <Icon name={open ? "expand_less" : "expand_more"} size={16} />
      </button>
      {open ? (
        <div className="conv-git-bar__menu conv-git-bar__menu--branches left-0 right-auto" role="menu">
          <div className="flex items-center gap-1 px-2 pt-1 pb-1 border-b border-outline-variant/50">
            <button
              type="button"
              className={`px-2 py-1 text-xs rounded border-0 cursor-pointer ${
                view === "branches"
                  ? "bg-primary/15 text-primary"
                  : "bg-transparent text-secondary hover:text-on-surface"
              }`}
              onClick={() => setView("branches")}
            >
              {t("git.branches")}
            </button>
            <button
              type="button"
              className={`px-2 py-1 text-xs rounded border-0 cursor-pointer ${
                view === "commits"
                  ? "bg-primary/15 text-primary"
                  : "bg-transparent text-secondary hover:text-on-surface"
              }`}
              onClick={() => setView("commits")}
            >
              {t("git.commits")}
            </button>
          </div>

          {actionError ? (
            <p className="text-xs text-error m-0 px-3 py-2">{actionError}</p>
          ) : null}

          {view === "branches" ? (
            <>
              {dirty ? (
                <div className="px-3 py-2 border-b border-outline-variant/50 space-y-2">
                  <p className="m-0 text-xs text-warn">
                    {t("git.dirtyTreeTitle").replace("{count}", String(dirty.files.length))}
                  </p>
                  <ul className="m-0 pl-4 text-[11px] text-secondary">
                    {dirty.files.slice(0, 5).map((f) => (
                      <li key={f} className="truncate font-mono">{f}</li>
                    ))}
                    {dirty.files.length > 5 ? <li>…</li> : null}
                  </ul>
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      className="dw-btn-secondary px-2 py-1 text-xs"
                      disabled={busy}
                      onClick={() => void doCheckout(dirty.branch, true)}
                    >
                      {t("git.dirtyTreeForce")}
                    </button>
                    <button
                      type="button"
                      className="dw-btn-ghost px-2 py-1 text-xs"
                      onClick={() => setDirty(null)}
                    >
                      {t("common.cancel")}
                    </button>
                  </div>
                </div>
              ) : null}

              <div className="flex items-center gap-1 px-2 py-2 border-b border-outline-variant/50">
                <input
                  type="text"
                  className="flex-1 min-w-0 text-xs px-2 py-1 rounded border border-outline-variant bg-surface-container-low"
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  placeholder={t("git.newBranchPlaceholder")}
                  autoComplete="off"
                  spellCheck={false}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void doCreate();
                    }
                  }}
                />
                <button
                  type="button"
                  className="dw-btn-secondary px-2 py-1 text-xs shrink-0"
                  disabled={busy || !newName.trim()}
                  onClick={() => void doCreate()}
                >
                  {t("git.createBranch")}
                </button>
              </div>

              <div className="conv-git-bar__menu-label">{t("git.localBranches")}</div>
              {local.map((b) => (
                <button
                  key={b.name}
                  type="button"
                  role="menuitem"
                  className="conv-git-bar__menu-item inline-flex items-center gap-2"
                  disabled={busy || b.current}
                  onClick={() => void doCheckout(b.name, false)}
                >
                  <span className="w-4 shrink-0 text-primary">
                    {b.current ? <Icon name="check" size={14} /> : null}
                  </span>
                  <span className="truncate font-mono text-xs">{b.name}</span>
                </button>
              ))}
              {remote.length > 0 ? (
                <>
                  <div className="conv-git-bar__menu-label">{t("git.remoteBranches")}</div>
                  {remote.map((b) => (
                    <div
                      key={b.name}
                      className="conv-git-bar__menu-item inline-flex items-center gap-2 opacity-60 cursor-default"
                    >
                      <span className="w-4 shrink-0 text-secondary">
                        <Icon name="cloud" size={14} />
                      </span>
                      <span className="truncate font-mono text-xs">{b.name}</span>
                    </div>
                  ))}
                </>
              ) : null}
            </>
          ) : (
            <>
              {commits.length === 0 ? (
                <p className="text-xs text-secondary text-center py-4 m-0">
                  {logQuery.isLoading ? t("common.loading") : t("git.noCommits")}
                </p>
              ) : (
                commits.map((c) => (
                  <button
                    key={c.hash}
                    type="button"
                    role="menuitem"
                    className="conv-git-bar__menu-item"
                    onClick={() => {
                      setOpen(false);
                      onShowCommit(c);
                    }}
                  >
                    <span className="block text-xs text-on-surface truncate">{c.subject}</span>
                    <span className="block text-[10px] text-secondary truncate">
                      <span className="font-mono">{c.short_hash}</span>
                      {" · "}
                      {c.author}
                      {" · "}
                      {c.date.slice(0, 10)}
                    </span>
                  </button>
                ))
              )}
            </>
          )}
        </div>
      ) : null}
    </div>
  );
}

export function ConversationGitBar({ projectId, trailing = null }: Props) {
  const t = useT();
  const queryClient = useQueryClient();
  const [menuOpen, setMenuOpen] = useState(false);
  const [treeOpen, setTreeOpen] = useState(false);
  const [selected, setSelected] = useState<GitFileChange | null>(null);
  const [commitView, setCommitView] = useState<GitLogEntry | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const statusQuery = useQuery({
    queryKey: ["project-git-status", projectId],
    queryFn: () => api.projectGitStatus(projectId),
    refetchInterval: 8_000,
    staleTime: 4_000,
  });

  const changesQuery = useQuery({
    queryKey: ["project-git-changes", projectId],
    queryFn: () => api.projectGitChanges(projectId),
    enabled: treeOpen || Boolean(selected),
    refetchInterval: 8_000,
    staleTime: 4_000,
  });

  const diffQuery = useQuery({
    queryKey: ["project-git-diff", projectId, selected?.path],
    queryFn: () =>
      api.projectGitFileDiff(projectId, selected!.path, selected!.kind),
    enabled: Boolean(selected),
  });

  const commitDiffQuery = useQuery({
    queryKey: ["project-git-commit-diff", projectId, commitView?.hash],
    queryFn: () => api.projectGitCommitDiff(projectId, commitView!.hash),
    enabled: Boolean(commitView),
  });

  const git = statusQuery.data?.git;
  const changes = changesQuery.data?.changes ?? [];

  useEffect(() => {
    if (!menuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [menuOpen]);

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: ["project-git-status", projectId] });
    void queryClient.invalidateQueries({ queryKey: ["project-git-changes", projectId] });
  };

  const commit = useMutation({
    mutationFn: (message?: string) => api.projectGitCommit(projectId, { message }),
    onSuccess: invalidate,
  });

  const push = useMutation({
    mutationFn: () => api.projectGitPush(projectId),
    onSuccess: invalidate,
  });

  const busy = commit.isPending || push.isPending;

  if (!git?.is_repo) {
    if (!trailing) return null;
    return (
      <div className="conv-git-bar px-1 pb-2">
        <div className="conv-git-bar__row">{trailing}</div>
      </div>
    );
  }

  const branchLabel = git.branch ?? t("git.detached");
  const syncHint =
    git.has_upstream && (git.ahead > 0 || git.behind > 0)
      ? t("git.syncHint")
          .replace("{ahead}", String(git.ahead))
          .replace("{behind}", String(git.behind))
      : null;

  const runCommit = () => {
    setMenuOpen(false);
    commit.mutate(undefined);
  };

  const runCommitPush = async () => {
    setMenuOpen(false);
    try {
      await commit.mutateAsync(undefined);
      await push.mutateAsync();
    } catch {
      /* errors surface via mutation state */
    }
  };

  const runPush = () => {
    setMenuOpen(false);
    push.mutate();
  };

  const actionError = (commit.error ?? push.error) as Error | null;
  const stagedCount = changes.filter((c) => c.staged).length;
  const unstagedCount = changes.length - stagedCount;

  const selectFile = (c: GitFileChange) => {
    setSelected(c);
  };

  return (
    <div className="conv-git-bar px-1 pb-2">
      <div className="conv-git-bar__row">
        <BranchMenu
          projectId={projectId}
          currentBranch={git.branch}
          onSwitched={invalidate}
          onShowCommit={(c) => {
            setSelected(null);
            setTreeOpen(false);
            setCommitView(c);
          }}
        />
        <button
          type="button"
          className="conv-git-bar__pill conv-git-bar__pill--action"
          title={t("git.changedFiles")}
          onClick={() => {
            setTreeOpen((v) => !v);
            setSelected(null);
          }}
          aria-expanded={treeOpen}
        >
          <span className="conv-git-bar__label">{t("git.changes")}</span>
          <span className="conv-git-bar__stat conv-git-bar__stat--add">+{git.insertions}</span>
          <span className="conv-git-bar__stat conv-git-bar__stat--del">-{git.deletions}</span>
          {syncHint ? (
            <span className="conv-git-bar__sync text-[11px] text-secondary ml-1">{syncHint}</span>
          ) : null}
          <Icon name={treeOpen ? "expand_less" : "expand_more"} size={16} />
        </button>

        <div className="relative" ref={menuRef}>
          <button
            type="button"
            className="conv-git-bar__pill conv-git-bar__pill--action"
            disabled={busy || (!git.has_changes && git.ahead === 0)}
            onClick={() => setMenuOpen((v) => !v)}
            aria-expanded={menuOpen}
            aria-haspopup="menu"
          >
            <span>{t("git.commitAndPush")}</span>
            <Icon name="expand_more" size={16} />
          </button>
          {menuOpen ? (
            <div className="conv-git-bar__menu" role="menu">
              <button
                type="button"
                role="menuitem"
                className="conv-git-bar__menu-item"
                disabled={busy || !git.has_changes}
                onClick={runCommit}
              >
                {t("git.commit")}
              </button>
              <button
                type="button"
                role="menuitem"
                className="conv-git-bar__menu-item"
                disabled={busy || (!git.has_changes && git.ahead === 0)}
                onClick={() => void runCommitPush()}
              >
                {t("git.commitAndPush")}
              </button>
              <button
                type="button"
                role="menuitem"
                className="conv-git-bar__menu-item"
                disabled={busy || !git.has_upstream || (git.ahead === 0 && !git.has_changes)}
                onClick={runPush}
              >
                {t("git.push")}
              </button>
            </div>
          ) : null}
        </div>

        {trailing}
      </div>

      {actionError ? (
        <p className="text-xs text-error m-0 mt-1 px-1">{actionError.message}</p>
      ) : null}

      {selected ? (
        <div className="conv-git-bar__diff">
          <div className="conv-git-bar__diff-head">
            <button
              type="button"
              className="conv-git-bar__file-name inline-flex items-center gap-1 text-left border-0 bg-transparent text-xs text-secondary cursor-pointer hover:text-on-surface"
              onClick={() => setSelected(null)}
            >
              <Icon name="chevron_left" size={16} />
              {t("git.backToChanges")}
            </button>
            <span className="flex items-center gap-1 min-w-0 truncate">
              {(() => {
                const badge = KIND_BADGE[selected.kind];
                return (
                  <span className={`conv-git-bar__file-badge ${badge.cls}`}>{t(badge.key)}</span>
                );
              })()}
              <span className="text-xs font-mono text-on-surface truncate">{selected.path}</span>
            </span>
          </div>
          <div className="px-3 py-1 flex items-center gap-2 text-[11px] border-b border-outline-variant/50">
            <span className="text-success">+{diffQuery.data?.diff.insertions ?? 0}</span>
            <span className="text-error">-{diffQuery.data?.diff.deletions ?? 0}</span>
            {diffQuery.isLoading ? <span className="text-secondary">{t("common.loading")}</span> : null}
          </div>
          {diffQuery.data ? <DiffBody diff={diffQuery.data.diff.diff} /> : null}
          {diffQuery.isError ? (
            <p className="text-xs text-error m-0 px-3 py-2">
              {(diffQuery.error as Error).message}
            </p>
          ) : null}
        </div>
      ) : null}

      {commitView ? (
        <div className="conv-git-bar__diff">
          <div className="conv-git-bar__diff-head">
            <button
              type="button"
              className="conv-git-bar__file-name inline-flex items-center gap-1 text-left border-0 bg-transparent text-xs text-secondary cursor-pointer hover:text-on-surface"
              onClick={() => setCommitView(null)}
            >
              <Icon name="chevron_left" size={16} />
              {t("git.commits")}
            </button>
            <span className="flex items-center gap-2 min-w-0 truncate">
              <span className="text-xs font-mono text-primary shrink-0">{commitView.short_hash}</span>
              <span className="text-xs text-on-surface truncate">{commitView.subject}</span>
              <span className="text-[10px] text-secondary shrink-0">
                {commitView.author} · {commitView.date.slice(0, 10)}
              </span>
            </span>
          </div>
          <div className="px-3 py-1 flex items-center gap-2 text-[11px] border-b border-outline-variant/50">
            <span className="text-success">+{commitDiffQuery.data?.diff.insertions ?? 0}</span>
            <span className="text-error">-{commitDiffQuery.data?.diff.deletions ?? 0}</span>
            {commitDiffQuery.isLoading ? <span className="text-secondary">{t("common.loading")}</span> : null}
          </div>
          {commitDiffQuery.data ? <DiffBody diff={commitDiffQuery.data.diff.diff} /> : null}
          {commitDiffQuery.isError ? (
            <p className="text-xs text-error m-0 px-3 py-2">
              {(commitDiffQuery.error as Error).message}
            </p>
          ) : null}
        </div>
      ) : null}

      {treeOpen && !selected ? (
        <div className="conv-git-bar__tree">
          <div className="conv-git-bar__tree-head">
            <span className="inline-flex items-center gap-1 min-w-0">
              <Icon name="timeline" size={13} />
              <span className="truncate font-mono text-[10px] text-secondary">{branchLabel}</span>
            </span>
            <span className="shrink-0">
              {t("git.changedFiles")} ({changes.length})
            </span>
            {stagedCount > 0 ? (
              <span className="text-[10px]">
                {t("git.staged")} {stagedCount} · {t("git.unstaged")} {unstagedCount}
              </span>
            ) : null}
          </div>
          {changes.length === 0 ? (
            <p className="text-xs text-secondary text-center py-4 m-0">{t("git.noChanges")}</p>
          ) : (
            <div className="max-h-[18rem] overflow-auto py-1">
              {changes.map((c) => {
                const badge = KIND_BADGE[c.kind];
                return (
                  <button
                    type="button"
                    key={c.path}
                    className="conv-git-bar__file"
                    onClick={() => selectFile(c)}
                  >
                    <span className={`conv-git-bar__file-badge ${badge.cls}`}>{t(badge.key)}</span>
                    <span className="conv-git-bar__file-name">{c.path}</span>
                    <span className="text-[10px] tabular-nums shrink-0">
                      <span className="text-success">+{c.insertions}</span>{" "}
                      <span className="text-error">-{c.deletions}</span>
                    </span>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}