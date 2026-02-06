import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { ChevronRight, Info, X } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import LauncherBrand from "../components/LauncherBrand";
import {
  desktopListSshHosts,
  desktopListSshPaths,
  desktopGetGitBranch,
  desktopPickFolder,
  desktopTestSsh,
  isDesktopApp,
  type DesktopSshPathEntry,
  type DesktopSshHost,
} from "../utils/desktop";

type WizardOption = {
  id: string;
  title: string;
  desc: string;
  badge?: string;
  advanced?: boolean;
};

type WizardStep = {
  key: string;
  title: string;
  note: string;
  options?: WizardOption[];
  body?: ReactNode;
  info?: string;
};

type SshRecent = {
  host: string;
  user?: string | null;
  updated_at_ms: number;
};

const SSH_RECENTS_KEY = "contextDesktopSshRecentsV1";
const LOCAL_TITLE_MODEL_SIZE = "1.3 GB";

const loadSshRecents = (): SshRecent[] => {
  try {
    const raw = localStorage.getItem(SSH_RECENTS_KEY);
    const parsed = raw ? JSON.parse(raw) : null;
    return Array.isArray(parsed) ? (parsed as SshRecent[]) : [];
  } catch {
    return [];
  }
};

const saveSshRecents = (recents: SshRecent[]) => {
  try {
    localStorage.setItem(SSH_RECENTS_KEY, JSON.stringify(recents.slice(0, 50)));
  } catch {
    // ignore
  }
};

const upsertSshRecent = (host: string, user?: string | null): SshRecent[] => {
  const recents = loadSshRecents();
  const key = `${user ?? ""}@${host}`;
  const next = [
    { host, user: user ?? null, updated_at_ms: Date.now() },
    ...recents.filter((r) => `${r.user ?? ""}@${r.host}` !== key),
  ];
  saveSshRecents(next);
  return next;
};

const parseUserHost = (raw: string): { host: string; user?: string | null } | null => {
  const trimmed = String(raw || "").trim();
  if (!trimmed) return null;
  const at = trimmed.lastIndexOf("@");
  if (at > 0) {
    const user = trimmed.slice(0, at).trim();
    const host = trimmed.slice(at + 1).trim();
    if (!host) return null;
    return { host, user: user || null };
  }
  return { host: trimmed };
};

export default function WorkspaceSetupPage() {
  const [stepIndex, setStepIndex] = useState(0);
  const [selections, setSelections] = useState<Record<string, string>>({});
  const [sshHosts, setSshHosts] = useState<DesktopSshHost[]>([]);
  const [sshRecents, setSshRecents] = useState<SshRecent[]>(() => loadSshRecents());
  const [remoteHostInput, setRemoteHostInput] = useState("");
  const [remoteStatus, setRemoteStatus] = useState<"idle" | "connecting" | "connected" | "error">("idle");
  const [remoteError, setRemoteError] = useState<string | null>(null);
  const [sourcePath, setSourcePath] = useState("");
  const [repoUrl, setRepoUrl] = useState("");
  const [repoBranch, setRepoBranch] = useState("");
  const [workspaceName, setWorkspaceName] = useState("");
  const [setupHook, setSetupHook] = useState("");
  const [targetBranch, setTargetBranch] = useState("main");
  const [targetBranchTouched, setTargetBranchTouched] = useState(false);
  const [verifyCommand, setVerifyCommand] = useState("");
  const [mergeAdvancedOpen, setMergeAdvancedOpen] = useState(false);
  const [pushOnSuccess, setPushOnSuccess] = useState(false);
  const [pushRemote, setPushRemote] = useState("origin");
  const [pushBranch, setPushBranch] = useState("main");
  const [pushBranchTouched, setPushBranchTouched] = useState(false);
  const [cloudEndpoint, setCloudEndpoint] = useState("");
  const [cloudApiKey, setCloudApiKey] = useState("");
  const [networkAllowlist, setNetworkAllowlist] = useState("");
  const [containerAdvancedOpen, setContainerAdvancedOpen] = useState(false);
  const [openInfoKey, setOpenInfoKey] = useState<string | null>(null);
  const [remotePathSuggestions, setRemotePathSuggestions] = useState<DesktopSshPathEntry[]>([]);
  const [remotePathStatus, setRemotePathStatus] = useState<"idle" | "loading" | "error">("idle");
  const [remotePathError, setRemotePathError] = useState<string | null>(null);
  const containerMode = selections.container;

  const steps = useMemo<WizardStep[]>(() => ([
      {
        key: "location",
        title: "Location",
        note: "Where will this workspace run?",
        options: [
          { id: "local", title: "Local", desc: "Agents run on this machine." },
          { id: "remote", title: "Remote", desc: "Agents run on your existing dev box (remote IDE experience)." },
        ],
      },
      {
        key: "container",
        title: "Agent Sandbox Isolation",
        note: "Choose the containerization strategy for your agents.",
        options: [
          {
            id: "sealed",
            title: "Fully sealed container",
            desc: "Managed volume, no host mounts. Useful for running agents without additional restrictions in a safe and controlled environment.",
            badge: "Recommended",
          },
          {
            id: "no-container",
            title: "No container",
            desc: "Run directly on the host. Useful if you already have isolation set up (e.g. a dev box or mini PC that is already agent-safe).",
          },
          {
            id: "host-mounted",
            title: "Host-mounted container",
            desc: "Use a host folder as the workspace. Agents are containerized but they write back directly to your project directory on your machine.",
            advanced: true,
          },
        ],
      },
      {
        key: "source",
        title: "Source",
        note: "How should we create the workspace?",
        options: [
          { id: "clone", title: "Clone repo", desc: "Git URL + optional branch." },
          { id: "import", title: "Import folder", desc: "Select a folder (host-mounted/no-container) or copy into a managed volume." },
          { id: "new", title: "New empty", desc: "Initialize a new git repo." },
        ],
      },
      {
        key: "network",
        title: "Network Policy",
        note: "Restrict or permit agent network access.",
        options: [
          {
            id: "providers",
            title: "LLM providers only",
            desc: "Only allow validated LLM provider traffic. This restricts the agent from using any other network access.",
          },
          {
            id: "allowlist",
            title: "Allowlist",
            desc: "Specific hosts you approve. Useful for known-safe sources such as internal sites or other trusted sources.",
          },
          {
            id: "full",
            title: "Full access",
            desc: "Unrestricted outbound. Useful if you are not working with private data and understand the risks of prompt injection attacks.",
          },
        ],
      },
      {
        key: "setup",
        title: "Worktree Setup Hook",
        note: "Choose a single shell command to run on new worktree creation (e.g., install dependencies).",
        info: [
          "Each ctx task runs on its own git worktree. A worktree is a separate working directory attached to the same repository.",
          "This allows one or more agents to work on a task branch isolated from other tasks, so work can be done simultaneously.",
          "",
          "Depending on your project, you might want to do setup every time a new worktree is created (for example, installing dependencies).",
          "",
          "If you do not know what to put here, skip it and come back later. You can also ask an agent what the best setup hook is for your project.",
          "",
          'Example prompt: "You are in a freshly created git worktree in this project. Is there any setup that ought to have occurred? If so, is there a single setup command we can run as a worktree setup hook?"',
        ].join("\n"),
      },
      {
        key: "merge-queue",
        title: "Merge Queue",
        note: "Branch to work from, plus an optional verification command.",
        info: [
          "A merge queue is a queue of pull requests waiting to be merged to a single branch. It helps ensure changes merge cleanly and pass checks.",
          "",
          "With stacked agent-driven changes, two PRs can pass individually but fail once combined. A personal merge queue helps you test stacked changes locally.",
          "",
          "In this setup step, choose a branch to work off of, and an optional verification command. Prefer a lightweight but robust command here (lint, format, build, unit tests), and defer expensive or flaky end-to-end tests to CI or on-demand runs.",
          "",
          "Advanced settings let you automatically push to a remote after a successful local merge.",
        ].join("\n"),
      },
      {
        key: "session-titling",
        title: "Generate Task Titles",
        note: "Use a small local or cloud model to generate helpful titles for your tasks.",
        options: [
          {
            id: "local-model",
            title: "Local model",
            desc: `Run Qwen3-1.7B locally (requires ~${LOCAL_TITLE_MODEL_SIZE} disk space).`,
            badge: "Recommended",
          },
          { id: "cloud-api", title: "Cloud API", desc: "Use your provider key and endpoint." },
        ],
      },
      {
        key: "confirm",
        title: "Confirm and create",
        note: "Review your choices before provisioning.",
      },
    ]), []);
  const step = steps[stepIndex];
  const infoStep = openInfoKey ? steps.find((s) => s.key === openInfoKey) : null;
  const isFirst = stepIndex === 0;
  const isLast = stepIndex === steps.length - 1;
  const requiresSelection = Boolean(step.options?.length);
  const hasSelection = Boolean(selections[step.key]);
  const mergeQueueSkipped = selections["merge-queue"] === "skip";
  const isRemoteStep = step.key === "location" && selections.location === "remote";
  const needsHostPath =
    step.key === "source" &&
    (selections.source === "import" || containerMode === "host-mounted" || containerMode === "no-container");
  const hasSourcePath = !needsHostPath || sourcePath.trim() !== "";
  const needsRepoUrl = step.key === "source" && selections.source === "clone";
  const hasRepoUrl = !needsRepoUrl || repoUrl.trim() !== "";
  const needsWorkspaceName = step.key === "source" && containerMode === "sealed" && selections.source === "new";
  const hasWorkspaceName = !needsWorkspaceName || workspaceName.trim() !== "";
  const needsTargetBranch = step.key === "merge-queue" && !mergeQueueSkipped;
  const hasTargetBranch = !needsTargetBranch || targetBranch.trim() !== "";
  const needsCloudCreds = step.key === "session-titling" && selections[step.key] === "cloud-api";
  const hasCloudCreds = !needsCloudCreds
    || (cloudEndpoint.trim() !== "" && cloudApiKey.trim() !== "");
  const needsAllowlist = step.key === "network" && selections.network === "allowlist";
  const hasAllowlist = !needsAllowlist
    || networkAllowlist.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).length > 0;
  const parsedRemote = parseUserHost(remoteHostInput);
  const hasRemoteHost = Boolean(parsedRemote?.host);
  const canAdvance = (!requiresSelection || hasSelection)
    && (!isRemoteStep || (hasRemoteHost && remoteStatus !== "connecting"))
    && hasSourcePath
    && hasRepoUrl
    && hasWorkspaceName
    && hasTargetBranch
    && hasCloudCreds
    && hasAllowlist;

  const shouldAutoAdvance = (stepKey: string, optionId: string): boolean => {
    if (stepKey === "location") return optionId === "local";
    if (stepKey === "container") return true;
    if (stepKey === "network") return optionId !== "allowlist";
    if (stepKey === "session-titling") return optionId === "local-model";
    return false;
  };

  const onSelect = (stepKey: string, optionId: string) => {
    setSelections((prev) => {
      const next = { ...prev, [stepKey]: optionId };
      if (stepKey === "container") {
        delete next.source;
      }
      return next;
    });
    if (stepKey === "location" && optionId === "local") {
      setRemoteStatus("idle");
      setRemoteError(null);
    }
    if (stepKey === "container") {
      setSourcePath("");
      setWorkspaceName("");
    }
    if (stepKey === "source") {
      if (optionId !== "clone") {
        setRepoUrl("");
        setRepoBranch("");
      }
      if (optionId !== "new") {
        setWorkspaceName("");
      }
    }
    if (stepKey === "session-titling" && optionId !== "cloud-api") {
      setCloudEndpoint("");
      setCloudApiKey("");
    }
    if (stepKey === "network" && optionId !== "allowlist") {
      setNetworkAllowlist("");
    }
  };

  const onSelectOption = (stepKey: string, optionId: string) => {
    onSelect(stepKey, optionId);
    if (shouldAutoAdvance(stepKey, optionId)) {
      setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
    }
  };

  const enableMergeQueueIfSkipped = () => {
    if (!mergeQueueSkipped) return;
    setSelections((prev) => {
      if (prev["merge-queue"] !== "skip") return prev;
      const next = { ...prev };
      delete next["merge-queue"];
      return next;
    });
  };

  useEffect(() => {
    if (selections.container === "host-mounted") {
      setContainerAdvancedOpen(true);
    }
  }, [selections.container]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpenInfoKey(null);
      }
    };
    if (openInfoKey) {
      window.addEventListener("keydown", onKeyDown);
      return () => window.removeEventListener("keydown", onKeyDown);
    }
    return;
  }, [openInfoKey]);

  useEffect(() => {
    setSshRecents(loadSshRecents());
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    desktopListSshHosts()
      .then((hosts) => setSshHosts(hosts))
      .catch(() => setSshHosts([]));
  }, []);

  useEffect(() => {
    const shouldSuggest = needsHostPath
      && selections.location === "remote"
      && remoteStatus === "connected"
      && Boolean(parsedRemote?.host)
      && isDesktopApp();
    if (!shouldSuggest) {
      setRemotePathSuggestions([]);
      setRemotePathStatus("idle");
      setRemotePathError(null);
      return;
    }
    const handle = window.setTimeout(() => {
      setRemotePathStatus("loading");
      setRemotePathError(null);
      desktopListSshPaths({
        host: parsedRemote!.host,
        user: parsedRemote!.user ?? null,
        path: sourcePath,
      })
        .then((entries) => {
          setRemotePathSuggestions(entries);
          setRemotePathStatus("idle");
        })
        .catch((err: any) => {
          setRemotePathStatus("error");
          setRemotePathError(err?.message ?? String(err));
        });
    }, 250);
    return () => window.clearTimeout(handle);
  }, [needsHostPath, selections.location, remoteStatus, parsedRemote?.host, parsedRemote?.user, sourcePath]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    if (selections.location !== "local") return;
    if (!sourcePath.trim()) return;
    if (targetBranchTouched) return;
    let cancelled = false;
    desktopGetGitBranch({ path: sourcePath })
      .then((branch) => {
        if (cancelled || !branch || targetBranchTouched) return;
        setTargetBranch(branch);
        if (!pushBranchTouched) {
          setPushBranch(branch);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [selections.location, sourcePath, targetBranchTouched, pushBranchTouched]);

  useEffect(() => {
    if (pushBranchTouched) return;
    if (!targetBranch.trim()) return;
    setPushBranch(targetBranch);
  }, [targetBranch, pushBranchTouched]);

  const sshSuggestions = useMemo(() => {
    const query = remoteHostInput.trim().toLowerCase();
    const list: Array<{ host: string; user?: string | null }> = [];
    const seen = new Set<string>();
    for (const recent of sshRecents) {
      const key = `${recent.user ?? ""}@${recent.host}`;
      if (seen.has(key)) continue;
      seen.add(key);
      list.push({ host: recent.host, user: recent.user ?? null });
    }
    for (const host of sshHosts) {
      const key = `${host.user ?? ""}@${host.host}`;
      if (seen.has(key)) continue;
      seen.add(key);
      list.push({ host: host.host, user: host.user ?? null });
    }
    return list.filter((entry) => {
      if (!query) return true;
      const hay = `${entry.user ?? ""}@${entry.host}`.toLowerCase();
      return hay.includes(query);
    }).slice(0, 6);
  }, [remoteHostInput, sshHosts, sshRecents]);

  const onRemoteInputChange = (value: string) => {
    setRemoteHostInput(value);
    if (remoteStatus !== "idle") {
      setRemoteStatus("idle");
      setRemoteError(null);
    }
  };

  const onPickLocalFolder = async () => {
    if (!isDesktopApp()) return;
    try {
      const picked = await desktopPickFolder();
      if (picked) {
        setSourcePath(picked);
      }
    } catch {
      // ignore
    }
  };

  const onNext = async () => {
    if (step.key === "location" && selections.location === "remote") {
      if (!parsedRemote) return;
      if (!isDesktopApp()) {
        setRemoteStatus("error");
        setRemoteError("Remote connections require the desktop app.");
        return;
      }
      if (remoteStatus === "connected") {
        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
        return;
      }
      setRemoteStatus("connecting");
      setRemoteError(null);
      try {
        await desktopTestSsh({
          host: parsedRemote.host,
          user: parsedRemote.user ?? null,
        });
        setRemoteStatus("connected");
        const nextRecents = upsertSshRecent(parsedRemote.host, parsedRemote.user ?? null);
        setSshRecents(nextRecents);
        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
      } catch (err: any) {
        setRemoteStatus("error");
        setRemoteError(err?.message ?? String(err));
      }
      return;
    }
    setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
  };

	  return (
	    <div className="launcher-shell launcher-shell--crt">
	      <LauncherBrand fullScreen>
	        <div className="wizard-panel">
	          {infoStep?.info && (
	            <div
	              className="wizard-modal-backdrop"
	              role="dialog"
	              aria-modal="true"
	              aria-label={`${infoStep.title} info`}
	              onClick={() => setOpenInfoKey(null)}
	            >
	              <div className="wizard-modal" onClick={(e) => e.stopPropagation()}>
	                <div className="wizard-modal-header">
	                  <div className="wizard-modal-title">{infoStep.title}</div>
	                  <button
	                    type="button"
	                    className="wizard-modal-close"
	                    aria-label="Close"
	                    onClick={() => setOpenInfoKey(null)}
	                  >
	                    <X size={16} aria-hidden="true" />
	                  </button>
	                </div>
	                <div className="wizard-modal-body">
	                  <div className="wizard-markdown">
	                    <ReactMarkdown remarkPlugins={[remarkGfm]}>
	                      {infoStep.info}
	                    </ReactMarkdown>
	                  </div>
	                </div>
	              </div>
	            </div>
	          )}
	          <div className="wizard-steps">
	            <div className="wizard-step">
	              <div className="wizard-step-header">
	                <div className="wizard-step-title-row">
	                  <div className="wizard-step-title">{step.title}</div>
	                  {step.info && (
	                    <button
	                      type="button"
	                      className="wizard-info-toggle"
	                      onClick={() => setOpenInfoKey((prev) => (prev === step.key ? null : step.key))}
	                      aria-label="Info"
	                    >
	                      <Info size={16} aria-hidden="true" />
	                    </button>
	                  )}
	                </div>
	                <div className="wizard-step-note">{step.note}</div>
	              </div>
	              <div className="wizard-step-body">
                {step.options && (
                  <>
                    <div className="wizard-option-grid">
                      {step.options
                        .filter((option) => step.key !== "container" || !option.advanced)
                        .map((option) => {
                          const selected = selections[step.key] === option.id;
                          return (
                            <button
                              key={option.id}
                              type="button"
                              className={`wizard-option${selected ? " is-selected" : ""}`}
                              onClick={() => onSelectOption(step.key, option.id)}
                              aria-pressed={selected}
                            >
                              <div className="wizard-option-title">
                                <span className="wizard-option-title-text">{option.title}</span>
                                {option.badge && <span className="wizard-option-badge">{option.badge}</span>}
                              </div>
                              <div className="wizard-option-desc">{option.desc}</div>
                            </button>
                          );
                        })}
                    </div>
	                    {step.key === "container" && (
	                      <>
	                        <button
	                          type="button"
	                          className="wizard-advanced-link"
	                          onClick={() => setContainerAdvancedOpen((open) => !open)}
	                          aria-expanded={containerAdvancedOpen}
	                        >
	                          <ChevronRight
	                            size={14}
	                            className={containerAdvancedOpen ? "is-open" : undefined}
	                            aria-hidden="true"
	                          />
	                          Advanced
	                        </button>
	                        {containerAdvancedOpen && (
	                          <div className="wizard-advanced-options">
	                            {step.options
	                              .filter((option) => Boolean(option.advanced))
	                              .map((option) => {
	                                const selected = selections[step.key] === option.id;
	                                return (
	                                  <button
                                    key={option.id}
                                    type="button"
                                    className={`wizard-option${selected ? " is-selected" : ""}`}
                                    onClick={() => onSelectOption(step.key, option.id)}
                                    aria-pressed={selected}
                                  >
                                    <div className="wizard-option-title">
                                      <span className="wizard-option-title-text">{option.title}</span>
                                      {option.badge && <span className="wizard-option-badge">{option.badge}</span>}
                                    </div>
                                    <div className="wizard-option-desc">{option.desc}</div>
                                  </button>
	                                );
	                              })}
	                          </div>
	                        )}
	                      </>
	                    )}
	                  </>
	                )}
	                {step.key === "session-titling" && selections[step.key] === "cloud-api" && (
	                  <div className="wizard-input wizard-cloud-inputs">
	                    <label>
	                      API endpoint
	                      <input
                        placeholder="https://api.openai.com/v1"
                        value={cloudEndpoint}
                        onChange={(e) => setCloudEndpoint(e.target.value)}
                      />
                    </label>
                    <label>
                      API key
                      <input
                        type="password"
                        placeholder="sk-..."
                        value={cloudApiKey}
                        onChange={(e) => setCloudApiKey(e.target.value)}
                      />
	                    </label>
	                  </div>
	                )}
	                {step.key === "location" && selections.location === "remote" && (
	                  <div className="wizard-remote">
	                    <div className="wizard-input">
	                      <label>
                        Remote host
                        <input
                          placeholder="user@host"
                          value={remoteHostInput}
                          onChange={(e) => onRemoteInputChange(e.target.value)}
                        />
                      </label>
                    </div>
                    {sshSuggestions.length > 0 && (
                      <div className="wizard-remote-list">
                        {sshSuggestions.map((entry) => {
                          const label = entry.user ? `${entry.user}@${entry.host}` : entry.host;
                          return (
                            <button
                              key={label}
                              type="button"
                              className="wizard-remote-suggestion"
                              onClick={() => onRemoteInputChange(label)}
                            >
                              {label}
                            </button>
                          );
                        })}
                      </div>
                    )}
                    {remoteStatus === "connecting" && (
                      <div className="wizard-note">Connecting…</div>
                    )}
                    {remoteStatus === "connected" && (
                      <div className="wizard-note">Connection verified.</div>
                    )}
                    {remoteStatus === "error" && remoteError && (
                      <div className="wizard-error">{remoteError}</div>
                    )}
                  </div>
                )}
                {step.key === "source" && needsHostPath && (
                  <div className="wizard-input">
                    <label>
                      {selections.source === "import"
                        ? (containerMode === "sealed" ? "Folder to import" : "Existing folder")
                        : "Destination folder"}
                      <div className="wizard-input-row">
                        <input
                          placeholder="/Users/example-user/project"
                          value={sourcePath}
                          onChange={(e) => setSourcePath(e.target.value)}
                        />
                        {selections.location === "local" && (
                          <button
                            type="button"
                            className="wizard-input-button"
                            onClick={onPickLocalFolder}
                          >
                            Browse
                          </button>
                        )}
                      </div>
                    </label>
                    {selections.location === "remote" && remotePathSuggestions.length > 0 && (
                      <div className="wizard-path-list">
                        {remotePathSuggestions.map((entry) => (
                          <button
                            key={entry.path}
                            type="button"
                            className="wizard-path-suggestion"
                            onClick={() => setSourcePath(`${entry.path}/`)}
                          >
                            {entry.name}
                          </button>
                        ))}
                      </div>
                    )}
                    {selections.location === "remote" && remotePathStatus === "loading" && (
                      <div className="wizard-note">Loading folders…</div>
                    )}
                    {selections.location === "remote" && remotePathStatus === "error" && remotePathError && (
                      <div className="wizard-error">{remotePathError}</div>
                    )}
                  </div>
                )}
	                {step.key === "network" && selections.network === "allowlist" && (
	                  <div className="wizard-input">
	                    <label>
	                      Allowlist (one host per line)
	                      <textarea
	                        placeholder={"registry.npmjs.org\napi.github.com"}
	                        value={networkAllowlist}
	                        onChange={(e) => setNetworkAllowlist(e.target.value)}
	                      />
	                    </label>
	                  </div>
	                )}
                {step.key === "source" && selections.source === "clone" && (
                  <div className="wizard-input">
                    <label>
                      Repo URL
                      <input
                        placeholder="https://github.com/org/repo.git"
                        value={repoUrl}
                        onChange={(e) => setRepoUrl(e.target.value)}
                      />
                    </label>
                    <label>
                      Branch (optional)
                      <input
                        placeholder="main"
                        value={repoBranch}
                        onChange={(e) => setRepoBranch(e.target.value)}
                      />
                    </label>
                  </div>
                )}
                {step.key === "setup" && (
                  <div className="wizard-input">
                    <input
                      placeholder="pnpm install"
                      value={setupHook}
                      onChange={(e) => setSetupHook(e.target.value)}
                    />
                  </div>
                )}
                {step.key === "source" && containerMode === "sealed" && selections.source === "new" && (
                  <div className="wizard-input">
                    <label>
                      Workspace name
                      <input
                        placeholder="workspace"
                        value={workspaceName}
                        onChange={(e) => setWorkspaceName(e.target.value)}
                      />
	                    </label>
                  </div>
                )}
                {step.key === "merge-queue" && (
	                  <div className="wizard-input">
                    <label>
                      Target branch
                      <input
                        placeholder="main"
                        value={targetBranch}
                        onChange={(e) => {
                          enableMergeQueueIfSkipped();
                          setTargetBranch(e.target.value);
                          setTargetBranchTouched(true);
                        }}
                        disabled={mergeQueueSkipped}
                      />
                    </label>
                    <label>
                      Verification command (optional)
                  <input
                        placeholder="pnpm test"
                        value={verifyCommand}
                        onChange={(e) => {
                          enableMergeQueueIfSkipped();
                          setVerifyCommand(e.target.value);
                        }}
                        disabled={mergeQueueSkipped}
                      />
                    </label>
                    <button
                      type="button"
                      className="wizard-advanced-link"
                      onClick={() => setMergeAdvancedOpen((open) => !open)}
                      aria-expanded={mergeAdvancedOpen}
                      disabled={mergeQueueSkipped}
                    >
                      <ChevronRight
                        size={14}
                        className={mergeAdvancedOpen ? "is-open" : undefined}
                        aria-hidden="true"
                      />
                      Advanced
                    </button>
                    {mergeAdvancedOpen && (
                      <div className="wizard-advanced-panel">
                        <label className="wizard-checkbox">
                          <input
                            type="checkbox"
                            checked={pushOnSuccess}
                            onChange={(e) => setPushOnSuccess(e.target.checked)}
                            disabled={mergeQueueSkipped}
                          />
                          Push to remote on success
                        </label>
                        {pushOnSuccess && (
                          <div className="wizard-input">
                            <label>
                              Push remote
                              <input
                                placeholder="origin"
                                value={pushRemote}
                                onChange={(e) => setPushRemote(e.target.value)}
                                disabled={mergeQueueSkipped}
                              />
                            </label>
                            <label>
                              Push branch
                              <input
                                placeholder={targetBranch || "main"}
                                value={pushBranch}
                                onChange={(e) => {
                                  setPushBranch(e.target.value);
                                  setPushBranchTouched(true);
                                }}
                                disabled={mergeQueueSkipped}
                              />
                            </label>
                          </div>
                        )}
                      </div>
                    )}
                    <button
                      type="button"
                      className="wizard-skip wizard-skip--left wizard-skip--below"
                      onClick={() => {
                        onSelect("merge-queue", "skip");
                        setMergeAdvancedOpen(false);
                        setPushOnSuccess(false);
                        setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
                      }}
                    >
                      Skip for now
                    </button>
	                  </div>
	                )}
                {step.key === "confirm" && (
                  <div className="wizard-step-summary">
                    <div className="wizard-summary">
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Location</div>
                        <div className="wizard-summary-v">
                          {selections.location === "remote" ? "Remote" : "Local"}
                          {selections.location === "remote" && remoteHostInput.trim()
                            ? ` (${remoteHostInput.trim()})`
                            : ""}
                        </div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Sandbox</div>
                        <div className="wizard-summary-v">
                          {selections.container === "sealed"
                            ? "Fully sealed container"
                            : selections.container === "host-mounted"
                              ? "Host-mounted container"
                              : "No container"}
                        </div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Source</div>
                        <div className="wizard-summary-v">
                          {selections.source === "clone"
                            ? `Clone repo${repoBranch.trim() ? ` (${repoBranch.trim()})` : ""}`
                            : selections.source === "import"
                              ? "Import folder"
                              : "New empty"}
                        </div>
                      </div>
                      {selections.source === "clone" && repoUrl.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Repo</div>
                          <div className="wizard-summary-v">{repoUrl.trim()}</div>
                        </div>
                      )}
                      {selections.source === "import" && sourcePath.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Folder</div>
                          <div className="wizard-summary-v">{sourcePath.trim()}</div>
                        </div>
                      )}
                      {selections.source === "new" && containerMode === "sealed" && workspaceName.trim() && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Name</div>
                          <div className="wizard-summary-v">{workspaceName.trim()}</div>
                        </div>
                      )}
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Network</div>
                        <div className="wizard-summary-v">
                          {selections.network === "providers"
                            ? "LLM providers only"
                            : selections.network === "allowlist"
                              ? "Allowlist"
                              : "Full access"}
                        </div>
                      </div>
                      {selections.network === "allowlist" && (
                        <div className="wizard-summary-row">
                          <div className="wizard-summary-k">Allowlist</div>
                          <div className="wizard-summary-v">
                            {networkAllowlist
                              .split(/\r?\n/)
                              .map((l) => l.trim())
                              .filter(Boolean)
                              .join(", ") || "(empty)"}
                          </div>
                        </div>
                      )}
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Worktree hook</div>
                        <div className="wizard-summary-v">{setupHook.trim() || "(none)"}</div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Merge queue</div>
                        <div className="wizard-summary-v">
                          {mergeQueueSkipped
                            ? "Disabled"
                            : `Target \`${targetBranch.trim() || "main"}\`${verifyCommand.trim() ? `, verify: ${verifyCommand.trim()}` : ""}`}
                        </div>
                      </div>
                      <div className="wizard-summary-row">
                        <div className="wizard-summary-k">Task titles</div>
                        <div className="wizard-summary-v">
                          {selections["session-titling"] === "local-model"
                            ? "Local model"
                            : selections["session-titling"] === "cloud-api"
                              ? `Cloud API (${cloudEndpoint.trim() || "endpoint not set"})`
                              : "Skip for now"}
                        </div>
                      </div>
                    </div>
                  </div>
                )}
                {step.key === "setup" && (
                  <button
                    type="button"
                    className="wizard-skip wizard-skip--left wizard-skip--below"
	                    onClick={onNext}
	                  >
	                    Skip for now
	                  </button>
	                )}
                {step.key === "session-titling" && (
                  <button
                    type="button"
                    className="wizard-skip wizard-skip--below"
                    onClick={() => {
                      onSelect(step.key, "skip");
                      setStepIndex((idx) => Math.min(steps.length - 1, idx + 1));
                    }}
                  >
                    Skip for now
                  </button>
                )}
              </div>
            </div>
          </div>

          <div className="wizard-pagination" role="tablist" aria-label="Setup steps">
            {(() => {
              const isStepSatisfied = (key: string): boolean => {
                if (key === "location") {
                  if (selections.location === "local") return true;
                  if (selections.location !== "remote") return false;
                  return remoteStatus === "connected" && Boolean(parseUserHost(remoteHostInput)?.host);
                }
                if (key === "container") return Boolean(selections.container);
                if (key === "source") {
                  if (!selections.source) return false;
                  if (selections.source === "clone" && !repoUrl.trim()) return false;
                  if (selections.source === "import" && !sourcePath.trim()) return false;
                  if (containerMode === "sealed" && selections.source === "new" && !workspaceName.trim()) return false;
                  if ((containerMode === "host-mounted" || containerMode === "no-container") && !sourcePath.trim()) {
                    return false;
                  }
                  return true;
                }
                if (key === "network") {
                  if (!selections.network) return false;
                  if (selections.network !== "allowlist") return true;
                  return (
                    networkAllowlist
                      .split(/\r?\n/)
                      .map((l) => l.trim())
                      .filter(Boolean).length > 0
                  );
                }
                if (key === "merge-queue") return mergeQueueSkipped || Boolean(targetBranch.trim());
                if (key === "session-titling") {
                  if (!selections["session-titling"]) return false;
                  if (selections["session-titling"] !== "cloud-api") return true;
                  return Boolean(cloudEndpoint.trim() && cloudApiKey.trim());
                }
                if (key === "setup") return true;
                if (key === "confirm") return true;
                return true;
              };

              let maxIdx = 0;
              for (let i = 0; i < steps.length; i++) {
                if (isStepSatisfied(steps[i].key)) {
                  maxIdx = Math.min(steps.length - 1, i + 1);
                } else {
                  maxIdx = Math.max(0, i);
                  break;
                }
              }

              return steps.map((item, idx) => {
                const disabled = idx > maxIdx;
                return (
                  <button
                    key={item.key}
                    type="button"
                    className={`wizard-dot${idx === stepIndex ? " is-active" : ""}`}
                    aria-label={`Go to step ${idx + 1}`}
                    aria-current={idx === stepIndex ? "true" : undefined}
                    disabled={disabled}
                    onClick={() => {
                      if (!disabled) setStepIndex(idx);
                    }}
                  />
                );
              });
            })()}
          </div>

          <div className="wizard-actions">
            {isFirst ? (
              <Link to="/" className="wizard-secondary">Back</Link>
            ) : (
              <button type="button" className="wizard-secondary" onClick={() => setStepIndex((idx) => Math.max(0, idx - 1))}>
                Back
              </button>
            )}
            {isLast ? (
              <button type="button" className="wizard-primary" disabled={!canAdvance}>Create workspace</button>
            ) : (
              <button
                type="button"
                className="wizard-primary"
                disabled={!canAdvance}
                onClick={onNext}
              >
                Next
              </button>
            )}
          </div>
        </div>
      </LauncherBrand>
    </div>
  );
}
