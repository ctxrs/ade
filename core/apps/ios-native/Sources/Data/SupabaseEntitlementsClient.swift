import Foundation

struct SupabaseTokenInfo {
    let baseURL: URL
    let email: String?
}

enum SupabaseEntitlementsError: Error {
    case invalidToken
    case invalidResponse
}

struct SupabaseEntitlementsClient {
    static func tokenInfo(from token: String) -> SupabaseTokenInfo? {
        let parts = token.split(separator: ".")
        guard parts.count > 1, let payloadData = decodeBase64URL(parts[1]) else {
            return nil
        }
        let decoder = JSONDecoder()
        guard let claims = try? decoder.decode(SupabaseTokenClaims.self, from: payloadData),
              let issuer = claims.iss,
              let issuerURL = URL(string: issuer),
              var components = URLComponents(url: issuerURL, resolvingAgainstBaseURL: false) else {
            return nil
        }
        components.path = ""
        components.query = nil
        components.fragment = nil
        guard let baseURL = components.url else {
            return nil
        }
        return SupabaseTokenInfo(baseURL: baseURL, email: claims.email)
    }

    static func fetchEntitlements(token: String, session: URLSession = .shared) async throws -> EntitlementsSnapshot {
        guard let info = tokenInfo(from: token) else {
            throw SupabaseEntitlementsError.invalidToken
        }
        let url = info.baseURL.appendingPathComponent("functions/v1/entitlements")
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              (200...299).contains(httpResponse.statusCode) else {
            throw SupabaseEntitlementsError.invalidResponse
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(EntitlementsSnapshot.self, from: data)
    }
}

private struct SupabaseTokenClaims: Decodable {
    let iss: String?
    let email: String?
}

private func decodeBase64URL(_ value: Substring) -> Data? {
    var base64 = String(value)
        .replacingOccurrences(of: "-", with: "+")
        .replacingOccurrences(of: "_", with: "/")
    let padding = 4 - (base64.count % 4)
    if padding < 4 {
        base64 += String(repeating: "=", count: padding)
    }
    return Data(base64Encoded: base64)
}
