declare module "chrome-tabs" {
  type AddTabOptions = { animate?: boolean; background?: boolean };
  type TabProperties = { id?: string; title?: string; favicon?: string | false };

  export default class ChromeTabs {
    init(el: HTMLElement): void;
    addTab(tabProperties?: TabProperties, options?: AddTabOptions): void;
    removeTab(tabEl: HTMLElement): void;
    updateTab(tabEl: HTMLElement, tabProperties?: TabProperties): void;
    setCurrentTab(tabEl: HTMLElement): void;
    layoutTabs(): void;
    setupDraggabilly(): void;
  }
}

