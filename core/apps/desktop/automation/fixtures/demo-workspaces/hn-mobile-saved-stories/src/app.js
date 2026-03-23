const STORAGE_KEY = "hn-mobile.saved-pages";
const DEFAULT_PAGE = Object.freeze({
  id: "item-47470773",
  location: "news.ycombinator.com/item?id=47470773",
  path: "/proxy/hn/item?id=47470773",
  subtitle: "Regular Hacker News comments, proxied into the local shell for deterministic demo recording.",
  title: "Tinybox – Offline AI device 120B parameters",
});

const pagesById = new Map([[DEFAULT_PAGE.id, DEFAULT_PAGE]]);
const state = {
  currentPageId: DEFAULT_PAGE.id,
  savedPageIds: loadSavedPageIds(),
  savedSheetOpen: false,
};

function loadSavedPageIds() {
  try {
    const savedValue = window.localStorage.getItem(STORAGE_KEY);
    const parsed = savedValue ? JSON.parse(savedValue) : [];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((pageId) => pagesById.has(pageId));
  } catch {
    return [];
  }
}

function persistSavedPageIds() {
  window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state.savedPageIds));
}

function currentPage() {
  return pagesById.get(state.currentPageId) ?? DEFAULT_PAGE;
}

function isPageSaved(pageId) {
  return state.savedPageIds.includes(pageId);
}

function isCurrentPageSaved() {
  return isPageSaved(currentPage().id);
}

function savedPages() {
  return state.savedPageIds.map((pageId) => pagesById.get(pageId)).filter(Boolean);
}

function toggleSavedCurrentPage() {
  const page = currentPage();
  if (isCurrentPageSaved()) {
    state.savedPageIds = state.savedPageIds.filter((pageId) => pageId !== page.id);
  } else {
    state.savedPageIds = [page.id, ...state.savedPageIds];
  }
  persistSavedPageIds();
}

function browseMarkup(page) {
  return `
    <iframe
      class="hn-page-frame"
      loading="eager"
      referrerpolicy="origin"
      src="${page.path}"
      title="${page.title}"
    ></iframe>
  `;
}

function savedSheetMarkup() {
  const savedEntries = savedPages();
  if (!savedEntries.length) {
    return `
      <div class="saved-sheet-root" data-saved-sheet="true">
        <button class="saved-sheet-scrim" data-close-saved-sheet="true" type="button" aria-label="Close saved pages"></button>
        <section class="saved-sheet" role="dialog" aria-modal="true" aria-label="Saved pages">
          <div class="saved-sheet-header">
            <div class="saved-sheet-copy">
              <div class="saved-sheet-title">Saved pages</div>
              <p>Save a Hacker News page and it will show up here.</p>
            </div>
            <button class="saved-sheet-dismiss" data-close-saved-sheet="true" type="button">Close</button>
          </div>

          <section class="saved-sheet-empty">
            <div class="saved-sheet-empty-title">Nothing saved yet</div>
            <p>Use the floating Save button to keep this HN page handy.</p>
          </section>
        </section>
      </div>
    `;
  }

  return `
    <div class="saved-sheet-root" data-saved-sheet="true">
      <button class="saved-sheet-scrim" data-close-saved-sheet="true" type="button" aria-label="Close saved pages"></button>
      <section class="saved-sheet" role="dialog" aria-modal="true" aria-label="Saved pages">
        <div class="saved-sheet-header">
          <div class="saved-sheet-copy">
            <div class="saved-sheet-title">Saved pages</div>
            <p>Native local state around regular Hacker News.</p>
          </div>
          <button class="saved-sheet-dismiss" data-close-saved-sheet="true" type="button">Close</button>
        </div>

        <div class="saved-sheet-list">
        ${savedEntries
          .map(
            (page) => `
              <button class="saved-sheet-entry" data-saved-entry-id="${page.id}" type="button">
                <div class="saved-sheet-entry-copy">
                  <div class="saved-sheet-entry-title">${page.title}</div>
                  <div class="saved-sheet-entry-url">${page.location}</div>
                </div>

                <span class="saved-sheet-entry-cta">Open</span>
              </button>
            `,
          )
          .join("")}
        </div>
      </section>
    </div>
  `;
}

function render(root) {
  const page = currentPage();
  const savedCount = state.savedPageIds.length;
  const currentPageSaved = isCurrentPageSaved();

  root.innerHTML = `
    <div class="hn-mobile-app">
      ${browseMarkup(page)}
      <div class="floating-controls" aria-label="Native controls">
        <button
          aria-pressed="${currentPageSaved}"
          class="floating-button floating-button-accent${currentPageSaved ? " floating-button-active" : ""}"
          data-save-current-page="true"
          type="button"
        >
          ${currentPageSaved ? "Saved" : "Save"}
        </button>
        <button
          aria-expanded="${state.savedSheetOpen}"
          class="floating-button floating-button-neutral"
          data-open-saved-sheet="true"
          type="button"
        >
          Saved (${savedCount})
        </button>
      </div>
      ${state.savedSheetOpen ? savedSheetMarkup() : ""}
    </div>
  `;

  root.querySelector('[data-save-current-page="true"]')?.addEventListener("click", () => {
    toggleSavedCurrentPage();
    render(root);
  });

  root.querySelector('[data-open-saved-sheet="true"]')?.addEventListener("click", () => {
    state.savedSheetOpen = true;
    render(root);
  });

  root.querySelectorAll("[data-close-saved-sheet]").forEach((button) => {
    button.addEventListener("click", () => {
      state.savedSheetOpen = false;
      render(root);
    });
  });

  root.querySelectorAll("[data-saved-entry-id]").forEach((button) => {
    button.addEventListener("click", () => {
      const nextPageId = button.getAttribute("data-saved-entry-id");
      if (!nextPageId || !pagesById.has(nextPageId)) return;
      state.currentPageId = nextPageId;
      state.savedSheetOpen = false;
      render(root);
    });
  });
}

export function mountApp(root) {
  render(root);
}
