import SwiftUI
import WebKit
import _Concurrency

enum WorkbenchPanel: String, Identifiable {
    case diff
    case sessions
    case terminal

    var id: String { rawValue }
}

private struct WorkbenchPanelHeader: View {
    let title: String
    let subtitle: String?
    let icon: LucideIconName
    let onRefresh: (() -> Void)?

    var body: some View {
        HStack(spacing: 12) {
            LucideIcon(name: icon, size: 18)
                .foregroundColor(.ctxAccent)

            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.headline)
                    .foregroundColor(.ctxTextPrimary)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle)
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                }
            }

            Spacer()

            if let onRefresh {
                Button("Refresh", action: onRefresh)
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                    .padding(.vertical, 6)
                    .padding(.horizontal, 10)
                    .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .stroke(Color.ctxLine, lineWidth: 1)
                    )
            }
        }
    }
}

private struct WorkbenchPanelScaffold<Content: View>: View {
    @Environment(\.dismiss) private var dismiss
    let title: String
    let subtitle: String?
    let icon: LucideIconName
    let onRefresh: (() -> Void)?
    let content: Content

    init(
        title: String,
        subtitle: String? = nil,
        icon: LucideIconName,
        onRefresh: (() -> Void)? = nil,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.subtitle = subtitle
        self.icon = icon
        self.onRefresh = onRefresh
        self.content = content()
    }

    var body: some View {
        NavigationStack {
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 16) {
                    WorkbenchPanelHeader(title: title, subtitle: subtitle, icon: icon, onRefresh: onRefresh)
                    content
                }
                .padding(16)
            }
            .background(CtxBackgroundView())
            .navigationTitle("")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

struct WorkbenchDiffPanelView: View {
    let session: SessionSummary?

    var body: some View {
        WorkbenchPanelScaffold(
            title: "Diff",
            icon: .gitBranch
        ) {
            GlassPanel {
                Text(session == nil ? "Select a task session to view a diff." : "Diff unavailable in trackless mode.")
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
        }
    }
}

struct WorkbenchSessionsPanelView: View {
    @EnvironmentObject private var connection: ConnectionStore
    let sessionId: String?

    @State private var sessions: [WebSessionInfo] = []
    @State private var isLoading = false
    @State private var errorMessage: String?
    @State private var selectedSessionId: String?
    @State private var daemonBaseURL: URL?
    @State private var authToken: String?

    private var isSecureConnection: Bool {
        connection.secureConfig != nil
    }

    private var selectedSession: WebSessionInfo? {
        if let selectedSessionId, let match = sessions.first(where: { $0.id == selectedSessionId }) {
            return match
        }
        return sessions.first
    }

    private var sessionSubtitle: String? {
        guard let sessionId, !sessionId.isEmpty else { return nil }
        return "Session \(sessionId.prefix(6))"
    }

    var body: some View {
        WorkbenchPanelScaffold(
            title: "Sessions",
            subtitle: sessionSubtitle,
            icon: .monitor,
            onRefresh: { _Concurrency.Task { await loadSessions() } }
        ) {
            if sessionId == nil {
                GlassPanel {
                    Text("Select a task session to view interactive sessions.")
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                }
            } else {
                if isSecureConnection {
                    GlassPanel {
                        Text("Streaming is unavailable over secure pairing right now.")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    }
                }

                if isLoading && sessions.isEmpty {
                    ProgressView()
                        .tint(.ctxAccent)
                } else if let errorMessage {
                    Text(errorMessage)
                        .font(.caption)
                        .foregroundColor(.ctxError)
                } else if sessions.isEmpty {
                    Text("No sessions available for this run.")
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                } else {
                    if sessions.count > 1 {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 8) {
                                ForEach(sessions) { session in
                                    Button(action: { selectedSessionId = session.id }) {
                                        Text(sessionLabel(session))
                                            .font(.caption.weight(.semibold))
                                            .foregroundColor(.ctxTextPrimary)
                                            .padding(.vertical, 6)
                                            .padding(.horizontal, 10)
                                            .background(
                                                Color.ctxSurface.opacity(selectedSession?.id == session.id ? 0.8 : 0.35),
                                                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                                            )
                                            .overlay(
                                                RoundedRectangle(cornerRadius: 10, style: .continuous)
                                                    .stroke(Color.ctxLine, lineWidth: 1)
                                            )
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                    }

                    GlassPanel {
                        if let selectedSession,
                           let streamUrl = streamURL(for: selectedSession) {
                            WebSessionStreamView(url: streamUrl, token: authToken)
                                .frame(minHeight: 320)
                        } else {
                            Text("Stream unavailable.")
                                .font(.caption)
                                .foregroundColor(.ctxTextMuted)
                        }
                    }
                }
            }
        }
        .task(id: sessionId) {
            await loadSessions()
        }
    }

    @MainActor
    private func loadSessions() async {
        guard let sessionId, !sessionId.isEmpty else {
            sessions = []
            errorMessage = nil
            return
        }
        guard let client = connection.apiClient else {
            sessions = []
            errorMessage = "Connect to a daemon first."
            return
        }
        isLoading = true
        errorMessage = nil
        do {
            let items = try await client.listWebSessions()
            let filtered = items.filter { session in
                let status = session.status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
                return session.sessionId == sessionId && status == "running"
            }
            sessions = filtered
            if selectedSessionId == nil || !sessions.contains(where: { $0.id == selectedSessionId }) {
                selectedSessionId = sessions.first?.id
            }
            daemonBaseURL = await client.daemonBaseURL()
            authToken = await client.authToken()
        } catch {
            sessions = []
            errorMessage = "Failed to load sessions."
        }
        isLoading = false
    }

    private func sessionLabel(_ session: WebSessionInfo) -> String {
        if let url = URL(string: session.url) {
            let host = url.host ?? ""
            let path = url.path == "/" ? "" : url.path
            let label = host + path
            if label.isEmpty {
                return String(session.id.prefix(8))
            }
            return label.count > 32 ? "\(label.prefix(28))..." : label
        }
        return String(session.id.prefix(8))
    }

    private func streamURL(for session: WebSessionInfo) -> URL? {
        if let streamUrl = session.streamUrl, let url = URL(string: streamUrl) {
            return appendToken(url: url, token: authToken)
        }
        guard let streamPath = session.streamPath,
              let base = daemonBaseURL else { return nil }
        let url = URL(string: streamPath, relativeTo: base)
        return url.flatMap { appendToken(url: $0, token: authToken) }
    }

    private func appendToken(url: URL, token: String?) -> URL {
        guard let token, !token.isEmpty else { return url }
        guard var components = URLComponents(url: url, resolvingAgainstBaseURL: true) else { return url }
        var items = components.queryItems ?? []
        if !items.contains(where: { $0.name == "token" }) {
            items.append(URLQueryItem(name: "token", value: token))
        }
        components.queryItems = items
        return components.url ?? url
    }
}

private struct WebSessionStreamView: UIViewRepresentable {
    let url: URL
    let token: String?

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.allowsInlineMediaPlayback = true
        let webView = WKWebView(frame: .zero, configuration: config)
        webView.isOpaque = false
        webView.backgroundColor = .clear
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        if context.coordinator.lastURL == url { return }
        context.coordinator.lastURL = url
        var request = URLRequest(url: url)
        if let token, !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        webView.load(request)
    }

    final class Coordinator {
        var lastURL: URL?
    }
}

private struct TerminalWebView: UIViewRepresentable {
    let wsURL: URL

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.allowsInlineMediaPlayback = true
        if #available(iOS 14.0, *) {
            config.defaultWebpagePreferences.allowsContentJavaScript = true
        }
        let webView = WKWebView(frame: .zero, configuration: config)
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.isScrollEnabled = false
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        let html = terminalHTML(wsURL: wsURL)
        if context.coordinator.lastHTML == html { return }
        context.coordinator.lastHTML = html
        webView.loadHTMLString(html, baseURL: nil)
    }

    private func terminalHTML(wsURL: URL) -> String {
        let wsLiteral = jsStringLiteral(wsURL.absoluteString)
        return """
        <!doctype html>
        <html>
          <head>
            <meta name="viewport" content="width=device-width, height=device-height, initial-scale=1, maximum-scale=1, user-scalable=no" />
            <link rel="stylesheet" href="https://unpkg.com/xterm@5.5.0/css/xterm.css" />
            <style>
              html, body {
                height: 100%;
                width: 100%;
                margin: 0;
                background: #252526;
              }
              #terminal {
                height: 100%;
                width: 100%;
                padding: 8px;
                box-sizing: border-box;
              }
            </style>
          </head>
          <body>
            <div id="terminal"></div>
            <script src="https://unpkg.com/xterm@5.5.0/lib/xterm.js"></script>
            <script src="https://unpkg.com/xterm-addon-fit@0.10.0/lib/xterm-addon-fit.js"></script>
            <script>
              const wsUrl = \(wsLiteral);
              const term = new Terminal({
                fontFamily: "SFMono-Regular, Menlo, Monaco, Consolas, \\"Liberation Mono\\", monospace",
                fontSize: 12,
                allowTransparency: true,
                theme: {
                  background: "#252526",
                  foreground: "#d4d4d4",
                  cursor: "#d4d4d4",
                  selectionBackground: "rgba(255, 255, 255, 0.2)"
                },
                scrollback: 2000
              });
              const fitAddon = new FitAddon.FitAddon();
              term.loadAddon(fitAddon);
              term.open(document.getElementById("terminal"));
              const ws = new WebSocket(wsUrl);
              ws.binaryType = "arraybuffer";
              const sendResize = () => {
                if (ws.readyState !== WebSocket.OPEN) return;
                ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
              };
              term.onData((data) => {
                if (ws.readyState === WebSocket.OPEN) {
                  ws.send(data);
                }
              });
              term.onResize(() => {
                sendResize();
              });
              ws.onopen = () => {
                fitAddon.fit();
                sendResize();
                term.focus();
              };
              ws.onmessage = (event) => {
                if (typeof event.data === "string") {
                  return;
                }
                term.write(new Uint8Array(event.data));
              };
              ws.onclose = () => {
                term.write("\\r\\n[connection closed]\\r\\n");
              };
              window.addEventListener("resize", () => {
                fitAddon.fit();
                sendResize();
              });
              setTimeout(() => {
                fitAddon.fit();
                sendResize();
                term.focus();
              }, 0);
            </script>
          </body>
        </html>
        """
    }

    private func jsStringLiteral(_ value: String) -> String {
        let escaped = value
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        return "\"\(escaped)\""
    }

    final class Coordinator {
        var lastHTML: String?
    }
}

struct WorkbenchTerminalPanelView: View {
    @EnvironmentObject private var connection: ConnectionStore
    let workspaceId: String?
    let taskId: String?
    let sessionId: String?
    let worktreeId: String?

    @State private var terminals: [TerminalSession] = []
    @State private var isLoading = false
    @State private var errorMessage: String?
    @State private var scope: TerminalScope = .workspace
    @State private var selectedTerminalId: String?
    @State private var daemonBaseURL: URL?
    @State private var authToken: String?
    @State private var isCreatingTerminal = false

    private var availableScopes: [TerminalScope] {
        taskId == nil ? [.workspace] : [.task, .workspace]
    }

    private var filteredTerminals: [TerminalSession] {
        switch scope {
        case .task:
            guard let taskId else { return [] }
            return terminals.filter { $0.taskId?.stringValue == taskId }
        case .workspace:
            return terminals
        }
    }

    private var isSecureConnection: Bool {
        connection.secureConfig != nil
    }

    private var selectedTerminal: TerminalSession? {
        if let selectedTerminalId, let match = filteredTerminals.first(where: { $0.id.stringValue == selectedTerminalId }) {
            return match
        }
        return filteredTerminals.first
    }

    var body: some View {
        WorkbenchPanelScaffold(
            title: "Terminal",
            subtitle: scope == .task ? "Task terminals" : "Workspace terminals",
            icon: .terminal,
            onRefresh: { _Concurrency.Task { await loadTerminals() } }
        ) {
            if workspaceId == nil {
                GlassPanel {
                    Text("Select a workspace to view terminals.")
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                }
            } else if isSecureConnection {
                GlassPanel {
                    Text("Terminal streaming is unavailable over secure pairing right now.")
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                }
            } else {
                if availableScopes.count > 1 {
                    Picker("Scope", selection: $scope) {
                        ForEach(availableScopes) { scope in
                            Text(scope.title).tag(scope)
                        }
                    }
                    .pickerStyle(.segmented)
                }

                HStack(spacing: 8) {
                    Button(action: { _Concurrency.Task { await createTerminal() } }) {
                        HStack(spacing: 6) {
                            LucideIcon(name: .plus, size: 14)
                            Text("New terminal")
                        }
                    }
                    .buttonStyle(CtxGhostButtonStyle())
                    .disabled(isCreatingTerminal)

                    if isCreatingTerminal {
                        ProgressView()
                            .tint(.ctxTextPrimary)
                    }

                    Spacer()
                }

                if isLoading && terminals.isEmpty {
                    ProgressView()
                        .tint(.ctxAccent)
                } else if let errorMessage {
                    Text(errorMessage)
                        .font(.caption)
                        .foregroundColor(.ctxError)
                } else if filteredTerminals.isEmpty {
                    Text(scope == .task ? "No task terminals yet." : "No workspace terminals yet.")
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                } else {
                    if filteredTerminals.count > 1 {
                        ScrollView(.horizontal, showsIndicators: false) {
                            HStack(spacing: 8) {
                                ForEach(filteredTerminals) { terminal in
                                    Button(action: { selectedTerminalId = terminal.id.stringValue }) {
                                        Text(terminalLabel(terminal))
                                            .font(.caption.weight(.semibold))
                                            .foregroundColor(.ctxTextPrimary)
                                            .padding(.vertical, 6)
                                            .padding(.horizontal, 10)
                                            .background(
                                                Color.ctxSurface.opacity(selectedTerminal?.id == terminal.id ? 0.8 : 0.35),
                                                in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                                            )
                                            .overlay(
                                                RoundedRectangle(cornerRadius: 10, style: .continuous)
                                                    .stroke(Color.ctxLine, lineWidth: 1)
                                            )
                                    }
                                    .buttonStyle(.plain)
                                }
                            }
                        }
                    }

                    GlassPanel {
                        if let selectedTerminal,
                           let streamURL = terminalWebSocketURL(for: selectedTerminal) {
                            VStack(alignment: .leading, spacing: 12) {
                                WorkbenchTerminalRow(terminal: selectedTerminal)
                                TerminalWebView(wsURL: streamURL)
                                    .frame(minHeight: 320)
                            }
                        } else {
                            Text("Terminal unavailable.")
                                .font(.caption)
                                .foregroundColor(.ctxTextMuted)
                        }
                    }
                }
            }
        }
        .onAppear {
            if taskId != nil {
                scope = .task
            }
        }
        .onChange(of: scope) { _ in
            updateSelection()
        }
        .task(id: workspaceId) {
            await loadTerminals()
        }
    }

    @MainActor
    private func loadTerminals() async {
        guard let workspaceId, !workspaceId.isEmpty else {
            terminals = []
            errorMessage = nil
            return
        }
        guard let client = connection.apiClient else {
            terminals = []
            errorMessage = "Connect to a daemon first."
            return
        }
        isLoading = true
        errorMessage = nil
        do {
            let items = try await client.listWorkspaceTerminals(workspaceId: workspaceId)
            terminals = items.sorted { $0.updatedAt > $1.updatedAt }
            daemonBaseURL = await client.daemonBaseURL()
            authToken = await client.authToken()
            updateSelection()
        } catch {
            terminals = []
            errorMessage = "Failed to load terminals."
        }
        isLoading = false
    }

    @MainActor
    private func createTerminal() async {
        guard let workspaceId, !workspaceId.isEmpty else {
            errorMessage = "Select a workspace to create a terminal."
            return
        }
        guard let client = connection.apiClient else {
            errorMessage = "Connect to a daemon first."
            return
        }
        let targetScope: TerminalScope = scope == .task && taskId != nil ? .task : .workspace
        isCreatingTerminal = true
        errorMessage = nil
        do {
            let terminal = try await client.createWorkspaceTerminal(
                workspaceId: workspaceId,
                taskId: targetScope == .task ? taskId : nil,
                sessionId: targetScope == .task ? sessionId : nil,
                worktreeId: targetScope == .task ? worktreeId : nil,
                cwd: nil,
                shell: nil
            )
            terminals.insert(terminal, at: 0)
            selectedTerminalId = terminal.id.stringValue
        } catch {
            errorMessage = "Failed to create terminal."
        }
        isCreatingTerminal = false
    }

    private func updateSelection() {
        if let selectedTerminalId,
           filteredTerminals.contains(where: { $0.id.stringValue == selectedTerminalId }) {
            return
        }
        selectedTerminalId = filteredTerminals.first?.id.stringValue
    }

    private func terminalLabel(_ terminal: TerminalSession) -> String {
        let trimmed = terminal.title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        let shell = terminal.shell.trimmingCharacters(in: .whitespacesAndNewlines)
        return shell.isEmpty ? "Terminal" : shell
    }

    private func terminalWebSocketURL(for terminal: TerminalSession) -> URL? {
        guard let base = daemonBaseURL else { return nil }
        guard var components = URLComponents(url: base, resolvingAgainstBaseURL: true) else { return nil }
        if components.scheme == "http" {
            components.scheme = "ws"
        } else if components.scheme == "https" {
            components.scheme = "wss"
        }
        let basePath = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let streamPath = "api/terminals/\(terminal.id.stringValue)/stream"
        components.path = basePath.isEmpty ? "/\(streamPath)" : "/\(basePath)/\(streamPath)"
        var items = components.queryItems ?? []
        if let token = authToken, !token.isEmpty, !items.contains(where: { $0.name == "token" }) {
            items.append(URLQueryItem(name: "token", value: token))
        }
        components.queryItems = items.isEmpty ? nil : items
        return components.url
    }

    private enum TerminalScope: String, CaseIterable, Identifiable {
        case task
        case workspace

        var id: String { rawValue }

        var title: String {
            switch self {
            case .task:
                return "Task"
            case .workspace:
                return "Workspace"
            }
        }
    }
}

private struct WorkbenchTerminalRow: View {
    let terminal: TerminalSession

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(terminalTitle)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(terminalMeta)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
            Text(terminalStatus)
                .font(.caption.weight(.semibold))
                .foregroundColor(statusColor)
                .padding(.vertical, 4)
                .padding(.horizontal, 8)
                .background(statusColor.opacity(0.15), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
    }

    private var terminalTitle: String {
        let trimmed = terminal.title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty { return trimmed }
        let shell = terminal.shell.trimmingCharacters(in: .whitespacesAndNewlines)
        return shell.isEmpty ? "Terminal" : shell
    }

    private var terminalMeta: String {
        let cwd = terminal.cwd.isEmpty ? "Unknown directory" : terminal.cwd
        return cwd
    }

    private var terminalStatus: String {
        terminal.status.capitalized
    }

    private var statusColor: Color {
        terminal.status.lowercased() == "running" ? .ctxAccent : .ctxTextMuted
    }
}
