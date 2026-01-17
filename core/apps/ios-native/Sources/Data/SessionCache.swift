import CryptoKit
import Foundation

actor DiskCache<Value: Codable> {
    private struct Entry: Codable {
        let key: String
        let filename: String
        var lastAccess: TimeInterval
    }

    private let maxEntries: Int
    private let directory: URL
    private let indexURL: URL
    private var entries: [String: Entry] = [:]
    private var isLoaded = false
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    init(cacheName: String, maxEntries: Int) {
        self.maxEntries = maxEntries
        let base = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
            ?? FileManager.default.temporaryDirectory
        directory = base.appendingPathComponent("ctx.ios.cache").appendingPathComponent(cacheName)
        indexURL = directory.appendingPathComponent("index.json")
    }

    func get(_ key: String) -> Value? {
        ensureLoaded()
        guard var entry = entries[key] else { return nil }
        let url = directory.appendingPathComponent(entry.filename)
        guard let data = try? Data(contentsOf: url),
              let value = try? decoder.decode(Value.self, from: data) else {
            removeEntry(entry)
            saveIndex()
            return nil
        }
        entry.lastAccess = Date().timeIntervalSince1970
        entries[key] = entry
        saveIndex()
        return value
    }

    func set(_ value: Value, for key: String) {
        ensureLoaded()
        let filename = entries[key]?.filename ?? Self.filename(for: key)
        let url = directory.appendingPathComponent(filename)
        guard let data = try? encoder.encode(value) else { return }
        do {
            try data.write(to: url, options: .atomic)
        } catch {
            return
        }
        entries[key] = Entry(
            key: key,
            filename: filename,
            lastAccess: Date().timeIntervalSince1970
        )
        evictIfNeeded()
        saveIndex()
    }

    func remove(_ key: String) {
        ensureLoaded()
        guard let entry = entries[key] else { return }
        removeEntry(entry)
        saveIndex()
    }

    private func ensureLoaded() {
        guard !isLoaded else { return }
        isLoaded = true
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        guard let data = try? Data(contentsOf: indexURL),
              let stored = try? decoder.decode([Entry].self, from: data) else { return }
        entries = Dictionary(uniqueKeysWithValues: stored.map { ($0.key, $0) })
    }

    private func saveIndex() {
        let stored = Array(entries.values)
        guard let data = try? encoder.encode(stored) else { return }
        try? data.write(to: indexURL, options: .atomic)
    }

    private func evictIfNeeded() {
        guard maxEntries > 0, entries.count > maxEntries else { return }
        let sorted = entries.values.sorted { $0.lastAccess < $1.lastAccess }
        let overflow = entries.count - maxEntries
        for entry in sorted.prefix(overflow) {
            removeEntry(entry)
        }
    }

    private func removeEntry(_ entry: Entry) {
        let url = directory.appendingPathComponent(entry.filename)
        try? FileManager.default.removeItem(at: url)
        entries.removeValue(forKey: entry.key)
    }

    private static func filename(for key: String) -> String {
        let digest = SHA256.hash(data: Data(key.utf8))
        let hex = digest.map { String(format: "%02x", $0) }.joined()
        return "\(hex).json"
    }
}

actor SessionHistoryPageCache {
    static let shared = SessionHistoryPageCache()
    private let cache = DiskCache<SessionHistoryPage>(cacheName: "session-history-pages", maxEntries: 60)

    func load(sessionId: String, beforeSeq: Int?, limit: Int?) async -> SessionHistoryPage? {
        await cache.get(makeKey(sessionId: sessionId, beforeSeq: beforeSeq, limit: limit))
    }

    func store(_ page: SessionHistoryPage, sessionId: String, beforeSeq: Int?, limit: Int?) async {
        await cache.set(page, for: makeKey(sessionId: sessionId, beforeSeq: beforeSeq, limit: limit))
    }

    private func makeKey(sessionId: String, beforeSeq: Int?, limit: Int?) -> String {
        let before = beforeSeq.map(String.init) ?? "nil"
        let limitValue = limit.map(String.init) ?? "nil"
        return "v1|\(sessionId)|\(before)|\(limitValue)"
    }
}

struct CachedWorkspaceActiveTaskSummary: Codable, Sendable {
    let task: Task
    let primarySession: SessionSnapshotSummary
    let sessions: [SessionSnapshotSummary]
    let sortAt: String

    init(activeSummary: WorkspaceActiveTaskSummary) {
        task = activeSummary.task
        primarySession = activeSummary.primarySession
        sessions = activeSummary.sessions
        sortAt = activeSummary.sortAt
    }
}

struct CachedWorkspaceActiveSnapshot: Codable, Sendable {
    let workspaceId: String
    var snapshotRev: Int
    var tasks: [CachedWorkspaceActiveTaskSummary]
    var cachedAt: TimeInterval

    init(workspaceId: String, snapshotRev: Int, tasks: [CachedWorkspaceActiveTaskSummary]) {
        self.workspaceId = workspaceId
        self.snapshotRev = snapshotRev
        self.tasks = tasks
        self.cachedAt = Date().timeIntervalSince1970
    }

    mutating func upsert(_ activeSummary: WorkspaceActiveTaskSummary) {
        let taskId = activeSummary.task.id.stringValue
        tasks.removeAll { $0.task.id.stringValue == taskId }
        tasks.append(CachedWorkspaceActiveTaskSummary(activeSummary: activeSummary))
    }

    mutating func remove(taskId: String) {
        tasks.removeAll { $0.task.id.stringValue == taskId }
    }
}

actor WorkspaceActiveSnapshotCache {
    static let shared = WorkspaceActiveSnapshotCache()
    private let cache = DiskCache<CachedWorkspaceActiveSnapshot>(cacheName: "ats-active-snapshots", maxEntries: 12)

    func load(workspaceId: String) async -> CachedWorkspaceActiveSnapshot? {
        await cache.get(makeKey(workspaceId: workspaceId))
    }

    func store(snapshot: WorkspaceActiveSnapshot) async {
        let tasks = snapshot.active.tasks.map(CachedWorkspaceActiveTaskSummary.init)
        let cached = CachedWorkspaceActiveSnapshot(
            workspaceId: snapshot.workspaceId.stringValue,
            snapshotRev: snapshot.snapshotRev,
            tasks: tasks
        )
        await cache.set(cached, for: makeKey(workspaceId: snapshot.workspaceId.stringValue))
    }

    func apply(event: WorkspaceActiveSnapshotEvent) async {
        switch event {
        case .activeTaskUpsert(let workspaceId, let snapshotRev, let task):
            await update(workspaceId: workspaceId.stringValue, snapshotRev: snapshotRev) { cached in
                cached.upsert(task)
            }
        case .activeTaskDelete(let workspaceId, let snapshotRev, let taskId):
            await update(workspaceId: workspaceId.stringValue, snapshotRev: snapshotRev) { cached in
                cached.remove(taskId: taskId.stringValue)
            }
        case .ready(let workspaceId, let snapshotRev):
            await update(workspaceId: workspaceId.stringValue, snapshotRev: snapshotRev) { _ in }
        default:
            break
        }
    }

    private func update(workspaceId: String, snapshotRev: Int, mutate: (inout CachedWorkspaceActiveSnapshot) -> Void) async {
        let key = makeKey(workspaceId: workspaceId)
        guard var cached = await cache.get(key) else { return }
        cached.snapshotRev = max(cached.snapshotRev, snapshotRev)
        cached.cachedAt = Date().timeIntervalSince1970
        mutate(&cached)
        await cache.set(cached, for: key)
    }

    private func makeKey(workspaceId: String) -> String {
        "v1|\(workspaceId)"
    }
}

struct CachedSessionHead: Codable, Sendable {
    let session: Session
    var turns: [SessionTurn]
    var toolSummaries: [SessionTurnToolSummary]?
    var events: [SessionEvent]?
    var messages: [Message]
    var lastEventSeq: Int
    var activity: SessionActivityState?
    var hasMoreTurns: Bool

    init(head: SessionHead) {
        session = head.session
        turns = head.turns
        toolSummaries = head.toolSummaries
        events = head.events
        messages = head.messages
        lastEventSeq = head.lastEventSeq
        activity = head.activity
        hasMoreTurns = head.hasMoreTurns
        trimIfNeeded()
    }

    mutating func apply(delta: SessionHeadDelta) {
        lastEventSeq = max(lastEventSeq, delta.lastEventSeq)
        if let turn = delta.turn {
            if let index = turns.firstIndex(where: { $0.turnId == turn.turnId }) {
                turns[index] = turn
            } else {
                turns.append(turn)
            }
            turns.sort { $0.startedAt < $1.startedAt }
        }
        if let event = delta.event {
            var next = events ?? []
            if !next.contains(where: { $0.seq == event.seq }) {
                next.append(event)
            }
            next.sort { $0.seq < $1.seq }
            events = next
        }
        if let message = delta.message {
            if let index = messages.firstIndex(where: { $0.id == message.id }) {
                messages[index] = message
            } else {
                messages.append(message)
            }
        }
        trimIfNeeded()
    }

    private mutating func trimIfNeeded() {
        if turns.count > CacheLimits.maxHeadTurns {
            turns = Array(turns.suffix(CacheLimits.maxHeadTurns))
        }
        if let storedEvents = events, storedEvents.count > CacheLimits.maxHeadEvents {
            events = Array(storedEvents.suffix(CacheLimits.maxHeadEvents))
        }
        if messages.count > CacheLimits.maxHeadMessages {
            messages = Array(messages.suffix(CacheLimits.maxHeadMessages))
        }
    }

    private enum CacheLimits {
        static let maxHeadTurns = 200
        static let maxHeadEvents = 200
        static let maxHeadMessages = 200
    }
}

actor ATSHeadCache {
    static let shared = ATSHeadCache()
    private let cache = DiskCache<CachedSessionHead>(cacheName: "ats-heads", maxEntries: 100)

    func load(sessionId: String) async -> CachedSessionHead? {
        await cache.get(sessionId)
    }

    func store(head: SessionHead) async {
        await cache.set(CachedSessionHead(head: head), for: head.session.id.stringValue)
    }

    func store(heads: [SessionHead]) async {
        for head in heads {
            await store(head: head)
        }
    }

    func apply(delta: SessionHeadDelta) async {
        let key = delta.sessionId.stringValue
        guard var cached = await cache.get(key) else { return }
        cached.apply(delta: delta)
        await cache.set(cached, for: key)
    }
}

extension WorkspaceTaskSummary {
    init(cachedActiveSummary: CachedWorkspaceActiveTaskSummary) {
        var sessions = [cachedActiveSummary.primarySession] + cachedActiveSummary.sessions
        var seen = Set<String>()
        sessions = sessions.filter { summary in
            let id = summary.session.id.stringValue
            guard !seen.contains(id) else { return false }
            seen.insert(id)
            return true
        }
        self.init(task: cachedActiveSummary.task, sessions: sessions, sortAt: cachedActiveSummary.sortAt)
    }
}
