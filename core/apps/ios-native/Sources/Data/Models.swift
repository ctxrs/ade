import Foundation

struct Workspace: Codable, Sendable, Identifiable {
    let id: CtxID
    let name: String
    let rootPath: String
    let createdAt: String
}

struct Task: Codable, Sendable, Identifiable {
    let id: CtxID
    let workspaceId: CtxID
    let title: String
    let description: String?
    let status: String
    let createdAt: String
    let updatedAt: String
    let archivedAt: String?
    let assistantSeenAt: String?
    let lastActivityAt: String?
    let lastAssistantMessageAt: String?
    let hasActiveSession: Bool?
    var primarySessionId: CtxID?
    var primaryWorktreeId: CtxID?
}

struct Worktree: Codable, Sendable, Identifiable {
    let id: CtxID
    let workspaceId: CtxID
    let rootPath: String
    let baseCommitSha: String
    let gitBranch: String?
    let createdAt: String
    let bootstrapStatus: String?
    let bootstrapStartedAt: String?
    let bootstrapFinishedAt: String?
    let bootstrapExitCode: Int?
    let bootstrapTimeoutSec: Int?
    let bootstrapError: String?
    let bootstrapLogPath: String?
    let bootstrapLogTruncated: Bool?
    let bootstrapConfigPath: String?
    let bootstrapConfigKey: String?
    let bootstrapCommand: String?
    let bootstrapScriptPath: String?
}

struct WorktreeSummary: Codable, Sendable, Identifiable {
    let id: CtxID
    let workspaceId: CtxID?
    let label: String?
    let rootPath: String
    let gitBranch: String?
}

struct Session: Codable, Sendable, Identifiable {
    let id: CtxID
    let taskId: CtxID
    let workspaceId: CtxID
    let worktreeId: CtxID
    let parentSessionId: CtxID?
    let relationship: String?
    let providerId: String
    let modelId: String
    let title: String
    let agentRole: String
    let status: String
    let envTarget: String?
    let createdAt: String?
    let updatedAt: String?
}

struct WebSessionViewport: Codable, Sendable {
    let width: Int
    let height: Int
}

struct WebSessionInfo: Codable, Sendable, Identifiable {
    let id: String
    let kind: String
    let sessionId: String?
    let worktreeId: String?
    let status: String
    let createdAt: String
    let updatedAt: String
    let lastActivity: String
    let url: String
    let viewport: WebSessionViewport?
    let fps: Int?
    let viewers: Int?
    let streamPath: String?
    let streamUrl: String?
}

struct TerminalSession: Codable, Sendable, Identifiable {
    let id: CtxID
    let workspaceId: CtxID
    let taskId: CtxID?
    let sessionId: CtxID?
    let worktreeId: CtxID?
    let cwd: String
    let shell: String
    let title: String
    let status: String
    let exitCode: Int?
    let createdAt: String
    let updatedAt: String
}

enum MessageRole: String, Codable, Sendable {
    case user
    case assistant
    case system
}

enum MessageDelivery: String, Codable, Sendable {
    case immediate
    case queued
}

struct Message: Codable, Sendable, Identifiable {
    let id: CtxID
    let sessionId: CtxID
    let turnId: CtxID?
    let turnSequence: Int?
    let role: MessageRole
    let content: String
    let attachments: [MessageAttachment]?
    let delivery: MessageDelivery
    let createdAt: String
}

struct Artifact: Codable, Sendable, Identifiable {
    let id: CtxID
    let sessionId: CtxID
    let taskId: CtxID
    let workspaceId: CtxID
    let worktreeId: CtxID
    let name: String?
    let absolutePath: String
    let mimeType: String
    let bytes: Int
    let createdAt: String
    let missing: Bool?
}

struct MessageAttachment: Codable, Sendable {
    enum Kind: String, Codable, Sendable {
        case image
        case imageRef = "image_ref"
    }

    let kind: Kind
    let mimeType: String
    let dataBase64: String?
    let blobId: String?
    let name: String?
}

struct SessionEvent: Codable, Sendable {
    let seq: Int
    let id: CtxID
    let sessionId: CtxID
    let runId: CtxID?
    let turnId: CtxID?
    let eventType: String
    let payloadJson: JSONValue
    let createdAt: String
}

enum SessionTurnStatus: String, Codable, Sendable {
    case queued
    case running
    case completed
    case interrupted
    case failed
}

struct SessionActivityState: Codable, Sendable {
    let isWorking: Bool
    let lastTurnStatus: SessionTurnStatus?
}

struct SessionTurn: Codable, Sendable {
    let turnId: CtxID
    let sessionId: CtxID
    let runId: CtxID?
    let userMessageId: CtxID?
    let status: SessionTurnStatus
    let startSeq: Int?
    let endSeq: Int?
    let startedAt: String
    let updatedAt: String
    let assistantPartial: String?
    let thoughtPartial: String?
    let metricsJson: JSONValue?
    let toolTotal: Int
    let toolPending: Int
    let toolRunning: Int
    let toolCompleted: Int
    let toolFailed: Int
}

struct SessionTurnTool: Codable, Sendable {
    let sessionId: CtxID
    let toolCallId: String
    let turnId: CtxID
    let toolKind: String?
    let title: String?
    let status: String?
    let inputJson: JSONValue?
    let outputText: String?
    let inputTruncated: Bool?
    let inputOriginalBytes: Int?
    let outputTruncated: Bool?
    let outputOriginalBytes: Int?
    let createdAt: String
    let updatedAt: String
}

struct SessionTurnToolSummary: Codable, Sendable {
    let sessionId: CtxID
    let toolCallId: String
    let turnId: CtxID
    let toolKind: String?
    let title: String?
    let status: String?
    let inputPreview: JSONValue?
    let outputPreview: String?
    let inputTruncated: Bool?
    let inputOriginalBytes: Int?
    let outputTruncated: Bool?
    let outputOriginalBytes: Int?
    let createdAt: String
    let updatedAt: String
}

struct SessionSummaryCheckpoint: Codable, Sendable {
    let sessionId: CtxID
    let checkpointId: String
    let summary: String
    let lastTurnId: CtxID?
    let lastEventSeq: Int?
    let createdAt: String
    let updatedAt: String
}

struct SessionHeadWindow: Codable, Sendable {
    let turnLimit: Int
    let messageLimit: Int
    let eventLimit: Int
    let byteLimit: Int
    let turnCount: Int
    let messageCount: Int
    let eventCount: Int
    let bytes: Int
    let truncated: Bool?
}

struct SessionHead: Codable, Sendable {
    let session: Session
    let turns: [SessionTurn]
    let toolSummaries: [SessionTurnToolSummary]?
    let events: [SessionEvent]?
    let messages: [Message]
    let lastEventSeq: Int
    let stateRev: Int?
    let activity: SessionActivityState?
    let hasMoreTurns: Bool
    let summaryCheckpoint: SessionSummaryCheckpoint?
    let headWindow: SessionHeadWindow?
}

struct SessionHeadDelta: Codable, Sendable {
    let sessionId: CtxID
    let lastEventSeq: Int
    let stateRev: Int?
    let event: SessionEvent?
    let turn: SessionTurn?
    let message: Message?
}

struct SessionHistoryPage: Codable, Sendable {
    let sessionId: CtxID
    let turns: [SessionTurn]
    let messages: [Message]
    let nextCursor: Int?
    let hasMore: Bool
}

struct SessionSnapshotSummary: Codable, Sendable {
    let session: Session
    let lastMessageAt: String?
    let lastMessagePreview: String?
    let lastEventSeq: Int?
    let stateRev: Int?
    let activity: SessionActivityState?
    let unread: Bool?
}

struct SessionSnapshot: Codable, Sendable {
    let summary: SessionSnapshotSummary
    let head: SessionHead
    let state: SessionState?
}

struct SessionGitStatusSummary: Codable, Sendable {
    let summaryLine: String
    let branch: String?
    let upstream: String?
    let ahead: Int
    let behind: Int
    let detached: Bool
    let staged: Int
    let unstaged: Int
    let untracked: Int
}

struct SessionState: Codable, Sendable {
    let artifacts: [Artifact]
    let gitStatus: SessionGitStatusSummary?
}

struct WorkspaceActiveTaskSummary: Codable, Sendable {
    let task: Task
    let primarySession: SessionSnapshotSummary
    let primarySessionHead: SessionHead?
    let sessions: [SessionSnapshotSummary]
    let sortAt: String

    private enum CodingKeys: String, CodingKey {
        case task
        case primarySession
        case primarySessionHead
        case sessions
        case sortAt
    }

    init(task: Task, primarySession: SessionSnapshotSummary, primarySessionHead: SessionHead?, sessions: [SessionSnapshotSummary], sortAt: String) {
        self.task = task
        self.primarySession = primarySession
        self.primarySessionHead = primarySessionHead
        self.sessions = sessions
        self.sortAt = sortAt
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        task = try container.decode(Task.self, forKey: .task)
        primarySession = try container.decode(SessionSnapshotSummary.self, forKey: .primarySession)
        primarySessionHead = try container.decodeIfPresent(SessionHead.self, forKey: .primarySessionHead)
        sessions = try container.decodeIfPresent([SessionSnapshotSummary].self, forKey: .sessions) ?? []
        sortAt = try container.decode(String.self, forKey: .sortAt)
    }
}

struct WorkspaceActivePage: Codable, Sendable {
    let tasks: [WorkspaceActiveTaskSummary]
    let totalCount: Int
}

struct WorkspaceActiveSnapshot: Codable, Sendable {
    let workspaceId: CtxID
    let snapshotRev: Int
    let archivedRev: Int?
    let active: WorkspaceActivePage
}

struct WorkspaceActiveHeadBatch: Codable, Sendable {
    let workspaceId: CtxID
    let snapshotRev: Int
    let heads: [SessionHead]
}

struct WorkspaceStreamActiveHeads: Decodable, Sendable {
    let heads: [SessionHead]

    private enum CodingKeys: String, CodingKey {
        case heads
    }

    init(from decoder: Decoder) throws {
        if let container = try? decoder.container(keyedBy: CodingKeys.self),
           let heads = try? container.decode([SessionHead].self, forKey: .heads) {
            self.heads = heads
            return
        }
        if let batch = try? WorkspaceActiveHeadBatch(from: decoder) {
            self.heads = batch.heads
            return
        }
        let container = try decoder.singleValueContainer()
        self.heads = try container.decode([SessionHead].self)
    }
}

struct WorkspaceStreamSnapshot: Sendable {
    let rev: Int
    let activeSnapshot: WorkspaceActiveSnapshot
    let activeHeads: WorkspaceStreamActiveHeads?
}

enum WorkspaceStreamServerMessage: Decodable, Sendable {
    case snapshot(WorkspaceStreamSnapshot)
    case event(rev: Int?, event: WorkspaceActiveSnapshotEvent)
    case resetRequired(latestRev: Int)

    private enum CodingKeys: String, CodingKey {
        case type
        case rev
        case activeSnapshot
        case activeHeads
        case event
        case latestRev
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        guard let rawType = try? container.decode(String.self, forKey: .type) else {
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "Missing workspace stream message type."
            )
        }
        switch rawType.lowercased() {
        case "snapshot":
            let rev = (try? container.decode(Int.self, forKey: .rev)) ?? 0
            let snapshot = try container.decode(WorkspaceActiveSnapshot.self, forKey: .activeSnapshot)
            let heads = try container.decodeIfPresent(WorkspaceStreamActiveHeads.self, forKey: .activeHeads)
            self = .snapshot(WorkspaceStreamSnapshot(rev: rev, activeSnapshot: snapshot, activeHeads: heads))
        case "event":
            let rev = try container.decodeIfPresent(Int.self, forKey: .rev)
            let event = try container.decodeIfPresent(WorkspaceActiveSnapshotEvent.self, forKey: .event)
                ?? WorkspaceActiveSnapshotEvent(from: decoder)
            self = .event(rev: rev, event: event)
        case "reset_required":
            let latestRev = (try? container.decode(Int.self, forKey: .latestRev))
                ?? (try? container.decode(Int.self, forKey: .rev))
                ?? 0
            self = .resetRequired(latestRev: latestRev)
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .type,
                in: container,
                debugDescription: "Unknown workspace stream message type."
            )
        }
    }
}

enum WorkspaceActiveSnapshotEvent: Codable, Sendable {
    case ready(workspaceId: CtxID, snapshotRev: Int, archivedRev: Int)
    case activeTaskUpsert(workspaceId: CtxID, snapshotRev: Int, task: WorkspaceActiveTaskSummary)
    case activeTaskDelete(workspaceId: CtxID, snapshotRev: Int, taskId: CtxID)
    case sessionSummary(workspaceId: CtxID, snapshotRev: Int, summary: SessionSnapshotSummary)
    case sessionHeadDelta(workspaceId: CtxID, snapshotRev: Int, delta: SessionHeadDelta)
    case sessionHeadReset(workspaceId: CtxID, snapshotRev: Int, head: SessionHead)
    case sessionGap(workspaceId: CtxID, snapshotRev: Int, sessionId: CtxID, afterSeq: Int, reason: String?)
    case worktreeBootstrap(workspaceId: CtxID, snapshotRev: Int, notice: WorktreeBootstrapNotice)
    case archivedTaskUpsert(workspaceId: CtxID, archivedRev: Int, task: WorkspaceArchivedTaskSummaryPayload)
    case archivedTaskDelete(workspaceId: CtxID, archivedRev: Int, taskId: CtxID)

    private enum EventType: String, Codable {
        case ready
        case activeTaskUpsert = "active_task_upsert"
        case activeTaskDelete = "active_task_delete"
        case sessionSummary = "session_summary"
        case sessionHeadDelta = "session_head_delta"
        case sessionHeadReset = "session_head_reset"
        case sessionGap = "session_gap"
        case worktreeBootstrap = "worktree_bootstrap"
        case archivedTaskUpsert = "archived_task_upsert"
        case archivedTaskDelete = "archived_task_delete"
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case workspaceId
        case snapshotRev
        case archivedRev
        case task
        case taskId
        case summary
        case delta
        case head
        case sessionId
        case afterSeq
        case reason
        case notice
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(EventType.self, forKey: .type)
        let workspaceId = try container.decode(CtxID.self, forKey: .workspaceId)
        let snapshotRev = try container.decodeIfPresent(Int.self, forKey: .snapshotRev) ?? 0
        let archivedRev = try container.decodeIfPresent(Int.self, forKey: .archivedRev) ?? 0
        switch type {
        case .ready:
            self = .ready(workspaceId: workspaceId, snapshotRev: snapshotRev, archivedRev: archivedRev)
        case .activeTaskUpsert:
            let task = try container.decode(WorkspaceActiveTaskSummary.self, forKey: .task)
            self = .activeTaskUpsert(workspaceId: workspaceId, snapshotRev: snapshotRev, task: task)
        case .activeTaskDelete:
            let taskId = try container.decode(CtxID.self, forKey: .taskId)
            self = .activeTaskDelete(workspaceId: workspaceId, snapshotRev: snapshotRev, taskId: taskId)
        case .sessionSummary:
            let summary = try container.decode(SessionSnapshotSummary.self, forKey: .summary)
            self = .sessionSummary(workspaceId: workspaceId, snapshotRev: snapshotRev, summary: summary)
        case .sessionHeadDelta:
            let delta = try container.decode(SessionHeadDelta.self, forKey: .delta)
            self = .sessionHeadDelta(workspaceId: workspaceId, snapshotRev: snapshotRev, delta: delta)
        case .sessionHeadReset:
            let head = try container.decode(SessionHead.self, forKey: .head)
            self = .sessionHeadReset(workspaceId: workspaceId, snapshotRev: snapshotRev, head: head)
        case .sessionGap:
            let sessionId = try container.decode(CtxID.self, forKey: .sessionId)
            let afterSeq = try container.decode(Int.self, forKey: .afterSeq)
            let reason = try container.decodeIfPresent(String.self, forKey: .reason)
            self = .sessionGap(workspaceId: workspaceId, snapshotRev: snapshotRev, sessionId: sessionId, afterSeq: afterSeq, reason: reason)
        case .worktreeBootstrap:
            let notice = try container.decode(WorktreeBootstrapNotice.self, forKey: .notice)
            self = .worktreeBootstrap(workspaceId: workspaceId, snapshotRev: snapshotRev, notice: notice)
        case .archivedTaskUpsert:
            let task = try container.decode(WorkspaceArchivedTaskSummaryPayload.self, forKey: .task)
            self = .archivedTaskUpsert(workspaceId: workspaceId, archivedRev: archivedRev, task: task)
        case .archivedTaskDelete:
            let taskId = try container.decode(CtxID.self, forKey: .taskId)
            self = .archivedTaskDelete(workspaceId: workspaceId, archivedRev: archivedRev, taskId: taskId)
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .ready(let workspaceId, let snapshotRev, let archivedRev):
            try container.encode(EventType.ready, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(archivedRev, forKey: .archivedRev)
        case .activeTaskUpsert(let workspaceId, let snapshotRev, let task):
            try container.encode(EventType.activeTaskUpsert, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(task, forKey: .task)
        case .activeTaskDelete(let workspaceId, let snapshotRev, let taskId):
            try container.encode(EventType.activeTaskDelete, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(taskId, forKey: .taskId)
        case .sessionSummary(let workspaceId, let snapshotRev, let summary):
            try container.encode(EventType.sessionSummary, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(summary, forKey: .summary)
        case .sessionHeadDelta(let workspaceId, let snapshotRev, let delta):
            try container.encode(EventType.sessionHeadDelta, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(delta, forKey: .delta)
        case .sessionHeadReset(let workspaceId, let snapshotRev, let head):
            try container.encode(EventType.sessionHeadReset, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(head, forKey: .head)
        case .sessionGap(let workspaceId, let snapshotRev, let sessionId, let afterSeq, let reason):
            try container.encode(EventType.sessionGap, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(sessionId, forKey: .sessionId)
            try container.encode(afterSeq, forKey: .afterSeq)
            try container.encodeIfPresent(reason, forKey: .reason)
        case .worktreeBootstrap(let workspaceId, let snapshotRev, let notice):
            try container.encode(EventType.worktreeBootstrap, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(notice, forKey: .notice)
        case .archivedTaskUpsert(let workspaceId, let archivedRev, let task):
            try container.encode(EventType.archivedTaskUpsert, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(archivedRev, forKey: .archivedRev)
            try container.encode(task, forKey: .task)
        case .archivedTaskDelete(let workspaceId, let archivedRev, let taskId):
            try container.encode(EventType.archivedTaskDelete, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(archivedRev, forKey: .archivedRev)
            try container.encode(taskId, forKey: .taskId)
        }
    }
}

struct WorkspaceActiveSnapshotSessionSubscription: Codable, Sendable {
    let sessionId: CtxID
    let afterSeq: Int?
}

struct WorkspaceActiveSnapshotClientMessage: Codable, Sendable {
    let type: String
    let workspaceId: CtxID?
    let fromRev: Int?
    let includeActiveHeads: Bool?
    let sessionIds: [CtxID]
    let sessions: [WorkspaceActiveSnapshotSessionSubscription]
}

struct WorkspaceIndexCursor: Codable, Sendable {
    let sortAt: String
    let taskId: CtxID
}

struct WorkspaceArchivedPage: Codable, Sendable {
    let workspaceId: CtxID
    let archivedRev: Int?
    let tasks: [WorkspaceArchivedTaskSummaryPayload]
    let nextCursor: WorkspaceIndexCursor?
    let totalArchived: Int
}

struct WorkspaceArchivedTaskSummaryPayload: Codable, Sendable {
    let task: Task
    let sessions: [ArchivedSessionSummary]
    let sortAt: String
    let primarySessionHead: SessionHead?

    private enum CodingKeys: String, CodingKey {
        case task
        case sessions
        case sortAt
        case primarySessionHead
    }

    init(task: Task, sessions: [ArchivedSessionSummary], sortAt: String, primarySessionHead: SessionHead?) {
        self.task = task
        self.sessions = sessions
        self.sortAt = sortAt
        self.primarySessionHead = primarySessionHead
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        task = try container.decode(Task.self, forKey: .task)
        sessions = try container.decodeIfPresent([ArchivedSessionSummary].self, forKey: .sessions) ?? []
        sortAt = try container.decode(String.self, forKey: .sortAt)
        primarySessionHead = try container.decodeIfPresent(SessionHead.self, forKey: .primarySessionHead)
    }
}

struct WorkspaceSessionSummaryPayload: Codable, Sendable {
    let id: CtxID
    let taskId: CtxID
    let workspaceId: CtxID
    let parentSessionId: CtxID?
    let relationship: String?
    let providerId: String
    let modelId: String
    let title: String
    let status: String
    let createdAt: String
    let updatedAt: String
    let worktreeId: CtxID?
    let agentRole: String?
    let envTarget: String?
}

enum ArchivedSessionSummary: Codable, Sendable {
    case snapshot(SessionSnapshotSummary)
    case summary(WorkspaceSessionSummaryPayload)

    init(from decoder: Decoder) throws {
        if let snapshot = try? SessionSnapshotSummary(from: decoder) {
            self = .snapshot(snapshot)
            return
        }
        self = .summary(try WorkspaceSessionSummaryPayload(from: decoder))
    }

    func encode(to encoder: Encoder) throws {
        switch self {
        case .snapshot(let snapshot):
            try snapshot.encode(to: encoder)
        case .summary(let summary):
            try summary.encode(to: encoder)
        }
    }

    func toSnapshotSummary(fallbackWorktreeId: CtxID?) -> SessionSnapshotSummary {
        switch self {
        case .snapshot(let snapshot):
            return snapshot
        case .summary(let summary):
            let worktreeId = summary.worktreeId ?? fallbackWorktreeId ?? CtxID("00000000-0000-0000-0000-000000000000")
            let session = Session(
                id: summary.id,
                taskId: summary.taskId,
                workspaceId: summary.workspaceId,
                worktreeId: worktreeId,
                parentSessionId: summary.parentSessionId,
                relationship: summary.relationship,
                providerId: summary.providerId,
                modelId: summary.modelId,
                title: summary.title,
                agentRole: summary.agentRole ?? "assistant",
                status: summary.status,
                envTarget: summary.envTarget,
                createdAt: summary.createdAt,
                updatedAt: summary.updatedAt
            )
            return SessionSnapshotSummary(
                session: session,
                lastMessageAt: nil,
                lastMessagePreview: nil,
                lastEventSeq: nil,
                stateRev: nil,
                activity: nil,
                unread: nil
            )
        }
    }
}

struct WorkspaceTaskSummary: Sendable {
    let task: Task
    let sessions: [SessionSnapshotSummary]
    let sortAt: String

    init(task: Task, sessions: [SessionSnapshotSummary], sortAt: String) {
        self.task = task
        self.sessions = sessions
        self.sortAt = sortAt
    }
}

struct WorktreeBootstrapNotice: Codable, Sendable {
    let worktreeId: CtxID
    let worktreeRoot: String
    let status: String
    let startedAt: String
    let finishedAt: String
    let exitCode: Int?
    let timeoutSec: Int?
    let configPath: String?
    let configKey: String?
    let command: String?
    let scriptPath: String?
    let logPath: String?
    let logTruncated: Bool?
    let error: String?
}

struct ProviderStatus: Codable, Sendable {
    let providerId: String
    let installed: Bool
    let detectedPath: String?
    let version: String?
    let health: String
    let diagnostics: [String]
    let details: [String: String]?
}

enum InstallStateKind: String, Codable, Sendable {
    case running
    case succeeded
    case failed
}

enum InstallEventLevel: String, Codable, Sendable {
    case info
    case warning
    case error
    case success
}

struct InstallProgressEvent: Codable, Sendable {
    let installId: String
    let providerId: String
    let at: String
    let stage: String
    let message: String
    let level: InstallEventLevel
    let bytes: Int?
    let totalBytes: Int?
    let attempt: Int?
}

struct InstallInfo: Codable, Sendable {
    let installId: String
    let providerId: String
    let state: InstallStateKind
    let startedAt: String
    let finishedAt: String?
    let error: String?
    let lastEvent: InstallProgressEvent?
}

struct InstallStartResponse: Codable, Sendable {
    let providerId: String
    let installId: String
}

struct Diagnostics: Codable, Sendable {
    struct Daemon: Codable, Sendable {
        let version: String
        let pid: Int
        let dataRoot: String
        let daemonUrl: String
        let authRequired: Bool
    }

    struct Platform: Codable, Sendable {
        let os: String
        let arch: String
    }

    struct Logs: Codable, Sendable {
        struct LogFile: Codable, Sendable {
            let name: String
            let bytes: Int
            let modifiedUtc: String?
        }

        let dir: String
        let files: [LogFile]
    }

    let daemon: Daemon
    let platform: Platform
    let logs: Logs
    let providers: [ProviderStatus]
    let managedInstalls: JSONValue?
}

struct MobileDeviceRegistration: Codable, Sendable {
    let id: String
    let profileId: String
    let deviceLabel: String?
    let platform: String?
    let pushToken: String?
    let pushProvider: String?
    let publicKey: String?
    let appVersion: String?
    let createdAt: String
    let lastSeenAt: String
}

enum MobileTunnelState: String, Codable, Sendable {
    case idle
    case running
    case error
}

struct MobileAccessStatus: Codable, Sendable {
    let enabled: Bool
    let tunnelId: String?
    let publicBaseUrl: String?
    let relayBaseUrl: String?
    let daemonPublicKey: String?
    let tunnelState: MobileTunnelState
    let lastError: String?
}

struct EnableMobileAccessResponse: Codable, Sendable {
    let status: MobileAccessStatus
    let qrPayload: JSONValue
    let pairingExpiresAt: String
}

struct EntitlementsSnapshot: Codable, Sendable {
    enum PlanType: String, Codable, Sendable {
        case freeLocal = "free_local"
        case pro
        case team
        case enterprise
    }

    let planType: PlanType
    let features: [String: String]
    let expiresAt: String?
    let graceExpiresAt: String?

    func isFeatureEnabled(_ key: String) -> Bool {
        features[key] == "enabled"
    }
}

struct PublicSettings: Codable, Sendable {
    let dictation: DictationSettings?
    let telemetry: TelemetrySettings?
    let titleGeneration: TitleGenerationSettings?
    let resourceGovernance: ResourceGovernanceSettings?
    let providerGuard: ProviderGuardSettings?
}

struct DictationSettings: Codable, Sendable {
    let enabled: Bool
    let provider: String
    let livekit: LiveKitDictationSettings?
}

struct LiveKitDictationSettings: Codable, Sendable {
    let baseUrl: String
    let apiKey: String
    let apiSecretSet: Bool?
    let model: String
    let language: String
}

struct TelemetrySettings: Codable, Sendable {
    let enabled: Bool
    let endpoint: String
}

struct TitleGenerationSettings: Codable, Sendable {
    let mode: String
    let remote: TitleGenerationRemoteSettings
    let local: TitleGenerationLocalSettings
}

struct TitleGenerationRemoteSettings: Codable, Sendable {
    let baseUrl: String
    let apiKey: String
    let model: String
    let useJson: Bool
}

struct TitleGenerationLocalSettings: Codable, Sendable {
    let modelId: String
    let useJson: Bool
}

struct TitleGenerationLocalStatus: Codable, Sendable {
    let ready: Bool
    let runtime: TitleGenerationLocalRuntimeStatus
    let model: TitleGenerationLocalModelStatus
    let installId: String?
    let installRunning: Bool?
}

struct TitleGenerationLocalRuntimeStatus: Codable, Sendable {
    let version: String
    let installed: Bool
    let path: String?
}

struct TitleGenerationLocalModelStatus: Codable, Sendable {
    let modelId: String
    let fileName: String
    let installed: Bool
    let version: String?
    let sha256: String?
    let sizeBytes: Int?
    let installedAt: String?
}

struct ResourceGovernanceSettings: Codable, Sendable {
    let enabled: Bool
    let mode: String
    let cpuQuotaPct: Int?
    let memoryHighMb: Int?
    let memoryMaxMb: Int?
    let effective: ResourceGovernanceLimits?
    let status: ResourceGovernanceStatus?
}

struct ResourceGovernanceLimits: Codable, Sendable {
    let cpuQuotaPct: Int
    let memoryHighMb: Int
    let memoryMaxMb: Int
}

struct ResourceGovernanceStatus: Codable, Sendable {
    let state: String
    let canApplyNow: Bool
    let requiresRestart: Bool
    let message: String?
}

struct ProviderGuardSettings: Codable, Sendable {
    let enabled: Bool
    let mode: String
    let memoryHighMb: Int?
    let memoryMaxMb: Int?
    let intervalMs: Int?
    let gracePeriodMs: Int?
}

struct SettingsUpdate: Encodable, Sendable {
    let telemetry: TelemetrySettingsUpdate?
}

struct TelemetrySettingsUpdate: Encodable, Sendable {
    let enabled: Bool
    let endpoint: String
}

struct WorkspaceSummary: Codable, Sendable, Identifiable {
    let id: String
    let name: String
    let rootPath: String
    let createdAt: String
}

struct TaskSummary: Codable, Sendable, Identifiable {
    let id: String
    let workspaceId: String
    let title: String
    let description: String?
    let status: String
    let updatedAt: String
    let lastActivityAt: String?
}

struct SessionSummary: Codable, Sendable, Identifiable {
    let id: String
    let taskId: String?
    let workspaceId: String?
    let worktreeId: String?
    let relationship: String?
    let providerId: String
    let modelId: String
    let title: String
    let status: String
    let agentRole: String
}

struct MessageSummary: Codable, Sendable, Identifiable {
    let id: String
    let sessionId: String
    let role: MessageRole
    let content: String
    let attachments: [MessageAttachment]?
    let delivery: MessageDelivery
    let createdAt: String
}

extension WorkspaceSummary {
    init(workspace: Workspace) {
        self.init(
            id: workspace.id.stringValue,
            name: workspace.name,
            rootPath: workspace.rootPath,
            createdAt: workspace.createdAt
        )
    }
}

extension TaskSummary {
    init(task: Task) {
        self.init(
            id: task.id.stringValue,
            workspaceId: task.workspaceId.stringValue,
            title: task.title,
            description: task.description,
            status: task.status,
            updatedAt: task.updatedAt,
            lastActivityAt: task.lastActivityAt
        )
    }
}

extension SessionSummary {
    init(session: Session) {
        self.init(
            id: session.id.stringValue,
            taskId: session.taskId.stringValue,
            workspaceId: session.workspaceId.stringValue,
            worktreeId: session.worktreeId.stringValue,
            relationship: session.relationship,
            providerId: session.providerId,
            modelId: session.modelId,
            title: session.title,
            status: session.status,
            agentRole: session.agentRole
        )
    }
}

extension SessionSnapshotSummary {
    init(session: Session) {
        self.init(
            session: session,
            lastMessageAt: nil,
            lastMessagePreview: nil,
            lastEventSeq: nil,
            stateRev: nil,
            activity: nil,
            unread: nil
        )
    }
}

extension WorkspaceTaskSummary {
    init(activeSummary: WorkspaceActiveTaskSummary) {
        var sessions = [activeSummary.primarySession] + activeSummary.sessions
        var seen = Set<String>()
        sessions = sessions.filter { summary in
            let id = summary.session.id.stringValue
            guard !seen.contains(id) else { return false }
            seen.insert(id)
            return true
        }
        self.init(task: activeSummary.task, sessions: sessions, sortAt: activeSummary.task.createdAt)
    }
}

extension WorkspaceArchivedTaskSummaryPayload {
    func toTaskSummary() -> WorkspaceTaskSummary {
        let fallbackWorktreeId = task.primaryWorktreeId
        let summaries = sessions.map { $0.toSnapshotSummary(fallbackWorktreeId: fallbackWorktreeId) }
        return WorkspaceTaskSummary(task: task, sessions: summaries, sortAt: sortAt)
    }
}

extension MessageSummary {
    init(message: Message) {
        self.init(
            id: message.id.stringValue,
            sessionId: message.sessionId.stringValue,
            role: message.role,
            content: message.content,
            attachments: message.attachments,
            delivery: message.delivery,
            createdAt: message.createdAt
        )
    }
}
