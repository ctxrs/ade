import Foundation

struct ConnectionHistoryEntry: Codable, Identifiable, Hashable {
    let baseURL: String
    let tokenPrefix: String?
    let lastUsedAt: String

    var id: String { baseURL }

    var lastUsedDate: Date? {
        ISO8601DateFormatter().date(from: lastUsedAt)
    }
}

enum ConnectionHistoryStore {
    private static let key = "ctx.ios.connection.history.v1"
    private static let maxEntries = 5

    static func load() -> [ConnectionHistoryEntry] {
        guard let data = UserDefaults.standard.data(forKey: key) else { return [] }
        guard let entries = try? JSONDecoder().decode([ConnectionHistoryEntry].self, from: data) else {
            return []
        }
        return entries
    }

    static func save(_ entries: [ConnectionHistoryEntry]) {
        guard let data = try? JSONEncoder().encode(entries) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }

    @discardableResult
    static func add(baseURL: String, tokenPrefix: String?) -> [ConnectionHistoryEntry] {
        let normalized = normalize(baseURL)
        guard !normalized.isEmpty else { return load() }
        let now = ISO8601DateFormatter().string(from: Date())
        let entry = ConnectionHistoryEntry(baseURL: normalized, tokenPrefix: tokenPrefix, lastUsedAt: now)
        let existing = load().filter { normalize($0.baseURL) != normalized }
        let updated = Array(([entry] + existing).prefix(maxEntries))
        save(updated)
        return updated
    }

    static func clear() {
        UserDefaults.standard.removeObject(forKey: key)
    }

    private static func normalize(_ value: String) -> String {
        value.trimmingCharacters(in: .whitespacesAndNewlines).trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    }
}
