import PhotosUI
import SwiftUI
import _Concurrency
import UIKit
import UniformTypeIdentifiers

struct WorkbenchShellView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var workspaceSelection: WorkspaceSelectionStore
    @EnvironmentObject private var workbenchSelection: WorkbenchSelectionStore
    @EnvironmentObject private var workspaceVisibility: WorkspaceVisibilityStore
    @State private var isDrawerOpen = false
    @State private var workspaces: [WorkspaceSummary] = []
    @State private var isLoadingWorkspaces = false
    @State private var workspaceError: String?
    @State private var activeTasks: [WorkspaceTaskSummary] = []
    @State private var archivedTasks: [WorkspaceTaskSummary] = []
    @State private var isLoadingTasks = false
    @State private var isRefreshingTasks = false
    @State private var taskError: String?
    @State private var lastWorkspaceId: String?
    @State private var sessionCreationTaskId: String?
    @State private var didResetSelection = false
    @State private var taskQuery = ""
    @State private var shouldAutoOpenDrawer = ProcessInfo.processInfo.environment["CTX_OPEN_DRAWER_ON_LAUNCH"] == "1"
    @State private var renameTarget: WorkspaceTaskSummary?
    @State private var renameText: String = ""
    @State private var renameError: String?
    @State private var isRenaming = false
    @State private var archiveInFlight: Set<String> = []
    @State private var markReadInFlight: Set<String> = []
    @State private var deleteInFlight: Set<String> = []
    @State private var deleteAlert: WorkbenchDeleteAlert?
    @State private var artifactsCount = 0
    @State private var activePanel: WorkbenchPanel?
    @State private var topBarAlert: WorkbenchTopBarAlert?
    @State private var sharePayload: SharePayload?
    @State private var streamTask: _Concurrency.Task<Void, Never>?
    @State private var streamSocket: URLSessionWebSocketTask?
    @State private var streamReconnectDelay: TimeInterval = 1
    @State private var isStreamConnected = false
    @State private var streamSecureContext: SecureConnectionContext?
    @State private var lastStreamSnapshotRev: Int = 0
    @State private var lastArchivedRev: Int = 0
    @State private var taskRowIndicatorCache = TaskRowIndicatorCache()
    @State private var taskRowIndicators: [String: TaskRowIndicators] = [:]

    private let streamClient = DaemonStreamClient()
    private let streamEncoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return encoder
    }()
    private let streamDecoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    private var activeSheetPanel: Binding<WorkbenchPanel?> {
        Binding(
            get: {
                switch activePanel {
                case .diff, .sessions, .terminal:
                    return activePanel
                case .artifacts, .none:
                    return nil
                }
            },
            set: { newValue in
                activePanel = newValue
            }
        )
    }

    var body: some View {
        GeometryReader { proxy in
            let drawerWidth = min(320, proxy.size.width * 0.78)
            let drawerHiddenOffset = drawerWidth + 32
            let taskTitle = resolvedTaskTitle()
            let conversationMenuContext = resolvedConversationMenuContext()
            let isArchived = selectedTask?.task.archivedAt != nil

            ZStack(alignment: .leading) {
                CtxBackgroundView()

                WorkbenchHomeView(
                    selectedWorkspace: selectedWorkspace,
                    isLoadingWorkspaces: isLoadingWorkspaces,
                    workspaceError: workspaceError,
                    isLoadingTasks: isLoadingTasks,
                    taskError: taskError,
                    selectedSession: resolvedSession,
                    activePanel: $activePanel,
                    artifactsCount: $artifactsCount,
                    hasTaskSelection: workbenchSelection.taskId != nil,
                    isArchived: isArchived,
                    isPreparingSession: isPreparingSession,
                    onTaskCreated: { _Concurrency.Task { await loadTasks() } }
                )
                .safeAreaInset(edge: .top, spacing: 0) {
                    WorkbenchTopBar(
                        title: taskTitle,
                        onMenuTap: {
                            dismissKeyboard()
                            isDrawerOpen = true
                        },
                        onArtifactsTap: { handleTopBarAction(.artifacts) },
                        artifactsCount: artifactsCount,
                        onDiffTap: { handleTopBarAction(.diff) },
                        onSessionsTap: { handleTopBarAction(.sessions) },
                        onTerminalTap: { handleTopBarAction(.terminal) },
                        activePanel: activePanel,
                        isSessionsEnabled: resolvedSession != nil,
                        conversationMenuContext: conversationMenuContext,
                        showsTaskActions: workbenchSelection.taskId != nil
                    )
                    .padding(.horizontal, 20)
                    .padding(.top, 8)
                    .padding(.bottom, 8)
                }
                .blur(radius: isDrawerOpen ? 8 : 0)
                .overlay {
                    if isDrawerOpen {
                        Color.black.opacity(0.35)
                            .ignoresSafeArea()
                            .onTapGesture { isDrawerOpen = false }
                            .accessibilityIdentifier("drawer.scrim")
                    }
                }

                WorkbenchDrawerView(
                    workspaces: workspaces,
                    selectedWorkspace: selectedWorkspace,
                    isLoadingWorkspaces: isLoadingWorkspaces,
                    workspaceError: workspaceError,
                    activeTasks: activeTasks,
                    archivedTasks: archivedTasks,
                    isLoadingTasks: isLoadingTasks,
                    isRefreshingTasks: isRefreshingTasks,
                    taskError: taskError,
                    activeTaskId: workbenchSelection.taskId,
                    taskRowIndicators: taskRowIndicators,
                    taskQuery: $taskQuery,
                    drawerWidth: drawerWidth,
                    onRefresh: { _Concurrency.Task { await loadWorkspaces() } },
                    onSelectTask: { task in
                        selectTask(task)
                        isDrawerOpen = false
                    },
                    onNewTask: {
                        workbenchSelection.clearSelection()
                        isDrawerOpen = false
                    },
                    onRenameTask: { task in
                        beginRename(task)
                    },
                    onArchiveToggle: { task in
                        _Concurrency.Task { await toggleArchive(task) }
                    },
                    markReadInFlight: markReadInFlight,
                    deleteInFlight: deleteInFlight,
                    onToggleReadState: { task in
                        let isWorking = taskHasWorkingSession(task)
                        let isUnread = taskHasUnread(task, isWorking: isWorking)
                        _Concurrency.Task { await toggleReadState(taskId: task.task.id.stringValue, markRead: isUnread) }
                    },
                    onDeleteTask: { task in
                        deleteAlert = WorkbenchDeleteAlert(taskId: task.task.id.stringValue, title: task.task.title)
                    }
                )
                .offset(x: isDrawerOpen ? 0 : -drawerHiddenOffset)
            }
            .animation(.spring(response: 0.35, dampingFraction: 0.85), value: isDrawerOpen)
        }
        .navigationBarBackButtonHidden(true)
        .toolbar(.hidden, for: .navigationBar)
        .task {
            await loadWorkspaces()
        }
        .task(id: selectedWorkspace?.id) {
            await loadTasks()
            await refreshWorkspaceStream()
        }
        .onAppear {
            if shouldAutoOpenDrawer {
                isDrawerOpen = true
            }
        }
        .onChange(of: connection.isConnected) { connected in
            guard connected else {
                stopWorkspaceStream()
                return
            }
            _Concurrency.Task {
                await loadWorkspaces()
                await loadTasks()
                await refreshWorkspaceStream()
            }
        }
        .onChange(of: workbenchSelection.taskId) { _ in
            resolveSelectionForCurrentTask()
        }
        .onChange(of: resolvedSession?.id) { _, newId in
            if newId == nil {
                artifactsCount = 0
            }
        }
        .onDisappear {
            stopWorkspaceStream()
        }
        .alert(item: $topBarAlert) { alert in
            Alert(title: Text(alert.title), message: Text(alert.message), dismissButton: .default(Text("OK")))
        }
        .alert(item: $deleteAlert) { alert in
            Alert(
                title: Text("Delete Task"),
                message: Text("Delete \"\(alert.title)\"? This removes the task and its worktree."),
                primaryButton: .destructive(Text("Delete")) {
                    _Concurrency.Task { await deleteTask(taskId: alert.taskId) }
                },
                secondaryButton: .cancel()
            )
        }
        .sheet(item: $sharePayload) { payload in
            ShareSheet(items: payload.items)
        }
        .sheet(item: activeSheetPanel) { panel in
            switch panel {
            case .diff:
                WorkbenchDiffPanelView(
                    session: resolvedSession
                )
            case .sessions:
                WorkbenchSessionsPanelView(sessionId: resolvedSession?.id)
            case .terminal:
                WorkbenchTerminalPanelView(
                    workspaceId: selectedWorkspace?.id,
                    taskId: workbenchSelection.taskId,
                    sessionId: resolvedSession?.id,
                    worktreeId: resolvedSession?.worktreeId
                )
            case .artifacts:
                EmptyView()
            }
        }
        .sheet(isPresented: $isRenaming) {
            RenameSheet(
                title: $renameText,
                errorMessage: renameError,
                onCancel: { renameTarget = nil; renameError = nil; isRenaming = false },
                onSave: { _Concurrency.Task { await submitRename() } }
            )
            .presentationDetents([.medium])
        }
    }

    private func workspaceDetailText(_ workspace: WorkspaceSummary) -> String {
        let url = URL(fileURLWithPath: workspace.rootPath)
        return url.lastPathComponent.isEmpty ? workspace.rootPath : url.lastPathComponent
    }

    private func resolvedTaskTitle() -> String {
        guard let selectedTask else { return "New task" }
        let trimmed = selectedTask.task.title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "New task" : trimmed
    }

    private func resolvedConversationMenuContext() -> WorkbenchConversationMenuContext? {
        guard let selectedTask else { return nil }
        let taskId = selectedTask.task.id.stringValue
        let session = resolvedSession
        let hasSession = session != nil
        let canCopyWorktree = session?.worktreeId?.isEmpty == false && selectedWorkspace?.id != nil
        let isArchived = selectedTask.task.archivedAt != nil
        let runningSubagentIds: [String] = selectedTask.sessions.compactMap { summary -> String? in
            guard summary.session.relationship == "sub_agent" else { return nil }
            guard summary.activity?.isWorking == true else { return nil }
            return summary.session.id.stringValue
        }
        return WorkbenchConversationMenuContext(
            hasSession: hasSession,
            canCopyWorktree: canCopyWorktree,
            isArchived: isArchived,
            isArchivePending: archiveInFlight.contains(taskId),
            hasRunningSubagents: !runningSubagentIds.isEmpty,
            onInterruptSubagents: { _Concurrency.Task { await interruptSubagentSessions(runningSubagentIds) } },
            onExportTranscript: { exportTranscript(sessionId: session?.id) },
            onCopyTranscript: { copyTranscript(sessionId: session?.id) },
            onExportSessionLog: { exportSessionLog(sessionId: session?.id) },
            onCopySessionLog: { copySessionLog(sessionId: session?.id) },
            onCopyWorktreeLocation: { copyWorktreeLocation(session: session) },
            onArchiveConversation: { _Concurrency.Task { await archiveConversation(taskId: taskId) } }
        )
    }

    @MainActor
    private func interruptSubagentSessions(_ sessionIds: [String]) async {
        guard let client = connection.apiClient else { return }
        for sessionId in sessionIds {
            do {
                try await client.interruptSession(sessionId: sessionId)
            } catch {
                continue
            }
        }
    }

    private func dismissKeyboard() {
        UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    }

    private func handleTopBarAction(_ action: WorkbenchTopBarAction) {
        switch action {
        case .artifacts:
            if resolvedSession != nil {
                togglePanel(.artifacts)
            } else {
                topBarAlert = WorkbenchTopBarAlert(
                    title: "Artifacts",
                    message: "Select a task session to view artifacts."
                )
            }
        case .diff:
            togglePanel(.diff)
        case .sessions:
            guard resolvedSession != nil else {
                topBarAlert = WorkbenchTopBarAlert(
                    title: "Sessions",
                    message: "Select a task session to view sessions."
                )
                return
            }
            togglePanel(.sessions)
        case .terminal:
            togglePanel(.terminal)
        }
    }

    private func togglePanel(_ panel: WorkbenchPanel) {
        if activePanel == panel {
            activePanel = nil
        } else {
            activePanel = panel
        }
    }

    private func exportTranscript(sessionId: String?) {
        guard let sessionId else {
            topBarAlert = WorkbenchTopBarAlert(title: "Export Transcript", message: "Select a session first.")
            return
        }
        guard let client = connection.apiClient else {
            topBarAlert = WorkbenchTopBarAlert(title: "Export Transcript", message: "Connect to a daemon first.")
            return
        }
        _Concurrency.Task {
            do {
                let messages = try await client.listMessages(sessionId: sessionId)
                let transcript = formatTranscript(messages: messages)
                await MainActor.run {
                    sharePayload = SharePayload(items: [transcript])
                }
            } catch {
                await MainActor.run {
                    topBarAlert = WorkbenchTopBarAlert(title: "Export Transcript", message: "Failed to load transcript.")
                }
            }
        }
    }

    private func copyTranscript(sessionId: String?) {
        guard let sessionId else {
            topBarAlert = WorkbenchTopBarAlert(title: "Copy Transcript", message: "Select a session first.")
            return
        }
        guard let client = connection.apiClient else {
            topBarAlert = WorkbenchTopBarAlert(title: "Copy Transcript", message: "Connect to a daemon first.")
            return
        }
        _Concurrency.Task {
            do {
                let messages = try await client.listMessages(sessionId: sessionId)
                let transcript = formatTranscript(messages: messages)
                await MainActor.run {
                    UIPasteboard.general.string = transcript
                    topBarAlert = WorkbenchTopBarAlert(title: "Copy Transcript", message: "Transcript copied to clipboard.")
                }
            } catch {
                await MainActor.run {
                    topBarAlert = WorkbenchTopBarAlert(title: "Copy Transcript", message: "Failed to load transcript.")
                }
            }
        }
    }

    private func exportSessionLog(sessionId: String?) {
        guard let sessionId else {
            topBarAlert = WorkbenchTopBarAlert(title: "Export Session Log", message: "Select a session first.")
            return
        }
        guard let client = connection.apiClient else {
            topBarAlert = WorkbenchTopBarAlert(title: "Export Session Log", message: "Connect to a daemon first.")
            return
        }
        _Concurrency.Task {
            do {
                let snapshot = try await client.getSessionSnapshot(sessionId: sessionId, limit: 200, includeEvents: true)
                let logText = formatSessionLog(head: snapshot.head)
                await MainActor.run {
                    sharePayload = SharePayload(items: [logText])
                }
            } catch {
                await MainActor.run {
                    topBarAlert = WorkbenchTopBarAlert(title: "Export Session Log", message: "Failed to load session log.")
                }
            }
        }
    }

    private func copySessionLog(sessionId: String?) {
        guard let sessionId else {
            topBarAlert = WorkbenchTopBarAlert(title: "Copy Session Log", message: "Select a session first.")
            return
        }
        guard let client = connection.apiClient else {
            topBarAlert = WorkbenchTopBarAlert(title: "Copy Session Log", message: "Connect to a daemon first.")
            return
        }
        _Concurrency.Task {
            do {
                let snapshot = try await client.getSessionSnapshot(sessionId: sessionId, limit: 200, includeEvents: true)
                let logText = formatSessionLog(head: snapshot.head)
                await MainActor.run {
                    UIPasteboard.general.string = logText
                    topBarAlert = WorkbenchTopBarAlert(title: "Copy Session Log", message: "Session log copied to clipboard.")
                }
            } catch {
                await MainActor.run {
                    topBarAlert = WorkbenchTopBarAlert(title: "Copy Session Log", message: "Failed to load session log.")
                }
            }
        }
    }

    private func copyWorktreeLocation(session: SessionSummary?) {
        guard let session, let worktreeId = session.worktreeId, !worktreeId.isEmpty else {
            topBarAlert = WorkbenchTopBarAlert(title: "Copy Worktree Location", message: "Select a worktree-backed session first.")
            return
        }
        guard let client = connection.apiClient, let workspaceId = selectedWorkspace?.id else {
            topBarAlert = WorkbenchTopBarAlert(title: "Copy Worktree Location", message: "Connect to a daemon first.")
            return
        }
        _Concurrency.Task {
            do {
                let worktrees = try await client.listWorktrees(workspaceId: workspaceId)
                if let worktree = worktrees.first(where: { $0.id.stringValue == worktreeId }) {
                    await MainActor.run {
                        UIPasteboard.general.string = worktree.rootPath
                        topBarAlert = WorkbenchTopBarAlert(title: "Copy Worktree Location", message: "Worktree path copied.")
                    }
                } else {
                    await MainActor.run {
                        topBarAlert = WorkbenchTopBarAlert(title: "Copy Worktree Location", message: "Worktree not found.")
                    }
                }
            } catch {
                await MainActor.run {
                    topBarAlert = WorkbenchTopBarAlert(title: "Copy Worktree Location", message: "Failed to load worktrees.")
                }
            }
        }
    }

    @MainActor
    private func archiveConversation(taskId: String) async {
        guard let client = connection.apiClient else {
            topBarAlert = WorkbenchTopBarAlert(title: "Archive Conversation", message: "Connect to a daemon first.")
            return
        }
        guard !archiveInFlight.contains(taskId) else { return }
        archiveInFlight.insert(taskId)
        defer { archiveInFlight.remove(taskId) }
        do {
            _ = try await client.archiveTask(taskId: taskId)
            await loadTasks()
        } catch {
            topBarAlert = WorkbenchTopBarAlert(title: "Archive Conversation", message: "Failed to archive task.")
        }
    }

    private var selectedWorkspace: WorkspaceSummary? {
        if let selectedId = workspaceSelection.workspaceId,
           let workspace = workspaces.first(where: { $0.id == selectedId }) {
            return workspace
        }
        return workspaces.first
    }

    private var selectedTask: WorkspaceTaskSummary? {
        guard let taskId = workbenchSelection.taskId else { return nil }
        if let match = activeTasks.first(where: { $0.task.id.stringValue == taskId }) {
            return match
        }
        return archivedTasks.first(where: { $0.task.id.stringValue == taskId })
    }

    private var isPreparingSession: Bool {
        guard let taskId = workbenchSelection.taskId else { return false }
        return sessionCreationTaskId == taskId
    }

    private var resolvedSession: SessionSummary? {
        resolvedSessionContext?.session
    }

    private var resolvedSessionContext: ResolvedPrimarySession? {
        guard let selectedTask else { return nil }
        return resolvePrimarySession(
            for: selectedTask,
            preferredSessionId: workbenchSelection.sessionId
        )
    }

    @MainActor
    private func loadWorkspaces() async {
        guard let client = connection.apiClient else {
            workspaces = []
            workspaceError = nil
            workbenchSelection.setContext(daemonKey: nil, workspaceId: nil)
            lastWorkspaceId = nil
            return
        }
        isLoadingWorkspaces = true
        workspaceError = nil
        do {
            let daemonKey = connection.baseURLText
            workspaceSelection.load(daemonKey: daemonKey)
            workspaceVisibility.load(daemonKey: daemonKey)
            let items = try await client.listWorkspaces()
            let hiddenIds = workspaceVisibility.hiddenWorkspaceIds
            let visibleItems = items.filter { !hiddenIds.contains($0.id) }
            workspaces = visibleItems
            let resolved = resolveWorkspaceSelection(from: visibleItems)
            if let resolved {
                workspaceSelection.setWorkspace(resolved, daemonKey: daemonKey)
            }
            let selectionWorkspaceId = resolved?.id ?? workspaceSelection.workspaceId
            workbenchSelection.setContext(daemonKey: daemonKey, workspaceId: selectionWorkspaceId)
            resetSelectionIfNeeded()
            if selectionWorkspaceId != lastWorkspaceId {
                workbenchSelection.setSelection(taskId: nil, sessionId: nil)
                lastWorkspaceId = selectionWorkspaceId
            }
        } catch {
            workspaces = []
            workspaceError = daemonErrorMessage(error, fallback: "Failed to load workspaces.")
        }
        isLoadingWorkspaces = false
    }

    @MainActor
    private func loadTasks() async {
        guard let client = connection.apiClient, let workspaceId = selectedWorkspace?.id else {
            activeTasks = []
            archivedTasks = []
            taskError = nil
            isLoadingTasks = false
            isRefreshingTasks = false
            refreshTaskRowIndicators()
            return
        }
        lastArchivedRev = 0
        await hydrateTasksFromCache(workspaceId: workspaceId)
        let hasTasks = !(activeTasks.isEmpty && archivedTasks.isEmpty)
        let showBlocking = !hasTasks
        if showBlocking {
            isLoadingTasks = true
        } else {
            isRefreshingTasks = true
        }
        taskError = nil
        var workspaceTasks: [Task] = []
        var workspaceTasksLoaded = false
        defer {
            isLoadingTasks = false
            isRefreshingTasks = false
        }
        do {
            let params = DaemonAPIClient.WorkspaceActiveSnapshotParams(limit: 50)
            let activeSnapshot = try await client.getWorkspaceActiveSnapshot(workspaceId: workspaceId, params: params)
            _Concurrency.Task {
                await ATSHeadCache.shared.store(heads: activeSnapshot.active.tasks.map { $0.primarySessionHead })
            }
            _Concurrency.Task { await WorkspaceActiveSnapshotCache.shared.store(snapshot: activeSnapshot) }
            lastStreamSnapshotRev = activeSnapshot.snapshotRev
            if let archivedRev = activeSnapshot.archivedRev {
                lastArchivedRev = max(lastArchivedRev, archivedRev)
            }

            var activeSummaries = activeSnapshot.active.tasks.map(WorkspaceTaskSummary.init)
            var extraActive: [WorkspaceTaskSummary] = []
            do {
                workspaceTasks = try await client.listWorkspaceTasks(workspaceId: workspaceId)
                workspaceTasksLoaded = true
            } catch {
                workspaceTasksLoaded = false
            }
            if workspaceTasksLoaded {
                let activeIds = Set(activeSummaries.map { $0.task.id.stringValue })
                for task in workspaceTasks where task.archivedAt == nil {
                    if activeIds.contains(task.id.stringValue) { continue }
                    let sortAt = task.lastActivityAt ?? task.updatedAt ?? task.createdAt
                    extraActive.append(WorkspaceTaskSummary(task: task, sessions: [], sortAt: sortAt))
                }
            }
            activeTasks = sortTasksBySortAt(activeSummaries + extraActive)
            refreshTaskRowIndicators()
            await loadArchivedHeadWindow(
                client: client,
                workspaceId: workspaceId,
                fallbackTasks: workspaceTasksLoaded ? workspaceTasks : nil
            )
            resolveSelectionForCurrentTask()
        } catch {
            if showBlocking {
                activeTasks = []
                archivedTasks = []
                refreshTaskRowIndicators()
            }
            taskError = "Failed to load tasks."
        }
    }

    @MainActor
    private func hydrateTasksFromCache(workspaceId: String) async {
        guard activeTasks.isEmpty && archivedTasks.isEmpty else { return }
        var didHydrate = false
        if let cached = await WorkspaceActiveSnapshotCache.shared.load(workspaceId: workspaceId) {
            let summaries = cached.tasks.map { WorkspaceTaskSummary(cachedActiveSummary: $0) }
            if !summaries.isEmpty {
                activeTasks = sortTasksBySortAt(summaries)
                lastStreamSnapshotRev = max(lastStreamSnapshotRev, cached.snapshotRev)
                if let archivedRev = cached.archivedRev {
                    lastArchivedRev = max(lastArchivedRev, archivedRev)
                }
                didHydrate = true
            }
        }
        if let cachedArchived = await WorkspaceArchivedSnapshotCache.shared.load(workspaceId: workspaceId) {
            let summaries = cachedArchived.tasks.map { WorkspaceTaskSummary(cachedArchivedSummary: $0) }
            if !summaries.isEmpty {
                archivedTasks = sortTasksBySortAt(summaries)
                if let archivedRev = cachedArchived.archivedRev {
                    lastArchivedRev = max(lastArchivedRev, archivedRev)
                }
                didHydrate = true
            }
        }
        if didHydrate {
            taskError = nil
            refreshTaskRowIndicators()
            resolveSelectionForCurrentTask()
        }
    }

    @MainActor
    private func loadArchivedHeadWindow(
        client: DaemonAPIClient,
        workspaceId: String,
        fallbackTasks: [Task]?
    ) async {
        do {
            let params = DaemonAPIClient.WorkspaceArchivedPageParams(limit: 50, cursor: nil)
            let page = try await client.listWorkspaceArchivedTaskSummaries(workspaceId: workspaceId, params: params)
            let summaries = page.tasks.map { $0.toTaskSummary() }
            archivedTasks = sortTasksBySortAt(summaries)
            if let archivedRev = page.archivedRev {
                lastArchivedRev = max(lastArchivedRev, archivedRev)
            }
            refreshTaskRowIndicators()
            let heads = page.tasks.compactMap { $0.primarySessionHead }
            if !heads.isEmpty {
                _Concurrency.Task { await ATSHeadCache.shared.store(heads: heads) }
            }
            _Concurrency.Task { await WorkspaceArchivedSnapshotCache.shared.store(page: page) }
            return
        } catch {
            guard let fallbackTasks else { return }
            var nextArchived: [WorkspaceTaskSummary] = []
            for task in fallbackTasks where task.archivedAt != nil {
                nextArchived.append(await buildArchivedTaskSummary(client: client, task: task))
            }
            archivedTasks = sortTasksBySortAt(nextArchived)
            refreshTaskRowIndicators()
        }
    }

    @MainActor
    private func refreshArchivedHeadWindow() async {
        guard let client = connection.apiClient, let workspaceId = selectedWorkspace?.id else { return }
        await loadArchivedHeadWindow(client: client, workspaceId: workspaceId, fallbackTasks: nil)
    }

    @MainActor
    private func upsertArchivedTaskSummary(_ summary: WorkspaceArchivedTaskSummaryPayload) {
        upsertTaskSummary(summary.toTaskSummary())
    }

    @MainActor
    private func refreshTaskRowIndicators() {
        taskRowIndicators = taskRowIndicatorCache.update(
            tasks: activeTasks + archivedTasks,
            snapshotRev: lastStreamSnapshotRev
        )
    }

    private func resolveSelectionForCurrentTask() {
        guard let taskId = workbenchSelection.taskId,
              let task = selectedTask else { return }
        let resolved = resolvePrimarySession(
            for: task,
            preferredSessionId: workbenchSelection.sessionId
        )
        if resolved.sessionId != workbenchSelection.sessionId {
            workbenchSelection.setSelection(taskId: taskId, sessionId: resolved.sessionId)
        }
        _Concurrency.Task { await markTaskReadIfNeeded(task) }
        guard resolved.sessionId == nil, task.task.archivedAt == nil else { return }
        _Concurrency.Task { await ensurePrimarySession(taskId: taskId) }
    }

    private func resetSelectionIfNeeded() {
        guard !didResetSelection else { return }
        if ProcessInfo.processInfo.environment["CTX_RESET_SELECTION"] == "1" {
            workbenchSelection.clearSelection()
        }
        didResetSelection = true
    }

    private func selectTask(_ task: WorkspaceTaskSummary) {
        let resolved = resolvePrimarySession(for: task, preferredSessionId: nil)
        workbenchSelection.setSelection(
            taskId: task.task.id.stringValue,
            sessionId: resolved.sessionId
        )
        _Concurrency.Task { await markTaskReadIfNeeded(task) }
        guard resolved.sessionId == nil, task.task.archivedAt == nil else { return }
        _Concurrency.Task { await ensurePrimarySession(taskId: task.task.id.stringValue) }
    }

    @MainActor
    private func ensurePrimarySession(taskId: String) async {
        guard let client = connection.apiClient, let workspaceId = selectedWorkspace?.id else { return }
        if sessionCreationTaskId == taskId {
            return
        }
        sessionCreationTaskId = taskId
        defer {
            if sessionCreationTaskId == taskId {
                sessionCreationTaskId = nil
            }
        }
        do {
            guard let task = (activeTasks + archivedTasks)
                .first(where: { $0.task.id.stringValue == taskId }) else { return }
            guard task.task.archivedAt == nil else { return }

            let resolved = resolvePrimarySession(
                for: task,
                preferredSessionId: workbenchSelection.sessionId
            )
            if let sessionId = resolved.sessionId {
                workbenchSelection.setSelection(taskId: taskId, sessionId: sessionId)
                return
            }

            let (providerId, modelId) = try await resolveDefaultSessionConfig(client: client, workspaceId: workspaceId)
            let session = try await client.createSession(taskId: taskId, providerId: providerId, modelId: modelId)
            workbenchSelection.setSelection(taskId: taskId, sessionId: session.id.stringValue)
            await loadTasks()
        } catch {
            taskError = "Failed to prepare session."
        }
    }

    private func resolveWorkspaceSelection(from items: [WorkspaceSummary]) -> WorkspaceSummary? {
        if let selectedId = workspaceSelection.workspaceId,
           let match = items.first(where: { $0.id == selectedId }) {
            return match
        }
        return items.first
    }

    private func beginRename(_ task: WorkspaceTaskSummary) {
        renameTarget = task
        let current = task.task.title.trimmingCharacters(in: .whitespacesAndNewlines)
        renameText = current.isEmpty ? "New task" : current
        renameError = nil
        isRenaming = true
    }

    @MainActor
    private func submitRename() async {
        guard let target = renameTarget else { return }
        let next = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !next.isEmpty else {
            renameError = "Title required."
            return
        }
        guard let client = connection.apiClient else {
            renameError = "Not connected."
            return
        }
        do {
            _ = try await client.updateTaskTitle(taskId: target.task.id.stringValue, title: next)
            renameTarget = nil
            renameError = nil
            isRenaming = false
            await loadTasks()
        } catch {
            renameError = "Failed to rename."
        }
    }

    @MainActor
    private func toggleArchive(_ task: WorkspaceTaskSummary) async {
        guard let client = connection.apiClient else {
            taskError = "Connect to a daemon to manage tasks."
            return
        }
        let taskId = task.task.id.stringValue
        if archiveInFlight.contains(taskId) { return }
        archiveInFlight.insert(taskId)
        defer { archiveInFlight.remove(taskId) }
        do {
            if task.task.archivedAt != nil {
                _ = try await client.unarchiveTask(taskId: taskId)
            } else {
                _ = try await client.archiveTask(taskId: taskId)
            }
            await loadTasks()
        } catch {
            taskError = "Failed to update task."
        }
    }

    @MainActor
    private func toggleReadState(taskId: String, markRead: Bool) async {
        guard let client = connection.apiClient else {
            topBarAlert = WorkbenchTopBarAlert(
                title: markRead ? "Mark as Read" : "Mark as Unread",
                message: "Connect to a daemon to update task status."
            )
            return
        }
        guard !markReadInFlight.contains(taskId) else { return }
        markReadInFlight.insert(taskId)
        defer { markReadInFlight.remove(taskId) }
        do {
            let updated = try await (markRead ? client.markTaskRead(taskId: taskId) : client.markTaskUnread(taskId: taskId))
            applyTaskUpdate(updated)
        } catch {
            topBarAlert = WorkbenchTopBarAlert(
                title: markRead ? "Mark as Read" : "Mark as Unread",
                message: "Failed to update task."
            )
        }
    }

    @MainActor
    private func deleteTask(taskId: String) async {
        guard let client = connection.apiClient else {
            topBarAlert = WorkbenchTopBarAlert(
                title: "Delete Task",
                message: "Connect to a daemon to delete tasks."
            )
            return
        }
        guard !deleteInFlight.contains(taskId) else { return }
        deleteInFlight.insert(taskId)
        defer { deleteInFlight.remove(taskId) }
        do {
            try await client.deleteTask(taskId: taskId)
            if workbenchSelection.taskId == taskId {
                workbenchSelection.clearSelection()
            }
            await loadTasks()
        } catch {
            topBarAlert = WorkbenchTopBarAlert(
                title: "Delete Task",
                message: "Failed to delete task."
            )
        }
    }

    @MainActor
    private func markTaskReadIfNeeded(_ task: WorkspaceTaskSummary) async {
        let taskId = task.task.id.stringValue
        guard !taskId.isEmpty else { return }
        guard !markReadInFlight.contains(taskId) else { return }
        let isWorking = taskHasWorkingSession(task)
        guard taskHasUnread(task, isWorking: isWorking) else { return }
        guard let client = connection.apiClient else { return }
        markReadInFlight.insert(taskId)
        defer { markReadInFlight.remove(taskId) }
        do {
            let updated = try await client.markTaskRead(taskId: taskId)
            applyTaskUpdate(updated)
        } catch {
            // ignore mark read failures
        }
    }

    @MainActor
    private func refreshWorkspaceStream() async {
        stopWorkspaceStream()
        guard connection.apiClient != nil, let workspaceId = selectedWorkspace?.id else { return }
        streamTask = _Concurrency.Task { await streamLoop(workspaceId: workspaceId) }
    }

    @MainActor
    private func stopWorkspaceStream() {
        streamTask?.cancel()
        streamTask = nil
        if let streamSocket {
            streamClient.disconnect(streamSocket)
        }
        streamSocket = nil
        isStreamConnected = false
        streamReconnectDelay = 1
    }

    @MainActor
    private func streamLoop(workspaceId: String) async {
        while !_Concurrency.Task.isCancelled {
            guard workspaceId == selectedWorkspace?.id else { break }
            guard connection.apiClient != nil else {
                try? await _Concurrency.Task.sleep(nanoseconds: 1_000_000_000)
                continue
            }
            do {
                let socket = try await openWorkspaceStream(workspaceId: workspaceId)
                await listenToWorkspaceStream(socket)
                if let streamSocket {
                    streamClient.disconnect(streamSocket)
                }
                streamSocket = nil
                isStreamConnected = false
            } catch {
                isStreamConnected = false
            }
            if _Concurrency.Task.isCancelled { break }
            let delay = min(streamReconnectDelay, 15)
            streamReconnectDelay = min(streamReconnectDelay * 2, 15)
            try? await _Concurrency.Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
    }

    @MainActor
    private func openWorkspaceStream(workspaceId: String) async throws -> URLSessionWebSocketTask {
        guard let client = connection.apiClient else { throw DaemonStreamError.invalidURL }
        let baseURL = await client.daemonBaseURL()
        if let secure = await client.secureConnectionContext() {
            let socket = try streamClient.connectSecureWorkspaceStream(
                baseURL: baseURL,
                workspaceId: workspaceId,
                deviceId: secure.deviceId
            )
            streamSecureContext = secure
            streamSocket = socket
            streamReconnectDelay = 1
            await sendWorkspaceSubscriptionIfNeeded()
            isStreamConnected = true
            return socket
        }
        let token = await client.authToken()
        let socket = try streamClient.connectWorkspaceStream(baseURL: baseURL, workspaceId: workspaceId, token: token)
        streamSecureContext = nil
        streamSocket = socket
        streamReconnectDelay = 1
        await sendWorkspaceSubscriptionIfNeeded()
        isStreamConnected = true
        return socket
    }

    @MainActor
    private func listenToWorkspaceStream(_ socket: URLSessionWebSocketTask) async {
        while !_Concurrency.Task.isCancelled {
            do {
                let message = try await streamClient.receive(from: socket)
                await handleWorkspaceStreamMessage(message)
            } catch {
                break
            }
        }
    }

    @MainActor
    private func sendWorkspaceSubscriptionIfNeeded() async {
        guard let socket = streamSocket else { return }
        let message = WorkspaceActiveSnapshotClientMessage(type: "subscribe", sessionIds: [], sessions: [])
        if let secureContext = streamSecureContext {
            let seq = await SecureSequenceStore.shared.next(for: secureContext.deviceId)
            guard let payload = try? streamEncoder.encode(message) else { return }
            guard let envelope = try? MobileE2EE.encryptPayload(
                deviceId: secureContext.deviceId,
                seq: seq,
                key: secureContext.key,
                plaintext: payload
            ) else { return }
            guard let wrapped = try? streamEncoder.encode(envelope),
                  let text = String(data: wrapped, encoding: .utf8) else { return }
            try? await streamClient.send(.string(text), via: socket)
            return
        }
        guard let payload = try? streamEncoder.encode(message),
              let text = String(data: payload, encoding: .utf8) else { return }
        try? await streamClient.send(.string(text), via: socket)
    }

    @MainActor
    private func handleWorkspaceStreamMessage(_ message: URLSessionWebSocketTask.Message) async {
        let data: Data?
        switch message {
        case .data(let payload):
            data = payload
        case .string(let text):
            data = text.data(using: .utf8)
        @unknown default:
            data = nil
        }
        guard let data else { return }
        if let secureContext = streamSecureContext {
            guard let envelope = try? streamDecoder.decode(SecureEnvelope.self, from: data) else { return }
            guard envelope.deviceId == secureContext.deviceId else { return }
            guard let decrypted = try? MobileE2EE.decryptEnvelope(envelope, key: secureContext.key) else { return }
            guard let event = try? streamDecoder.decode(WorkspaceActiveSnapshotEvent.self, from: decrypted) else { return }
            handleWorkspaceStreamEvent(event)
            return
        }
        guard let event = try? streamDecoder.decode(WorkspaceActiveSnapshotEvent.self, from: data) else { return }
        handleWorkspaceStreamEvent(event)
    }

    @MainActor
    private func handleWorkspaceStreamEvent(_ event: WorkspaceActiveSnapshotEvent) {
        _Concurrency.Task { await WorkspaceActiveSnapshotCache.shared.apply(event: event) }
        _Concurrency.Task { await WorkspaceArchivedSnapshotCache.shared.apply(event: event) }
        let snapshotRev: Int?
        let archivedRev: Int?
        let isReady: Bool
        switch event {
        case .ready(_, let rev, let archived):
            snapshotRev = rev
            archivedRev = archived
            isReady = true
        case .activeTaskUpsert(_, let rev, _):
            snapshotRev = rev
            archivedRev = nil
            isReady = false
        case .activeTaskDelete(_, let rev, _):
            snapshotRev = rev
            archivedRev = nil
            isReady = false
        case .sessionSummary(_, let rev, _):
            snapshotRev = rev
            archivedRev = nil
            isReady = false
        case .sessionHeadDelta(_, let rev, _):
            snapshotRev = rev
            archivedRev = nil
            isReady = false
        case .sessionGap(_, let rev, _, _, _):
            snapshotRev = rev
            archivedRev = nil
            isReady = false
        case .worktreeBootstrap(_, let rev, _):
            snapshotRev = rev
            archivedRev = nil
            isReady = false
        case .archivedTaskUpsert(_, let archived, _, _):
            snapshotRev = nil
            archivedRev = archived
            isReady = false
        case .archivedTaskDelete(_, let archived, _):
            snapshotRev = nil
            archivedRev = archived
            isReady = false
        }

        if let snapshotRev {
            if snapshotRev > lastStreamSnapshotRev + 1 {
                _Concurrency.Task { await loadTasks() }
            }
            if isReady, snapshotRev != lastStreamSnapshotRev {
                _Concurrency.Task { await loadTasks() }
            }
            lastStreamSnapshotRev = max(lastStreamSnapshotRev, snapshotRev)
        }
        if let archivedRev {
            if archivedRev > lastArchivedRev + 1 {
                _Concurrency.Task { await refreshArchivedHeadWindow() }
            }
            if isReady, archivedRev != lastArchivedRev {
                _Concurrency.Task { await refreshArchivedHeadWindow() }
            }
            lastArchivedRev = max(lastArchivedRev, archivedRev)
        }

        switch event {
        case .activeTaskUpsert(_, _, let task):
            upsertTaskSummary(WorkspaceTaskSummary(activeSummary: task))
            _Concurrency.Task { await ATSHeadCache.shared.store(head: task.primarySessionHead) }
        case .activeTaskDelete(_, _, let taskId):
            removeTask(taskId: taskId.stringValue)
        case .sessionSummary(_, _, let summary):
            applySessionSummary(summary)
        case .sessionHeadDelta(_, _, let delta):
            _Concurrency.Task { await ATSHeadCache.shared.apply(delta: delta) }
        case .sessionGap:
            _Concurrency.Task { await loadTasks() }
        case .archivedTaskUpsert(_, _, let task, let snapshot):
            upsertArchivedTaskSummary(task)
            if let snapshot {
                _Concurrency.Task { await ATSHeadCache.shared.store(head: snapshot.head) }
            } else if let head = task.primarySessionHead {
                _Concurrency.Task { await ATSHeadCache.shared.store(head: head) }
            }
        case .archivedTaskDelete(_, _, let taskId):
            removeTask(taskId: taskId.stringValue)
        default:
            break
        }
    }

    @MainActor
    private func upsertTaskSummary(_ summary: WorkspaceTaskSummary) {
        let taskId = summary.task.id.stringValue
        activeTasks.removeAll { $0.task.id.stringValue == taskId }
        archivedTasks.removeAll { $0.task.id.stringValue == taskId }
        if summary.task.archivedAt == nil {
            activeTasks = sortTasksBySortAt(activeTasks + [summary])
        } else {
            archivedTasks = sortTasksBySortAt(archivedTasks + [summary])
        }
        refreshTaskRowIndicators()
        resolveSelectionForCurrentTask()
    }

    @MainActor
    private func removeTask(taskId: String) {
        activeTasks.removeAll { $0.task.id.stringValue == taskId }
        archivedTasks.removeAll { $0.task.id.stringValue == taskId }
        if workbenchSelection.taskId == taskId {
            workbenchSelection.setSelection(taskId: nil, sessionId: nil)
        }
        refreshTaskRowIndicators()
    }

    @MainActor
    private func applySessionSummary(_ summary: SessionSnapshotSummary) {
        let taskId = summary.session.taskId.stringValue
        let sessionId = summary.session.id.stringValue
        func update(_ tasks: inout [WorkspaceTaskSummary]) -> Bool {
            guard let taskIndex = tasks.firstIndex(where: { $0.task.id.stringValue == taskId }) else { return false }
            let taskSummary = tasks[taskIndex]
            var sessions = taskSummary.sessions
            if let sessionIndex = sessions.firstIndex(where: { $0.session.id.stringValue == sessionId }) {
                sessions[sessionIndex] = summary
            } else {
                sessions.append(summary)
            }
            tasks[taskIndex] = WorkspaceTaskSummary(
                task: taskSummary.task,
                sessions: sessions,
                sortAt: taskSummary.sortAt
            )
            return true
        }
        if !update(&activeTasks) {
            _ = update(&archivedTasks)
        }
        refreshTaskRowIndicators()
        resolveSelectionForCurrentTask()
    }

    @MainActor
    private func applyTaskUpdate(_ task: Task) {
        let taskId = task.id.stringValue
        func update(_ tasks: inout [WorkspaceTaskSummary]) -> WorkspaceTaskSummary? {
            guard let index = tasks.firstIndex(where: { $0.task.id.stringValue == taskId }) else { return nil }
            let summary = tasks[index]
            tasks.remove(at: index)
            return WorkspaceTaskSummary(
                task: task,
                sessions: summary.sessions,
                sortAt: summary.sortAt
            )
        }

        if let updated = update(&activeTasks) {
            if updated.task.archivedAt == nil {
                activeTasks = sortTasksBySortAt(activeTasks + [updated])
            } else {
                archivedTasks = sortTasksBySortAt(archivedTasks + [updated])
            }
            refreshTaskRowIndicators()
            return
        }

        if let updated = update(&archivedTasks) {
            if updated.task.archivedAt == nil {
                activeTasks = sortTasksBySortAt(activeTasks + [updated])
            } else {
                archivedTasks = sortTasksBySortAt(archivedTasks + [updated])
            }
            refreshTaskRowIndicators()
        }
    }
}

private struct WorkbenchTopBarAlert: Identifiable {
    let id = UUID()
    let title: String
    let message: String
}

private struct SharePayload: Identifiable {
    let id = UUID()
    let items: [Any]
}

private struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}

private struct WorkbenchDeleteAlert: Identifiable {
    let id = UUID()
    let taskId: String
    let title: String
}

private struct WorkbenchHomeView: View {
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let isLoadingTasks: Bool
    let taskError: String?
    let selectedSession: SessionSummary?
    @Binding var activePanel: WorkbenchPanel?
    @Binding var artifactsCount: Int
    let hasTaskSelection: Bool
    let isArchived: Bool
    let isPreparingSession: Bool
    let onTaskCreated: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            WorkbenchNavigationFlowView(
                selectedWorkspace: selectedWorkspace,
                isLoadingWorkspaces: isLoadingWorkspaces,
                workspaceError: workspaceError,
                isLoadingTasks: isLoadingTasks,
                taskError: taskError,
                selectedSession: selectedSession,
                activePanel: $activePanel,
                artifactsCount: $artifactsCount,
                hasTaskSelection: hasTaskSelection,
                isArchived: isArchived,
                isPreparingSession: isPreparingSession,
                onTaskCreated: onTaskCreated
            )
                .padding(.top, 8)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    }
}

private enum WorkbenchTopBarAction {
    case artifacts
    case diff
    case sessions
    case terminal
}

private struct WorkbenchConversationMenuContext {
    let hasSession: Bool
    let canCopyWorktree: Bool
    let isArchived: Bool
    let isArchivePending: Bool
    let hasRunningSubagents: Bool
    let onInterruptSubagents: () -> Void
    let onExportTranscript: () -> Void
    let onCopyTranscript: () -> Void
    let onExportSessionLog: () -> Void
    let onCopySessionLog: () -> Void
    let onCopyWorktreeLocation: () -> Void
    let onArchiveConversation: () -> Void
}

private struct WorkbenchTopBar: View {
    let title: String
    let onMenuTap: () -> Void
    let onArtifactsTap: () -> Void
    let artifactsCount: Int
    let onDiffTap: () -> Void
    let onSessionsTap: () -> Void
    let onTerminalTap: () -> Void
    let activePanel: WorkbenchPanel?
    let isSessionsEnabled: Bool
    let conversationMenuContext: WorkbenchConversationMenuContext?
    let showsTaskActions: Bool

    var body: some View {
        HStack(spacing: 12) {
            Button(action: onMenuTap) {
                Image(systemName: "line.3.horizontal")
                    .font(.title2)
            }
            .foregroundColor(.ctxTextPrimary)
            .accessibilityIdentifier("drawer.open")

            Text(title)
                .font(.headline)
                .foregroundColor(.ctxTextPrimary)
                .lineLimit(1)

            Spacer(minLength: 0)

            if showsTaskActions {
                HStack(spacing: 14) {
                    Button(action: onArtifactsTap) {
                        ZStack(alignment: .topTrailing) {
                            LucideIcon(name: .image, size: 16)
                                .foregroundColor(activePanel == .artifacts ? .ctxAccent : .ctxTextPrimary)
                            if artifactsCount > 0 {
                                WorkbenchIconBadge(count: artifactsCount)
                                    .offset(x: 8, y: -6)
                            }
                        }
                    }
                    .accessibilityIdentifier("topbar.artifacts")

                    Button(action: onDiffTap) {
                        LucideIcon(name: .gitBranch, size: 16)
                            .foregroundColor(activePanel == .diff ? .ctxAccent : .ctxTextPrimary)
                    }
                    .accessibilityIdentifier("topbar.diff")

                    /*
                    Button(action: onSessionsTap) {
                        LucideIcon(name: .monitor, size: 16)
                            .foregroundColor(
                                isSessionsEnabled
                                ? (activePanel == .sessions ? .ctxAccent : .ctxTextPrimary)
                                : .ctxTextMuted
                            )
                    }
                    .accessibilityIdentifier("topbar.sessions")
                    .disabled(!isSessionsEnabled)
                    */

                    Button(action: onTerminalTap) {
                        LucideIcon(name: .terminal, size: 16)
                            .foregroundColor(activePanel == .terminal ? .ctxAccent : .ctxTextPrimary)
                    }
                    .accessibilityIdentifier("topbar.terminal")

                    Menu {
                        if let context = conversationMenuContext {
                            Button("Export Transcript", action: context.onExportTranscript)
                                .disabled(!context.hasSession)
                            Button("Copy Transcript", action: context.onCopyTranscript)
                                .disabled(!context.hasSession)
                            Button("Export Session Log", action: context.onExportSessionLog)
                                .disabled(!context.hasSession)
                            Button("Copy Session Log", action: context.onCopySessionLog)
                                .disabled(!context.hasSession)
                            Button("Interrupt All Subagents", action: context.onInterruptSubagents)
                                .disabled(!context.hasRunningSubagents)
                            Button("Copy Worktree Location", action: context.onCopyWorktreeLocation)
                                .disabled(!context.canCopyWorktree)
                            Button("Archive Conversation", action: context.onArchiveConversation)
                                .disabled(context.isArchived || context.isArchivePending)
                        } else {
                            Text("Select a task to manage it.")
                        }
                    } label: {
                        LucideIcon(name: .ellipsis, size: 16)
                    }
                    .accessibilityIdentifier("topbar.taskmenu")
                    .disabled(conversationMenuContext == nil)
                }
                .foregroundColor(.ctxTextPrimary)
            }
        }
    }
}

private struct WorkbenchIconBadge: View {
    let count: Int

    var body: some View {
        let label = count > 99 ? "99+" : "\(count)"
        Text(label)
            .font(.caption2.weight(.bold))
            .foregroundColor(.white)
            .padding(.horizontal, 4)
            .padding(.vertical, 1)
            .background(Capsule().fill(Color.ctxAccent))
    }
}

private struct WorkbenchDrawerView: View {
    let workspaces: [WorkspaceSummary]
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let activeTasks: [WorkspaceTaskSummary]
    let archivedTasks: [WorkspaceTaskSummary]
    let isLoadingTasks: Bool
    let isRefreshingTasks: Bool
    let taskError: String?
    let activeTaskId: String?
    let taskRowIndicators: [String: TaskRowIndicators]
    @Binding var taskQuery: String
    let drawerWidth: CGFloat
    let onRefresh: () -> Void
    let onSelectTask: (WorkspaceTaskSummary) -> Void
    let onNewTask: () -> Void
    let onRenameTask: (WorkspaceTaskSummary) -> Void
    let onArchiveToggle: (WorkspaceTaskSummary) -> Void
    let markReadInFlight: Set<String>
    let deleteInFlight: Set<String>
    let onToggleReadState: (WorkspaceTaskSummary) -> Void
    let onDeleteTask: (WorkspaceTaskSummary) -> Void
    @State private var showArchived = false

    var body: some View {
        let trimmedQuery = taskQuery.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let filteredActive = filterTasks(activeTasks, matching: trimmedQuery)
        let filteredArchived = filterTasks(archivedTasks, matching: trimmedQuery)

        VStack(spacing: 0) {
            VStack(spacing: 12) {
                HStack(spacing: 10) {
                    WorkbenchSearchField(text: $taskQuery)
                    Button(action: onNewTask) {
                        Image(systemName: "square.and.pencil")
                            .font(.headline)
                            .foregroundColor(.ctxTextPrimary)
                            .frame(width: 40, height: 40)
                            .background(Color.white.opacity(0.04), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: 12, style: .continuous)
                                    .stroke(Color.ctxLine, lineWidth: 1)
                            )
                    }
                    .accessibilityIdentifier("drawer.newtask")
                }
                .padding(.horizontal, 16)
                .padding(.top, 16)

                Divider()
                    .background(Color.ctxLine.opacity(0.8))
            }

            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 16) {
                    WorkbenchSectionHeaderView(title: "Active", isRefreshing: isRefreshingTasks)

                    if selectedWorkspace == nil {
                        Text("Select a workspace in settings to view tasks.")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    } else {
                        if let taskError {
                            Text(taskError)
                                .font(.caption)
                                .foregroundColor(.ctxError)
                        }

                        if isLoadingTasks && filteredActive.isEmpty {
                            Text("Loading tasks...")
                                .font(.caption)
                                .foregroundColor(.ctxTextMuted)
                        } else if filteredActive.isEmpty {
                            Text("No active tasks.")
                                .font(.caption)
                                .foregroundColor(.ctxTextMuted)
                        } else {
                            LazyVStack(spacing: 2) {
                                ForEach(filteredActive, id: \.task.id) { task in
                                    let taskId = task.task.id.stringValue
                                    let indicators = taskRowIndicators[taskId] ?? TaskRowIndicators(task: task)
                                    Button {
                                        onSelectTask(task)
                                    } label: {
                                        WorkbenchTaskRowView(
                                            task: task,
                                            indicators: indicators,
                                            isSelected: activeTaskId == taskId
                                        )
                                    }
                                    .buttonStyle(.plain)
                                    .contextMenu {
                                        let archived = task.task.archivedAt != nil
                                        let hasAssistantMessages = task.task.lastAssistantMessageAt != nil
                                        let isWorking = indicators.isWorking
                                        let isUnread = indicators.isUnread

                                        Button("Rename Task") { onRenameTask(task) }
                                        Button(archived ? "Unarchive" : "Archive") { onArchiveToggle(task) }
                                        Button(isUnread ? "Mark as Read" : "Mark as Unread") { onToggleReadState(task) }
                                            .disabled(!hasAssistantMessages || markReadInFlight.contains(taskId))
                                        Button("Delete Task", role: .destructive) { onDeleteTask(task) }
                                            .disabled(deleteInFlight.contains(taskId))
                                    }
                                    .accessibilityIdentifier("drawer.task.\(task.task.id.stringValue)")
                                }
                            }
                        }

                        DisclosureGroup(isExpanded: $showArchived) {
                            if filteredArchived.isEmpty {
                                Text("No archived tasks.")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                                    .padding(.top, 6)
                            } else {
                                LazyVStack(spacing: 2) {
                                    ForEach(filteredArchived, id: \.task.id) { task in
                                        let taskId = task.task.id.stringValue
                                        let indicators = taskRowIndicators[taskId] ?? TaskRowIndicators(task: task)
                                        Button {
                                            onSelectTask(task)
                                        } label: {
                                        WorkbenchTaskRowView(
                                            task: task,
                                            indicators: indicators,
                                            isSelected: activeTaskId == taskId
                                        )
                                        }
                                        .buttonStyle(.plain)
                                        .contextMenu {
                                            let hasAssistantMessages = task.task.lastAssistantMessageAt != nil
                                            let isWorking = indicators.isWorking
                                            let isUnread = indicators.isUnread

                                            Button("Rename Task") { onRenameTask(task) }
                                            Button("Unarchive") { onArchiveToggle(task) }
                                            Button(isUnread ? "Mark as Read" : "Mark as Unread") { onToggleReadState(task) }
                                                .disabled(!hasAssistantMessages || markReadInFlight.contains(taskId))
                                            Button("Delete Task", role: .destructive) { onDeleteTask(task) }
                                                .disabled(deleteInFlight.contains(taskId))
                                        }
                                        .accessibilityIdentifier("drawer.task.\(task.task.id.stringValue)")
                                    }
                                }
                                .padding(.top, 6)
                            }
                        } label: {
                            HStack {
                                WorkbenchSectionHeaderView(title: "Archived")
                                Spacer()
                                Text("\(filteredArchived.count)")
                                    .font(.system(size: 12))
                                    .foregroundColor(.ctxTextSecondary)
                                LucideIcon(name: .chevronDown, size: 12)
                                    .foregroundColor(.ctxTextSecondary)
                                    .rotationEffect(.degrees(showArchived ? 0 : -90))
                            }
                        }
                        .disclosureGroupStyle(WorkbenchDisclosureGroupStyle())
                        .accessibilityIdentifier("drawer.archived.toggle")
                        .accentColor(.ctxTextSecondary)
                    }
                }
                .padding(16)
            }

            Divider()
                .background(Color.ctxLine.opacity(0.8))

            WorkbenchWorkspaceSwitcherView(
                workspaces: workspaces,
                selectedWorkspace: selectedWorkspace,
                isLoadingWorkspaces: isLoadingWorkspaces,
                workspaceError: workspaceError,
                onRefresh: onRefresh
            )
        }
        .accessibilityIdentifier("drawer.container")
        .frame(width: drawerWidth)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(.ultraThinMaterial)
        .shadow(color: Color.ctxShadow, radius: 24, x: 0, y: 12)
        .padding(.leading, 12)
    }
}

private struct WorkbenchWorkspaceSwitcherView: View {
    let workspaces: [WorkspaceSummary]
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let onRefresh: () -> Void
    @State private var isWorkspaceMenuPresented = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                Button {
                    isWorkspaceMenuPresented = true
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "folder")
                            .foregroundColor(.ctxAccent)
                        Text(selectedWorkspace?.name ?? "No workspace")
                            .font(.subheadline.weight(.semibold))
                            .foregroundColor(.ctxTextPrimary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Spacer()
                        LucideIcon(name: .chevronDown, size: 12)
                            .foregroundColor(.ctxTextSecondary)
                    }
                    .padding(.vertical, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("drawer.workspace.switch")

                NavigationLink {
                    SettingsView(selectedWorkspace: selectedWorkspace)
                } label: {
                    LucideIcon(name: .settings, size: 16)
                        .foregroundColor(.ctxTextSecondary)
                        .frame(width: 32, height: 32)
                        .background(Color.ctxSurface.opacity(0.5), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("drawer.workspace.settings")
            }

            if isLoadingWorkspaces {
                HStack(spacing: 8) {
                    ProgressView()
                        .tint(.ctxAccent)
                    Text("Loading workspaces...")
                        .foregroundColor(.ctxTextMuted)
                }
                .font(.caption)
            } else if let workspaceError {
                Text(workspaceError)
                    .font(.caption)
                    .foregroundColor(.ctxError)
            } else if workspaces.isEmpty {
                Text("No workspaces connected yet.")
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .sheet(isPresented: $isWorkspaceMenuPresented, onDismiss: onRefresh) {
            NavigationStack {
                WorkspaceSwitchView()
            }
            .presentationDetents([.medium, .large])
        }
    }
}

private struct WorkbenchDisclosureGroupStyle: DisclosureGroupStyle {
    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.2)) {
                    configuration.isExpanded.toggle()
                }
            } label: {
                configuration.label
            }
            .buttonStyle(.plain)

            if configuration.isExpanded {
                configuration.content
            }
        }
    }
}

private struct WorkbenchNavigationFlowView: View {
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let isLoadingTasks: Bool
    let taskError: String?
    let selectedSession: SessionSummary?
    @Binding var activePanel: WorkbenchPanel?
    @Binding var artifactsCount: Int
    let hasTaskSelection: Bool
    let isArchived: Bool
    let isPreparingSession: Bool
    let onTaskCreated: () -> Void

    var body: some View {
        if isLoadingWorkspaces {
            VStack(spacing: 12) {
                ProgressView()
                    .tint(.ctxAccent)
                Text("Loading workspaces...")
                    .font(.footnote)
                    .foregroundColor(.ctxTextMuted)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let workspaceError {
            WorkbenchInfoCard(text: workspaceError, tint: .ctxError)
                .padding(.horizontal, 20)
        } else if let workspace = selectedWorkspace {
            NavigationStack {
                if let selectedSession {
                    ChatDetailView(
                        session: selectedSession,
                        isArchived: isArchived,
                        activePanel: $activePanel,
                        artifactCount: $artifactsCount
                    )
                } else if hasTaskSelection {
                    if isPreparingSession {
                        WorkbenchEmptyStateView(
                            title: "Preparing session",
                            message: "Setting up the primary session..."
                        )
                    } else {
                        WorkbenchEmptyStateView(
                            title: "No sessions yet",
                            message: "This task doesn't have a session yet."
                        )
                    }
                } else {
                    WorkbenchNewTaskView(
                        workspace: workspace,
                        isLoadingTasks: isLoadingTasks,
                        taskError: taskError,
                        onTaskCreated: onTaskCreated
                    )
                }
            }
            .toolbar(.hidden, for: .navigationBar)
        } else {
            WorkbenchEmptyStateView(
                title: "No workspace selected",
                message: "Pick a workspace in settings to start browsing tasks."
            )
        }
    }
}

private struct WorkbenchNewTaskView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var workbenchSelection: WorkbenchSelectionStore
    let workspace: WorkspaceSummary
    let isLoadingTasks: Bool
    let taskError: String?
    let onTaskCreated: () -> Void

    @State private var prompt = ""
    @State private var providers: [ProviderStatus] = []
    @State private var models: [String] = []
    @State private var selectedProviderId = ""
    @State private var selectedModelId = ""
    @State private var selectedEffortId = ""
    @State private var selectedMode: ComposerMode = .default
    @State private var isLoadingProviders = false
    @State private var isLoadingModels = false
    @State private var errorMessage: String?
    @State private var isSubmitting = false
    @State private var routingEntry: ModelRoutingEntry?
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @State private var pendingAttachments: [MessageAttachment] = []
    @FocusState private var isPromptFocused: Bool

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 16) {
                composerCard
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .task(id: workspace.id) {
            loadRoutingDefaults()
            await loadProviders()
        }
        .task(id: effectiveProviderId) {
            await loadModels()
        }
        .onChange(of: selectedPhotos) { newItems in
            _Concurrency.Task { await loadAttachments(from: newItems) }
        }
        .onChange(of: models) { _ in
            syncEffortSelection()
        }
        .onChange(of: selectedModelId) { _, _ in
            syncEffortSelection()
        }
        .onAppear {
            let env = ProcessInfo.processInfo.environment
            let isUITest = env["CTX_UI_TEST_MODE"] == "1"
            let shouldFocusPrompt = isUITest && env["CTX_OPEN_DRAWER_ON_LAUNCH"] != "1"
            if shouldFocusPrompt {
                _Concurrency.Task { @MainActor in
                    try? await _Concurrency.Task.sleep(nanoseconds: 200_000_000)
                    isPromptFocused = true
                }
            }
        }
    }

    private var composerCard: some View {
        let catalog = buildModelCatalog(modelChoices)
        let resolvedModelId = resolveModelIdForUI()
        let parsedModel = parseModelId(resolvedModelId, catalog: catalog)
        let baseId = parsedModel.base.isEmpty ? resolvedModelId : parsedModel.base
        let baseOptions = catalog.baseIds.isEmpty ? [resolvedModelId] : catalog.baseIds
        let modelLabel = isLoadingModels ? "Loading..." : (catalog.displayNameByBase[baseId] ?? baseId)
        let effortOptions = catalog.effortsByBase[baseId] ?? []
        let resolvedEffortId = resolveEffortId(parsed: parsedModel, efforts: effortOptions)
        let effortLabel = resolvedEffortId.map(formatEffortLabel) ?? "Effort"

        let baseBinding = Binding<String>(
            get: { baseId },
            set: { selectBase($0, catalog: catalog) }
        )
        let effortBinding = Binding<String>(
            get: { resolvedEffortId ?? "" },
            set: { selectEffort($0, baseId: baseId, catalog: catalog) }
        )

        return VStack(alignment: .leading, spacing: 12) {
            if let taskError {
                WorkbenchInfoCard(text: taskError, tint: .ctxError)
            }

            if let errorMessage {
                WorkbenchInfoCard(text: errorMessage, tint: .ctxError)
            }

            ZStack(alignment: .topLeading) {
                if prompt.isEmpty {
                    Text("@ for context, / for commands")
                        .font(CtxChatStyle.bodyFont)
                        .foregroundColor(.ctxTextMuted)
                        .padding(.top, 2)
                }
                TextField("", text: $prompt, axis: .vertical)
                    .lineLimit(1...6)
                    .font(CtxChatStyle.bodyFont)
                    .foregroundColor(.ctxTextPrimary)
                    .tint(.ctxAccent)
                    .focused($isPromptFocused)
                    .accessibilityIdentifier("newtask.prompt")
            }

            if !pendingAttachments.isEmpty {
                ComposerAttachmentsRow(attachments: pendingAttachments) { index in
                    pendingAttachments.remove(at: index)
                }
            }

            VStack(spacing: 8) {
                WorkbenchInlineHarnessPicker(
                    value: providerLabel,
                    providerId: providerIconId,
                    isDisabled: providerOptionsDisabled,
                    options: availableProviderIds,
                    displayName: formatProviderName,
                    onSelect: { id in
                        selectedProviderId = id
                        selectedModelId = ""
                        selectedEffortId = ""
                    }
                )

                HStack(spacing: 12) {
                    Menu {
                        Picker("Model", selection: baseBinding) {
                            ForEach(baseOptions, id: \.self) { base in
                                let label = catalog.displayNameByBase[base] ?? base
                                Text(label).tag(base)
                            }
                        }
                        .pickerStyle(.inline)
                    } label: {
                        newTaskMenuLabel(modelLabel)
                    }
                    .disabled(modelOptionsDisabled || baseOptions.isEmpty)

                    if !effortOptions.isEmpty {
                        Menu {
                            Picker("Effort", selection: effortBinding) {
                                ForEach(effortOptions, id: \.self) { effort in
                                    Text(formatEffortLabel(effort)).tag(effort)
                                }
                            }
                            .pickerStyle(.inline)
                        } label: {
                            newTaskMenuLabel(effortLabel)
                        }
                        .disabled(modelOptionsDisabled)
                    }

                    Spacer(minLength: 0)
                }

                HStack(spacing: 12) {
                    Menu {
                        Picker("Mode", selection: $selectedMode) {
                            ForEach(ComposerMode.allCases) { mode in
                                Text(mode.label).tag(mode)
                            }
                        }
                        .pickerStyle(.inline)
                    } label: {
                        newTaskMenuLabel(selectedMode.label)
                    }

                    Spacer(minLength: 0)
                }
            }

            HStack(spacing: 8) {
                PhotosPicker(selection: $selectedPhotos, matching: .images) {
                    ComposerToolIcon(name: .image)
                }
                .accessibilityIdentifier("newtask.attach")

                Spacer(minLength: 0)

                if isSendEnabled {
                    ComposerCircleButton(icon: .arrowUp, accessibilityId: "newtask.send") {
                        startTask()
                    }
                } else {
                    Button {} label: {
                        ComposerToolIcon(name: .mic)
                    }
                    .frame(width: CtxChatStyle.composerPrimarySize, height: CtxChatStyle.composerPrimarySize)
                    .accessibilityIdentifier("newtask.mic")
                }
            }
        }
        .padding(.horizontal, CtxChatStyle.composerInnerHorizontalPadding)
        .padding(.vertical, CtxChatStyle.composerInnerVerticalPadding)
        .background(
            Color.ctxSurfaceRaised,
            in: RoundedRectangle(cornerRadius: CtxChatStyle.composerCornerRadius, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: CtxChatStyle.composerCornerRadius, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }

    @ViewBuilder
    private func newTaskMenuLabel(_ text: String) -> some View {
        HStack(spacing: 6) {
            Text(text)
                .font(.footnote.weight(.semibold))
                .foregroundColor(.ctxTextPrimary)
                .lineLimit(1)
            Image(systemName: "chevron.down")
                .font(.caption2.weight(.semibold))
                .foregroundColor(.ctxTextMuted)
        }
        .padding(.vertical, 7)
        .padding(.horizontal, 10)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }

    private var promptTrimmed: String {
        prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var daemonKey: String? {
        let raw = connection.secureConfig?.baseURL ?? connection.baseURLText
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private var availableProviderIds: [String] {
        let installed = providers.filter { $0.installed && $0.health == "ok" }
        return installed.map { $0.providerId }.filter { !$0.isEmpty }
    }

    private var effectiveProviderId: String {
        if !selectedProviderId.isEmpty {
            return selectedProviderId
        }
        if let routingEntry, availableProviderIds.contains(routingEntry.providerId) {
            return routingEntry.providerId
        }
        if availableProviderIds.contains("codex") {
            return "codex"
        }
        return availableProviderIds.first ?? ""
    }

    private var providerLabel: String {
        if isLoadingProviders { return "Loading..." }
        if availableProviderIds.isEmpty { return "No harnesses" }
        return formatProviderName(effectiveProviderId)
    }

    private var providerIconId: String? {
        if isLoadingProviders || availableProviderIds.isEmpty { return nil }
        return effectiveProviderId
    }

    private var modelChoices: [String] {
        models.isEmpty ? ["default"] : models
    }

    private var effectiveModelId: String {
        resolveModelIdForUI()
    }

    private var providerOptionsDisabled: Bool {
        isLoadingProviders || availableProviderIds.isEmpty
    }

    private var modelOptionsDisabled: Bool {
        isLoadingModels || effectiveProviderId.isEmpty || modelChoices.isEmpty
    }

    private var isSendEnabled: Bool {
        (!promptTrimmed.isEmpty || !pendingAttachments.isEmpty) && !effectiveProviderId.isEmpty && !effectiveModelId.isEmpty && !isSubmitting
    }

    private func formatProviderName(_ providerId: String) -> String {
        providerId
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .capitalized
    }

    @MainActor
    private func loadProviders() async {
        guard let client = connection.apiClient else {
            providers = []
            errorMessage = "Connect to a daemon to start."
            isLoadingProviders = false
            return
        }
        loadRoutingDefaults()
        isLoadingProviders = true
        errorMessage = nil
        do {
            providers = try await client.listProviders()
            if selectedProviderId.isEmpty {
                selectedProviderId = effectiveProviderId
            }
        } catch {
            providers = []
            errorMessage = "Failed to load harnesses."
        }
        isLoadingProviders = false
    }

    @MainActor
    private func loadRoutingDefaults() {
        guard let daemonKey else {
            routingEntry = nil
            return
        }
        routingEntry = ModelRoutingDefaults.load(daemonKey: daemonKey, workspaceId: workspace.id)
    }

    @MainActor
    private func loadModels() async {
        guard let client = connection.apiClient, !effectiveProviderId.isEmpty else {
            models = []
            isLoadingModels = false
            return
        }
        isLoadingModels = true
        errorMessage = nil
        do {
            let options = try await client.getProviderOptions(workspaceId: workspace.id, providerId: effectiveProviderId)
            models = extractModelIds(from: options.models)
            syncEffortSelection()
            if !models.contains(selectedModelId) {
                selectedModelId = ""
            }
        } catch {
            models = []
            errorMessage = "Failed to load models."
        }
        isLoadingModels = false
        syncEffortSelection()
    }

    private func syncEffortSelection() {
        let catalog = buildModelCatalog(modelChoices)
        let parsed = parseModelId(resolveModelIdForUI(), catalog: catalog)
        let baseId = parsed.base.isEmpty ? resolveModelIdForUI() : parsed.base
        let efforts = catalog.effortsByBase[baseId] ?? []

        if efforts.isEmpty {
            if !selectedEffortId.isEmpty {
                selectedEffortId = ""
            }
            return
        }

        var nextEffort = selectedEffortId
        if nextEffort.isEmpty || !efforts.contains(nextEffort) {
            if let parsedEffort = parsed.effort, efforts.contains(parsedEffort) {
                nextEffort = parsedEffort
            } else {
                nextEffort = pickDefaultEffort(efforts) ?? ""
            }
        }

        if selectedEffortId != nextEffort {
            selectedEffortId = nextEffort
        }

        let resolvedModelId = deriveFullModelIdForBase(catalog: catalog, baseId: baseId, preferredEffort: nextEffort)
        if !resolvedModelId.isEmpty, selectedModelId != resolvedModelId {
            selectedModelId = resolvedModelId
        }
    }

    private func selectBase(_ baseId: String, catalog: ModelCatalog) {
        selectedModelId = baseId
        selectedEffortId = ""
        let efforts = catalog.effortsByBase[baseId] ?? []
        if let defaultEffort = pickDefaultEffort(efforts) {
            selectedEffortId = defaultEffort
        }
    }

    private func selectEffort(_ effortId: String, baseId: String, catalog: ModelCatalog) {
        let next = deriveFullModelIdForBase(catalog: catalog, baseId: baseId, preferredEffort: effortId)
        if !next.isEmpty {
            selectedModelId = next
            selectedEffortId = effortId
        }
    }

    private func resolveEffortId(parsed: ParsedModelId, efforts: [String]) -> String? {
        if let effort = parsed.effort, efforts.contains(effort) {
            return effort
        }
        if !selectedEffortId.isEmpty, efforts.contains(selectedEffortId) {
            return selectedEffortId
        }
        return nil
    }

    private func formatEffortLabel(_ effort: String) -> String {
        effort.replacingOccurrences(of: "_", with: " ").capitalized
    }

    private func resolveModelIdForUI() -> String {
        if !selectedModelId.isEmpty {
            return selectedModelId
        }
        if let routingEntry, routingEntry.providerId == effectiveProviderId {
            if modelChoices.isEmpty || modelChoices.contains(routingEntry.modelId) {
                return routingEntry.modelId
            }
        }
        return modelChoices.first ?? "default"
    }

    private func startTask() {
        let promptValue = promptTrimmed
        guard !promptValue.isEmpty || !pendingAttachments.isEmpty else { return }
        let providerId = effectiveProviderId
        let modelId = effectiveModelId
        let attachments = pendingAttachments
        let modeId = selectedMode.rawValue
        let envTarget = "worktree"
        guard !providerId.isEmpty, !modelId.isEmpty else { return }
        isSubmitting = true
        errorMessage = nil
        _Concurrency.Task {
            guard let client = connection.apiClient else {
                await MainActor.run {
                    errorMessage = "Connect to a daemon to start."
                    isSubmitting = false
                }
                return
            }
            do {
                let task = try await client.createTask(
                    workspaceId: workspace.id,
                    title: "New task",
                    description: nil
                )
                let session = try await client.createSession(
                    taskId: task.id.stringValue,
                    providerId: providerId,
                    modelId: modelId,
                    envTarget: envTarget
                )
                try? await client.setSessionMode(sessionId: session.id.stringValue, modeId: modeId)
                _ = try await client.postMessage(
                    sessionId: session.id.stringValue,
                    content: promptValue,
                    delivery: .immediate,
                    attachments: attachments
                )
                await MainActor.run {
                    workbenchSelection.setSelection(
                        taskId: task.id.stringValue,
                        sessionId: session.id.stringValue
                    )
                    prompt = ""
                    pendingAttachments = []
                    selectedPhotos = []
                    onTaskCreated()
                }
            } catch {
                await MainActor.run {
                    errorMessage = "Failed to start task."
                }
            }
            await MainActor.run {
                isSubmitting = false
            }
        }
    }

    @MainActor
    private func loadAttachments(from items: [PhotosPickerItem]) async {
        guard !items.isEmpty else { return }
        var nextAttachments: [MessageAttachment] = []
        for item in items {
            guard let data = try? await item.loadTransferable(type: Data.self) else { continue }
            let contentType = item.supportedContentTypes.first
            let mimeType = contentType?.preferredMIMEType ?? "image/*"
            let fileExtension = contentType?.preferredFilenameExtension
            let name = suggestedAttachmentName(extension: fileExtension)
            nextAttachments.append(
                MessageAttachment(
                    kind: .image,
                    mimeType: mimeType,
                    dataBase64: data.base64EncodedString(),
                    blobId: nil,
                    name: name
                )
            )
        }
        if !nextAttachments.isEmpty {
            pendingAttachments.append(contentsOf: nextAttachments)
        }
        selectedPhotos = []
    }

    private func suggestedAttachmentName(extension fileExtension: String?) -> String? {
        let suffix = (fileExtension?.isEmpty == false) ? ".\(fileExtension ?? "")" : ""
        let shortId = String(UUID().uuidString.prefix(8))
        return "photo-\(shortId)\(suffix)"
    }
}

private struct CtxTextArea: View {
    let placeholder: String
    @Binding var text: String
    let accessibilityId: String?
    let isFocused: FocusState<Bool>.Binding?

    init(
        placeholder: String,
        text: Binding<String>,
        accessibilityId: String? = nil,
        isFocused: FocusState<Bool>.Binding? = nil
    ) {
        self.placeholder = placeholder
        self._text = text
        self.accessibilityId = accessibilityId
        self.isFocused = isFocused
    }

    var body: some View {
        let isUITest = ProcessInfo.processInfo.environment["CTX_UI_TEST_MODE"] == "1"
        ZStack(alignment: .topLeading) {
            if text.isEmpty {
                Text(placeholder)
                    .foregroundColor(.ctxTextMuted)
                    .font(.subheadline)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 12)
            }
            if let accessibilityId {
                if isUITest {
                    if let isFocused {
                        TextField("", text: $text)
                            .foregroundColor(.ctxTextPrimary)
                            .font(.subheadline)
                            .frame(minHeight: 140, alignment: .topLeading)
                            .padding(8)
                            .accessibilityIdentifier(accessibilityId)
                            .focused(isFocused)
                    } else {
                        TextField("", text: $text)
                            .foregroundColor(.ctxTextPrimary)
                            .font(.subheadline)
                            .frame(minHeight: 140, alignment: .topLeading)
                            .padding(8)
                            .accessibilityIdentifier(accessibilityId)
                    }
                } else {
                    if let isFocused {
                        TextEditor(text: $text)
                            .foregroundColor(.ctxTextPrimary)
                            .font(.subheadline)
                            .frame(minHeight: 140)
                            .scrollContentBackground(.hidden)
                            .padding(8)
                            .accessibilityIdentifier(accessibilityId)
                            .focused(isFocused)
                    } else {
                        TextEditor(text: $text)
                            .foregroundColor(.ctxTextPrimary)
                            .font(.subheadline)
                            .frame(minHeight: 140)
                            .scrollContentBackground(.hidden)
                            .padding(8)
                            .accessibilityIdentifier(accessibilityId)
                    }
                }
            } else {
                if let isFocused {
                    TextEditor(text: $text)
                        .foregroundColor(.ctxTextPrimary)
                        .font(.subheadline)
                        .frame(minHeight: 140)
                        .scrollContentBackground(.hidden)
                        .padding(8)
                        .focused(isFocused)
                } else {
                    TextEditor(text: $text)
                        .foregroundColor(.ctxTextPrimary)
                        .font(.subheadline)
                        .frame(minHeight: 140)
                        .scrollContentBackground(.hidden)
                        .padding(8)
                }
            }
        }
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct WorkbenchMenuPicker: View {
    let title: String
    let value: String
    let isDisabled: Bool
    let options: [String]
    let onSelect: (String) -> Void

    var body: some View {
        Menu {
            ForEach(options, id: \.self) { option in
                Button(option) { onSelect(option) }
            }
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                Text(title)
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
                HStack {
                    Text(value)
                        .foregroundColor(.ctxTextPrimary)
                        .font(.subheadline.weight(.semibold))
                    Spacer()
                    LucideIcon(name: .chevronDown, size: 12)
                        .foregroundColor(.ctxTextSecondary)
                }
            }
            .padding(12)
            .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(Color.ctxLine, lineWidth: 1)
            )
        }
        .disabled(isDisabled)
    }
}

private struct WorkbenchInlineMenuPicker: View {
    let title: String
    let value: String
    let isDisabled: Bool
    let options: [String]
    let onSelect: (String) -> Void

    var body: some View {
        Menu {
            ForEach(options, id: \.self) { option in
                Button(option) { onSelect(option) }
            }
        } label: {
            HStack(spacing: 10) {
                Text(title)
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
                Spacer()
                Text(value)
                    .foregroundColor(.ctxTextPrimary)
                    .font(.subheadline.weight(.semibold))
                LucideIcon(name: .chevronDown, size: 12)
                    .foregroundColor(.ctxTextSecondary)
            }
            .padding(.vertical, 6)
        }
        .disabled(isDisabled)
    }
}

private struct WorkbenchHarnessPicker: View {
    let title: String
    let value: String
    let providerId: String?
    let isDisabled: Bool
    let options: [String]
    let displayName: (String) -> String
    let onSelect: (String) -> Void

    var body: some View {
        Menu {
            ForEach(options, id: \.self) { option in
                Button(displayName(option)) { onSelect(option) }
            }
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                Text(title)
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
                HStack(spacing: 10) {
                    HarnessLogoView(providerId: providerId)
                    Text(value)
                        .foregroundColor(.ctxTextPrimary)
                        .font(.subheadline.weight(.semibold))
                    Spacer()
                    LucideIcon(name: .chevronDown, size: 12)
                        .foregroundColor(.ctxTextSecondary)
                }
                .padding(.vertical, 6)
            }
            .contentShape(Rectangle())
        }
        .disabled(isDisabled)
    }
}

private struct WorkbenchInlineHarnessPicker: View {
    let value: String
    let providerId: String?
    let isDisabled: Bool
    let options: [String]
    let displayName: (String) -> String
    let onSelect: (String) -> Void

    var body: some View {
        Menu {
            ForEach(options, id: \.self) { option in
                Button(displayName(option)) { onSelect(option) }
            }
        } label: {
            HStack(spacing: 10) {
                HarnessLogoView(providerId: providerId)
                Spacer()
                LucideIcon(name: .chevronDown, size: 12)
                    .foregroundColor(.ctxTextSecondary)
            }
            .padding(.vertical, 6)
            .accessibilityLabel(value)
        }
        .allowsHitTesting(!isDisabled)
    }
}

private struct HarnessLogoView: View {
    let providerId: String?

    var body: some View {
        let base = RoundedRectangle(cornerRadius: 8, style: .continuous)
        if let providerId,
           let harness = HarnessCatalog.entry(for: providerId) {
            Image(harness.assetName)
                .resizable()
                .renderingMode(.original)
                .scaledToFit()
                .frame(width: 20, height: 20)
                .clipShape(base)
                .modifier(HarnessInvertModifier(shouldInvert: harness.invertInDark))
                .background(
                    base
                        .fill(Color.white.opacity(0.08))
                )
        } else {
            base
                .fill(Color.white.opacity(0.10))
                .frame(width: 20, height: 20)
                .overlay(
                    base.stroke(Color(red: 0.071, green: 0.071, blue: 0.071).opacity(0.8), lineWidth: 1)
                )
        }
    }
}

private struct WorkbenchNavRowView: View {
    let title: String
    let subtitle: String
    let status: String?
    let icon: String
    let showsChevron: Bool
    let isSelected: Bool

    init(
        title: String,
        subtitle: String,
        status: String?,
        icon: String,
        showsChevron: Bool,
        isSelected: Bool = false
    ) {
        self.title = title
        self.subtitle = subtitle
        self.status = status
        self.icon = icon
        self.showsChevron = showsChevron
        self.isSelected = isSelected
    }

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.ctxSurfaceRaised)
                Image(systemName: icon)
                    .foregroundColor(.ctxAccent)
            }
            .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
                    .lineLimit(2)
            }
            Spacer()
            if let status {
                GlassPill(text: status, tint: .ctxAccent)
            }
            if isSelected {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundColor(.ctxAccent)
            }
            if showsChevron {
                Image(systemName: "chevron.right")
                    .foregroundColor(.ctxTextSecondary)
                    .font(.caption)
            }
        }
        .padding(12)
        .background(
            Color.ctxSurface.opacity(isSelected ? 0.75 : 0.6),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(isSelected ? Color.ctxAccent.opacity(0.6) : Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct DrawerWorkspaceRowView: View {
    let workspace: WorkspaceSummary
    let isSelected: Bool

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.ctxSurfaceRaised)
                Image(systemName: "folder")
                    .foregroundColor(.ctxTextSecondary)
            }
            .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 2) {
                Text(workspace.name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(workspaceDetailText)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
            if isSelected {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundColor(.ctxAccent)
            } else {
                Image(systemName: "circle")
                    .foregroundColor(.ctxTextSecondary)
            }
        }
        .padding(10)
        .background(
            Color.ctxSurface.opacity(isSelected ? 0.75 : 0.6),
            in: RoundedRectangle(cornerRadius: 14, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(isSelected ? Color.ctxAccent.opacity(0.6) : Color.ctxLine, lineWidth: 1)
        )
    }

    private var workspaceDetailText: String {
        let url = URL(fileURLWithPath: workspace.rootPath)
        return url.lastPathComponent.isEmpty ? workspace.rootPath : url.lastPathComponent
    }
}

private struct WorkbenchInfoCard: View {
    let text: String
    let tint: Color

    var body: some View {
        Text(text)
            .font(.footnote)
            .foregroundColor(tint)
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(Color.ctxLine, lineWidth: 1)
            )
    }
}

private struct WorkbenchEmptyStateView: View {
    let title: String
    let message: String

    var body: some View {
        VStack(spacing: 12) {
            Text(title)
                .font(.title3.weight(.semibold))
                .foregroundColor(.ctxTextPrimary)
            Text(message)
                .font(.footnote)
                .foregroundColor(.ctxTextMuted)
                .multilineTextAlignment(.center)
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct ResolvedPrimarySession {
    let sessionId: String?
    let session: SessionSummary?
}

private func resolvePrimarySession(
    for task: WorkspaceTaskSummary,
    preferredSessionId: String?
) -> ResolvedPrimarySession {
    let sessions = task.sessions
    guard let primaryId = task.task.primarySessionId?.stringValue else {
        return ResolvedPrimarySession(sessionId: nil, session: nil)
    }
    let match = sessions.first(where: { $0.session.id.stringValue == primaryId })
    let summary = match.map { SessionSummary(session: $0.session) }
    return ResolvedPrimarySession(sessionId: primaryId, session: summary)
}

private func extractModelIds(from models: JSONValue?) -> [String] {
    guard let models else { return [] }
    switch models {
    case .array(let values):
        let rawIds: [String] = values.compactMap { value in
            switch value {
            case .string(let string):
                return string
            case .object(let object):
                if case .string(let id) = object["modelId"] ?? object["id"] {
                    return id
                }
                return nil
            default:
                return nil
            }
        }
        return uniqueTrimmed(rawIds)
    case .object(let object):
        if let available = object["availableModels"] {
            return extractModelIds(from: available)
        }
        if let models = object["models"] {
            return extractModelIds(from: models)
        }
        if let choices = object["choices"] {
            return extractModelIds(from: choices)
        }
        if case .string(let current) = object["currentModelId"] {
            return uniqueTrimmed([current])
        }
        let keys = object.keys.filter { !$0.isEmpty && $0 != "choices" }.sorted()
        return keys
    default:
        return []
    }
}

private func uniqueTrimmed(_ values: [String]) -> [String] {
    var seen = Set<String>()
    var out: [String] = []
    for value in values {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { continue }
        guard !seen.contains(trimmed) else { continue }
        seen.insert(trimmed)
        out.append(trimmed)
    }
    return out
}

private func resolveDefaultSessionConfig(
    client: DaemonAPIClient,
    workspaceId: String
) async throws -> (providerId: String, modelId: String) {
    let providers = try await client.listProviders()
    let installed = providers.filter { $0.installed && $0.health == "ok" }
    let daemonKey = await client.daemonBaseURL().absoluteString
        .trimmingCharacters(in: .whitespacesAndNewlines)
    if !daemonKey.isEmpty,
       let entry = ModelRoutingDefaults.load(daemonKey: daemonKey, workspaceId: workspaceId),
       !entry.providerId.isEmpty,
       !entry.modelId.isEmpty,
       installed.contains(where: { $0.providerId == entry.providerId }) {
        let options = try await client.getProviderOptions(workspaceId: workspaceId, providerId: entry.providerId)
        let modelIds = extractModelIds(from: options.models)
        if modelIds.isEmpty || modelIds.contains(entry.modelId) {
            return (entry.providerId, entry.modelId)
        }
    }
    guard let preferred = installed.first(where: { $0.providerId == "codex" }) ?? installed.first else {
        throw DaemonAPIError.requestFailed(statusCode: 400, message: "No available harnesses.")
    }
    let providerId = preferred.providerId
    let options = try await client.getProviderOptions(workspaceId: workspaceId, providerId: providerId)
    let modelIds = extractModelIds(from: options.models)
    let modelId = modelIds.first ?? "default"
    return (providerId, modelId)
}

private func buildArchivedTaskSummary(
    client: DaemonAPIClient,
    task: Task
) async -> WorkspaceTaskSummary {
    let sessions = (try? await client.listTaskSessions(taskId: task.id.stringValue)) ?? []
    var summaries = sessions.map(SessionSnapshotSummary.init)
    if let preferredSessionId = pickArchivedSessionId(task: task, sessions: sessions) {
        if let snapshot = try? await client.getSessionSnapshot(sessionId: preferredSessionId, limit: 200, includeEvents: false) {
            let snapshotId = snapshot.summary.session.id.stringValue
            if summaries.contains(where: { $0.session.id.stringValue == snapshotId }) {
                summaries = summaries.map { summary in
                    if summary.session.id.stringValue == snapshotId {
                        return snapshot.summary
                    }
                    return summary
                }
            } else {
                summaries.append(snapshot.summary)
            }
            await ATSHeadCache.shared.store(head: snapshot.head)
        }
        _ = try? await client.getSessionHistory(sessionId: preferredSessionId, beforeSeq: nil, limit: 1)
    }
    let sortAt = task.lastActivityAt ?? task.updatedAt ?? task.createdAt
    return WorkspaceTaskSummary(task: task, sessions: summaries, sortAt: sortAt)
}

private func pickArchivedSessionId(task: Task, sessions: [Session]) -> String? {
    return task.primarySessionId?.stringValue
}

// MARK: - Task list helpers (web parity)

private struct WorkbenchSearchField: View {
    @Binding var text: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundColor(.ctxTextSecondary)
            TextField("Search Tasks", text: $text)
                .textFieldStyle(.plain)
                .foregroundColor(.ctxTextPrimary)
                .font(.system(size: 13))
                .disableAutocorrection(true)
                .textInputAutocapitalization(.never)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(Color.white.opacity(0.04), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
        .accessibilityIdentifier("drawer.search")
    }
}

private struct WorkbenchSectionHeaderView: View {
    let title: String
    var isRefreshing: Bool = false

    var body: some View {
        HStack(spacing: 8) {
            Text(title.uppercased())
                .font(.system(size: 12, weight: .semibold))
                .foregroundColor(.ctxTextMuted)
                .kerning(0.6)
            if isRefreshing {
                Text("Refreshing")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(.ctxTextSecondary)
            }
        }
    }
}

private struct WorkbenchTaskRowView: View {
    let task: WorkspaceTaskSummary
    let indicators: TaskRowIndicators
    let isSelected: Bool

    private var title: String {
        let trimmed = task.task.title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "New task" : trimmed
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                TaskHarnessView(providerIds: indicators.providerIds)

                Text(title)
                    .font(.system(size: 12.5))
                    .foregroundColor(.ctxTextPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)

                Spacer()

                HStack(spacing: 6) {
                    Text(formatRelativeAgeShort(task.task.lastActivityAt ?? task.task.updatedAt ?? task.task.createdAt))
                        .font(.system(size: 12))
                        .foregroundColor(.ctxTextMuted)
                        .frame(minWidth: 28, alignment: .trailing)

                    TaskSpinnerView(isActive: indicators.isWorking)

                    if indicators.dotKind == .unread {
                        TaskStatusDot(color: Color.ctxAccent)
                    } else if indicators.dotKind == .error {
                        TaskStatusDot(color: Color.ctxError)
                    }
                }
            }

            if let preview = indicators.previewText {
                Text(preview)
                    .font(.system(size: 11))
                    .foregroundColor(.ctxTextMuted)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
        }
        .padding(.vertical, 6)
        .padding(.horizontal, 8)
        .background(isSelected ? Color.white.opacity(0.06) : Color.clear, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .stroke(isSelected ? Color.ctxLine : Color.clear, lineWidth: 1)
        )
    }
}

private struct TaskHarnessView: View {
    let providerIds: [String]

    private var displayIds: [String] {
        Array(providerIds.prefix(2))
    }

    private var overflowCount: Int {
        max(0, providerIds.count - displayIds.count)
    }

    var body: some View {
        HStack(spacing: -6) {
            ForEach(displayIds, id: \.self) { providerId in
                TaskHarnessIcon(providerId: providerId)
            }
            if overflowCount > 0 {
                Text("+\(overflowCount)")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundColor(.ctxTextSecondary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.ctxSurface.opacity(0.7), in: Capsule())
                    .overlay(
                        Capsule()
                            .stroke(Color.ctxLine, lineWidth: 1)
                    )
            }
        }
    }
}

private struct TaskHarnessIcon: View {
    let providerId: String

    var body: some View {
        let base = RoundedRectangle(cornerRadius: 6, style: .continuous)
        if let harness = HarnessCatalog.entry(for: providerId) {
            Image(harness.assetName)
                .resizable()
                .renderingMode(.original)
                .scaledToFit()
                .frame(width: 16, height: 16)
                .clipShape(base)
                .modifier(HarnessInvertModifier(shouldInvert: harness.invertInDark))
                .background(
                    base
                        .fill(Color.white.opacity(0.04))
                )
        } else {
            base
                .fill(Color.white.opacity(0.10))
                .frame(width: 16, height: 16)
                .overlay(
                    base.stroke(Color(red: 0.071, green: 0.071, blue: 0.071).opacity(0.8), lineWidth: 1)
                )
        }
    }
}

private struct TaskSpinnerView: View {
    let isActive: Bool
    @State private var isAnimating = false

    var body: some View {
        Circle()
            .trim(from: 0.15, to: 1)
            .stroke(
                AngularGradient(
                    gradient: Gradient(colors: [Color.ctxAccent, Color.white.opacity(0.22)]),
                    center: .center
                ),
                style: StrokeStyle(lineWidth: 2, lineCap: .round)
            )
            .frame(width: isActive ? 12 : 0, height: isActive ? 12 : 0)
            .opacity(isActive ? 1 : 0)
            .rotationEffect(.degrees(isAnimating ? 360 : 0))
            .animation(isActive ? .linear(duration: 0.8).repeatForever(autoreverses: false) : .default, value: isAnimating)
            .onAppear { isAnimating = true }
            .onChange(of: isActive) { next in
                if next { isAnimating = true }
            }
    }
}

private struct TaskStatusDot: View {
    let color: Color

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 6, height: 6)
    }
}

private struct RenameSheet: View {
    @Binding var title: String
    let errorMessage: String?
    let onCancel: () -> Void
    let onSave: () -> Void

    var body: some View {
        NavigationStack {
            Form {
                Section("Title") {
                    TextField("Task title", text: $title)
                        .textInputAutocapitalization(.sentences)
                }
                if let errorMessage {
                    Section {
                        Text(errorMessage)
                            .foregroundColor(.ctxError)
                            .font(.footnote)
                    }
                }
            }
            .navigationTitle("Rename Task")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onCancel)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save", action: onSave)
                        .disabled(title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}

private func formatTranscript(messages: [MessageSummary]) -> String {
    var lines: [String] = []
    for message in messages {
        let roleLabel: String
        switch message.role {
        case .assistant:
            roleLabel = "Assistant"
        case .user:
            roleLabel = "User"
        case .system:
            roleLabel = "System"
        }
        lines.append("\(roleLabel): \(message.content)")
        lines.append("")
    }
    return lines.joined(separator: "\n")
}

private func formatSessionLog(head: SessionHead) -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    guard let data = try? encoder.encode(head),
          let text = String(data: data, encoding: .utf8) else {
        return ""
    }
    return text
}

private struct TaskRowIndicators {
    let providerIds: [String]
    let isWorking: Bool
    let isUnread: Bool
    let dotKind: TaskRowDotKind?
    let previewText: String?

    init(task: WorkspaceTaskSummary) {
        providerIds = resolveProviderIds(for: task)
        isWorking = taskHasWorkingSession(task)
        let hasError = taskHasErrorSession(task)
        isUnread = taskHasUnread(task, isWorking: isWorking)
        if hasError {
            dotKind = .error
        } else if isUnread {
            dotKind = .unread
        } else {
            dotKind = nil
        }
        previewText = resolveTaskPreview(task)
    }
}

private struct TaskRowSessionFingerprint: Hashable {
    let sessionId: String
    let lastEventSeq: Int?
    let status: String
    let providerId: String
    let activityWorking: Bool
    let lastMessagePreview: String?
}

private struct TaskRowFingerprint: Hashable {
    let taskId: String
    let snapshotRev: Int
    let sortAt: String
    let updatedAt: String
    let lastActivityAt: String?
    let assistantSeenAt: String?
    let lastAssistantMessageAt: String?
    let sessions: [TaskRowSessionFingerprint]

    init(task: WorkspaceTaskSummary, snapshotRev: Int) {
        taskId = task.task.id.stringValue
        self.snapshotRev = snapshotRev
        sortAt = task.sortAt
        updatedAt = task.task.updatedAt
        lastActivityAt = task.task.lastActivityAt
        assistantSeenAt = task.task.assistantSeenAt
        lastAssistantMessageAt = task.task.lastAssistantMessageAt
        sessions = task.sessions.map { summary in
            TaskRowSessionFingerprint(
                sessionId: summary.session.id.stringValue,
                lastEventSeq: summary.lastEventSeq,
                status: summary.session.status,
                providerId: summary.session.providerId,
                activityWorking: summary.activity?.isWorking == true,
                lastMessagePreview: summary.lastMessagePreview
            )
        }
    }
}

private struct TaskRowIndicatorCacheEntry {
    let fingerprint: TaskRowFingerprint
    let indicators: TaskRowIndicators
}

private struct TaskRowIndicatorCache {
    private var entries: [String: TaskRowIndicatorCacheEntry] = [:]

    mutating func update(tasks: [WorkspaceTaskSummary], snapshotRev: Int) -> [String: TaskRowIndicators] {
        var nextEntries: [String: TaskRowIndicatorCacheEntry] = [:]
        var indicatorsById: [String: TaskRowIndicators] = [:]
        for task in tasks {
            let taskId = task.task.id.stringValue
            let fingerprint = TaskRowFingerprint(task: task, snapshotRev: snapshotRev)
            if let cached = entries[taskId], cached.fingerprint == fingerprint {
                nextEntries[taskId] = cached
                indicatorsById[taskId] = cached.indicators
                continue
            }
            let indicators = TaskRowIndicators(task: task)
            let entry = TaskRowIndicatorCacheEntry(fingerprint: fingerprint, indicators: indicators)
            nextEntries[taskId] = entry
            indicatorsById[taskId] = indicators
        }
        entries = nextEntries
        return indicatorsById
    }
}

private enum TaskRowDotKind {
    case unread
    case error
}

private func resolveTaskPreview(_ task: WorkspaceTaskSummary) -> String? {
    if let primaryId = task.task.primarySessionId,
       let summary = task.sessions.first(where: { $0.session.id == primaryId }),
       let preview = summary.lastMessagePreview?.trimmingCharacters(in: .whitespacesAndNewlines),
       !preview.isEmpty {
        return preview
    }
    for summary in task.sessions {
        if let preview = summary.lastMessagePreview?.trimmingCharacters(in: .whitespacesAndNewlines),
           !preview.isEmpty {
            return preview
        }
    }
    return nil
}


private func taskHasWorkingSession(_ task: WorkspaceTaskSummary) -> Bool {
    if let primaryId = task.task.primarySessionId?.stringValue,
       let summary = task.sessions.first(where: { $0.session.id.stringValue == primaryId }) {
        let status = summary.session.status.lowercased()
        if status == "failed" || status == "cancelled" || status == "completed" {
            return false
        }
        return summary.activity?.isWorking == true
    }
    return false
}

private func taskHasErrorSession(_ task: WorkspaceTaskSummary) -> Bool {
    for summary in task.sessions {
        let status = summary.session.status.lowercased()
        if status == "failed" || status == "cancelled" {
            return true
        }
    }
    return false
}

private func taskHasUnread(_ task: WorkspaceTaskSummary, isWorking: Bool) -> Bool {
    guard !isWorking else { return false }
    guard let lastAssistantIso = task.task.lastAssistantMessageAt else { return false }
    let lastAssistant = parseIso(lastAssistantIso)
    let seen = parseIso(task.task.assistantSeenAt)
    guard let lastAssistant else { return false }
    if let seen {
        return lastAssistant > seen
    }
    return true
}

private func resolveProviderIds(for task: WorkspaceTaskSummary) -> [String] {
    var ordered: [String] = []
    for summary in task.sessions {
        let providerId = summary.session.providerId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !providerId.isEmpty else { continue }
        if !ordered.contains(providerId) {
            ordered.append(providerId)
        }
    }
    return ordered
}

private func filterTasks(_ tasks: [WorkspaceTaskSummary], matching query: String) -> [WorkspaceTaskSummary] {
    guard !query.isEmpty else { return tasks }
    return tasks.filter { task in
        let title = task.task.title.lowercased()
        return title.contains(query) || task.task.id.stringValue.lowercased().contains(query)
    }
}

private func sortTasksBySortAt(_ tasks: [WorkspaceTaskSummary]) -> [WorkspaceTaskSummary] {
    tasks.sorted { left, right in
        let leftDate = parseIso(left.sortAt)
        let rightDate = parseIso(right.sortAt)
        switch (leftDate, rightDate) {
        case let (lhs?, rhs?):
            return lhs > rhs
        case (.some, .none):
            return true
        case (.none, .some):
            return false
        case (.none, .none):
            return left.task.id.stringValue > right.task.id.stringValue
        }
    }
}

private let isoFormatter: ISO8601DateFormatter = {
    let f = ISO8601DateFormatter()
    f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return f
}()

private func parseIso(_ iso: String?) -> Date? {
    guard let iso else { return nil }
    return isoFormatter.date(from: iso) ?? ISO8601DateFormatter().date(from: iso)
}

private func formatRelativeAgeShort(_ iso: String?) -> String {
    guard let iso, let date = parseIso(iso) else { return "Now" }
    let seconds = max(0, Int(Date().timeIntervalSince(date)))
    if seconds < 60 { return "Now" }
    if seconds < 3600 { return "\(max(1, seconds / 60))m" }
    if seconds < 86_400 { return "\(max(1, seconds / 3600))h" }
    return "\(max(1, seconds / 86_400))d"
}

#Preview {
    WorkbenchShellView()
        .environmentObject(ConnectionStore())
        .environmentObject(WorkspaceSelectionStore())
        .environmentObject(WorkbenchSelectionStore())
        .environmentObject(WorkspaceVisibilityStore())
}
