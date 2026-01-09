import CryptoKit
import Foundation
import Security

struct SecureConnectionConfig: Codable, Equatable {
    let baseURL: String
    let deviceId: String
    let daemonPublicKey: String
}

struct DeviceIdentity: Codable, Equatable {
    let deviceId: String
    let publicKey: String
    let secretKey: String
}

struct SecureConnectionContext: Equatable {
    let deviceId: String
    let key: [UInt8]
}

actor DeviceIdentityStore {
    private let tokenStore: KeychainTokenStore

    init(tokenStore: KeychainTokenStore = KeychainTokenStore(service: "rs.ctx.mobile", account: "deviceIdentity")) {
        self.tokenStore = tokenStore
    }

    func load() async -> DeviceIdentity? {
        guard let stored = try? await tokenStore.loadToken() else { return nil }
        guard let data = stored.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(DeviceIdentity.self, from: data)
    }

    func save(_ identity: DeviceIdentity) async throws {
        let data = try JSONEncoder().encode(identity)
        guard let string = String(data: data, encoding: .utf8) else {
            throw KeychainTokenStoreError.unexpectedStatus(errSecParam)
        }
        try await tokenStore.saveToken(string)
    }

    func loadOrCreate() async throws -> DeviceIdentity {
        if let identity = await load() {
            return identity
        }
        let identity = createIdentity()
        try await save(identity)
        return identity
    }

    private func createIdentity() -> DeviceIdentity {
        let privateKey = Curve25519.KeyAgreement.PrivateKey()
        let publicKey = privateKey.publicKey
        return DeviceIdentity(
            deviceId: UUID().uuidString.lowercased(),
            publicKey: privateKeyBase64(publicKey.rawRepresentation),
            secretKey: privateKeyBase64(privateKey.rawRepresentation)
        )
    }

    private func privateKeyBase64(_ data: Data) -> String {
        data.base64EncodedString()
    }
}

actor SecureSequenceStore {
    static let shared = SecureSequenceStore()
    private let prefix = "ctx.ios.secure.seq.v1."
    private var cache: [String: Int] = [:]

    func next(for deviceId: String) -> Int {
        let cached = cache[deviceId]
        let stored = cached ?? UserDefaults.standard.integer(forKey: prefix + deviceId)
        let now = Int(Date().timeIntervalSince1970 * 1000)
        let next = max(now, stored + 1)
        cache[deviceId] = next
        UserDefaults.standard.set(next, forKey: prefix + deviceId)
        return next
    }
}

enum SecureConnectionDefaults {
    private static let key = "ctx.ios.secure.connection.v1"

    static func load() -> SecureConnectionConfig? {
        guard let data = UserDefaults.standard.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(SecureConnectionConfig.self, from: data)
    }

    static func save(_ config: SecureConnectionConfig?) {
        guard let config else {
            UserDefaults.standard.removeObject(forKey: key)
            return
        }
        guard let data = try? JSONEncoder().encode(config) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }
}
