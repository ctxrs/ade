export type WorkbenchMessageListContext = {
  loaded: boolean;
  loadingOlder: boolean;
  renderRevision?: string;
  renderRevisionByItemId?: Readonly<Record<string, number>>;
  expandedTurnHeaders?: Readonly<Record<string, boolean>>;
  expandedTurnDetailsById?: Readonly<Record<string, boolean>>;
  expandedToolById?: Readonly<Record<string, boolean>>;
  expandedMessageById?: Readonly<Record<string, boolean>>;
  turnToolsLoading?: readonly string[];
  verbosity?: string;
};
