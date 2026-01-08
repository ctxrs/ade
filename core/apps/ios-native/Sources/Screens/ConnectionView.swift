import SwiftUI

struct ConnectionView: View {
    var body: some View {
        ZStack {
            Color.ctxBackground.ignoresSafeArea()
            VStack(spacing: 16) {
                Text("ctx")
                    .font(.system(size: 34, weight: .semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text("Connect to a daemon to start.")
                    .foregroundColor(.ctxTextSecondary)
            }
        }
    }
}

#Preview {
    ConnectionView()
}
