import Foundation

struct ChatDraftStore {
    private static let prefix = "ctx.ios.chat.draft.v1"

    static func load(sessionId: String?) -> String {
        guard let sessionId, !sessionId.isEmpty else { return "" }
        let key = "\(prefix).\(sessionId)"
        return UserDefaults.standard.string(forKey: key) ?? ""
    }

    static func save(_ text: String, sessionId: String?) {
        guard let sessionId, !sessionId.isEmpty else { return }
        let key = "\(prefix).\(sessionId)"
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            UserDefaults.standard.removeObject(forKey: key)
        } else {
            UserDefaults.standard.set(text, forKey: key)
        }
    }
}
