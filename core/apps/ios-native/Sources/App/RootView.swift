import SwiftUI

struct RootView: View {
    @EnvironmentObject private var rootNavigation: RootNavigationStore
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var workspaceSelection: WorkspaceSelectionStore
    @State private var didBootstrap = false

    var body: some View {
        NavigationStack {
            Group {
                switch rootNavigation.route {
                case .launcher:
                    ConnectionView()
                case .workbench:
                    WorkbenchShellView()
                }
            }
            .toolbar(.hidden, for: .navigationBar)
        }
        .preferredColorScheme(.dark)
        .task { await bootstrapIfNeeded() }
        .onChange(of: connection.isConnected) { connected in
            if connected {
                rootNavigation.route = .workbench
            } else if rootNavigation.route == .workbench {
                rootNavigation.route = .launcher
            }
        }
    }

    @MainActor
    private func bootstrapIfNeeded() async {
        guard !didBootstrap else { return }
        didBootstrap = true
        let daemonKey = connection.baseURLText
        workspaceSelection.load(daemonKey: daemonKey)
        guard workspaceSelection.workspaceId != nil else {
            rootNavigation.route = .launcher
            return
        }
        let connected = await connection.autoConnectIfPossible()
        rootNavigation.route = connected ? .workbench : .launcher
    }
}

#Preview {
    RootView()
        .environmentObject(ConnectionStore())
        .environmentObject(WorkspaceSelectionStore())
        .environmentObject(WorkbenchSelectionStore())
        .environmentObject(RootNavigationStore())
}
