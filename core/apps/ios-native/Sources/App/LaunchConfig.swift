import Foundation

struct LaunchConfig {
    let baseURL: String?
    let token: String?
    let autoConnect: Bool

    static func fromProcessInfo() -> LaunchConfig? {
        let env = ProcessInfo.processInfo.environment
        let args = ProcessInfo.processInfo.arguments

        let autoConnect = env["CTX_AUTO_CONNECT"].map { $0 != "0" } ?? true

        var baseURL = env["CTX_IOS_BASE_URL"] ?? env["CTX_BASE_URL"]
        var token = env["CTX_IOS_TOKEN"] ?? env["CTX_TOKEN"]

        if let qrPayload = env["CTX_IOS_QR_PAYLOAD"] ?? env["CTX_QR_PAYLOAD"] ?? argValue(args, flag: "--ctx-qr-payload") {
            let parsed = parseQrPayload(qrPayload)
            baseURL = baseURL ?? parsed?.baseURL
            token = token ?? parsed?.token
        }

        baseURL = baseURL ?? argValue(args, flag: "--ctx-base-url")
        token = token ?? argValue(args, flag: "--ctx-token")

        if baseURL == nil && token == nil {
            return nil
        }

        return LaunchConfig(baseURL: baseURL, token: token, autoConnect: autoConnect)
    }

    private static func argValue(_ args: [String], flag: String) -> String? {
        guard let index = args.firstIndex(of: flag), index + 1 < args.count else {
            return nil
        }
        let value = args[index + 1].trimmingCharacters(in: .whitespacesAndNewlines)
        return value.isEmpty ? nil : value
    }

    private static func parseQrPayload(_ payload: String) -> (baseURL: String, token: String)? {
        guard let data = payload.data(using: .utf8) else { return nil }
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }

        if let baseURL = object["baseUrl"] as? String,
           let token = object["token"] as? String {
            return (baseURL, token)
        }

        if let profile = object["connection_profile"] as? [String: Any],
           let connection = profile["connection"] as? [String: Any],
           let auth = profile["auth"] as? [String: Any],
           let baseURL = connection["base_url"] as? String,
           let token = auth["api_token"] as? String {
            return (baseURL, token)
        }

        return nil
    }
}
