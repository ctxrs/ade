import SwiftUI
import UIKit
import _Concurrency

struct ConnectionView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var rootNavigation: RootNavigationStore
    @State private var daemonURL = ""
    @State private var accessToken = ""
    @State private var rememberDevice = true
    @State private var isConnecting = false
    @State private var isShowingScanner = false
    @State private var recentConnections: [ConnectionHistoryEntry] = []

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

                            CtxField(
                                title: "Daemon URL",
                                placeholder: "https://your-daemon.local",
                                text: $daemonURL,
                                accessibilityId: "connection.daemon"
                            )
                            CtxField(
                                title: "Access Token",
                                placeholder: "Paste token",
                                text: $accessToken,
                                isSecure: true,
                                accessibilityId: "connection.token"
                            )

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
                                    recentConnections = ConnectionHistoryStore.load()
                                    if connection.isConnected {
                                        rootNavigation.route = .workbench
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
                            .accessibilityIdentifier("connection.connect")

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
                            .accessibilityIdentifier("connection.scan")
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 12) {
                            HStack {
                                Text("Recent connections")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                Spacer()
                                if !recentConnections.isEmpty {
                                    Button("Clear") {
                                        ConnectionHistoryStore.clear()
                                        recentConnections = []
                                    }
                                    .buttonStyle(.plain)
                                    .font(.caption.weight(.semibold))
                                    .foregroundColor(.ctxTextSecondary)
                                }
                            }

                            if recentConnections.isEmpty {
                                Text("No recent connections yet.")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                            } else {
                                VStack(spacing: 10) {
                                    ForEach(recentConnections) { entry in
                                        Button {
                                            applyConnection(entry)
                                        } label: {
                                            ConnectionRowView(
                                                title: connectionTitle(entry),
                                                subtitle: connectionSubtitle(entry),
                                                status: connectionStatus(entry)
                                            )
                                        }
                                        .buttonStyle(.plain)
                                    }
                                }
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
            recentConnections = ConnectionHistoryStore.load()
        }
        .onChange(of: connection.isConnected) { connected in
            if connected {
                rootNavigation.route = .workbench
            }
        }
        .onChange(of: connection.baseURLText) { baseURL in
            daemonURL = baseURL
        }
        .onChange(of: connection.tokenText) { token in
            accessToken = token
        }
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

    private func applyConnection(_ entry: ConnectionHistoryEntry) {
        daemonURL = entry.baseURL
        connection.baseURLText = entry.baseURL
        accessToken = ""
        connection.tokenText = ""
    }

    private func connectionTitle(_ entry: ConnectionHistoryEntry) -> String {
        let host = URL(string: entry.baseURL)?.host
        return host?.isEmpty == false ? host! : entry.baseURL
    }

    private func connectionSubtitle(_ entry: ConnectionHistoryEntry) -> String {
        if let detail = tokenDetail(entry) {
            return "\(entry.baseURL) · \(detail)"
        }
        return entry.baseURL
    }

    private func connectionStatus(_ entry: ConnectionHistoryEntry) -> String {
        guard let date = entry.lastUsedDate else { return "Recent" }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }

    private func tokenDetail(_ entry: ConnectionHistoryEntry) -> String? {
        guard let prefix = entry.tokenPrefix, !prefix.isEmpty else { return nil }
        if prefix == "secure" {
            return "Secure pairing"
        }
        return "Token \(prefix)…"
    }

    @MainActor
    private func handleSecurePairing(baseURL: String, pairingToken: String, daemonPublicKey: String) async {
        isConnecting = true
        connection.lastError = nil
        defer {
            isConnecting = false
        }
        guard let url = URL(string: baseURL) else {
            setPairingError("Invalid daemon URL.")
            return
        }
        recordPairingError("Pairing start (baseURL: \(baseURL))", error: nil)
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
                setPairingError("Pairing response device mismatch.")
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
                setPairingError("Pairing was not accepted.")
                return
            }
            let config = SecureConnectionConfig(baseURL: baseURL, deviceId: identity.deviceId, daemonPublicKey: daemonPublicKey)
            connection.setSecureConfig(config)
            connection.baseURLText = baseURL
            connection.tokenText = ""
            await connection.connect()
            if connection.isConnected {
                rootNavigation.route = .workbench
            }
        } catch let error as DaemonAPIError {
            setPairingError(formatPairingError(error), error: error)
        } catch let error as MobileE2EEError {
            setPairingError(formatCryptoError(error), error: error)
        } catch {
            setPairingError(formatUnknownError(error), error: error)
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

    private func formatPairingError(_ error: DaemonAPIError) -> String {
        switch error {
        case .invalidURL:
            return "Pairing failed: invalid daemon URL."
        case .missingToken:
            return "Pairing failed: missing auth token."
        case .invalidResponse:
            return "Pairing failed: invalid response."
        case .requestFailed(let statusCode, let message):
            let trimmed = message.trimmingCharacters(in: .whitespacesAndNewlines)
            let apiError = parseApiError(trimmed)
            if let apiError {
                return apiError
            }
            if trimmed.isEmpty {
                return "Pairing failed: HTTP \(statusCode)."
            }
            return trimmed
        }
    }

    private func parseApiError(_ message: String) -> String? {
        guard let data = message.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let error = json["error"] as? String,
              !error.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return nil
        }
        return error
    }

    private func formatCryptoError(_ error: MobileE2EEError) -> String {
        switch error {
        case .invalidBase64:
            return "Pairing failed: invalid payload."
        case .invalidKey:
            return "Pairing failed: invalid key."
        case .encryptFailed:
            return "Pairing failed: encryption error."
        case .decryptFailed:
            return "Pairing failed: could not decrypt response."
        }
    }

    private func formatUnknownError(_ error: Error) -> String {
        let localized = error.localizedDescription.trimmingCharacters(in: .whitespacesAndNewlines)
        if !localized.isEmpty {
            return localized
        }
        let fallback = String(describing: error).trimmingCharacters(in: .whitespacesAndNewlines)
        return fallback.isEmpty ? "Pairing failed." : fallback
    }

    private func setPairingError(_ message: String, error: Error? = nil) {
        connection.lastError = message
        recordPairingError(message, error: error)
    }

    private func recordPairingError(_ message: String, error: Error?) {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let timestamp = formatter.string(from: Date())
        var entry = "[\(timestamp)] \(message)"
        if let error {
            entry += "\nerror: \(String(describing: error))"
        }
        entry += "\n"
        guard let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else {
            return
        }
        let logURL = documents.appendingPathComponent("pairing_error.log")
        guard let data = entry.data(using: .utf8) else { return }
        if FileManager.default.fileExists(atPath: logURL.path),
           let handle = try? FileHandle(forWritingTo: logURL) {
            defer { try? handle.close() }
            try? handle.seekToEnd()
            try? handle.write(contentsOf: data)
        } else {
            try? data.write(to: logURL, options: .atomic)
        }
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
        .environmentObject(RootNavigationStore())
}
