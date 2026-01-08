import SwiftUI

struct RootView: View {
    var body: some View {
        NavigationStack {
            ChatView()
                .navigationBarHidden(true)
        }
        .preferredColorScheme(.dark)
    }
}

#Preview {
    RootView()
}
