import Foundation

private enum SelectionDefaults {
    static let workspaceVersion = 1
    static let workbenchVersion = 1

    static func normalizedKey(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let trimmed, !trimmed.isEmpty else { return nil }
        return trimmed
    }

    static func normalizedId(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let trimmed, !trimmed.isEmpty else { return nil }
        return trimmed
    }

    static func workspaceKey(daemonKey: String) -> String {
        "ctx.ios.workspace.selection.v\(workspaceVersion).\(daemonKey)"
    }

    static func workbenchKey(daemonKey: String, workspaceId: String) -> String {
        "ctx.ios.workbench.selection.v\(workbenchVersion).\(daemonKey).\(workspaceId)"
    }

    static func loadRecord<T: Decodable>(_ type: T.Type, key: String) -> T? {
        guard let data = UserDefaults.standard.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(type, from: data)
    }

    static func saveRecord<T: Encodable>(_ value: T, key: String) {
        guard let data = try? JSONEncoder().encode(value) else { return }
        UserDefaults.standard.set(data, forKey: key)
    }

    static func deleteRecord(key: String) {
        UserDefaults.standard.removeObject(forKey: key)
    }
}

private struct PersistedWorkspaceSelection: Codable {
    let v: Int
    let id: String
    let name: String
}

private struct PersistedWorkbenchSelection: Codable {
    let v: Int
    let taskId: String?
    let trackId: String?
    let sessionId: String?
}

@MainActor
final class WorkspaceSelectionStore: ObservableObject {
    @Published private(set) var workspaceId: String?
    @Published private(set) var workspaceName: String?
    private var daemonKey: String?

    func load(daemonKey: String?) {
        let nextKey = SelectionDefaults.normalizedKey(daemonKey)
        self.daemonKey = nextKey
        workspaceId = nil
        workspaceName = nil
        guard let nextKey else { return }
        let key = SelectionDefaults.workspaceKey(daemonKey: nextKey)
        guard let record = SelectionDefaults.loadRecord(PersistedWorkspaceSelection.self, key: key),
              record.v == SelectionDefaults.workspaceVersion else { return }
        workspaceId = SelectionDefaults.normalizedId(record.id)
        workspaceName = record.name
    }

    func setWorkspace(_ workspace: WorkspaceSummary, daemonKey: String?) {
        guard let daemonKey = SelectionDefaults.normalizedKey(daemonKey),
              let workspaceId = SelectionDefaults.normalizedId(workspace.id) else { return }
        self.daemonKey = daemonKey
        self.workspaceId = workspaceId
        workspaceName = workspace.name
        let record = PersistedWorkspaceSelection(
            v: SelectionDefaults.workspaceVersion,
            id: workspaceId,
            name: workspace.name
        )
        SelectionDefaults.saveRecord(record, key: SelectionDefaults.workspaceKey(daemonKey: daemonKey))
    }

    func clear(daemonKey: String?) {
        workspaceId = nil
        workspaceName = nil
        guard let daemonKey = SelectionDefaults.normalizedKey(daemonKey) else { return }
        SelectionDefaults.deleteRecord(key: SelectionDefaults.workspaceKey(daemonKey: daemonKey))
    }
}

@MainActor
final class WorkbenchSelectionStore: ObservableObject {
    @Published private(set) var taskId: String?
    @Published private(set) var trackId: String?
    @Published private(set) var sessionId: String?

    private var workspaceId: String?
    private var daemonKey: String?

    func setContext(daemonKey: String?, workspaceId: String?) {
        let nextDaemonKey = SelectionDefaults.normalizedKey(daemonKey)
        let nextWorkspaceId = SelectionDefaults.normalizedId(workspaceId)
        guard nextDaemonKey != self.daemonKey || nextWorkspaceId != self.workspaceId else { return }
        self.daemonKey = nextDaemonKey
        self.workspaceId = nextWorkspaceId
        loadSelection()
    }

    func setSelection(taskId: String?, trackId: String?, sessionId: String?) {
        let normalized = normalizedSelection(taskId: taskId, trackId: trackId, sessionId: sessionId)
        self.taskId = normalized.taskId
        self.trackId = normalized.trackId
        self.sessionId = normalized.sessionId
        persistSelection(normalized)
    }

    func clearSelection() {
        taskId = nil
        trackId = nil
        sessionId = nil
        guard let daemonKey, let workspaceId else { return }
        SelectionDefaults.deleteRecord(key: SelectionDefaults.workbenchKey(daemonKey: daemonKey, workspaceId: workspaceId))
    }

    private func loadSelection() {
        taskId = nil
        trackId = nil
        sessionId = nil
        guard let daemonKey, let workspaceId else { return }
        let key = SelectionDefaults.workbenchKey(daemonKey: daemonKey, workspaceId: workspaceId)
        guard let record = SelectionDefaults.loadRecord(PersistedWorkbenchSelection.self, key: key),
              record.v == SelectionDefaults.workbenchVersion else { return }
        let normalized = normalizedSelection(taskId: record.taskId, trackId: record.trackId, sessionId: record.sessionId)
        taskId = normalized.taskId
        trackId = normalized.trackId
        sessionId = normalized.sessionId
    }

    private func persistSelection(_ selection: PersistedWorkbenchSelection) {
        guard let daemonKey, let workspaceId else { return }
        SelectionDefaults.saveRecord(selection, key: SelectionDefaults.workbenchKey(daemonKey: daemonKey, workspaceId: workspaceId))
    }

    private func normalizedSelection(taskId: String?, trackId: String?, sessionId: String?) -> PersistedWorkbenchSelection {
        let normalizedTaskId = SelectionDefaults.normalizedId(taskId)
        let normalizedTrackId = normalizedTaskId == nil ? nil : SelectionDefaults.normalizedId(trackId)
        let normalizedSessionId = (normalizedTaskId == nil || normalizedTrackId == nil)
            ? nil
            : SelectionDefaults.normalizedId(sessionId)
        return PersistedWorkbenchSelection(
            v: SelectionDefaults.workbenchVersion,
            taskId: normalizedTaskId,
            trackId: normalizedTrackId,
            sessionId: normalizedSessionId
        )
    }
}
