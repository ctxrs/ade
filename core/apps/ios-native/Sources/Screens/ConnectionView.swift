import SwiftUI

struct ConnectionView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @State private var daemonURL = ""
    @State private var accessToken = ""
    @State private var rememberDevice = true
    @State private var isConnecting = false
    @State private var shouldNavigate = false
    @State private var isShowingScanner = false

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

                            if let error = connection.lastError {
                                Text(error)
                                    .font(.footnote)
                                    .foregroundColor(.ctxError)
                            }

                            Button {
                                _Concurrency.Task {
                                    isConnecting = true
                                    connection.baseURLText = daemonURL
                                    connection.tokenText = accessToken
                                    await connection.connect()
                                    isConnecting = false
                                    if connection.isConnected {
                                        shouldNavigate = true
                                    }
                                }
                            } label: {
                                HStack {
                                    Text(isConnecting ? "Connecting..." : "Connect to ctx")
                                    Spacer()
                                    Image(systemName: "arrow.right.circle.fill")
                                }
                            }
                            .buttonStyle(CtxPrimaryButtonStyle())
                            .disabled(isConnecting)

                            Button {
                                isShowingScanner = true
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
        .onAppear {
            if daemonURL.isEmpty {
                daemonURL = connection.baseURLText
            }
        }
        .onChange(of: connection.isConnected) { connected in
            if connected {
                shouldNavigate = true
            }
        }
        .onChange(of: connection.baseURLText) { baseURL in
            daemonURL = baseURL
        }
        .onChange(of: connection.tokenText) { token in
            accessToken = token
        }
        .background(
            NavigationLink("", destination: WorkbenchShellView(), isActive: $shouldNavigate)
                .opacity(0)
        )
        .fullScreenCover(isPresented: $isShowingScanner) {
            QRCodeScannerView { result in
                daemonURL = result.baseURL
                accessToken = result.token
                connection.baseURLText = result.baseURL
                connection.tokenText = result.token
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
        .environmentObject(ConnectionStore())
}
