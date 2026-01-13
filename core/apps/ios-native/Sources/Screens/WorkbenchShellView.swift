import SwiftUI
import UIKit
import _Concurrency

struct WorkbenchShellView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var workspaceSelection: WorkspaceSelectionStore
    @EnvironmentObject private var workbenchSelection: WorkbenchSelectionStore
    @State private var isDrawerOpen = false
    @State private var workspaces: [WorkspaceSummary] = []
    @State private var isLoadingWorkspaces = false
    @State private var workspaceError: String?
    @State private var activeTasks: [WorkspaceCatchupTaskSummary] = []
    @State private var archivedTasks: [WorkspaceCatchupTaskSummary] = []
    @State private var isLoadingTasks = false
    @State private var isRefreshingTasks = false
    @State private var taskError: String?
    @State private var lastWorkspaceId: String?
    @State private var sessionCreationTaskId: String?
    @State private var didResetSelection = false
    @State private var taskQuery = ""
    @State private var shouldAutoOpenDrawer = ProcessInfo.processInfo.environment["CTX_OPEN_DRAWER_ON_LAUNCH"] == "1"
    @State private var renameTarget: WorkspaceCatchupTaskSummary?
    @State private var renameText: String = ""
    @State private var renameError: String?
    @State private var isRenaming = false
    @State private var archiveInFlight: Set<String> = []
    @State private var markReadInFlight: Set<String> = []
    @State private var isArtifactsPresented = false
    @State private var topBarAlert: WorkbenchTopBarAlert?
    @State private var streamTask: _Concurrency.Task<Void, Never>?
    @State private var streamSocket: URLSessionWebSocketTask?
    @State private var streamReconnectDelay: TimeInterval = 1
    @State private var isStreamConnected = false
    @State private var streamSecureContext: SecureConnectionContext?
    @State private var lastStreamSnapshotRev: Int = 0

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

    var body: some View {
        GeometryReader { proxy in
            let drawerWidth = min(320, proxy.size.width * 0.78)
            let taskTitle = resolvedTaskTitle()
            let taskMenuContext = resolvedTaskMenuContext()

            ZStack(alignment: .leading) {
                CtxBackgroundView()

                WorkbenchHomeView(
                    selectedWorkspace: selectedWorkspace,
                    isLoadingWorkspaces: isLoadingWorkspaces,
                    workspaceError: workspaceError,
                    isLoadingTasks: isLoadingTasks,
                    taskError: taskError,
                    selectedSession: resolvedSession,
                    isArtifactsPresented: $isArtifactsPresented,
                    hasTaskSelection: workbenchSelection.taskId != nil,
                    isPreparingSession: isPreparingSession,
                    onTaskCreated: { _Concurrency.Task { await loadTasks() } }
                )
                .safeAreaInset(edge: .top, spacing: 0) {
                    WorkbenchTopBar(
                        title: taskTitle,
                        onMenuTap: { isDrawerOpen = true },
                        onArtifactsTap: { handleTopBarAction(.artifacts) },
                        onDiffTap: { handleTopBarAction(.diff) },
                        onSessionsTap: { handleTopBarAction(.sessions) },
                        onTerminalTap: { handleTopBarAction(.terminal) },
                        taskMenuContext: taskMenuContext
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
                    }
                )
                .offset(x: isDrawerOpen ? 0 : -drawerWidth - 24)
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
        .onDisappear {
            stopWorkspaceStream()
        }
        .alert(item: $topBarAlert) { alert in
            Alert(title: Text(alert.title), message: Text(alert.message), dismissButton: .default(Text("OK")))
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
        guard let selectedTask else { return "Select a task" }
        let trimmed = selectedTask.task.title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "New Task" : trimmed
    }

    private func resolvedTaskMenuContext() -> WorkbenchTaskMenuContext? {
        guard let selectedTask else { return nil }
        let title = resolvedTaskTitle()
        let taskId = selectedTask.task.id.stringValue
        let isArchived = selectedTask.task.archivedAt != nil
        return WorkbenchTaskMenuContext(
            title: title,
            taskId: taskId,
            isArchived: isArchived,
            onRename: { beginRename(selectedTask) },
            onArchiveToggle: { _Concurrency.Task { await toggleArchive(selectedTask) } },
            onCopyTitle: { UIPasteboard.general.string = title },
            onCopyId: { UIPasteboard.general.string = taskId },
            onCopyTranscript: { copyTranscript(for: resolvedSession) }
        )
    }

    private func handleTopBarAction(_ action: WorkbenchTopBarAction) {
        switch action {
        case .artifacts:
            if resolvedSession != nil {
                isArtifactsPresented = true
            } else {
                topBarAlert = WorkbenchTopBarAlert(
                    title: "Artifacts",
                    message: "Select a task session to view artifacts."
                )
            }
        case .diff:
            topBarAlert = WorkbenchTopBarAlert(
                title: "Diff",
                message: "Diff view is stubbed for now."
            )
        case .sessions:
            topBarAlert = WorkbenchTopBarAlert(
                title: "Sessions",
                message: "Sessions panel is stubbed for now."
            )
        case .terminal:
            topBarAlert = WorkbenchTopBarAlert(
                title: "Terminal",
                message: "Terminal panel is stubbed for now."
            )
        }
    }

    private func copyTranscript(for session: SessionSummary?) {
        guard let session else {
            topBarAlert = WorkbenchTopBarAlert(
                title: "Export Transcript",
                message: "Select a task session to export its transcript."
            )
            return
        }
        _Concurrency.Task {
            guard let client = connection.apiClient else { return }
            do {
                let items = try await client.listMessages(sessionId: session.id)
                let formatted = items.map { summary in
                    let role = summary.role == .user ? "User" : "Assistant"
                    return "\(role): \(summary.content)"
                }
                let transcript = formatted.joined(separator: "\n\n")
                await MainActor.run {
                    UIPasteboard.general.string = transcript
                    topBarAlert = WorkbenchTopBarAlert(
                        title: "Export Transcript",
                        message: "Transcript copied to clipboard."
                    )
                }
            } catch {
                await MainActor.run {
                    topBarAlert = WorkbenchTopBarAlert(
                        title: "Export Transcript",
                        message: "Failed to export transcript."
                    )
                }
            }
        }
    }

    private var selectedWorkspace: WorkspaceSummary? {
        if let selectedId = workspaceSelection.workspaceId,
           let workspace = workspaces.first(where: { $0.id == selectedId }) {
            return workspace
        }
        return workspaces.first
    }

    private var selectedTask: WorkspaceCatchupTaskSummary? {
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
        guard let selectedTask else { return nil }
        return resolvePrimarySession(
            for: selectedTask,
            preferredTrackId: workbenchSelection.trackId,
            preferredSessionId: workbenchSelection.sessionId
        ).session
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
            let items = try await client.listWorkspaces()
            workspaces = items
            let resolved = resolveWorkspaceSelection(from: items)
            if let resolved {
                workspaceSelection.setWorkspace(resolved, daemonKey: daemonKey)
            }
            let selectionWorkspaceId = resolved?.id ?? workspaceSelection.workspaceId
            workbenchSelection.setContext(daemonKey: daemonKey, workspaceId: selectionWorkspaceId)
            resetSelectionIfNeeded()
            if selectionWorkspaceId != lastWorkspaceId {
                workbenchSelection.setSelection(taskId: nil, trackId: nil, sessionId: nil)
                lastWorkspaceId = selectionWorkspaceId
            }
        } catch {
            workspaces = []
            workspaceError = "Failed to load workspaces."
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
            return
        }
        let hasTasks = !(activeTasks.isEmpty && archivedTasks.isEmpty)
        let showBlocking = !hasTasks
        if showBlocking {
            isLoadingTasks = true
        } else {
            isRefreshingTasks = true
        }
        taskError = nil
        do {
            let snapshot = try await client.getWorkspaceCatchupSnapshot(workspaceId: workspaceId, includeArchived: true)
            lastStreamSnapshotRev = snapshot.snapshotRev
            activeTasks = snapshot.active.tasks
            archivedTasks = snapshot.archived?.tasks ?? []
            resolveSelectionForCurrentTask()
        } catch {
            if showBlocking {
                activeTasks = []
                archivedTasks = []
            }
            taskError = "Failed to load tasks."
        }
        isLoadingTasks = false
        isRefreshingTasks = false
    }

    private func resolveSelectionForCurrentTask() {
        guard let taskId = workbenchSelection.taskId,
              let task = selectedTask else { return }
        let resolved = resolvePrimarySession(
            for: task,
            preferredTrackId: workbenchSelection.trackId,
            preferredSessionId: workbenchSelection.sessionId
        )
        if resolved.trackId != workbenchSelection.trackId || resolved.sessionId != workbenchSelection.sessionId {
            workbenchSelection.setSelection(taskId: taskId, trackId: resolved.trackId, sessionId: resolved.sessionId)
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

    private func selectTask(_ task: WorkspaceCatchupTaskSummary) {
        let resolved = resolvePrimarySession(for: task, preferredTrackId: nil, preferredSessionId: nil)
        workbenchSelection.setSelection(
            taskId: task.task.id.stringValue,
            trackId: resolved.trackId,
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
            let snapshot = try await client.getWorkspaceCatchupSnapshot(workspaceId: workspaceId, includeArchived: true)
            let allTasks = snapshot.active.tasks + (snapshot.archived?.tasks ?? [])
            guard let task = allTasks.first(where: { $0.task.id.stringValue == taskId }) else { return }
            guard task.task.archivedAt == nil else { return }

            let resolved = resolvePrimarySession(
                for: task,
                preferredTrackId: workbenchSelection.trackId,
                preferredSessionId: workbenchSelection.sessionId
            )
            if let sessionId = resolved.sessionId, let trackId = resolved.trackId {
                workbenchSelection.setSelection(taskId: taskId, trackId: trackId, sessionId: sessionId)
                return
            }

            let trackId: String
            if let resolvedTrackId = resolved.trackId {
                trackId = resolvedTrackId
            } else {
                let createdTrack = try await client.createTrack(taskId: taskId, label: nil, envTarget: "worktree")
                trackId = createdTrack.id.stringValue
            }

            let (providerId, modelId) = try await resolveDefaultSessionConfig(client: client, workspaceId: workspaceId)
            let session = try await client.createSession(trackId: trackId, providerId: providerId, modelId: modelId)
            workbenchSelection.setSelection(taskId: taskId, trackId: trackId, sessionId: session.id.stringValue)
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

    private func beginRename(_ task: WorkspaceCatchupTaskSummary) {
        renameTarget = task
        let current = task.task.title.trimmingCharacters(in: .whitespacesAndNewlines)
        renameText = current.isEmpty ? "New Task" : current
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
    private func toggleArchive(_ task: WorkspaceCatchupTaskSummary) async {
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
    private func markTaskReadIfNeeded(_ task: WorkspaceCatchupTaskSummary) async {
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
        let message = WorkspaceCatchupClientMessage(type: "subscribe", sessionIds: nil, sessions: [])
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
            guard let event = try? streamDecoder.decode(WorkspaceCatchupEvent.self, from: decrypted) else { return }
            handleWorkspaceStreamEvent(event)
            return
        }
        guard let event = try? streamDecoder.decode(WorkspaceCatchupEvent.self, from: data) else { return }
        handleWorkspaceStreamEvent(event)
    }

    @MainActor
    private func handleWorkspaceStreamEvent(_ event: WorkspaceCatchupEvent) {
        let snapshotRev: Int
        switch event {
        case .ready(_, let rev):
            snapshotRev = rev
        case .taskUpsert(_, let rev, _):
            snapshotRev = rev
        case .taskDelete(_, let rev, _):
            snapshotRev = rev
        case .trackUpsert(_, let rev, _):
            snapshotRev = rev
        case .sessionSummary(_, let rev, _):
            snapshotRev = rev
        case .sessionHeadDelta(_, let rev, _):
            snapshotRev = rev
        case .sessionGap(_, let rev, _, _, _):
            snapshotRev = rev
        case .worktreeBootstrap(_, let rev, _):
            snapshotRev = rev
        }

        if snapshotRev > lastStreamSnapshotRev + 1 {
            _Concurrency.Task { await loadTasks() }
        }
        if case .ready = event, snapshotRev != lastStreamSnapshotRev {
            _Concurrency.Task { await loadTasks() }
        }
        lastStreamSnapshotRev = max(lastStreamSnapshotRev, snapshotRev)

        switch event {
        case .taskUpsert(_, _, let task):
            upsertTaskSummary(task)
        case .taskDelete(_, _, let taskId):
            removeTask(taskId: taskId.stringValue)
        case .trackUpsert(_, _, let track):
            applyTrackSummary(track)
        case .sessionSummary(_, _, let summary):
            applySessionSummary(summary)
        case .sessionGap:
            _Concurrency.Task { await loadTasks() }
        default:
            break
        }
    }

    @MainActor
    private func upsertTaskSummary(_ summary: WorkspaceCatchupTaskSummary) {
        let taskId = summary.task.id.stringValue
        activeTasks.removeAll { $0.task.id.stringValue == taskId }
        archivedTasks.removeAll { $0.task.id.stringValue == taskId }
        if summary.task.archivedAt == nil {
            activeTasks = sortTasksBySortAt(activeTasks + [summary])
        } else {
            archivedTasks = sortTasksBySortAt(archivedTasks + [summary])
        }
        resolveSelectionForCurrentTask()
    }

    @MainActor
    private func removeTask(taskId: String) {
        activeTasks.removeAll { $0.task.id.stringValue == taskId }
        archivedTasks.removeAll { $0.task.id.stringValue == taskId }
        if workbenchSelection.taskId == taskId {
            workbenchSelection.setSelection(taskId: nil, trackId: nil, sessionId: nil)
        }
    }

    @MainActor
    private func applyTrackSummary(_ summary: WorkspaceCatchupTrackSummary) {
        let taskId = summary.track.taskId.stringValue
        let trackId = summary.track.id.stringValue
        func update(_ tasks: inout [WorkspaceCatchupTaskSummary]) -> Bool {
            guard let index = tasks.firstIndex(where: { $0.task.id.stringValue == taskId }) else { return false }
            let taskSummary = tasks[index]
            var tracks = taskSummary.tracks
            if let trackIndex = tracks.firstIndex(where: { $0.track.id.stringValue == trackId }) {
                tracks[trackIndex] = summary
            } else {
                tracks.append(summary)
            }
            tasks[index] = WorkspaceCatchupTaskSummary(
                task: taskSummary.task,
                tracks: tracks,
                sortAt: taskSummary.sortAt
            )
            return true
        }
        if !update(&activeTasks) {
            _ = update(&archivedTasks)
        }
        resolveSelectionForCurrentTask()
    }

    @MainActor
    private func applySessionSummary(_ summary: SessionCatchupSummary) {
        let taskId = summary.session.taskId.stringValue
        let trackId = summary.session.trackId.stringValue
        let sessionId = summary.session.id.stringValue
        func update(_ tasks: inout [WorkspaceCatchupTaskSummary]) -> Bool {
            guard let taskIndex = tasks.firstIndex(where: { $0.task.id.stringValue == taskId }) else { return false }
            let taskSummary = tasks[taskIndex]
            guard let trackIndex = taskSummary.tracks.firstIndex(where: { $0.track.id.stringValue == trackId }) else { return false }
            let trackSummary = taskSummary.tracks[trackIndex]
            var sessions = trackSummary.sessions
            if let sessionIndex = sessions.firstIndex(where: { $0.session.id.stringValue == sessionId }) {
                sessions[sessionIndex] = summary
            } else {
                sessions.append(summary)
            }
            let updatedTrack = WorkspaceCatchupTrackSummary(
                track: trackSummary.track,
                primarySessionId: trackSummary.primarySessionId,
                sessions: sessions,
                diffSummary: trackSummary.diffSummary
            )
            var tracks = taskSummary.tracks
            tracks[trackIndex] = updatedTrack
            tasks[taskIndex] = WorkspaceCatchupTaskSummary(
                task: taskSummary.task,
                tracks: tracks,
                sortAt: taskSummary.sortAt
            )
            return true
        }
        if !update(&activeTasks) {
            _ = update(&archivedTasks)
        }
        resolveSelectionForCurrentTask()
    }

    @MainActor
    private func applyTaskUpdate(_ task: Task) {
        let taskId = task.id.stringValue
        func update(_ tasks: inout [WorkspaceCatchupTaskSummary]) -> WorkspaceCatchupTaskSummary? {
            guard let index = tasks.firstIndex(where: { $0.task.id.stringValue == taskId }) else { return nil }
            let summary = tasks[index]
            tasks.remove(at: index)
            return WorkspaceCatchupTaskSummary(task: task, tracks: summary.tracks, sortAt: summary.sortAt)
        }

        if let updated = update(&activeTasks) {
            if updated.task.archivedAt == nil {
                activeTasks = sortTasksBySortAt(activeTasks + [updated])
            } else {
                archivedTasks = sortTasksBySortAt(archivedTasks + [updated])
            }
            return
        }

        if let updated = update(&archivedTasks) {
            if updated.task.archivedAt == nil {
                activeTasks = sortTasksBySortAt(activeTasks + [updated])
            } else {
                archivedTasks = sortTasksBySortAt(archivedTasks + [updated])
            }
        }
    }
}

private struct WorkbenchTopBarAlert: Identifiable {
    let id = UUID()
    let title: String
    let message: String
}

private struct WorkbenchHomeView: View {
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let isLoadingTasks: Bool
    let taskError: String?
    let selectedSession: SessionSummary?
    @Binding var isArtifactsPresented: Bool
    let hasTaskSelection: Bool
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
                isArtifactsPresented: $isArtifactsPresented,
                hasTaskSelection: hasTaskSelection,
                isPreparingSession: isPreparingSession,
                onTaskCreated: onTaskCreated
            )
                .padding(.top, 8)
        }
    }
}

private enum WorkbenchTopBarAction {
    case artifacts
    case diff
    case sessions
    case terminal
}

private struct WorkbenchTaskMenuContext {
    let title: String
    let taskId: String
    let isArchived: Bool
    let onRename: () -> Void
    let onArchiveToggle: () -> Void
    let onCopyTitle: () -> Void
    let onCopyId: () -> Void
    let onCopyTranscript: () -> Void
}

private struct WorkbenchTopBar: View {
    let title: String
    let onMenuTap: () -> Void
    let onArtifactsTap: () -> Void
    let onDiffTap: () -> Void
    let onSessionsTap: () -> Void
    let onTerminalTap: () -> Void
    let taskMenuContext: WorkbenchTaskMenuContext?

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

            HStack(spacing: 14) {
                Button(action: onArtifactsTap) {
                    Image(systemName: "photo.stack")
                        .font(.system(size: 16, weight: .semibold))
                }
                .accessibilityIdentifier("topbar.artifacts")

                Button(action: onDiffTap) {
                    Image(systemName: "doc.text.magnifyingglass")
                        .font(.system(size: 16, weight: .semibold))
                }
                .accessibilityIdentifier("topbar.diff")

                Button(action: onSessionsTap) {
                    Image(systemName: "rectangle.stack")
                        .font(.system(size: 16, weight: .semibold))
                }
                .accessibilityIdentifier("topbar.sessions")

                Button(action: onTerminalTap) {
                    Image(systemName: "terminal")
                        .font(.system(size: 16, weight: .semibold))
                }
                .accessibilityIdentifier("topbar.terminal")

                Menu {
                    if let context = taskMenuContext {
                        Button("Rename task", action: context.onRename)
                        Button(context.isArchived ? "Unarchive task" : "Archive task", action: context.onArchiveToggle)
                        Divider()
                        Button("Copy task title", action: context.onCopyTitle)
                        Button("Copy task ID", action: context.onCopyId)
                        Button("Export transcript", action: context.onCopyTranscript)
                    } else {
                        Text("Select a task to manage it.")
                    }
                } label: {
                    Image(systemName: "ellipsis.circle")
                        .font(.system(size: 16, weight: .semibold))
                }
                .accessibilityIdentifier("topbar.taskmenu")
                .disabled(taskMenuContext == nil)
            }
            .foregroundColor(.ctxTextPrimary)
        }
    }
}

private struct WorkbenchDrawerView: View {
    let workspaces: [WorkspaceSummary]
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let activeTasks: [WorkspaceCatchupTaskSummary]
    let archivedTasks: [WorkspaceCatchupTaskSummary]
    let isLoadingTasks: Bool
    let isRefreshingTasks: Bool
    let taskError: String?
    let activeTaskId: String?
    @Binding var taskQuery: String
    let drawerWidth: CGFloat
    let onRefresh: () -> Void
    let onSelectTask: (WorkspaceCatchupTaskSummary) -> Void
    let onNewTask: () -> Void
    let onRenameTask: (WorkspaceCatchupTaskSummary) -> Void
    let onArchiveToggle: (WorkspaceCatchupTaskSummary) -> Void
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
                                    Button {
                                        onSelectTask(task)
                                    } label: {
                                        WorkbenchTaskRowView(
                                            task: task,
                                            isSelected: activeTaskId == task.task.id.stringValue
                                        )
                                    }
                                    .buttonStyle(.plain)
                                    .contextMenu {
                                        Button("Rename") { onRenameTask(task) }
                                        let archived = task.task.archivedAt != nil
                                        Button(archived ? "Unarchive" : "Archive") { onArchiveToggle(task) }
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
                                        Button {
                                            onSelectTask(task)
                                        } label: {
                                            WorkbenchTaskRowView(
                                                task: task,
                                                isSelected: activeTaskId == task.task.id.stringValue
                                            )
                                        }
                                        .buttonStyle(.plain)
                                        .contextMenu {
                                            Button("Rename") { onRenameTask(task) }
                                            Button("Unarchive") { onArchiveToggle(task) }
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
                                Image(systemName: "chevron.down")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextSecondary)
                                    .rotationEffect(.degrees(showArchived ? 0 : -90))
                            }
                        }
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
        .safeAreaInset(edge: .top, spacing: 0) {
            Color.clear.frame(height: 12)
        }
        .frame(width: drawerWidth)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .stroke(Color.ctxGlassStroke, lineWidth: 0.8)
        )
        .shadow(color: Color.ctxShadow, radius: 24, x: 0, y: 12)
        .padding(.bottom, 24)
        .padding(.leading, 12)
    }
}

private struct DrawerLinkRowView: View {
    let title: String
    let icon: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .foregroundColor(.ctxAccent)
            Text(title)
                .foregroundColor(.ctxTextPrimary)
                .font(.subheadline.weight(.semibold))
            Spacer()
            Image(systemName: "chevron.right")
                .foregroundColor(.ctxTextSecondary)
                .font(.caption)
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct WorkbenchWorkspaceSwitcherView: View {
    let workspaces: [WorkspaceSummary]
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let onRefresh: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Workspace")
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
                Spacer()
                Button(action: onRefresh) {
                    Image(systemName: "arrow.clockwise")
                        .foregroundColor(.ctxTextSecondary)
                        .font(.caption)
                }
            }

            if isLoadingWorkspaces {
                ProgressView()
                    .tint(.ctxAccent)
            } else if let workspaceError {
                Text(workspaceError)
                    .font(.caption)
                    .foregroundColor(.ctxError)
            } else if workspaces.isEmpty {
                Text("No workspaces connected yet.")
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            } else {
                NavigationLink {
                    WorkspaceSwitchView()
                } label: {
                    HStack(spacing: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(selectedWorkspace?.name ?? "No workspace")
                                .font(.subheadline.weight(.semibold))
                                .foregroundColor(.ctxTextPrimary)
                            if let detail = selectedWorkspace?.rootPath {
                                Text(workspaceDetailText(detail))
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                            }
                        }
                        Spacer()
                        HStack(spacing: 6) {
                            Text("Switch")
                                .font(.subheadline.weight(.semibold))
                            Image(systemName: "chevron.right")
                                .font(.caption)
                        }
                        .foregroundColor(.ctxAccent)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(Color.ctxSurfaceRaised, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("drawer.workspace.switch")
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
    }

    private func workspaceDetailText(_ rootPath: String) -> String {
        let url = URL(fileURLWithPath: rootPath)
        return url.lastPathComponent.isEmpty ? rootPath : url.lastPathComponent
    }
}

private struct WorkbenchNavigationFlowView: View {
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let isLoadingTasks: Bool
    let taskError: String?
    let selectedSession: SessionSummary?
    @Binding var isArtifactsPresented: Bool
    let hasTaskSelection: Bool
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
                    ChatDetailView(session: selectedSession, isArtifactsPresented: $isArtifactsPresented)
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
            .id(workspace.id)
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
    @State private var isLoadingProviders = false
    @State private var isLoadingModels = false
    @State private var errorMessage: String?
    @State private var isSubmitting = false
    @FocusState private var isPromptFocused: Bool

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 18) {
                Text("New task")
                    .font(.title3.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)

                if isLoadingTasks {
                    Text("Refreshing tasks...")
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                } else if let taskError {
                    WorkbenchInfoCard(text: taskError, tint: .ctxError)
                }

                GlassPanel {
                    VStack(alignment: .leading, spacing: 14) {
                        Text("Prompt")
                            .font(.caption.weight(.semibold))
                            .foregroundColor(.ctxTextMuted)
                        CtxTextArea(
                            placeholder: "Describe what you want...",
                            text: $prompt,
                            accessibilityId: "newtask.prompt",
                            isFocused: $isPromptFocused
                        )

                        WorkbenchMenuPicker(
                            title: "Harness",
                            value: providerLabel,
                            isDisabled: providerOptionsDisabled,
                            options: availableProviderIds,
                            onSelect: { id in
                                selectedProviderId = id
                                selectedModelId = ""
                            }
                        )

                        WorkbenchMenuPicker(
                            title: "Model",
                            value: modelLabel,
                            isDisabled: modelOptionsDisabled,
                            options: modelChoices,
                            onSelect: { id in
                                selectedModelId = id
                            }
                        )
                    }
                }

                if let errorMessage {
                    WorkbenchInfoCard(text: errorMessage, tint: .ctxError)
                }

                Button(action: startTask) {
                    Text(isSubmitting ? "Starting..." : "Start")
                }
                .buttonStyle(CtxPrimaryButtonStyle())
                .disabled(!canSubmit)
                .accessibilityIdentifier("newtask.start")
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .task(id: workspace.id) {
            await loadProviders()
        }
        .task(id: effectiveProviderId) {
            await loadModels()
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

    private var promptTrimmed: String {
        prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var availableProviderIds: [String] {
        let installed = providers.filter { $0.installed && $0.health == "ok" }
        return installed.map { $0.providerId }.filter { !$0.isEmpty }
    }

    private var effectiveProviderId: String {
        if !selectedProviderId.isEmpty {
            return selectedProviderId
        }
        if availableProviderIds.contains("codex") {
            return "codex"
        }
        return availableProviderIds.first ?? ""
    }

    private var providerLabel: String {
        if isLoadingProviders { return "Loading..." }
        if availableProviderIds.isEmpty { return "No harnesses" }
        return effectiveProviderId
    }

    private var modelChoices: [String] {
        models.isEmpty ? ["default"] : models
    }

    private var effectiveModelId: String {
        if !selectedModelId.isEmpty {
            return selectedModelId
        }
        return modelChoices.first ?? "default"
    }

    private var modelLabel: String {
        if isLoadingModels { return "Loading..." }
        if modelChoices.isEmpty { return "default" }
        return effectiveModelId
    }

    private var providerOptionsDisabled: Bool {
        isLoadingProviders || availableProviderIds.isEmpty
    }

    private var modelOptionsDisabled: Bool {
        isLoadingModels || effectiveProviderId.isEmpty || modelChoices.isEmpty
    }

    private var canSubmit: Bool {
        !promptTrimmed.isEmpty && !effectiveProviderId.isEmpty && !effectiveModelId.isEmpty && !isSubmitting
    }

    @MainActor
    private func loadProviders() async {
        guard let client = connection.apiClient else {
            providers = []
            errorMessage = "Connect to a daemon to start."
            isLoadingProviders = false
            return
        }
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
            if !models.contains(selectedModelId) {
                selectedModelId = ""
            }
        } catch {
            models = []
            errorMessage = "Failed to load models."
        }
        isLoadingModels = false
    }

    private func startTask() {
        let promptValue = promptTrimmed
        guard !promptValue.isEmpty else { return }
        let providerId = effectiveProviderId
        let modelId = effectiveModelId
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
                    title: "New Task",
                    description: nil,
                    createDefaultTrack: false,
                    defaultTrackLabel: nil
                )
                let track = try await client.createTrack(taskId: task.id.stringValue, label: nil, envTarget: "worktree")
                let session = try await client.createSession(trackId: track.id.stringValue, providerId: providerId, modelId: modelId)
                _ = try await client.postMessage(sessionId: session.id.stringValue, content: promptValue, delivery: .immediate, attachments: [])
                await MainActor.run {
                    workbenchSelection.setSelection(
                        taskId: task.id.stringValue,
                        trackId: track.id.stringValue,
                        sessionId: session.id.stringValue
                    )
                    prompt = ""
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
                    Image(systemName: "chevron.down")
                        .foregroundColor(.ctxTextSecondary)
                        .font(.caption)
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
    let trackId: String?
    let sessionId: String?
    let session: SessionSummary?
}

private func resolvePrimarySession(
    for task: WorkspaceCatchupTaskSummary,
    preferredTrackId: String?,
    preferredSessionId: String?
) -> ResolvedPrimarySession {
    let tracks = task.tracks
    guard !tracks.isEmpty else {
        return ResolvedPrimarySession(trackId: nil, sessionId: nil, session: nil)
    }
    let selectedTrack = tracks.first(where: { $0.track.id.stringValue == preferredTrackId }) ?? tracks.first
    guard let selectedTrack else {
        return ResolvedPrimarySession(trackId: nil, sessionId: nil, session: nil)
    }
    let trackId = selectedTrack.track.id.stringValue
    let sessions = selectedTrack.sessions
    guard !sessions.isEmpty else {
        return ResolvedPrimarySession(trackId: trackId, sessionId: nil, session: nil)
    }

    let nonSubagents = sessions.filter { $0.session.relationship != "sub_agent" }
    let candidates = nonSubagents.isEmpty ? sessions : nonSubagents

    if let preferredSessionId,
       let match = candidates.first(where: { $0.session.id.stringValue == preferredSessionId }) {
        let summary = SessionSummary(session: match.session)
        return ResolvedPrimarySession(trackId: trackId, sessionId: preferredSessionId, session: summary)
    }

    if let primaryId = selectedTrack.primarySessionId?.stringValue,
       let match = candidates.first(where: { $0.session.id.stringValue == primaryId }) {
        let summary = SessionSummary(session: match.session)
        return ResolvedPrimarySession(trackId: trackId, sessionId: primaryId, session: summary)
    }

    if let running = candidates.first(where: { $0.session.status == "active" || $0.session.status == "running" }) {
        let summary = SessionSummary(session: running.session)
        let sessionId = running.session.id.stringValue
        return ResolvedPrimarySession(trackId: trackId, sessionId: sessionId, session: summary)
    }

    if let recent = mostRecentSession(in: candidates) {
        let summary = SessionSummary(session: recent.session)
        let sessionId = recent.session.id.stringValue
        return ResolvedPrimarySession(trackId: trackId, sessionId: sessionId, session: summary)
    }

    return ResolvedPrimarySession(trackId: trackId, sessionId: nil, session: nil)
}

private func mostRecentSession(in sessions: [SessionCatchupSummary]) -> SessionCatchupSummary? {
    sessions.max { left, right in
        let leftKey = left.session.updatedAt ?? left.session.createdAt ?? ""
        let rightKey = right.session.updatedAt ?? right.session.createdAt ?? ""
        return leftKey < rightKey
    }
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
    guard let preferred = installed.first(where: { $0.providerId == "codex" }) ?? installed.first else {
        throw DaemonAPIError.requestFailed(statusCode: 400, message: "No available harnesses.")
    }
    let providerId = preferred.providerId
    let options = try await client.getProviderOptions(workspaceId: workspaceId, providerId: providerId)
    let modelIds = extractModelIds(from: options.models)
    let modelId = modelIds.first ?? "default"
    return (providerId, modelId)
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
    let task: WorkspaceCatchupTaskSummary
    let isSelected: Bool

    private var title: String {
        let trimmed = task.task.title.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "New Task" : trimmed
    }

    private var indicators: TaskRowIndicators {
        TaskRowIndicators(task: task)
    }

    var body: some View {
        HStack(spacing: 8) {
            TaskHarnessView(
                providerIds: indicators.providerIds
            )
            .frame(width: 18, height: 18)

            Text(title)
                .font(.system(size: 12.5))
                .foregroundColor(.ctxTextPrimary)
                .lineLimit(1)
                .truncationMode(.tail)

            Spacer()

            HStack(spacing: 6) {
                Text(indicators.ageLabel)
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

    private var primaryProviderId: String? {
        providerIds.first
    }

    var body: some View {
        let base = RoundedRectangle(cornerRadius: 6, style: .continuous)
        if let providerId = primaryProviderId,
           let harness = HarnessCatalog.entry(for: providerId) {
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

private struct TaskRowIndicators {
    let providerIds: [String]
    let isWorking: Bool
    let dotKind: TaskRowDotKind?
    let ageLabel: String

    init(task: WorkspaceCatchupTaskSummary) {
        providerIds = resolveProviderIds(for: task)
        isWorking = taskHasWorkingSession(task)
        let hasError = taskHasErrorSession(task)
        let unread = taskHasUnread(task, isWorking: isWorking)
        if hasError {
            dotKind = .error
        } else if unread {
            dotKind = .unread
        } else {
            dotKind = nil
        }
        ageLabel = formatRelativeAgeShort(task.task.lastActivityAt ?? task.task.updatedAt ?? task.task.createdAt)
    }
}

private enum TaskRowDotKind {
    case unread
    case error
}


private func taskHasWorkingSession(_ task: WorkspaceCatchupTaskSummary) -> Bool {
    for track in task.tracks {
        for summary in track.sessions {
            let status = summary.session.status.lowercased()
            if status == "failed" || status == "cancelled" || status == "completed" {
                continue
            }
            if summary.activity?.isWorking == true {
                return true
            }
        }
    }
    return false
}

private func taskHasErrorSession(_ task: WorkspaceCatchupTaskSummary) -> Bool {
    for track in task.tracks {
        for summary in track.sessions {
            let status = summary.session.status.lowercased()
            if status == "failed" || status == "cancelled" {
                return true
            }
        }
    }
    return false
}

private func taskHasUnread(_ task: WorkspaceCatchupTaskSummary, isWorking: Bool) -> Bool {
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

private func resolveProviderIds(for task: WorkspaceCatchupTaskSummary) -> [String] {
    var ordered: [String] = []
    for track in task.tracks {
        for summary in track.sessions {
            let providerId = summary.session.providerId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !providerId.isEmpty else { continue }
            if !ordered.contains(providerId) {
                ordered.append(providerId)
            }
        }
    }
    return ordered
}

private func filterTasks(_ tasks: [WorkspaceCatchupTaskSummary], matching query: String) -> [WorkspaceCatchupTaskSummary] {
    guard !query.isEmpty else { return tasks }
    return tasks.filter { task in
        let title = task.task.title.lowercased()
        return title.contains(query) || task.task.id.stringValue.lowercased().contains(query)
    }
}

private func sortTasksBySortAt(_ tasks: [WorkspaceCatchupTaskSummary]) -> [WorkspaceCatchupTaskSummary] {
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
}
