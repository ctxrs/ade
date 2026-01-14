import Foundation

struct ModelRoutingEntry: Codable, Equatable {
    let providerId: String
    let modelId: String
    let updatedAt: String
}

enum ModelRoutingDefaults {
    private static let prefix = "ctx.ios.model_routing.v1."

    static func load(daemonKey: String, workspaceId: String) -> ModelRoutingEntry? {
        let key = storageKey(daemonKey: daemonKey, workspaceId: workspaceId)
        guard let data = UserDefaults.standard.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(ModelRoutingEntry.self, from: data)
    }

    static func save(daemonKey: String, workspaceId: String, providerId: String, modelId: String) {
        let key = storageKey(daemonKey: daemonKey, workspaceId: workspaceId)
        let entry = ModelRoutingEntry(
            providerId: providerId,
            modelId: modelId,
            updatedAt: ISO8601DateFormatter().string(from: Date())
        )
        guard let data = try? JSONEncoder().encode(entry) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }

    static func clear(daemonKey: String, workspaceId: String) {
        let key = storageKey(daemonKey: daemonKey, workspaceId: workspaceId)
        UserDefaults.standard.removeObject(forKey: key)
    }

    private static func storageKey(daemonKey: String, workspaceId: String) -> String {
        let trimmedDaemon = daemonKey.trimmingCharacters(in: .whitespacesAndNewlines)
        return "\(prefix)\(trimmedDaemon).\(workspaceId)"
    }
}
