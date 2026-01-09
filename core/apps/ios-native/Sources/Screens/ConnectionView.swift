import SwiftUI
import UIKit
import _Concurrency

struct ConnectionView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @State private var daemonURL = ""
    @State private var accessToken = ""
    @State private var rememberDevice = true
    @State private var isConnecting = false
    @State private var shouldNavigate = false
    @State private var isShowingScanner = false

    private let deviceIdentityStore = DeviceIdentityStore()

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
                switch result {
                case .legacy(let baseURL, let token):
                    daemonURL = baseURL
                    accessToken = token
                    connection.baseURLText = baseURL
                    connection.tokenText = token
                    connection.setSecureConfig(nil)
                case .secure(let baseURL, let pairingToken, let daemonPublicKey):
                    _Concurrency.Task {
                        await handleSecurePairing(baseURL: baseURL, pairingToken: pairingToken, daemonPublicKey: daemonPublicKey)
                    }
                }
            }
        }
        .toolbar(.hidden, for: .navigationBar)
    }

    @MainActor
    private func handleSecurePairing(baseURL: String, pairingToken: String, daemonPublicKey: String) async {
        isConnecting = true
        connection.lastError = nil
        defer {
            isConnecting = false
        }
        guard let url = URL(string: baseURL) else {
            connection.lastError = "Invalid daemon URL."
            return
        }
        do {
            let identity = try await deviceIdentityStore.loadOrCreate()
            let client = DaemonAPIClient(baseURL: url, tokenStore: KeychainTokenStore())
            let payload = DaemonAPIClient.PairMobileDeviceRequest(
                pairingToken: pairingToken,
                deviceId: identity.deviceId,
                deviceLabel: UIDevice.current.name,
                platform: UIDevice.current.systemName,
                publicKey: identity.publicKey,
                appVersion: appVersionString()
            )
            let response = try await client.pairMobileDevice(baseURL: url, payload: payload)
            let envelope = try decodeSecureEnvelope(response)
            guard envelope.deviceId == identity.deviceId else {
                connection.lastError = "Pairing response device mismatch."
                return
            }
            let key = try MobileE2EE.deriveKey(
                deviceId: identity.deviceId,
                deviceSecretKey: identity.secretKey,
                daemonPublicKey: daemonPublicKey
            )
            let decrypted = try MobileE2EE.decryptEnvelope(envelope, key: key)
            let ack = try JSONDecoder().decode(PairingAck.self, from: decrypted)
            guard ack.paired == true else {
                connection.lastError = "Pairing was not accepted."
                return
            }
            let config = SecureConnectionConfig(baseURL: baseURL, deviceId: identity.deviceId, daemonPublicKey: daemonPublicKey)
            connection.setSecureConfig(config)
            connection.baseURLText = baseURL
            connection.tokenText = ""
            await connection.connect()
            if connection.isConnected {
                shouldNavigate = true
            }
        } catch {
            connection.lastError = "Pairing failed."
        }
    }

    private func decodeSecureEnvelope(_ value: JSONValue) throws -> SecureEnvelope {
        let data = try JSONEncoder().encode(value)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(SecureEnvelope.self, from: data)
    }

    private func appVersionString() -> String? {
        let info = Bundle.main.infoDictionary
        return info?["CFBundleShortVersionString"] as? String
    }

    private struct PairingAck: Decodable {
        let paired: Bool?
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
