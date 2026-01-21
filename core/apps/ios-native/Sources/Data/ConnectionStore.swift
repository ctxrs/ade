import Foundation

@MainActor
final class ConnectionStore: ObservableObject {
    @Published var baseURLText: String
    @Published var tokenText: String
    @Published private(set) var isConnected: Bool = false
    @Published var lastError: String?
    @Published private(set) var secureConfig: SecureConnectionConfig?

    private(set) var apiClient: DaemonAPIClient?
    private let tokenStore: KeychainTokenStore
    private let deviceIdentityStore = DeviceIdentityStore()

    init(baseURLText: String = "http://127.0.0.1:4399", tokenText: String = "") {
        let storedSecure = SecureConnectionDefaults.load()
        let lastBaseURL = ConnectionHistoryStore.load().first?.baseURL
        self.secureConfig = storedSecure
        self.baseURLText = storedSecure?.baseURL ?? lastBaseURL ?? baseURLText
        self.tokenText = tokenText
        self.tokenStore = KeychainTokenStore()
    }

    func applyLaunchConfig(_ config: LaunchConfig) {
        if config.baseURL != nil || config.token != nil {
            setSecureConfig(nil)
        }
        if let baseURL = config.baseURL {
            baseURLText = baseURL
        }
        if let token = config.token {
            tokenText = token
        }
    }


    func setSecureConfig(_ config: SecureConnectionConfig?) {
        secureConfig = config
        SecureConnectionDefaults.save(config)
        if let config {
            baseURLText = config.baseURL
        }
    }


    func autoConnectIfPossible() async -> Bool {
        guard await hasStoredCredentials() else { return false }
        await connect()
        return isConnected
    }

    private func hasStoredCredentials() async -> Bool {
        if secureConfig != nil {
            return true
        }
        if !tokenText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return true
        }
        let stored = (try? await tokenStore.loadToken()) ?? nil
        if let stored, !stored.isEmpty {
            return true
        }
        return false
    }

    func connect() async {
        lastError = nil
        let baseValue = secureConfig?.baseURL ?? baseURLText
        let baseString = baseValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let baseURL = URL(string: baseValue) else {
            lastError = "Invalid daemon URL."
            return
        }

        if tokenText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, let secureConfig {
            guard let identity = await deviceIdentityStore.load() else {
                lastError = "Secure identity missing. Re-scan the pairing QR code."
                return
            }
            do {
                let key = try MobileE2EE.deriveKey(
                    deviceId: secureConfig.deviceId,
                    deviceSecretKey: identity.secretKey,
                    daemonPublicKey: secureConfig.daemonPublicKey
                )
                let context = SecureConnectionContext(deviceId: secureConfig.deviceId, key: key)
                apiClient = DaemonAPIClient(baseURL: baseURL, tokenStore: tokenStore, secureContext: context)
                isConnected = true
                ConnectionHistoryStore.add(baseURL: baseString, tokenPrefix: "secure")
                return
            } catch {
                lastError = "Failed to establish secure connection."
                return
            }
        }

        let client = DaemonAPIClient(baseURL: baseURL, tokenStore: tokenStore)
        var tokenPrefix: String?
        do {
            let trimmedToken = tokenText.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmedToken.isEmpty {
                guard let stored = try await client.loadToken(), !stored.isEmpty else {
                    lastError = "Missing access token. Scan the QR code again."
                    return
                }
                tokenPrefix = String(stored.prefix(6))
            } else {
                try await client.setToken(tokenText)
                tokenPrefix = String(trimmedToken.prefix(6))
            }
            apiClient = client
            isConnected = true
            ConnectionHistoryStore.add(baseURL: baseString, tokenPrefix: tokenPrefix)
        } catch {
            lastError = "Failed to store token."
        }
    }

    func disconnect() async {
        do {
            try await tokenStore.deleteToken()
        } catch {
            lastError = "Failed to clear token."
        }
        apiClient = nil
        isConnected = false
    }
}
