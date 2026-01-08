import Foundation

enum DaemonAPIError: Error {
    case invalidURL
    case missingToken
    case invalidResponse
    case requestFailed(statusCode: Int, message: String)
}

enum HTTPMethod: String {
    case get = "GET"
    case post = "POST"
    case delete = "DELETE"
}

actor DaemonAPIClient {
    struct TrackDiffResponse: Codable, Sendable {
        let diff: String
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

    struct WorkspaceCatchupParams: Sendable {
        let limit: Int?
        let includeArchived: Bool?
        let archivedOnly: Bool?
        let activeCursor: WorkspaceCatchupCursor?
        let archivedCursor: WorkspaceCatchupCursor?
    }

    private struct EmptyResponse: Decodable {}

    private let baseURL: URL
    private let session: URLSession
    private let tokenStore: KeychainTokenStore
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder
    private var tokenCache: String?

    init(baseURL: URL, tokenStore: KeychainTokenStore, session: URLSession = .shared) {
        self.baseURL = baseURL
        self.session = session
        self.tokenStore = tokenStore
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        self.encoder = encoder
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        self.decoder = decoder
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
        let items: [Task] = try await request("/api/workspaces/\(workspaceId)/tasks")
        return items.map(TaskSummary.init)
    }

    func listTracks(taskId: String) async throws -> [TrackSummary] {
        let items: [Track] = try await request("/api/tasks/\(taskId)/tracks")
        return items.map(TrackSummary.init)
    }

    func listSessions(forTrack trackId: String) async throws -> [SessionSummary] {
        let items: [Session] = try await request("/api/tracks/\(trackId)/sessions")
        return items.map(SessionSummary.init)
    }

    func listMessages(sessionId: String) async throws -> [MessageSummary] {
        let items: [Message] = try await request("/api/sessions/\(sessionId)/messages")
        return items.map(MessageSummary.init)
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

    func fetchTrackDiff(trackId: String) async throws -> TrackDiffResponse {
        try await request("/api/tracks/\(trackId)/diff")
    }

    func listProviders() async throws -> [ProviderStatus] {
        try await request("/api/providers")
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

    func getWorkspaceCatchup(workspaceId: String, params: WorkspaceCatchupParams?) async throws -> WorkspaceCatchupSnapshot {
        var queryItems: [URLQueryItem] = []
        if let limit = params?.limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        if params?.includeArchived == true {
            queryItems.append(URLQueryItem(name: "include_archived", value: "1"))
        }
        if params?.archivedOnly == true {
            queryItems.append(URLQueryItem(name: "archived_only", value: "1"))
        }
        if let activeCursor = params?.activeCursor {
            queryItems.append(URLQueryItem(name: "active_cursor_sort_at", value: activeCursor.sortAt))
            queryItems.append(URLQueryItem(name: "active_cursor_task_id", value: activeCursor.taskId.stringValue))
        }
        if let archivedCursor = params?.archivedCursor {
            queryItems.append(URLQueryItem(name: "archived_cursor_sort_at", value: archivedCursor.sortAt))
            queryItems.append(URLQueryItem(name: "archived_cursor_task_id", value: archivedCursor.taskId.stringValue))
        }
        return try await request("/api/workspaces/\(workspaceId)/catchup", queryItems: queryItems)
    }

    func createTask(workspaceId: String, title: String, description: String?, createDefaultTrack: Bool?, defaultTrackLabel: String?) async throws -> Task {
        struct Payload: Encodable {
            let title: String
            let description: String?
            let createDefaultTrack: Bool?
            let defaultTrackLabel: String?
        }
        let payload = Payload(
            title: title,
            description: description,
            createDefaultTrack: createDefaultTrack,
            defaultTrackLabel: defaultTrackLabel
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

    func createTrack(taskId: String, label: String?, envTarget: String?) async throws -> Track {
        struct Payload: Encodable {
            let label: String?
            let envTarget: String?
        }
        let payload = Payload(label: label, envTarget: envTarget)
        return try await request("/api/tasks/\(taskId)/tracks", method: .post, body: payload)
    }

    func createSession(trackId: String, providerId: String, modelId: String) async throws -> Session {
        struct Payload: Encodable {
            let providerId: String
            let modelId: String
        }
        let payload = Payload(providerId: providerId, modelId: modelId)
        return try await request("/api/tracks/\(trackId)/sessions", method: .post, body: payload)
    }

    func getSessionHead(sessionId: String, limit: Int?, includeEvents: Bool?) async throws -> SessionHead {
        var queryItems: [URLQueryItem] = []
        if let limit = limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        if let includeEvents = includeEvents {
            queryItems.append(URLQueryItem(name: "include_events", value: includeEvents ? "1" : "0"))
        }
        return try await request("/api/sessions/\(sessionId)/head", queryItems: queryItems)
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

    private func requireToken() throws -> String {
        if let tokenCache {
            return tokenCache
        }
        throw DaemonAPIError.missingToken
    }

    private func request<T: Decodable>(_ path: String, method: HTTPMethod = .get, queryItems: [URLQueryItem] = [], body: Encodable? = nil) async throws -> T {
        let request = try buildRequest(path: path, method: method, queryItems: queryItems, body: body)
        return try await perform(request)
    }

    private func buildRequest(path: String, method: HTTPMethod, queryItems: [URLQueryItem] = [], body: Encodable? = nil) throws -> URLRequest {
        let token = try requireToken()
        return try buildRequest(baseURL: baseURL, path: path, method: method, queryItems: queryItems, body: body, token: token)
    }

    private func buildRequest(baseURL: URL, path: String, method: HTTPMethod, queryItems: [URLQueryItem] = [], body: Encodable? = nil, token: String?) throws -> URLRequest {
        let normalizedPath = path.hasPrefix("/") ? path : "/\(path)"
        guard let base = URL(string: normalizedPath, relativeTo: baseURL) else {
            throw DaemonAPIError.invalidURL
        }
        guard var components = URLComponents(url: base, resolvingAgainstBaseURL: true) else {
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
