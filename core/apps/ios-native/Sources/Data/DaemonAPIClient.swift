import Foundation
import _Concurrency

enum DaemonAPIError: Error {
    case invalidURL
    case missingToken
    case invalidResponse
    case requestFailed(statusCode: Int, message: String)
}

extension DaemonAPIError {
    var userMessage: String {
        switch self {
        case .invalidURL:
            return "Invalid daemon URL."
        case .missingToken:
            return "Missing access token. Open the launcher to reconnect."
        case .invalidResponse:
            return "Invalid response from daemon."
        case .requestFailed(let statusCode, let message):
            if message.isEmpty {
                return "Request failed (\(statusCode))."
            }
            return "Request failed (\(statusCode)): \(message)"
        }
    }
}

func daemonErrorMessage(_ error: Error, fallback: String) -> String {
    if let apiError = error as? DaemonAPIError {
        return apiError.userMessage
    }
    if let urlError = error as? URLError {
        return "Network error: \(urlError.localizedDescription)"
    }
    return fallback
}

enum HTTPMethod: String {
    case get = "GET"
    case post = "POST"
    case delete = "DELETE"
}

actor DaemonAPIClient {
    struct SessionDiffResponse: Codable, Sendable {
        let diff: String
    }

    struct SessionGitStatusResponse: Codable, Sendable {
        let summary: String
        let staged: Int?
        let unstaged: Int?
        let untracked: Int?
        let entries: [GitStatusEntry]

        init(summary: String) {
            self.summary = summary
            self.staged = nil
            self.unstaged = nil
            self.untracked = nil
            self.entries = []
        }

        init(from decoder: Decoder) throws {
            if let single = try? decoder.singleValueContainer().decode(String.self) {
                self.summary = single
                self.staged = nil
                self.unstaged = nil
                self.untracked = nil
                self.entries = []
                return
            }
            let container = try decoder.container(keyedBy: CodingKeys.self)
            if let summary = try container.decodeIfPresent(String.self, forKey: .raw) {
                self.summary = summary
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            if let summary = try container.decodeIfPresent(String.self, forKey: .output) {
                self.summary = summary
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            if let lines = try container.decodeIfPresent([String].self, forKey: .lines) {
                self.summary = lines.joined(separator: "\n")
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            if let summary = try container.decodeIfPresent(String.self, forKey: .summary) {
                self.summary = summary
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            if let summary = try container.decodeIfPresent(String.self, forKey: .summaryLine) {
                self.summary = summary
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            if let summary = try container.decodeIfPresent(String.self, forKey: .summaryLineSnake) {
                self.summary = summary
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            if let summary = try container.decodeIfPresent(String.self, forKey: .statusSummary) {
                self.summary = summary
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            if let summary = try container.decodeIfPresent(String.self, forKey: .status) {
                self.summary = summary
                self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
                self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
                self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
                self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
                return
            }
            self.summary = ""
            self.staged = try container.decodeIfPresent(Int.self, forKey: .staged)
            self.unstaged = try container.decodeIfPresent(Int.self, forKey: .unstaged)
            self.untracked = try container.decodeIfPresent(Int.self, forKey: .untracked)
            self.entries = try container.decodeIfPresent([GitStatusEntry].self, forKey: .entries) ?? []
        }

        private enum CodingKeys: String, CodingKey {
            case summary
            case summaryLine = "summaryLine"
            case summaryLineSnake = "summary_line"
            case statusSummary
            case status
            case raw
            case output
            case lines
            case staged
            case unstaged
            case untracked
            case entries
        }
    }

    struct GitStatusEntry: Codable, Sendable {
        let path: String
        let origPath: String?
        let indexStatus: String?
        let worktreeStatus: String?

        private enum CodingKeys: String, CodingKey {
            case path
            case origPath = "orig_path"
            case indexStatus = "index_status"
            case worktreeStatus = "worktree_status"
        }
    }

    struct SessionGitDiffSummaryResponse: Codable, Sendable {
        let fileCount: Int
        let additions: Int
        let deletions: Int

        init(fileCount: Int, additions: Int, deletions: Int) {
            self.fileCount = fileCount
            self.additions = additions
            self.deletions = deletions
        }

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            let fileCount = try container.decodeIfPresent(Int.self, forKey: .fileCount)
                ?? container.decodeIfPresent(Int.self, forKey: .files)
                ?? container.decodeIfPresent(Int.self, forKey: .fileCountSnake)
                ?? container.decodeIfPresent(Int.self, forKey: .changedFiles)
                ?? container.decodeIfPresent(Int.self, forKey: .filesChanged)
                ?? 0
            let additions = try container.decodeIfPresent(Int.self, forKey: .additions)
                ?? container.decodeIfPresent(Int.self, forKey: .lineAdditions)
                ?? container.decodeIfPresent(Int.self, forKey: .added)
                ?? container.decodeIfPresent(Int.self, forKey: .insertions)
                ?? 0
            let deletions = try container.decodeIfPresent(Int.self, forKey: .deletions)
                ?? container.decodeIfPresent(Int.self, forKey: .lineDeletions)
                ?? container.decodeIfPresent(Int.self, forKey: .deleted)
                ?? container.decodeIfPresent(Int.self, forKey: .removed)
                ?? 0
            self.fileCount = fileCount
            self.additions = additions
            self.deletions = deletions
        }

        private enum CodingKeys: String, CodingKey {
            case fileCount
            case files
            case fileCountSnake = "file_count"
            case changedFiles
            case filesChanged
            case additions
            case lineAdditions = "line_additions"
            case added
            case insertions
            case deletions
            case lineDeletions = "line_deletions"
            case deleted
            case removed
        }
    }

    struct ProviderOptions: Codable, Sendable {
        let providerId: String
        let workspaceId: String
        let installed: Bool?
        let probeOk: Bool?
        let probeError: String?
        let supportsLoad: Bool
        let authRequired: Bool
        let authMethods: JSONValue?
        let modes: JSONValue?
        let models: JSONValue?
        let acpError: JSONValue?
        let verify: JSONValue?
        let probedAt: String
    }

    struct RegisterMobileDeviceRequest: Codable, Sendable {
        let deviceId: String
        let deviceLabel: String?
        let platform: String?
        let pushToken: String?
        let pushProvider: String?
        let publicKey: String?
        let appVersion: String?
    }

    struct PairMobileDeviceRequest: Codable, Sendable {
        let pairingToken: String
        let deviceId: String
        let deviceLabel: String?
        let platform: String?
        let publicKey: String
        let appVersion: String?
    }

    struct MobileAccessRequest: Codable, Sendable {
        let supabaseToken: String
    }

    struct WorkspaceActiveSnapshotParams: Sendable {
        let limit: Int?
    }

    struct WorkspaceArchivedPageParams: Sendable {
        let limit: Int?
        let cursor: WorkspaceIndexCursor?
    }

    private struct EmptyResponse: Decodable {}
    private struct EmptyPayload: Encodable {}

    private let baseURL: URL
    private let session: URLSession
    private let tokenStore: KeychainTokenStore
    private let secureContext: SecureConnectionContext?
    private let secureSequenceStore = SecureSequenceStore.shared
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private var tokenCache: String?

    init(baseURL: URL, tokenStore: KeychainTokenStore, secureContext: SecureConnectionContext? = nil, session: URLSession = .shared) {
        self.baseURL = baseURL
        self.session = session
        self.tokenStore = tokenStore
        self.secureContext = secureContext
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        self.encoder = encoder
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        self.decoder = decoder
    }

    func daemonBaseURL() -> URL {
        baseURL
    }

    func secureConnectionContext() -> SecureConnectionContext? {
        secureContext
    }

    func authToken() async -> String? {
        if let tokenCache {
            return tokenCache
        }
        if let token = try? await tokenStore.loadToken() {
            tokenCache = token
            return token
        }
        return nil
    }

    func loadToken() async throws -> String? {
        let token = try await tokenStore.loadToken()
        tokenCache = token
        return token
    }

    func setToken(_ token: String) async throws {
        try await tokenStore.saveToken(token)
        tokenCache = token
    }

    func clearToken() async throws {
        try await tokenStore.deleteToken()
        tokenCache = nil
    }

    func listWorkspaces() async throws -> [WorkspaceSummary] {
        let items: [Workspace] = try await request("/api/workspaces")
        return items.map(WorkspaceSummary.init)
    }

    func createWorkspace(rootPath: String, name: String?) async throws -> WorkspaceSummary {
        struct Payload: Encodable {
            let rootPath: String
            let name: String?
        }
        let workspace: Workspace = try await request("/api/workspaces", method: .post, body: Payload(rootPath: rootPath, name: name))
        return WorkspaceSummary(workspace: workspace)
    }

    func listTasks(workspaceId: String) async throws -> [TaskSummary] {
        let tasks = try await listWorkspaceTasks(workspaceId: workspaceId)
        return tasks.map(TaskSummary.init)
    }

    func listSessions(workspaceId: String, taskId: String) async throws -> [SessionSummary] {
        let sessions = try await listTaskSessions(taskId: taskId)
        return sessions.map(SessionSummary.init)
    }

    func listWorkspaceTasks(workspaceId: String) async throws -> [Task] {
        try await request("/api/workspaces/\(workspaceId)/tasks")
    }

    func listTaskSessions(taskId: String) async throws -> [Session] {
        try await request("/api/tasks/\(taskId)/sessions")
    }

    func listWorktrees(workspaceId: String) async throws -> [WorktreeSummary] {
        try await request("/api/workspaces/\(workspaceId)/worktrees")
    }

    func deleteWorkspace(workspaceId: String) async throws {
        let request = try buildRequest(path: "/api/workspaces/\(workspaceId)", method: .delete)
        try await performVoid(request)
    }

    func listMessages(sessionId: String) async throws -> [MessageSummary] {
        let snapshot = try await getSessionSnapshot(sessionId: sessionId, limit: 200, includeEvents: false)
        return snapshot.head.messages.map(MessageSummary.init)
    }

    func postMessage(sessionId: String, content: String, delivery: MessageDelivery?, attachments: [MessageAttachment]?) async throws -> MessageSummary {
        struct Payload: Encodable {
            let content: String
            let delivery: MessageDelivery?
            let attachments: [MessageAttachment]
        }
        let payload = Payload(content: content, delivery: delivery, attachments: attachments ?? [])
        let message: Message = try await request("/api/sessions/\(sessionId)/messages", method: .post, body: payload)
        return MessageSummary(message: message)
    }

    func listQueue(sessionId: String) async throws -> [MessageSummary] {
        let items: [Message] = try await request("/api/sessions/\(sessionId)/queue")
        return items.map(MessageSummary.init)
    }

    func listSessionArtifacts(sessionId: String) async throws -> [Artifact] {
        try await request("/api/sessions/\(sessionId)/artifacts")
    }

    func listWebSessions() async throws -> [WebSessionInfo] {
        try await request("/api/sessions/web")
    }

    func listWorkspaceTerminals(workspaceId: String) async throws -> [TerminalSession] {
        try await request("/api/workspaces/\(workspaceId)/terminals")
    }

    func createWorkspaceTerminal(
        workspaceId: String,
        taskId: String?,
        sessionId: String?,
        worktreeId: String?,
        cwd: String?,
        shell: String?
    ) async throws -> TerminalSession {
        struct Payload: Encodable {
            let taskId: String?
            let sessionId: String?
            let worktreeId: String?
            let cwd: String?
            let shell: String?
        }
        let payload = Payload(
            taskId: taskId,
            sessionId: sessionId,
            worktreeId: worktreeId,
            cwd: cwd,
            shell: shell
        )
        return try await request("/api/workspaces/\(workspaceId)/terminals", method: .post, body: payload)
    }

    func deleteMessage(messageId: String) async throws {
        let request = try buildRequest(path: "/api/messages/\(messageId)", method: .delete, body: EmptyPayload())
        try await performVoid(request)
    }

    func interruptSession(sessionId: String) async throws {
        let request = try buildRequest(path: "/api/sessions/\(sessionId)/interrupt", method: .post, body: EmptyPayload())
        try await performVoid(request)
    }

    func setSessionModel(sessionId: String, modelId: String) async throws -> Session {
        struct Payload: Encodable {
            let modelId: String
        }
        let payload = Payload(modelId: modelId)
        return try await request("/api/sessions/\(sessionId)/model", method: .post, body: payload)
    }

    func setSessionMode(sessionId: String, modeId: String) async throws {
        struct Payload: Encodable {
            let modeId: String
        }
        let payload = Payload(modeId: modeId)
        let request = try buildRequest(path: "/api/sessions/\(sessionId)/mode", method: .post, body: payload)
        try await performVoid(request)
    }

    func submitAskUserQuestion(
        sessionId: String,
        toolCallId: String,
        outcome: String,
        answers: [String: String]
    ) async throws {
        struct Payload: Encodable {
            let toolCallId: String
            let outcome: String
            let answers: [String: String]
        }
        let payload = Payload(toolCallId: toolCallId, outcome: outcome, answers: answers)
        let request = try buildRequest(path: "/api/sessions/\(sessionId)/ask_user_question", method: .post, body: payload)
        try await performVoid(request)
    }

    func fetchSessionGitStatus(sessionId: String) async throws -> SessionGitStatusResponse {
        try await request("/api/sessions/\(sessionId)/git/status")
    }

    func fetchSessionGitDiffSummary(sessionId: String) async throws -> SessionGitDiffSummaryResponse {
        try await request("/api/sessions/\(sessionId)/diff/summary")
    }

    func fetchSessionGitDiff(sessionId: String) async throws -> SessionDiffResponse {
        try await request("/api/sessions/\(sessionId)/diff")
    }

    func fetchSessionDiff(sessionId: String) async throws -> SessionDiffResponse {
        try await request("/api/sessions/\(sessionId)/diff")
    }

    func listProviders() async throws -> [ProviderStatus] {
        try await request("/api/providers")
    }

    func installProvider(providerId: String) async throws -> InstallStartResponse {
        try await request("/api/providers/\(providerId)/install", method: .post)
    }

    func installAllProviders() async throws -> [InstallStartResponse] {
        try await request("/api/providers/install_all", method: .post)
    }

    func getInstall(installId: String) async throws -> InstallInfo {
        try await request("/api/providers/install/\(installId)")
    }

    func getSettings() async throws -> PublicSettings {
        try await request("/api/settings")
    }

    func updateSettings(_ update: SettingsUpdate) async throws -> PublicSettings {
        try await request("/api/settings", method: .post, body: update)
    }

    func getDiagnostics() async throws -> Diagnostics {
        try await request("/api/diagnostics")
    }

    func getMobileAccessStatus() async throws -> MobileAccessStatus {
        try await request("/api/mobile/access/status")
    }

    func enableMobileAccess(supabaseToken: String) async throws -> EnableMobileAccessResponse {
        let payload = MobileAccessRequest(supabaseToken: supabaseToken)
        return try await request("/api/mobile/access/enable", method: .post, body: payload)
    }

    func disableMobileAccess(supabaseToken: String) async throws {
        let payload = MobileAccessRequest(supabaseToken: supabaseToken)
        let request = try buildRequest(path: "/api/mobile/access/disable", method: .post, body: payload)
        try await performVoid(request)
    }

    func registerMobileDevice(_ payload: RegisterMobileDeviceRequest) async throws -> MobileDeviceRegistration {
        try await request("/api/mobile/register", method: .post, body: payload)
    }

    func pairMobileDevice(baseURL: URL, payload: PairMobileDeviceRequest) async throws -> JSONValue {
        let request = try buildRequest(baseURL: baseURL, path: "/api/mobile/pair", method: .post, body: payload, token: nil)
        return try await perform(request)
    }

    func createTask(workspaceId: String, title: String, description: String?) async throws -> Task {
        struct Payload: Encodable {
            let title: String
            let description: String?
        }
        let payload = Payload(
            title: title,
            description: description
        )
        return try await request("/api/workspaces/\(workspaceId)/tasks", method: .post, body: payload)
    }

    func updateTaskTitle(taskId: String, title: String) async throws -> Task {
        struct Payload: Encodable { let title: String }
        return try await request("/api/tasks/\(taskId)/title", method: .post, body: Payload(title: title))
    }

    func deleteTask(taskId: String) async throws {
        let request = try buildRequest(path: "/api/tasks/\(taskId)", method: .delete)
        try await performVoid(request)
    }

    func archiveTask(taskId: String) async throws -> Task {
        try await request("/api/tasks/\(taskId)/archive", method: .post)
    }

    func unarchiveTask(taskId: String) async throws -> Task {
        try await request("/api/tasks/\(taskId)/unarchive", method: .post)
    }

    func markTaskRead(taskId: String) async throws -> Task {
        try await request("/api/tasks/\(taskId)/mark_read", method: .post)
    }

    func markTaskUnread(taskId: String) async throws -> Task {
        try await request("/api/tasks/\(taskId)/mark_unread", method: .post)
    }

    func createSession(taskId: String, providerId: String, modelId: String, envTarget: String? = nil) async throws -> Session {
        struct Payload: Encodable {
            let providerId: String
            let modelId: String
            let envTarget: String?
        }
        let payload = Payload(providerId: providerId, modelId: modelId, envTarget: envTarget)
        return try await request("/api/tasks/\(taskId)/sessions", method: .post, body: payload)
    }

    func getSessionSnapshot(sessionId: String, limit: Int?, includeEvents: Bool?) async throws -> SessionSnapshot {
        var queryItems: [URLQueryItem] = []
        if let limit = limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        if let includeEvents = includeEvents {
            queryItems.append(URLQueryItem(name: "include_events", value: includeEvents ? "1" : "0"))
        }
        return try await request("/api/sessions/\(sessionId)/snapshot", queryItems: queryItems)
    }

    func getSessionState(sessionId: String) async throws -> SessionState {
        try await request("/api/sessions/\(sessionId)/state")
    }

    func getSessionHistory(sessionId: String, beforeSeq: Int?, limit: Int?) async throws -> SessionHistoryPage {
        var queryItems: [URLQueryItem] = []
        if let beforeSeq = beforeSeq {
            queryItems.append(URLQueryItem(name: "before_seq", value: String(beforeSeq)))
        }
        if let limit = limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        return try await request("/api/sessions/\(sessionId)/history", queryItems: queryItems)
    }

    func listTurnTools(sessionId: String, turnId: String) async throws -> [SessionTurnTool] {
        try await request("/api/sessions/\(sessionId)/turns/\(turnId)/tools")
    }

    func getProviderOptions(workspaceId: String, providerId: String) async throws -> ProviderOptions {
        try await request("/api/workspaces/\(workspaceId)/providers/\(providerId)/options")
    }

    func authenticateProviderForWorkspace(workspaceId: String, providerId: String, methodId: String? = nil) async throws -> JSONValue {
        struct Payload: Encodable {
            let methodId: String?
        }
        let payload = Payload(methodId: methodId)
        return try await request(
            "/api/workspaces/\(workspaceId)/providers/\(providerId)/authenticate",
            method: .post,
            body: payload
        )
    }

    func verifyProviderForWorkspace(workspaceId: String, providerId: String) async throws -> JSONValue {
        try await request("/api/workspaces/\(workspaceId)/providers/\(providerId)/verify", method: .post)
    }

    func getWorkspaceActiveSnapshot(workspaceId: String, params: WorkspaceActiveSnapshotParams?) async throws -> WorkspaceActiveSnapshot {
        var queryItems: [URLQueryItem] = []
        if let limit = params?.limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        return try await request("/api/workspaces/\(workspaceId)/active_snapshot", queryItems: queryItems)
    }

    func getWorkspaceActiveHeads(workspaceId: String) async throws -> WorkspaceActiveHeadBatch {
        try await request("/api/workspaces/\(workspaceId)/active_heads")
    }

    func listWorkspaceArchivedTaskSummaries(workspaceId: String, params: WorkspaceArchivedPageParams?) async throws -> WorkspaceArchivedPage {
        var queryItems: [URLQueryItem] = []
        if let limit = params?.limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        if let cursor = params?.cursor {
            let sortAt = cursor.sortAt.trimmingCharacters(in: .whitespacesAndNewlines)
            let taskId = cursor.taskId.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if !sortAt.isEmpty && !taskId.isEmpty {
                queryItems.append(URLQueryItem(name: "cursor_sort_at", value: sortAt))
                queryItems.append(URLQueryItem(name: "cursor_task_id", value: taskId))
            }
        }
        return try await request("/api/workspaces/\(workspaceId)/archived_task_summaries", queryItems: queryItems)
    }

    private func requireToken() throws -> String {
        if let tokenCache {
            return tokenCache
        }
        throw DaemonAPIError.missingToken
    }

    private func request<T: Decodable>(_ path: String, method: HTTPMethod = .get, queryItems: [URLQueryItem] = [], body: Encodable? = nil) async throws -> T {
        if let secureContext {
            return try await secureRequest(path, method: method, queryItems: queryItems, body: body, context: secureContext)
        }
        let request = try buildRequest(path: path, method: method, queryItems: queryItems, body: body)
        return try await perform(request)
    }

    func fetchAssetData(path: String, queryItems: [URLQueryItem] = []) async throws -> Data {
        if let secureContext {
            return try await secureRequestData(path, method: .get, queryItems: queryItems, body: nil, context: secureContext)
        }
        let request = try buildRequest(path: path, method: .get, queryItems: queryItems, body: nil)
        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw DaemonAPIError.invalidResponse
        }
        guard (200...299).contains(httpResponse.statusCode) else {
            let message = String(data: data, encoding: .utf8) ?? "\(httpResponse.statusCode) \(HTTPURLResponse.localizedString(forStatusCode: httpResponse.statusCode))"
            throw DaemonAPIError.requestFailed(statusCode: httpResponse.statusCode, message: message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return data
    }

    private func secureRequest<T: Decodable>(
        _ path: String,
        method: HTTPMethod,
        queryItems: [URLQueryItem],
        body: Encodable?,
        context: SecureConnectionContext
    ) async throws -> T {
        let normalizedPath = path.hasPrefix("/") ? path : "/\(path)"
        var components = URLComponents()
        components.path = normalizedPath
        if !queryItems.isEmpty {
            components.queryItems = queryItems
        }
        let query = components.query?.isEmpty == false ? components.query : nil
        let bodyData: Data?
        if let body {
            bodyData = try encoder.encode(AnyEncodable(body))
        } else {
            bodyData = nil
        }
        let headers = bodyData == nil ? [] : [["content-type", "application/json"]]
        let payload = SecureRequestPayload(
            method: method.rawValue,
            path: normalizedPath,
            query: query,
            headers: headers,
            bodyB64: bodyData?.base64EncodedString() ?? ""
        )
        let seq = await secureSequenceStore.next(for: context.deviceId)
        let plaintext = try encoder.encode(payload)
        let envelope = try MobileE2EE.encryptPayload(deviceId: context.deviceId, seq: seq, key: context.key, plaintext: plaintext)
        let request = try buildRequest(baseURL: baseURL, path: "/api/mobile/secure", method: .post, body: envelope, token: nil)
        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw DaemonAPIError.invalidResponse
        }
        guard (200...299).contains(httpResponse.statusCode) else {
            let message = String(data: data, encoding: .utf8) ?? "\(httpResponse.statusCode) \(HTTPURLResponse.localizedString(forStatusCode: httpResponse.statusCode))"
            throw DaemonAPIError.requestFailed(statusCode: httpResponse.statusCode, message: message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        if data.isEmpty {
            if T.self == EmptyResponse.self {
                return EmptyResponse() as! T
            }
            throw DaemonAPIError.invalidResponse
        }
        let responseEnvelope = try decoder.decode(SecureEnvelope.self, from: data)
        guard responseEnvelope.deviceId == context.deviceId else {
            throw DaemonAPIError.invalidResponse
        }
        let decrypted = try MobileE2EE.decryptEnvelope(responseEnvelope, key: context.key)
        let responsePayload = try decoder.decode(SecureResponsePayload.self, from: decrypted)
        guard (200...299).contains(responsePayload.status) else {
            let bodyData = try? MobileE2EE.decodeBase64(responsePayload.bodyB64)
            let message = bodyData.flatMap { String(data: $0, encoding: .utf8) } ?? "Request failed (\(responsePayload.status))"
            throw DaemonAPIError.requestFailed(statusCode: responsePayload.status, message: message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        if responsePayload.bodyB64.isEmpty {
            if T.self == EmptyResponse.self {
                return EmptyResponse() as! T
            }
            throw DaemonAPIError.invalidResponse
        }
        let payloadBody = try MobileE2EE.decodeBase64(responsePayload.bodyB64)
        if payloadBody.isEmpty {
            if T.self == EmptyResponse.self {
                return EmptyResponse() as! T
            }
            throw DaemonAPIError.invalidResponse
        }
        return try decoder.decode(T.self, from: payloadBody)
    }

    private func secureRequestData(
        _ path: String,
        method: HTTPMethod,
        queryItems: [URLQueryItem],
        body: Encodable?,
        context: SecureConnectionContext
    ) async throws -> Data {
        let normalizedPath = path.hasPrefix("/") ? path : "/\(path)"
        var components = URLComponents()
        components.path = normalizedPath
        if !queryItems.isEmpty {
            components.queryItems = queryItems
        }
        let query = components.query?.isEmpty == false ? components.query : nil
        let bodyData: Data?
        if let body {
            bodyData = try encoder.encode(AnyEncodable(body))
        } else {
            bodyData = nil
        }
        let headers = bodyData == nil ? [] : [["content-type", "application/json"]]
        let payload = SecureRequestPayload(
            method: method.rawValue,
            path: normalizedPath,
            query: query,
            headers: headers,
            bodyB64: bodyData?.base64EncodedString() ?? ""
        )
        let seq = await secureSequenceStore.next(for: context.deviceId)
        let plaintext = try encoder.encode(payload)
        let envelope = try MobileE2EE.encryptPayload(deviceId: context.deviceId, seq: seq, key: context.key, plaintext: plaintext)
        let request = try buildRequest(baseURL: baseURL, path: "/api/mobile/secure", method: .post, body: envelope, token: nil)
        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw DaemonAPIError.invalidResponse
        }
        guard (200...299).contains(httpResponse.statusCode) else {
            let message = String(data: data, encoding: .utf8) ?? "\(httpResponse.statusCode) \(HTTPURLResponse.localizedString(forStatusCode: httpResponse.statusCode))"
            throw DaemonAPIError.requestFailed(statusCode: httpResponse.statusCode, message: message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        if data.isEmpty {
            throw DaemonAPIError.invalidResponse
        }
        let responseEnvelope = try decoder.decode(SecureEnvelope.self, from: data)
        guard responseEnvelope.deviceId == context.deviceId else {
            throw DaemonAPIError.invalidResponse
        }
        let decrypted = try MobileE2EE.decryptEnvelope(responseEnvelope, key: context.key)
        let responsePayload = try decoder.decode(SecureResponsePayload.self, from: decrypted)
        guard (200...299).contains(responsePayload.status) else {
            let bodyData = try? MobileE2EE.decodeBase64(responsePayload.bodyB64)
            let message = bodyData.flatMap { String(data: $0, encoding: .utf8) } ?? "Request failed (\(responsePayload.status))"
            throw DaemonAPIError.requestFailed(statusCode: responsePayload.status, message: message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        if responsePayload.bodyB64.isEmpty {
            throw DaemonAPIError.invalidResponse
        }
        return try MobileE2EE.decodeBase64(responsePayload.bodyB64)
    }

    private func buildRequest(path: String, method: HTTPMethod, queryItems: [URLQueryItem] = [], body: Encodable? = nil) throws -> URLRequest {
        let token = try requireToken()
        return try buildRequest(baseURL: baseURL, path: path, method: method, queryItems: queryItems, body: body, token: token)
    }

    private func buildRequest(baseURL: URL, path: String, method: HTTPMethod, queryItems: [URLQueryItem] = [], body: Encodable? = nil, token: String?) throws -> URLRequest {
        guard let base = resolveURL(baseURL: baseURL, path: path),
              var components = URLComponents(url: base, resolvingAgainstBaseURL: true) else {
            throw DaemonAPIError.invalidURL
        }
        if !queryItems.isEmpty {
            components.queryItems = queryItems
        }
        guard let url = components.url else {
            throw DaemonAPIError.invalidURL
        }
        var request = URLRequest(url: url)
        request.httpMethod = method.rawValue
        if let token {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try encoder.encode(AnyEncodable(body))
        }
        return request
    }

    private func resolveURL(baseURL: URL, path: String) -> URL? {
        let trimmedPath = path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if trimmedPath.isEmpty {
            return baseURL
        }
        if baseURL.path.isEmpty || baseURL.path == "/" {
            let normalizedPath = "/\(trimmedPath)"
            return URL(string: normalizedPath, relativeTo: baseURL)
        }
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: true) else {
            return nil
        }
        let basePath = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.path = basePath.isEmpty ? "/\(trimmedPath)" : "/\(basePath)/\(trimmedPath)"
        components.query = nil
        components.fragment = nil
        return components.url
    }

    private func perform<T: Decodable>(_ request: URLRequest) async throws -> T {
        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw DaemonAPIError.invalidResponse
        }
        guard (200...299).contains(httpResponse.statusCode) else {
            let message = String(data: data, encoding: .utf8) ?? "\(httpResponse.statusCode) \(HTTPURLResponse.localizedString(forStatusCode: httpResponse.statusCode))"
            throw DaemonAPIError.requestFailed(statusCode: httpResponse.statusCode, message: message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        if data.isEmpty {
            if T.self == EmptyResponse.self {
                return EmptyResponse() as! T
            }
            throw DaemonAPIError.invalidResponse
        }
        return try decoder.decode(T.self, from: data)
    }

    private func performVoid(_ request: URLRequest) async throws {
        let (_, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw DaemonAPIError.invalidResponse
        }
        guard (200...299).contains(httpResponse.statusCode) else {
            throw DaemonAPIError.requestFailed(statusCode: httpResponse.statusCode, message: "\(httpResponse.statusCode)")
        }
    }
}

private struct AnyEncodable: Encodable {
    private let encodeClosure: (Encoder) throws -> Void

    init(_ wrapped: Encodable) {
        self.encodeClosure = wrapped.encode
    }

    func encode(to encoder: Encoder) throws {
        try encodeClosure(encoder)
    }
}
