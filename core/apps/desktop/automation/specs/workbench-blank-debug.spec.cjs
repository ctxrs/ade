const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const { waitForTauri, waitForTestId } = require("./helpers/tauri.cjs");

const runChecked = (cmd, args) => {
  const res = spawnSync(cmd, args, { encoding: "utf8" });
  if (res.status === 0) return;
  throw new Error(
    [
      `command failed: ${cmd} ${args.join(" ")}`,
      String(res.stderr || "").trim(),
      String(res.stdout || "").trim(),
    ].filter(Boolean).join("\n"),
  );
};

const initTempImportRepo = () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-workbench-debug-"));
  runChecked("git", ["init", "--", root]);
  runChecked("git", ["-C", root, "config", "user.email", "ctx-e2e@example.com"]);
  runChecked("git", ["-C", root, "config", "user.name", "ctx-e2e"]);
  fs.writeFileSync(path.join(root, "README.md"), "# debug\n", "utf8");
  runChecked("git", ["-C", root, "add", "README.md"]);
  runChecked("git", ["-C", root, "commit", "-m", "init"]);
  return root;
};

const clickIfEnabled = async (selector) => {
  return await browser.execute((s) => {
    const el = document.querySelector(s);
    if (!(el instanceof HTMLButtonElement)) return false;
    if (el.disabled) return false;
    el.click();
    return true;
  }, selector);
};

const clickSelector = async (selector) => {
  return await browser.execute((s) => {
    const el = document.querySelector(s);
    if (!el) return false;
    el.click();
    return true;
  }, selector);
};

const setInputSelector = async (selector, value) => {
  return await browser.execute((s, v) => {
    const el = document.querySelector(s);
    if (!(el instanceof HTMLInputElement)) return false;
    const desc = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
    const setter = desc && desc.set;
    if (!setter) return false;
    setter.call(el, String(v));
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  }, selector, value);
};

const readStepState = async () => {
  return await browser.execute(() => {
    const root = document.querySelector('[data-testid="workspace-setup"]');
    const step = root ? root.getAttribute("data-step-key") : null;
    const pathname = window.location.pathname;
    const nextBtn = document.querySelector('[data-testid="wizard-next"]');
    const createBtn = document.querySelector('[data-testid="wizard-create"]');
    return {
      pathname,
      step,
      nextDisabled: nextBtn instanceof HTMLButtonElement ? nextBtn.disabled : null,
      createDisabled: createBtn instanceof HTMLButtonElement ? createBtn.disabled : null,
      hasSourcePath: Boolean(document.querySelector('[data-testid="wizard-source-path"]')),
      hasSourceImport: Boolean(document.querySelector('[data-testid="wizard-option-source-import"]')),
      hasContainerHost: Boolean(document.querySelector('[data-testid="wizard-option-container-host"]')),
      hasContainerSandbox: Boolean(document.querySelector('[data-testid="wizard-option-container-sandbox"]')),
    };
  });
};

const collectWorkbenchDiagnostics = async () => {
  return await browser.execute(() => {
    const bodyText = String(document.body?.innerText || "").trim();
    const wal = window.__CTX_WAL__;
    const rawEvents = wal?.dump?.({ limit: 500 }) ?? [];
    const walEvents = Array.isArray(rawEvents)
      ? rawEvents
          .filter((entry) => {
            if (!entry || typeof entry !== "object") return false;
            const kind = String(entry.kind || "");
            if (kind === "error" || kind === "unhandledrejection" || kind === "resource:error") return true;
            if (kind !== "console") return false;
            const level = String(entry.data?.level || "");
            return level === "error" || level === "warn";
          })
          .slice(-40)
      : [];
    const debugErrors = Array.isArray(window.__ctxDebugErrors)
      ? window.__ctxDebugErrors.slice(-30)
      : [];
    return {
      pathname: window.location.pathname,
      href: window.location.href,
      title: document.title,
      bodySnippet: bodyText.slice(0, 1400),
      bodyHtmlSnippet: String(document.body?.innerHTML || "").slice(0, 1800),
      hasRootDiv: Boolean(document.getElementById("root")),
      hasWorkbenchRoot: Boolean(document.querySelector(".wb-root")),
      hasWorkbenchMain: Boolean(document.querySelector(".wb-main")),
      hasTopbar: Boolean(document.querySelector(".wb-topbar")),
      hasTaskSearch: Boolean(document.querySelector('[data-testid="workbench-task-search"]')),
      hasComposer: Boolean(document.querySelector("textarea.wb-new-composer-textarea")),
      topbarTitle: String(document.querySelector(".wb-topbar-title")?.textContent || "").trim(),
      walStatus: wal?.getStatus?.() ?? null,
      walEvents,
      debugErrors,
    };
  });
};

describe("workbench blank debug (e2e)", () => {
  let importRepo = "";

  before(() => {
    importRepo = initTempImportRepo();
  });

  after(() => {
    if (importRepo) {
      fs.rmSync(importRepo, { recursive: true, force: true });
    }
  });

  it("creates a local workspace and asserts workbench renders", async () => {
    await browser.url(`tauri://localhost/workspace-setup?debug=${Date.now()}`);
    await waitForTauri();
    await waitForTestId("workspace-setup", 60000);

    await browser.execute(() => {
      try { localStorage.clear(); } catch {}
      try { sessionStorage.clear(); } catch {}
    });
    await browser.execute(() => {
      const globalRef = window;
      if (globalRef.__ctxDebugErrorsInstalled === true) return;
      globalRef.__ctxDebugErrorsInstalled = true;
      globalRef.__ctxDebugErrors = [];
      const push = (kind, payload) => {
        try {
          globalRef.__ctxDebugErrors.push({
            ts: Date.now(),
            kind,
            payload,
          });
          if (globalRef.__ctxDebugErrors.length > 200) {
            globalRef.__ctxDebugErrors.splice(0, globalRef.__ctxDebugErrors.length - 200);
          }
        } catch {
          // ignore
        }
      };
      window.addEventListener("error", (event) => {
        const target = event.target;
        push("error", {
          message: event.message,
          filename: event.filename,
          lineno: event.lineno,
          colno: event.colno,
          stack: event.error instanceof Error ? event.error.stack : undefined,
          targetTag: target && typeof target === "object" && "tagName" in target ? String(target.tagName) : undefined,
        });
      });
      window.addEventListener("unhandledrejection", (event) => {
        const reason = event.reason;
        push("unhandledrejection", {
          reason:
            reason instanceof Error
              ? { name: reason.name, message: reason.message, stack: reason.stack }
              : String(reason),
        });
      });
      const originalConsoleError = window.console?.error;
      if (typeof originalConsoleError === "function") {
        window.console.error = (...args) => {
          push("console.error", {
            args: args.map((arg) => {
              if (arg instanceof Error) {
                return { name: arg.name, message: arg.message, stack: arg.stack };
              }
              try {
                return JSON.parse(JSON.stringify(arg));
              } catch {
                return String(arg);
              }
            }),
          });
          return Reflect.apply(originalConsoleError, window.console, args);
        };
      }
    });

    const deadline = Date.now() + 180000;
    while (Date.now() < deadline) {
      const state = await browser.execute((repoPath) => {
        const pathname = window.location.pathname;
        const root = document.querySelector('[data-testid="workspace-setup"]');
        const step = root ? root.getAttribute("data-step-key") : null;
        if (!pathname.startsWith("/workspaces/")) {
          const click = (selector) => {
            const el = document.querySelector(selector);
            if (!el) return false;
            if (el instanceof HTMLButtonElement && el.disabled) return false;
            el.click();
            return true;
          };
          const setInput = (selector, value) => {
            const el = document.querySelector(selector);
            if (!(el instanceof HTMLInputElement)) return false;
            const desc = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value");
            const setter = desc && desc.set;
            if (!setter) return false;
            setter.call(el, String(value));
            el.dispatchEvent(new Event("input", { bubbles: true }));
            el.dispatchEvent(new Event("change", { bubbles: true }));
            return true;
          };
          if (step === "location") {
            click('[data-testid="wizard-option-location-local"]');
          } else if (step === "container") {
            if (!click('[data-testid="wizard-option-container-host"]')) {
              click('[data-testid="wizard-option-container-sandbox"]');
            }
          } else if (step === "source") {
            click('[data-testid="wizard-option-source-import"]');
            setInput('[data-testid="wizard-source-path"]', String(repoPath || ""));
          } else if (step === "auth-import") {
            const stepRoot = document.querySelector('[data-testid="wizard-step"][data-step-key="auth-import"]');
            const skip = stepRoot
              ? Array.from(stepRoot.querySelectorAll("button"))
                  .find((btn) => String(btn.textContent || "").trim() === "Skip for now")
              : null;
            if (skip) skip.click();
          } else if (step === "session-titling") {
            click('[data-testid="wizard-titling-skip"]');
          } else if (step === "merge-queue") {
            click('[data-testid="wizard-merge-skip"]');
          }
          if (!click('[data-testid="wizard-create"]')) {
            click('[data-testid="wizard-next"]');
          }
        }
        return { pathname, step };
      }, importRepo);
      if (String(state.pathname || "").startsWith("/workspaces/")) break;
      await browser.pause(300);
    }

    const createdWorkspaceId = await browser.waitUntil(async () => {
      const pathname = await browser.execute(() => window.location.pathname);
      if (!String(pathname || "").startsWith("/workspaces/")) return false;
      const id = String(pathname || "").split("/")[2] || "";
      return id || false;
    }, {
      timeout: 60000,
      interval: 150,
      timeoutMsg: "did not navigate to /workspaces/:id after wizard submit",
    });

    let rendered = false;
    let lastDiag = null;
    try {
      await browser.waitUntil(async () => {
        const diag = await collectWorkbenchDiagnostics();
        lastDiag = diag;
        return Boolean(diag.hasWorkbenchMain || diag.hasTaskSearch || diag.hasComposer);
      }, {
        timeout: 30000,
        interval: 200,
        timeoutMsg: "workbench did not render expected elements",
      });
      rendered = true;
    } catch {
      rendered = false;
    }

    let finalDiag = lastDiag;
    let finalDiagError = null;
    try {
      finalDiag = await collectWorkbenchDiagnostics();
    } catch (error) {
      finalDiagError = String(error);
    }
    if (!rendered) {
      throw new Error(
        `workbench appears blank after wizard (workspace=${String(createdWorkspaceId)}): ${JSON.stringify({ finalDiag, finalDiagError })}`,
      );
    }
  });
});
