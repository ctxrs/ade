import SwiftUI

@main
struct CtxIOSApp: App {
    @StateObject private var connectionStore = ConnectionStore()
    @StateObject private var workspaceSelectionStore = WorkspaceSelectionStore()
    @StateObject private var workbenchSelectionStore = WorkbenchSelectionStore()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(connectionStore)
                .environmentObject(workspaceSelectionStore)
                .environmentObject(workbenchSelectionStore)
        }
    }
}
