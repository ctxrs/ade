import Foundation

enum DaemonStreamError: Error {
    case invalidURL
}

final class DaemonStreamClient {
    private let session: URLSession

    init(session: URLSession = .shared) {
        self.session = session
    }

    func connectWorkspaceStream(baseURL: URL, workspaceId: String, token: String?) throws -> URLSessionWebSocketTask {
        guard let url = webSocketURL(baseURL: baseURL, path: "/api/workspaces/\(workspaceId)/stream", token: token) else {
            throw DaemonStreamError.invalidURL
        }
        let task = session.webSocketTask(with: url)
        task.resume()
        return task
    }

    func connectSecureWorkspaceStream(baseURL: URL, workspaceId: String, deviceId: String) throws -> URLSessionWebSocketTask {
        guard let url = webSocketURL(
            baseURL: baseURL,
            path: "/api/mobile/secure/workspaces/\(workspaceId)/stream",
            token: nil,
            queryItems: [URLQueryItem(name: "device_id", value: deviceId)]
        ) else {
            throw DaemonStreamError.invalidURL
        }
        let task = session.webSocketTask(with: url)
        task.resume()
        return task
    }

    func send(_ message: URLSessionWebSocketTask.Message, via task: URLSessionWebSocketTask) async throws {
        try await task.send(message)
    }

    func receive(from task: URLSessionWebSocketTask) async throws -> URLSessionWebSocketTask.Message {
        try await task.receive()
    }

    func disconnect(_ task: URLSessionWebSocketTask) {
        task.cancel(with: .normalClosure, reason: nil)
    }

    private func webSocketURL(baseURL: URL, path: String, token: String?, queryItems: [URLQueryItem] = []) -> URL? {
        let normalizedPath = path.hasPrefix("/") ? path : "/\(path)"
        guard let url = URL(string: normalizedPath, relativeTo: baseURL),
              var components = URLComponents(url: url, resolvingAgainstBaseURL: true) else {
            return nil
        }
        if components.scheme == "https" {
            components.scheme = "wss"
        } else if components.scheme == "http" {
            components.scheme = "ws"
        }
        var mergedItems = components.queryItems ?? []
        if let token {
            mergedItems.append(URLQueryItem(name: "token", value: token))
        }
        mergedItems.append(contentsOf: queryItems)
        if !mergedItems.isEmpty {
            components.queryItems = mergedItems
        }
        return components.url
    }
}
