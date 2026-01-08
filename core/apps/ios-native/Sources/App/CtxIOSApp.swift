import SwiftUI

@main
struct CtxIOSApp: App {
    @StateObject private var connectionStore = ConnectionStore()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(connectionStore)
        }
    }
}
