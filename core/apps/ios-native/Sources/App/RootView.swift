import SwiftUI

struct RootView: View {
    var body: some View {
        NavigationStack {
            ConnectionView()
        }
        .preferredColorScheme(.dark)
    }
}

#Preview {
    RootView()
}
