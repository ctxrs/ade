import SwiftUI

struct RootView: View {
    var body: some View {
        NavigationStack {
            ConnectionView()
        }
        .toolbar(.hidden, for: .navigationBar)
        .preferredColorScheme(.dark)
    }
}

#Preview {
    RootView()
        .environmentObject(ConnectionStore())
        .environmentObject(WorkspaceSelectionStore())
        .environmentObject(WorkbenchSelectionStore())
}
