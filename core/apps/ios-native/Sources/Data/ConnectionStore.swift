import Foundation

@MainActor
final class ConnectionStore: ObservableObject {
    @Published var baseURLText: String
    @Published var tokenText: String
    @Published private(set) var isConnected: Bool = false
    @Published var lastError: String?

    private(set) var apiClient: DaemonAPIClient?
    private let tokenStore: KeychainTokenStore

    init(baseURLText: String = "http://127.0.0.1:4399", tokenText: String = "") {
        self.baseURLText = baseURLText
        self.tokenText = tokenText
        self.tokenStore = KeychainTokenStore()
    }

    func connect() async {
        lastError = nil
        guard let baseURL = URL(string: baseURLText) else {
            lastError = "Invalid daemon URL."
            return
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
