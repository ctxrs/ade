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
        self.secureConfig = storedSecure
        self.baseURLText = storedSecure?.baseURL ?? baseURLText
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

    func connect() async {
        lastError = nil
        let baseValue = secureConfig?.baseURL ?? baseURLText
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
                return
            } catch {
                lastError = "Failed to establish secure connection."
                return
            }
        }

        let client = DaemonAPIClient(baseURL: baseURL, tokenStore: tokenStore)
        do {
            if tokenText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                _ = try await client.loadToken()
            } else {
                try await client.setToken(tokenText)
            }
            apiClient = client
            isConnected = true
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
