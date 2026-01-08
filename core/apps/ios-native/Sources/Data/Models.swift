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
}

struct Track: Codable, Sendable, Identifiable {
    let id: CtxID
    let taskId: CtxID
    let workspaceId: CtxID
    let worktreeId: CtxID
    let label: String
    let status: String
    let createdAt: String?
    let updatedAt: String?
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

struct Session: Codable, Sendable, Identifiable {
    let id: CtxID
    let trackId: CtxID
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
    let createdAt: String
    let updatedAt: String
}

struct SessionHead: Codable, Sendable {
    let session: Session
    let turns: [SessionTurn]
    let toolSummaries: [SessionTurnToolSummary]?
    let events: [SessionEvent]?
    let messages: [Message]
    let lastEventSeq: Int
    let activity: SessionActivityState?
    let hasMoreTurns: Bool
}

struct SessionHeadDelta: Codable, Sendable {
    let sessionId: CtxID
    let lastEventSeq: Int
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

struct WorkspaceCatchupCursor: Codable, Sendable {
    let sortAt: String
    let taskId: CtxID
}

struct SessionCatchupSummary: Codable, Sendable {
    let session: Session
    let lastMessageAt: String?
    let lastMessagePreview: String?
    let lastEventSeq: Int?
    let activity: SessionActivityState?
    let unread: Bool?
}

struct TrackDiffSummary: Codable, Sendable {
    let fileCount: Int
    let lineAdditions: Int
    let lineDeletions: Int
    let updatedAt: String
}

struct TrackDiffSummaryResponse: Codable, Sendable {
    let summary: TrackDiffSummary?
    let tooLarge: Bool
}

struct WorkspaceCatchupTrackSummary: Codable, Sendable {
    let track: Track
    let primarySessionId: CtxID?
    let sessions: [SessionCatchupSummary]
    let diffSummary: TrackDiffSummary?
}

struct WorkspaceCatchupTaskSummary: Codable, Sendable {
    let task: Task
    let tracks: [WorkspaceCatchupTrackSummary]
    let sortAt: String
}

struct WorkspaceCatchupPage: Codable, Sendable {
    let tasks: [WorkspaceCatchupTaskSummary]
    let nextCursor: WorkspaceCatchupCursor?
    let totalCount: Int
}

struct WorkspaceCatchupSnapshot: Codable, Sendable {
    let workspaceId: CtxID
    let snapshotRev: Int
    let active: WorkspaceCatchupPage
    let archived: WorkspaceCatchupPage?
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

enum WorkspaceCatchupEvent: Codable, Sendable {
    case ready(workspaceId: CtxID, snapshotRev: Int)
    case taskUpsert(workspaceId: CtxID, snapshotRev: Int, task: WorkspaceCatchupTaskSummary)
    case taskDelete(workspaceId: CtxID, snapshotRev: Int, taskId: CtxID)
    case trackUpsert(workspaceId: CtxID, snapshotRev: Int, track: WorkspaceCatchupTrackSummary)
    case sessionSummary(workspaceId: CtxID, snapshotRev: Int, summary: SessionCatchupSummary)
    case sessionHeadDelta(workspaceId: CtxID, snapshotRev: Int, delta: SessionHeadDelta)
    case sessionGap(workspaceId: CtxID, snapshotRev: Int, sessionId: CtxID, afterSeq: Int, reason: String?)
    case worktreeBootstrap(workspaceId: CtxID, snapshotRev: Int, notice: WorktreeBootstrapNotice)

    private enum EventType: String, Codable {
        case ready
        case taskUpsert = "task_upsert"
        case taskDelete = "task_delete"
        case trackUpsert = "track_upsert"
        case sessionSummary = "session_summary"
        case sessionHeadDelta = "session_head_delta"
        case sessionGap = "session_gap"
        case worktreeBootstrap = "worktree_bootstrap"
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case workspaceId
        case snapshotRev
        case task
        case taskId
        case track
        case summary
        case delta
        case sessionId
        case afterSeq
        case reason
        case notice
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decode(EventType.self, forKey: .type)
        let workspaceId = try container.decode(CtxID.self, forKey: .workspaceId)
        let snapshotRev = try container.decode(Int.self, forKey: .snapshotRev)
        switch type {
        case .ready:
            self = .ready(workspaceId: workspaceId, snapshotRev: snapshotRev)
        case .taskUpsert:
            let task = try container.decode(WorkspaceCatchupTaskSummary.self, forKey: .task)
            self = .taskUpsert(workspaceId: workspaceId, snapshotRev: snapshotRev, task: task)
        case .taskDelete:
            let taskId = try container.decode(CtxID.self, forKey: .taskId)
            self = .taskDelete(workspaceId: workspaceId, snapshotRev: snapshotRev, taskId: taskId)
        case .trackUpsert:
            let track = try container.decode(WorkspaceCatchupTrackSummary.self, forKey: .track)
            self = .trackUpsert(workspaceId: workspaceId, snapshotRev: snapshotRev, track: track)
        case .sessionSummary:
            let summary = try container.decode(SessionCatchupSummary.self, forKey: .summary)
            self = .sessionSummary(workspaceId: workspaceId, snapshotRev: snapshotRev, summary: summary)
        case .sessionHeadDelta:
            let delta = try container.decode(SessionHeadDelta.self, forKey: .delta)
            self = .sessionHeadDelta(workspaceId: workspaceId, snapshotRev: snapshotRev, delta: delta)
        case .sessionGap:
            let sessionId = try container.decode(CtxID.self, forKey: .sessionId)
            let afterSeq = try container.decode(Int.self, forKey: .afterSeq)
            let reason = try container.decodeIfPresent(String.self, forKey: .reason)
            self = .sessionGap(workspaceId: workspaceId, snapshotRev: snapshotRev, sessionId: sessionId, afterSeq: afterSeq, reason: reason)
        case .worktreeBootstrap:
            let notice = try container.decode(WorktreeBootstrapNotice.self, forKey: .notice)
            self = .worktreeBootstrap(workspaceId: workspaceId, snapshotRev: snapshotRev, notice: notice)
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .ready(let workspaceId, let snapshotRev):
            try container.encode(EventType.ready, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
        case .taskUpsert(let workspaceId, let snapshotRev, let task):
            try container.encode(EventType.taskUpsert, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(task, forKey: .task)
        case .taskDelete(let workspaceId, let snapshotRev, let taskId):
            try container.encode(EventType.taskDelete, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(taskId, forKey: .taskId)
        case .trackUpsert(let workspaceId, let snapshotRev, let track):
            try container.encode(EventType.trackUpsert, forKey: .type)
            try container.encode(workspaceId, forKey: .workspaceId)
            try container.encode(snapshotRev, forKey: .snapshotRev)
            try container.encode(track, forKey: .track)
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
        }
    }
}

struct WorkspaceCatchupSessionSubscription: Codable, Sendable {
    let sessionId: CtxID
    let afterSeq: Int?
}

struct WorkspaceCatchupClientMessage: Codable, Sendable {
    let type: String
    let sessionIds: [CtxID]?
    let sessions: [WorkspaceCatchupSessionSubscription]?
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

struct TrackSummary: Codable, Sendable, Identifiable {
    let id: String
    let taskId: String
    let label: String
    let status: String
    let worktreeId: String
}

struct SessionSummary: Codable, Sendable, Identifiable {
    let id: String
    let trackId: String
    let taskId: String?
    let workspaceId: String?
    let worktreeId: String?
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

extension TrackSummary {
    init(track: Track) {
        self.init(
            id: track.id.stringValue,
            taskId: track.taskId.stringValue,
            label: track.label,
            status: track.status,
            worktreeId: track.worktreeId.stringValue
        )
    }
}

extension SessionSummary {
    init(session: Session) {
        self.init(
            id: session.id.stringValue,
            trackId: session.trackId.stringValue,
            taskId: session.taskId.stringValue,
            workspaceId: session.workspaceId.stringValue,
            worktreeId: session.worktreeId.stringValue,
            providerId: session.providerId,
            modelId: session.modelId,
            title: session.title,
            status: session.status,
            agentRole: session.agentRole
        )
    }
}

extension MessageSummary {
    init(message: Message) {
        self.init(
            id: message.id.stringValue,
            sessionId: message.sessionId.stringValue,
            role: message.role,
            content: message.content,
            delivery: message.delivery,
            createdAt: message.createdAt
        )
    }
}
