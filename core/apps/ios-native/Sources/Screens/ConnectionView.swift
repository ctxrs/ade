import SwiftUI

struct ConnectionView: View {
    @State private var daemonURL = "https://localhost:8080"
    @State private var accessToken = ""
    @State private var rememberDevice = true

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 24) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text("ctx")
                            .font(.system(size: 40, weight: .semibold, design: .rounded))
                            .foregroundColor(.ctxTextPrimary)
                        Text("Connect to your daemon to start work.")
                            .font(.title3.weight(.medium))
                            .foregroundColor(.ctxTextSecondary)
                        Text("Secure, local-first sessions with native streaming and diagnostics.")
                            .font(.subheadline)
                            .foregroundColor(.ctxTextMuted)
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 18) {
                            HStack {
                                Text("Connection")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                Spacer()
                                GlassPill(text: "Local network", tint: .ctxAccent)
                            }

                            CtxField(title: "Daemon URL", placeholder: "https://your-daemon.local", text: $daemonURL)
                            CtxField(title: "Access Token", placeholder: "Paste token", text: $accessToken, isSecure: true)

                            Toggle(isOn: $rememberDevice) {
                                Text("Remember this device")
                                    .foregroundColor(.ctxTextSecondary)
                            }
                            .tint(.ctxAccent)

                            NavigationLink {
                                WorkbenchShellView()
                            } label: {
                                HStack {
                                    Text("Connect to ctx")
                                    Spacer()
                                    Image(systemName: "arrow.right.circle.fill")
                                }
                            }
                            .buttonStyle(CtxPrimaryButtonStyle())

                            Button {
                            } label: {
                                HStack {
                                    Image(systemName: "qrcode.viewfinder")
                                    Text("Scan QR instead")
                                    Spacer()
                                }
                            }
                            .buttonStyle(CtxGhostButtonStyle())
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 12) {
                            Text("Recent connections")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            VStack(spacing: 10) {
                                ConnectionRowView(title: "Example Mac", subtitle: "https://192.0.2.21:8443", status: "Healthy")
                                ConnectionRowView(title: "Remote Devbox", subtitle: "https://example.test", status: "Idle")
                            }
                        }
                    }
                }
                .padding(.horizontal, 22)
                .padding(.top, 28)
                .padding(.bottom, 40)
            }
        }
        .toolbar(.hidden, for: .navigationBar)
    }
}

private struct ConnectionRowView: View {
    let title: String
    let subtitle: String
    let status: String

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                Circle()
                    .fill(Color.ctxSurfaceRaised)
                Image(systemName: "server.rack")
                    .foregroundColor(.ctxTextSecondary)
            }
            .frame(width: 36, height: 36)

            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }

            Spacer()

            GlassPill(text: status, tint: .ctxAccent)
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

#Preview {
    ConnectionView()
}
