export type DropScope = {
  element: HTMLElement;
  accepts?: (dt: DataTransfer | null) => boolean;
  onDragOver?: (dt: DataTransfer | null, ev: DragEvent) => void;
  onDrop?: (dt: DataTransfer | null, ev: DragEvent) => void;
};

const scopes = new Map<HTMLElement, DropScope>();
let listenersInstalled = false;

function dataTransferTypes(dt: DataTransfer | null): string[] {
  if (!dt) return [];
  const types = dt.types;
  if (!types) return [];
  if (Array.isArray(types)) return types.map(String);
  try {
    return Array.from(types as ArrayLike<string>).map(String);
  } catch {
    return [];
  }
}

function hasFileLikeItem(dt: DataTransfer | null): boolean {
  if (!dt) return false;
  if (dt.files && dt.files.length > 0) return true;
  const items = dt.items;
  if (items && items.length > 0) {
    for (const item of Array.from(items)) {
      if (item.kind === "file") return true;
    }
  }
  const types = dataTransferTypes(dt);
  return (
    types.includes("Files") ||
    types.includes("application/x-moz-file") ||
    // Safari / WebKit variants:
    types.includes("public.file-url") ||
    types.includes("public.url")
  );
}

function hasImageUrlLike(dt: DataTransfer | null): boolean {
  const types = dataTransferTypes(dt);
  return types.includes("text/uri-list") || types.includes("text/html");
}

function defaultAccepts(dt: DataTransfer | null): boolean {
  return hasFileLikeItem(dt) || hasImageUrlLike(dt);
}

function scopeForEvent(ev: DragEvent): DropScope | null {
  const doc = ev.view?.document ?? document;
  const x = typeof ev.clientX === "number" ? ev.clientX : 0;
  const y = typeof ev.clientY === "number" ? ev.clientY : 0;
  const pointEl =
    x || y ? (doc.elementFromPoint(x, y) as Element | null) : (ev.target instanceof Element ? ev.target : null);
  if (!pointEl) return null;

  let el: Element | null = pointEl;
  while (el) {
    if (el instanceof HTMLElement) {
      const s = scopes.get(el);
      if (s) return s;
    }
    el = el.parentElement;
  }
  return null;
}

function ensureListeners() {
  if (listenersInstalled) return;
  listenersInstalled = true;

  const onDragOver = (ev: DragEvent) => {
    const scope = scopeForEvent(ev);
    if (!scope) return;
    ev.preventDefault();
    try {
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = "copy";
    } catch {}
    const accepts = (scope.accepts ?? defaultAccepts)(ev.dataTransfer);
    if (accepts) scope.onDragOver?.(ev.dataTransfer, ev);
  };

  const onDrop = (ev: DragEvent) => {
    const scope = scopeForEvent(ev);
    if (!scope) return;
    ev.preventDefault();
    ev.stopPropagation();
    const accepts = (scope.accepts ?? defaultAccepts)(ev.dataTransfer);
    if (accepts) scope.onDrop?.(ev.dataTransfer, ev);
  };

  window.addEventListener("dragover", onDragOver, true);
  window.addEventListener("drop", onDrop, true);
}

export function registerDropScope(scope: DropScope): () => void {
  ensureListeners();
  scopes.set(scope.element, scope);
  return () => {
    scopes.delete(scope.element);
  };
}
