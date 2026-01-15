export type RootStackParamList = {
  Connection: undefined;
  Workbench: undefined;
  NewTask: undefined;
  Workspaces: undefined;
  Tasks: { workspaceId: string; workspaceName: string };
  Sessions: { taskId: string; taskTitle: string };
  SessionDetail: { sessionId: string; sessionTitle: string };
  SessionDiff: { sessionId: string; sessionTitle: string };
  Diagnostics: undefined;
  Settings: undefined;
  QrScanner: undefined;
};
