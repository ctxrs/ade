import Foundation
import _Concurrency

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
    struct SessionDiffResponse: Codable, Sendable {
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
        let snapshot = try await getWorkspaceCatchupSnapshot(workspaceId: workspaceId)
        return snapshot.active.tasks.map { TaskSummary(task: $0.task) }
    }

    func listSessions(workspaceId: String, taskId: String) async throws -> [SessionSummary] {
        let snapshot = try await getWorkspaceCatchupSnapshot(workspaceId: workspaceId)
        if let task = snapshot.active.tasks.first(where: { $0.task.id.stringValue == taskId }) {
            return task.sessions.map { SessionSummary(session: $0.session) }
        }
        if let archived = snapshot.archived,
           let task = archived.tasks.first(where: { $0.task.id.stringValue == taskId }) {
            return task.sessions.map { SessionSummary(session: $0.session) }
        }
        return []
    }

    func listWorktrees(workspaceId: String) async throws -> [WorktreeSummary] {
        try await request("/api/workspaces/\(workspaceId)/worktrees")
    }

    func deleteWorkspace(workspaceId: String) async throws {
        let request = try buildRequest(path: "/api/workspaces/\(workspaceId)", method: .delete)
        try await performVoid(request)
    }

    func listMessages(sessionId: String) async throws -> [MessageSummary] {
        let head = try await getSessionHead(sessionId: sessionId, limit: 200, includeEvents: false)
        return head.messages.map(MessageSummary.init)
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

    func createSession(taskId: String, providerId: String, modelId: String) async throws -> Session {
        struct Payload: Encodable {
            let providerId: String
            let modelId: String
        }
        let payload = Payload(providerId: providerId, modelId: modelId)
        return try await request("/api/tasks/\(taskId)/sessions", method: .post, body: payload)
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

    func getWorkspaceCatchupSnapshot(workspaceId: String, includeArchived: Bool) async throws -> WorkspaceCatchupSnapshot {
        let params = WorkspaceCatchupParams(
            limit: nil,
            includeArchived: includeArchived,
            archivedOnly: false,
            activeCursor: nil,
            archivedCursor: nil
        )
        return try await getWorkspaceCatchup(workspaceId: workspaceId, params: params)
    }

    private func requireToken() throws -> String {
        if let tokenCache {
            return tokenCache
        }
        throw DaemonAPIError.missingToken
    }

    private func getWorkspaceCatchupSnapshot(workspaceId: String) async throws -> WorkspaceCatchupSnapshot {
        try await getWorkspaceCatchupSnapshot(workspaceId: workspaceId, includeArchived: false)
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
