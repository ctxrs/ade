import SwiftUI

struct RootView: View {
    var body: some View {
        NavigationStack {
            ConnectionView()
                .navigationBarHidden(true)
        }
        .preferredColorScheme(.dark)
    }
}

#Preview {
    RootView()
        .environmentObject(ConnectionStore())
}
