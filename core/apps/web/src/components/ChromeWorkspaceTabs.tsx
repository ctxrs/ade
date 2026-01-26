import React, { useEffect, useMemo, useRef } from "react";
import ChromeTabs from "chrome-tabs";

import "chrome-tabs/css/chrome-tabs.css";
import "chrome-tabs/css/chrome-tabs-dark-theme.css";

export type ChromeWorkspaceTab = {
  id: string;
  title: string;
  favicon?: string | false;
};

type ChromeTabsEventDetail = {
  tabEl: HTMLElement;
};

export function ChromeWorkspaceTabs({
  tabs,
  activeId,
  onActivate,
  onClose,
  onReorder,
  theme = "dark",
}: {
  tabs: ChromeWorkspaceTab[];
  activeId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onReorder?: (ids: string[]) => void;
  theme?: "dark" | "light";
}) {
  const elRef = useRef<HTMLDivElement | null>(null);
  const chromeTabsRef = useRef<ChromeTabs | null>(null);
  const syncingRef = useRef(false);

  const tabsKey = useMemo(() => tabs.map((t) => t.id).join(","), [tabs]);

  useEffect(() => {
    const el = elRef.current;
    if (!el) return;

    // The ChromeTabs lib attaches a dblclick handler that adds a new tab.
    // In ctx we use an explicit "Open workspace" button, so suppress dblclick.
    const suppressDblClick = (e: MouseEvent) => {
      e.preventDefault();
      e.stopImmediatePropagation();
    };
    el.addEventListener("dblclick", suppressDblClick, true);

    const chromeTabs = new ChromeTabs();
    chromeTabs.init(el);
    chromeTabsRef.current = chromeTabs;

    const onActiveTabChange: EventListener = (e) => {
      if (syncingRef.current) return;
      const tabEl = (e as CustomEvent<ChromeTabsEventDetail>)?.detail?.tabEl;
      const id = tabEl?.getAttribute("data-tab-id") ?? "";
      if (id) onActivate(id);
    };
    const onTabRemove: EventListener = (e) => {
      if (syncingRef.current) return;
      const tabEl = (e as CustomEvent<ChromeTabsEventDetail>)?.detail?.tabEl;
      const id = tabEl?.getAttribute("data-tab-id") ?? "";
      if (id) onClose(id);
    };
    const onTabReorder: EventListener = () => {
      if (syncingRef.current) return;
      if (!onReorder) return;
      const ids = Array.from(el.querySelectorAll<HTMLElement>(".chrome-tab[data-tab-id]"))
        .map((n) => n.getAttribute("data-tab-id") ?? "")
        .filter((v) => v.trim());
      onReorder(ids);
    };

    el.addEventListener("activeTabChange", onActiveTabChange);
    el.addEventListener("tabRemove", onTabRemove);
    el.addEventListener("tabReorder", onTabReorder);

    return () => {
      try {
        el.removeEventListener("dblclick", suppressDblClick, true);
        el.removeEventListener("activeTabChange", onActiveTabChange);
        el.removeEventListener("tabRemove", onTabRemove);
        el.removeEventListener("tabReorder", onTabReorder);
      } catch {
        // ignore
      }
      chromeTabsRef.current = null;
    };
  }, [onActivate, onClose, onReorder]);

  useEffect(() => {
    const el = elRef.current;
    const chromeTabs = chromeTabsRef.current;
    if (!el || !chromeTabs) return;

    syncingRef.current = true;
    try {
      const byId = new Map<string, HTMLElement>();
      for (const tabEl of Array.from(el.querySelectorAll<HTMLElement>(".chrome-tab[data-tab-id]"))) {
        const id = tabEl.getAttribute("data-tab-id") ?? "";
        if (id) byId.set(id, tabEl);
      }

      const desiredIds = new Set(tabs.map((t) => t.id));

      for (const [id, tabEl] of byId.entries()) {
        if (!desiredIds.has(id)) {
          try {
            chromeTabs.removeTab(tabEl);
          } catch {
            tabEl.parentElement?.removeChild(tabEl);
          }
          byId.delete(id);
        }
      }

      for (const t of tabs) {
        if (byId.has(t.id)) continue;
        chromeTabs.addTab(
          { id: t.id, title: t.title, favicon: t.favicon ?? false },
          { animate: false, background: true },
        );
      }

      for (const t of tabs) {
        const tabEl =
          byId.get(t.id) ?? (el.querySelector(`.chrome-tab[data-tab-id="${CSS.escape(t.id)}"]`) as HTMLElement | null);
        if (!tabEl) continue;
        chromeTabs.updateTab(tabEl, { id: t.id, title: t.title, favicon: t.favicon ?? false });
        byId.set(t.id, tabEl);
      }

      const parent = el.querySelector(".chrome-tabs-content");
      if (parent) {
        const currentOrder = Array.from(parent.querySelectorAll<HTMLElement>(".chrome-tab[data-tab-id]"))
          .map((n) => n.getAttribute("data-tab-id") ?? "")
          .filter((v) => v.trim());
        const desiredOrder = tabs.map((t) => t.id);
        if (currentOrder.join(",") !== desiredOrder.join(",")) {
          for (const id of desiredOrder) {
            const node = byId.get(id);
            if (!node) continue;
            parent.appendChild(node);
          }
        }
      }

      const active = activeId ? byId.get(activeId) : null;
      if (active) chromeTabs.setCurrentTab(active);
      chromeTabs.layoutTabs();
      chromeTabs.setupDraggabilly();

      for (const node of Array.from(
        el.querySelectorAll<HTMLElement>(".chrome-tab, .chrome-tab-content, .chrome-tab-close, .chrome-tab-drag-handle"),
      )) {
        node.setAttribute("data-tauri-drag-region", "false");
      }
    } finally {
      syncingRef.current = false;
    }
  }, [activeId, tabsKey, tabs]);

  return (
    <div
      ref={elRef}
      className={`chrome-tabs ${theme === "dark" ? "chrome-tabs-dark-theme" : ""}`}
    >
      <div className="chrome-tabs-content" />
      <div className="chrome-tabs-bottom-bar" />
    </div>
  );
}
