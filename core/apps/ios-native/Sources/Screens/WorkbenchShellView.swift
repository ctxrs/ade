import SwiftUI
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
    @State private var taskError: String?
    @State private var lastWorkspaceId: String?
    @State private var sessionCreationTaskId: String?
    @State private var didResetSelection = false

    var body: some View {
        GeometryReader { proxy in
            let drawerWidth = min(320, proxy.size.width * 0.78)
            let workspaceName = selectedWorkspace?.name ?? "Workspace"
            let workspaceDetail = selectedWorkspace.map(workspaceDetailText) ?? "Select in drawer"

            ZStack(alignment: .leading) {
                CtxBackgroundView()

                WorkbenchHomeView(
                    selectedWorkspace: selectedWorkspace,
                    isLoadingWorkspaces: isLoadingWorkspaces,
                    workspaceError: workspaceError,
                    isLoadingTasks: isLoadingTasks,
                    taskError: taskError,
                    selectedSession: resolvedSession,
                    hasTaskSelection: workbenchSelection.taskId != nil,
                    isPreparingSession: isPreparingSession,
                    onTaskCreated: { _Concurrency.Task { await loadTasks() } }
                )
                .safeAreaInset(edge: .top, spacing: 0) {
                    WorkbenchTopBar(
                        workspaceName: workspaceName,
                        workspaceDetail: workspaceDetail,
                        onMenuTap: { isDrawerOpen = true },
                        onWorkspaceTap: { isDrawerOpen = true }
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
                    taskError: taskError,
                    activeTaskId: workbenchSelection.taskId,
                    drawerWidth: drawerWidth,
                    onClose: { isDrawerOpen = false },
                    onRefresh: { _Concurrency.Task { await loadWorkspaces() } },
                    onSelectTask: { task in
                        selectTask(task)
                        isDrawerOpen = false
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
        }
        .onChange(of: connection.isConnected) { connected in
            guard connected else { return }
            _Concurrency.Task {
                await loadWorkspaces()
                await loadTasks()
            }
        }
        .onChange(of: workbenchSelection.taskId) { _ in
            resolveSelectionForCurrentTask()
        }
    }

    private func workspaceDetailText(_ workspace: WorkspaceSummary) -> String {
        let url = URL(fileURLWithPath: workspace.rootPath)
        return url.lastPathComponent.isEmpty ? workspace.rootPath : url.lastPathComponent
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
            return
        }
        isLoadingTasks = true
        taskError = nil
        do {
            let snapshot = try await client.getWorkspaceCatchupSnapshot(workspaceId: workspaceId, includeArchived: true)
            activeTasks = snapshot.active.tasks
            archivedTasks = snapshot.archived?.tasks ?? []
            resolveSelectionForCurrentTask()
        } catch {
            activeTasks = []
            archivedTasks = []
            taskError = "Failed to load tasks."
        }
        isLoadingTasks = false
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
}

private struct WorkbenchHomeView: View {
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let isLoadingTasks: Bool
    let taskError: String?
    let selectedSession: SessionSummary?
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
                hasTaskSelection: hasTaskSelection,
                isPreparingSession: isPreparingSession,
                onTaskCreated: onTaskCreated
            )
                .padding(.top, 8)
        }
    }
}

private struct WorkbenchTopBar: View {
    let workspaceName: String
    let workspaceDetail: String
    let onMenuTap: () -> Void
    let onWorkspaceTap: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button(action: onMenuTap) {
                Image(systemName: "line.3.horizontal")
                    .font(.title2)
            }
            .foregroundColor(.ctxTextPrimary)
            .accessibilityIdentifier("drawer.open")

            Button(action: onWorkspaceTap) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(workspaceName)
                        .font(.headline)
                        .foregroundColor(.ctxTextPrimary)
                    Text(workspaceDetail)
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                }
            }

            Spacer()

            GlassPill(text: "Connected", tint: .ctxAccent)
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
    let taskError: String?
    let activeTaskId: String?
    let drawerWidth: CGFloat
    let onClose: () -> Void
    let onRefresh: () -> Void
    let onSelectTask: (WorkspaceCatchupTaskSummary) -> Void
    @State private var showArchived = false

    var body: some View {
        VStack(spacing: 0) {
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    HStack {
                        Text("ctx")
                            .font(.title2.weight(.semibold))
                            .foregroundColor(.ctxTextPrimary)
                        Spacer()
                        Button(action: onClose) {
                            Image(systemName: "xmark")
                                .font(.headline)
                        }
                        .foregroundColor(.ctxTextSecondary)
                        .accessibilityIdentifier("drawer.close")
                    }

                    VStack(alignment: .leading, spacing: 12) {
                        Text("Tasks")
                            .font(.caption.weight(.semibold))
                            .foregroundColor(.ctxTextMuted)
                        if selectedWorkspace == nil {
                            Text("Select a workspace in settings to view tasks.")
                                .font(.caption)
                                .foregroundColor(.ctxTextMuted)
                        } else {
                            if isLoadingTasks {
                                ProgressView()
                                    .tint(.ctxAccent)
                            } else if let taskError {
                                Text(taskError)
                                    .font(.caption)
                                    .foregroundColor(.ctxError)
                            } else {
                                if activeTasks.isEmpty {
                                    Text("No active tasks.")
                                        .font(.caption)
                                        .foregroundColor(.ctxTextMuted)
                                } else {
                                    VStack(spacing: 10) {
                                        ForEach(activeTasks, id: \.task.id) { task in
                                            Button {
                                                onSelectTask(task)
                                            } label: {
                                                WorkbenchNavRowView(
                                                    title: task.task.title,
                                                    subtitle: task.task.description ?? "Status: \(task.task.status)",
                                                    status: task.task.status.capitalized,
                                                    icon: "list.bullet.rectangle",
                                                    showsChevron: false,
                                                    isSelected: activeTaskId == task.task.id.stringValue
                                                )
                                            }
                                            .buttonStyle(.plain)
                                            .accessibilityIdentifier("drawer.task.\(task.task.id.stringValue)")
                                        }
                                    }
                                }

                                DisclosureGroup(isExpanded: $showArchived) {
                                    if archivedTasks.isEmpty {
                                        Text("No archived tasks.")
                                            .font(.caption)
                                            .foregroundColor(.ctxTextMuted)
                                            .padding(.top, 6)
                                    } else {
                                        VStack(spacing: 10) {
                                            ForEach(archivedTasks, id: \.task.id) { task in
                                                Button {
                                                    onSelectTask(task)
                                                } label: {
                                                    WorkbenchNavRowView(
                                                        title: task.task.title,
                                                        subtitle: task.task.description ?? "Status: \(task.task.status)",
                                                        status: task.task.status.capitalized,
                                                        icon: "archivebox",
                                                        showsChevron: false,
                                                        isSelected: activeTaskId == task.task.id.stringValue
                                                    )
                                                }
                                                .buttonStyle(.plain)
                                                .accessibilityIdentifier("drawer.task.\(task.task.id.stringValue)")
                                            }
                                        }
                                        .padding(.top, 8)
                                    }
                                } label: {
                                    HStack {
                                        Text("Archived")
                                            .font(.caption.weight(.semibold))
                                            .foregroundColor(.ctxTextMuted)
                                        Spacer()
                                        Text("\(archivedTasks.count)")
                                            .font(.caption)
                                            .foregroundColor(.ctxTextSecondary)
                                    }
                                }
                                .accessibilityIdentifier("drawer.archived.toggle")
                                .accentColor(.ctxTextSecondary)
                            }
                        }
                    }

                    VStack(alignment: .leading, spacing: 12) {
                        Text("Shortcuts")
                            .font(.caption.weight(.semibold))
                            .foregroundColor(.ctxTextMuted)
                        NavigationLink {
                            SettingsView(selectedWorkspace: selectedWorkspace)
                        } label: {
                            DrawerLinkRowView(title: "Settings", icon: "gearshape")
                        }
                        NavigationLink {
                            DiagnosticsView()
                        } label: {
                            DrawerLinkRowView(title: "Diagnostics", icon: "waveform.path.ecg")
                        }
                    }

                    HStack(spacing: 12) {
                        GlassPill(text: "Daemon healthy", tint: .ctxAccent)
                        Spacer()
                        Image(systemName: "antenna.radiowaves.left.and.right")
                            .foregroundColor(.ctxTextSecondary)
                    }
                }
                .padding(20)
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
                    ChatDetailView(session: selectedSession)
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
            let isUITest = ProcessInfo.processInfo.environment["CTX_UI_TEST_MODE"] == "1"
            if isUITest {
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

#Preview {
    WorkbenchShellView()
        .environmentObject(ConnectionStore())
        .environmentObject(WorkspaceSelectionStore())
        .environmentObject(WorkbenchSelectionStore())
}
