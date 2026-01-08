import SwiftUI

struct WorkbenchShellView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @State private var isDrawerOpen = false
    @State private var workspaces: [WorkspaceSummary] = []
    @State private var selectedWorkspace: WorkspaceSummary?
    @State private var isLoadingWorkspaces = false
    @State private var workspaceError: String?

    var body: some View {
        GeometryReader { proxy in
            let drawerWidth = min(320, proxy.size.width * 0.78)
            let topInset = proxy.safeAreaInsets.top
            let workspaceName = selectedWorkspace?.name ?? "Workspace"
            let workspaceDetail = selectedWorkspace.map(workspaceDetailText) ?? "Select in drawer"

            ZStack(alignment: .leading) {
                CtxBackgroundView()

                WorkbenchHomeView(
                    isDrawerOpen: $isDrawerOpen,
                    workspaceName: workspaceName,
                    workspaceDetail: workspaceDetail,
                    selectedWorkspace: selectedWorkspace,
                    isLoadingWorkspaces: isLoadingWorkspaces,
                    workspaceError: workspaceError,
                    topInset: topInset
                )
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
                    selectedWorkspaceId: selectedWorkspace?.id,
                    isLoadingWorkspaces: isLoadingWorkspaces,
                    workspaceError: workspaceError,
                    drawerWidth: drawerWidth,
                    onClose: { isDrawerOpen = false },
                    onRefresh: { Task { await loadWorkspaces() } },
                    onSelectWorkspace: { workspace in
                        selectedWorkspace = workspace
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
        .onChange(of: connection.isConnected) { connected in
            guard connected else { return }
            Task { await loadWorkspaces() }
        }
    }

    private func workspaceDetailText(_ workspace: WorkspaceSummary) -> String {
        let url = URL(fileURLWithPath: workspace.rootPath)
        return url.lastPathComponent.isEmpty ? workspace.rootPath : url.lastPathComponent
    }

    @MainActor
    private func loadWorkspaces() async {
        guard let client = connection.apiClient else {
            workspaces = []
            selectedWorkspace = nil
            return
        }
        isLoadingWorkspaces = true
        workspaceError = nil
        do {
            let items = try await client.listWorkspaces()
            workspaces = items
            if let selected = selectedWorkspace,
               let refreshed = items.first(where: { $0.id == selected.id }) {
                selectedWorkspace = refreshed
            } else {
                selectedWorkspace = items.first
            }
        } catch {
            workspaces = []
            selectedWorkspace = nil
            workspaceError = "Failed to load workspaces."
        }
        isLoadingWorkspaces = false
    }
}

private struct WorkbenchHomeView: View {
    @Binding var isDrawerOpen: Bool
    let workspaceName: String
    let workspaceDetail: String
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let topInset: CGFloat

    var body: some View {
        VStack(spacing: 0) {
            WorkbenchTopBar(
                workspaceName: workspaceName,
                workspaceDetail: workspaceDetail,
                onMenuTap: { isDrawerOpen = true },
                onWorkspaceTap: { isDrawerOpen = true }
            )
            .padding(.horizontal, 20)
            .padding(.top, topInset + 8)

            WorkbenchNavigationFlowView(
                selectedWorkspace: selectedWorkspace,
                isLoadingWorkspaces: isLoadingWorkspaces,
                workspaceError: workspaceError
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
    let selectedWorkspaceId: String?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?
    let drawerWidth: CGFloat
    let onClose: () -> Void
    let onRefresh: () -> Void
    let onSelectWorkspace: (WorkspaceSummary) -> Void

    var body: some View {
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
                }

                VStack(alignment: .leading, spacing: 12) {
                    HStack {
                        Text("Workspaces")
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
                        ForEach(workspaces) { workspace in
                            Button {
                                onSelectWorkspace(workspace)
                            } label: {
                                DrawerWorkspaceRowView(
                                    workspace: workspace,
                                    isSelected: workspace.id == selectedWorkspaceId
                                )
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }

                VStack(alignment: .leading, spacing: 12) {
                    Text("Shortcuts")
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.ctxTextMuted)
                    NavigationLink {
                        SettingsView()
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
        .frame(width: drawerWidth)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .stroke(Color.ctxGlassStroke, lineWidth: 0.8)
        )
        .shadow(color: Color.ctxShadow, radius: 24, x: 0, y: 12)
        .padding(.top, 12)
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

private struct WorkbenchNavigationFlowView: View {
    let selectedWorkspace: WorkspaceSummary?
    let isLoadingWorkspaces: Bool
    let workspaceError: String?

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
                TaskListView(workspace: workspace)
            }
            .id(workspace.id)
            .toolbar(.hidden, for: .navigationBar)
        } else {
            WorkbenchEmptyStateView(
                title: "No workspace selected",
                message: "Pick a workspace from the drawer to start browsing tasks."
            )
        }
    }
}

private struct TaskListView: View {
    @EnvironmentObject private var connection: ConnectionStore
    let workspace: WorkspaceSummary
    @State private var tasks: [TaskSummary] = []
    @State private var isLoading = false
    @State private var errorMessage: String?

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 16) {
                WorkbenchListHeader(title: "Tasks", subtitle: workspace.name, showsBack: false)
                content
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .task(id: workspace.id) {
            await loadTasks()
        }
        .onChange(of: connection.isConnected) { connected in
            guard connected else { return }
            Task { await loadTasks() }
        }
    }

    @ViewBuilder
    private var content: some View {
        if isLoading {
            ProgressView()
                .tint(.ctxAccent)
        } else if let errorMessage {
            WorkbenchInfoCard(text: errorMessage, tint: .ctxError)
        } else if tasks.isEmpty {
            WorkbenchInfoCard(text: "No tasks yet.", tint: .ctxTextMuted)
        } else {
            VStack(spacing: 12) {
                ForEach(tasks) { task in
                    NavigationLink {
                        TrackListView(task: task)
                    } label: {
                        WorkbenchNavRowView(
                            title: task.title,
                            subtitle: task.description ?? "Status: \(task.status)",
                            status: task.status.capitalized,
                            icon: "list.bullet.rectangle",
                            showsChevron: true
                        )
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    @MainActor
    private func loadTasks() async {
        guard let client = connection.apiClient else { return }
        isLoading = true
        errorMessage = nil
        do {
            tasks = try await client.listTasks(workspaceId: workspace.id)
        } catch {
            errorMessage = "Failed to load tasks."
            tasks = []
        }
        isLoading = false
    }
}

private struct TrackListView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @Environment(\.dismiss) private var dismiss
    let task: TaskSummary
    @State private var tracks: [TrackSummary] = []
    @State private var isLoading = false
    @State private var errorMessage: String?

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 16) {
                WorkbenchListHeader(
                    title: "Tracks",
                    subtitle: task.title,
                    showsBack: true,
                    onBack: { dismiss() }
                )
                content
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .task(id: task.id) {
            await loadTracks()
        }
        .onChange(of: connection.isConnected) { connected in
            guard connected else { return }
            Task { await loadTracks() }
        }
    }

    @ViewBuilder
    private var content: some View {
        if isLoading {
            ProgressView()
                .tint(.ctxAccent)
        } else if let errorMessage {
            WorkbenchInfoCard(text: errorMessage, tint: .ctxError)
        } else if tracks.isEmpty {
            WorkbenchInfoCard(text: "No tracks yet.", tint: .ctxTextMuted)
        } else {
            VStack(spacing: 12) {
                ForEach(tracks) { track in
                    NavigationLink {
                        SessionListView(track: track)
                    } label: {
                        WorkbenchNavRowView(
                            title: track.label,
                            subtitle: "Worktree \(track.worktreeId.prefix(8))",
                            status: track.status.capitalized,
                            icon: "point.topleft.down.curvedto.point.bottomright.up",
                            showsChevron: true
                        )
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    @MainActor
    private func loadTracks() async {
        guard let client = connection.apiClient else { return }
        isLoading = true
        errorMessage = nil
        do {
            tracks = try await client.listTracks(taskId: task.id)
        } catch {
            errorMessage = "Failed to load tracks."
            tracks = []
        }
        isLoading = false
    }
}

private struct SessionListView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @Environment(\.dismiss) private var dismiss
    let track: TrackSummary
    @State private var sessions: [SessionSummary] = []
    @State private var isLoading = false
    @State private var errorMessage: String?

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 16) {
                WorkbenchListHeader(
                    title: "Sessions",
                    subtitle: track.label,
                    showsBack: true,
                    onBack: { dismiss() }
                )
                content
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 20)
        }
        .task(id: track.id) {
            await loadSessions()
        }
        .onChange(of: connection.isConnected) { connected in
            guard connected else { return }
            Task { await loadSessions() }
        }
    }

    @ViewBuilder
    private var content: some View {
        if isLoading {
            ProgressView()
                .tint(.ctxAccent)
        } else if let errorMessage {
            WorkbenchInfoCard(text: errorMessage, tint: .ctxError)
        } else if sessions.isEmpty {
            WorkbenchInfoCard(text: "No sessions yet.", tint: .ctxTextMuted)
        } else {
            VStack(spacing: 12) {
                ForEach(sessions) { session in
                    WorkbenchNavRowView(
                        title: session.title,
                        subtitle: "\(session.agentRole) / \(session.providerId) / \(session.modelId)",
                        status: session.status.capitalized,
                        icon: "bubble.left.and.bubble.right",
                        showsChevron: false
                    )
                }
            }
        }
    }

    @MainActor
    private func loadSessions() async {
        guard let client = connection.apiClient else { return }
        isLoading = true
        errorMessage = nil
        do {
            sessions = try await client.listSessions(forTrack: track.id)
        } catch {
            errorMessage = "Failed to load sessions."
            sessions = []
        }
        isLoading = false
    }
}

private struct WorkbenchListHeader: View {
    let title: String
    let subtitle: String
    let showsBack: Bool
    var onBack: (() -> Void)?

    init(title: String, subtitle: String, showsBack: Bool, onBack: (() -> Void)? = nil) {
        self.title = title
        self.subtitle = subtitle
        self.showsBack = showsBack
        self.onBack = onBack
    }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            if showsBack, let onBack {
                Button(action: onBack) {
                    Image(systemName: "chevron.left")
                        .foregroundColor(.ctxTextPrimary)
                        .font(.headline)
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.title3.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
        }
    }
}

private struct WorkbenchNavRowView: View {
    let title: String
    let subtitle: String
    let status: String?
    let icon: String
    let showsChevron: Bool

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
            if showsChevron {
                Image(systemName: "chevron.right")
                    .foregroundColor(.ctxTextSecondary)
                    .font(.caption)
            }
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
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

#Preview {
    WorkbenchShellView()
        .environmentObject(ConnectionStore())
}
