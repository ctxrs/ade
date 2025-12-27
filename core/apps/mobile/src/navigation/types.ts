export type RootStackParamList = {
  Connection: undefined;
  Workbench: undefined;
  NewTask: undefined;
  Workspaces: undefined;
  Tasks: { workspaceId: string; workspaceName: string };
  Tracks: { taskId: string; taskTitle: string };
  Sessions: { trackId: string; trackLabel: string };
  SessionDetail: { sessionId: string; sessionTitle: string };
  TrackDiff: { trackId: string; trackLabel: string };
  Diagnostics: undefined;
  Settings: undefined;
  QrScanner: undefined;
};
