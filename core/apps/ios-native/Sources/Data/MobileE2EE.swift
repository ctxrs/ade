import CryptoKit
import Foundation
import Sodium

enum MobileE2EEError: Error {
    case invalidBase64
    case invalidKey
    case encryptFailed
    case decryptFailed
}

struct SecureEnvelope: Codable, Equatable {
    let deviceId: String
    let seq: Int
    let nonce: String
    let ciphertext: String
}

struct SecureRequestPayload: Codable, Equatable {
    let method: String
    let path: String
    let query: String?
    let headers: [[String]]
    let bodyB64: String
}

struct SecureResponsePayload: Codable, Equatable {
    let status: Int
    let headers: [[String]]
    let bodyB64: String
}

enum MobileE2EE {
    private static let hkdfInfo = Data("ctx-mobile-e2ee-v1".utf8)

    static func deriveKey(deviceId: String, deviceSecretKey: String, daemonPublicKey: String) throws -> [UInt8] {
        let deviceSecret = try decodeBase64(deviceSecretKey)
        let daemonPublic = try decodeBase64(daemonPublicKey)
        let privateKey = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: deviceSecret)
        let publicKey = try Curve25519.KeyAgreement.PublicKey(rawRepresentation: daemonPublic)
        let shared = try privateKey.sharedSecretFromKeyAgreement(with: publicKey)
        let salt = SHA256.hash(data: Data(deviceId.utf8))
        let symmetricKey = shared.hkdfDerivedSymmetricKey(
            using: SHA256.self,
            salt: Data(salt),
            sharedInfo: hkdfInfo,
            outputByteCount: 32
        )
        return Array(symmetricKey.withUnsafeBytes { Data($0) })
    }

    static func encryptPayload(deviceId: String, seq: Int, key: [UInt8], plaintext: Data) throws -> SecureEnvelope {
        let sodium = Sodium()
        let aad = Array("\(deviceId):\(seq)".utf8)
        guard let (ciphertext, nonce) = sodium.aead.xchacha20poly1305ietf.encrypt(
            message: Array(plaintext),
            secretKey: key,
            additionalData: aad
        ) else {
            throw MobileE2EEError.encryptFailed
        }
        return SecureEnvelope(
            deviceId: deviceId,
            seq: seq,
            nonce: encodeBase64URL(Data(nonce)),
            ciphertext: encodeBase64URL(Data(ciphertext))
        )
    }

    static func decryptEnvelope(_ envelope: SecureEnvelope, key: [UInt8]) throws -> Data {
        let sodium = Sodium()
        let aad = Array("\(envelope.deviceId):\(envelope.seq)".utf8)
        let nonce = try decodeBase64(envelope.nonce)
        let ciphertext = try decodeBase64(envelope.ciphertext)
        guard let plaintext = sodium.aead.xchacha20poly1305ietf.decrypt(
            authenticatedCipherText: Array(ciphertext),
            secretKey: key,
            nonce: Array(nonce),
            additionalData: aad
        ) else {
            throw MobileE2EEError.decryptFailed
        }
        return Data(plaintext)
    }

    static func decodeBase64(_ value: String) throws -> Data {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            return Data()
        }
        var normalized = trimmed.replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        while normalized.count % 4 != 0 {
            normalized.append("=")
        }
        guard let data = Data(base64Encoded: normalized) else {
            throw MobileE2EEError.invalidBase64
        }
        return data
    }

    static func encodeBase64URL(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
