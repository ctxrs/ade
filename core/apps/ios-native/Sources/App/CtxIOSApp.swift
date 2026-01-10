import SwiftUI
import _Concurrency

@main
struct CtxIOSApp: App {
    @StateObject private var connectionStore: ConnectionStore
    @StateObject private var workspaceSelectionStore = WorkspaceSelectionStore()
    @StateObject private var workbenchSelectionStore = WorkbenchSelectionStore()

    init() {
        let store = ConnectionStore()
        if let launchConfig = LaunchConfig.fromProcessInfo() {
            _Concurrency.Task { @MainActor in
                store.applyLaunchConfig(launchConfig)
                if launchConfig.autoConnect {
                    await store.connect()
                }
            }
        }
        _connectionStore = StateObject(wrappedValue: store)
    }

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(connectionStore)
                .environmentObject(workspaceSelectionStore)
                .environmentObject(workbenchSelectionStore)
        }
    }
}
