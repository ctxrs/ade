import SwiftUI
import WebKit
import _Concurrency

enum WorkbenchPanel: String, Identifiable {
    case artifacts
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
    @EnvironmentObject private var connection: ConnectionStore
    let session: SessionSummary?
    private let preview: DiffPreview?

    @State private var diffText = ""
    @State private var diffFiles: [DiffFile] = []
    @State private var expandedFileIds: Set<String> = []
    @State private var statusSummary = ""
    @State private var diffSummary: DiffSummary?
    @State private var isLoading = false
    @State private var errorMessage: String?

    init(session: SessionSummary?) {
        self.session = session
        self.preview = nil
    }

    private init(session: SessionSummary?, preview: DiffPreview?) {
        self.session = session
        self.preview = preview
    }

    static func uiTestView() -> WorkbenchDiffPanelView {
        WorkbenchDiffPanelView(session: nil, preview: .uiTest)
    }

    private var hasChanges: Bool {
        !diffText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var fileCountLabel: String? {
        guard !isLoading, errorMessage == nil else { return nil }
        let count = diffSummary?.fileCount ?? diffFiles.count
        return "\(count) file" + (count == 1 ? "" : "s")
    }

    var body: some View {
        WorkbenchPanelScaffold(
            title: "Diff",
            subtitle: nil,
            icon: .gitBranch,
            onRefresh: { _Concurrency.Task { await loadDiff() } }
        ) {
            VStack(alignment: .leading, spacing: 16) {
                GlassPanel {
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(alignment: .center, spacing: 8) {
                            Text("Repo Status")
                                .font(.subheadline.weight(.semibold))
                                .foregroundColor(.ctxTextPrimary)

                            Spacer()

                            if let fileCountLabel {
                                GlassPill(text: fileCountLabel, tint: .ctxAccent)
                            }
                        }

                        if isLoading && statusSummary.isEmpty {
                            ProgressView()
                                .tint(.ctxAccent)
                        } else if statusSummary.isEmpty {
                            Text("Status unavailable.")
                                .font(.caption)
                                .foregroundColor(.ctxTextMuted)
                        } else {
                            Text(verbatim: statusSummary)
                                .font(.system(size: 12, weight: .regular, design: .monospaced))
                                .foregroundColor(.ctxTextSecondary)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .textSelection(.enabled)
                        }
                    }
                }

                GlassPanel {
                    if isLoading {
                        ProgressView()
                            .tint(.ctxAccent)
                    } else if let errorMessage {
                        Text(errorMessage)
                            .font(.caption)
                            .foregroundColor(.ctxError)
                    } else if !hasChanges {
                        Text("No pending changes.")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    } else if diffFiles.isEmpty {
                        DiffTextView(diffText: diffText)
                    } else {
                        VStack(alignment: .leading, spacing: 12) {
                            ForEach(diffFiles) { file in
                                DiffFileRow(
                                    file: file,
                                    isExpanded: expandedFileIds.contains(file.id),
                                    onToggle: { toggleFile(file.id) }
                                )
                            }
                        }
                    }
                }
            }
        }
        .task(id: session?.id) {
            await loadDiff()
        }
    }

    @MainActor
    private func loadDiff() async {
        if let preview {
            diffText = preview.diffText
            diffFiles = preview.files
            statusSummary = preview.statusSummary
            diffSummary = preview.diffSummary
            expandedFileIds = Set(preview.files.map(\.id))
            errorMessage = nil
            isLoading = false
            return
        }
        guard let sessionId = session?.id, !sessionId.isEmpty else {
            diffText = ""
            diffFiles = []
            statusSummary = ""
            diffSummary = nil
            expandedFileIds = []
            errorMessage = "Select a task session to view a diff."
            return
        }
        guard let client = connection.apiClient else {
            diffText = ""
            diffFiles = []
            statusSummary = ""
            diffSummary = nil
            expandedFileIds = []
            errorMessage = "Connect to a daemon first."
            return
        }
        isLoading = true
        errorMessage = nil
        statusSummary = ""
        diffSummary = nil
        expandedFileIds = []
        do {
            async let statusResponse = client.fetchSessionGitStatus(sessionId: sessionId)
            async let summaryResponse = client.fetchSessionGitDiffSummary(sessionId: sessionId)

            let diff = try await fetchDiff(client: client, sessionId: sessionId)
            diffText = diff

            let parseResult = parseUnifiedDiff(diff)
            diffFiles = parseResult.files
            let fallbackSummary = DiffSummary(
                fileCount: parseResult.files.count,
                additions: parseResult.additions,
                deletions: parseResult.deletions
            )

            if let summary = try? await summaryResponse {
                diffSummary = DiffSummary(
                    fileCount: summary.fileCount,
                    additions: summary.additions,
                    deletions: summary.deletions
                )
            } else {
                diffSummary = fallbackSummary
            }

            if let status = try? await statusResponse {
                statusSummary = status.summary
            }
        } catch {
            diffText = ""
            diffFiles = []
            errorMessage = "Failed to load diff."
        }
        isLoading = false
    }

    private func fetchDiff(client: DaemonAPIClient, sessionId: String) async throws -> String {
        let response = try await client.fetchSessionDiff(sessionId: sessionId)
        return response.diff
    }

    private func toggleFile(_ fileId: String) {
        if expandedFileIds.contains(fileId) {
            expandedFileIds.remove(fileId)
        } else {
            expandedFileIds.insert(fileId)
        }
    }
}

private struct DiffTextView: View {
    let diffText: String

    private var attributedDiff: AttributedString {
        var output = AttributedString()
        let lines = diffText.split(omittingEmptySubsequences: false) { $0.isNewline }
        for (index, line) in lines.enumerated() {
            let text = String(line)
            var fragment = AttributedString(text)
            fragment.font = .system(size: 12, weight: .regular, design: .monospaced)
            fragment.foregroundColor = diffLineColor(text)
            output.append(fragment)
            if index < lines.count - 1 {
                output.append(AttributedString("\n"))
            }
        }
        return output
    }

    var body: some View {
        Text(attributedDiff)
            .frame(maxWidth: .infinity, alignment: .leading)
            .textSelection(.enabled)
    }

    private func diffLineColor(_ line: String) -> Color {
        if line.hasPrefix("+"), !line.hasPrefix("+++") {
            return .ctxDiffAdded
        }
        if line.hasPrefix("-"), !line.hasPrefix("---") {
            return .ctxDiffRemoved
        }
        return .ctxTextPrimary
    }
}

private struct DiffSummary: Equatable {
    let fileCount: Int
    let additions: Int
    let deletions: Int
}

private struct DiffPreview {
    let diffText: String
    let statusSummary: String
    let diffSummary: DiffSummary
    let files: [DiffFile]

    static let uiTest: DiffPreview = {
        let diffText = """
        diff --git a/core/apps/ios-native/Sources/App/RootView.swift b/core/apps/ios-native/Sources/App/RootView.swift
        index 2b4e9b4..a83e3d1 100644
        --- a/core/apps/ios-native/Sources/App/RootView.swift
        +++ b/core/apps/ios-native/Sources/App/RootView.swift
        @@ -10,6 +10,10 @@ struct RootView: View {
         @State private var didBootstrap = false

         var body: some View {
        +    let uiTestScreen = UITestOverrides.screen
        +    if let uiTestScreen {
        +        UITestScreenOverrideView(screen: uiTestScreen)
        +    }
        diff --git a/core/apps/ios-native/Sources/App/UITestOverrides.swift b/core/apps/ios-native/Sources/App/UITestOverrides.swift
        new file mode 100644
        index 0000000..1d2f3a4
        --- /dev/null
        +++ b/core/apps/ios-native/Sources/App/UITestOverrides.swift
        @@ -0,0 +1,4 @@
        +import Foundation
        +enum UITestOverrides {}
        """
        let parseResult = parseUnifiedDiff(diffText)
        let diffSummary = DiffSummary(
            fileCount: parseResult.files.count,
            additions: parseResult.additions,
            deletions: parseResult.deletions
        )
        let statusSummary = """
         M core/apps/ios-native/Sources/App/RootView.swift
         A core/apps/ios-native/Sources/App/UITestOverrides.swift
        """
        return DiffPreview(
            diffText: diffText,
            statusSummary: statusSummary,
            diffSummary: diffSummary,
            files: parseResult.files
        )
    }()
}

private struct DiffFile: Identifiable, Hashable {
    let id: String
    let filePath: String
    let diffText: String
    let addedLines: Int
    let deletedLines: Int
    let isNew: Bool
    let isDeleted: Bool
}

private struct DiffParseResult {
    let files: [DiffFile]
    let additions: Int
    let deletions: Int
}

private struct DiffFileRow: View {
    let file: DiffFile
    let isExpanded: Bool
    let onToggle: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button(action: onToggle) {
                HStack(spacing: 8) {
                    LucideIcon(name: isExpanded ? .chevronDown : .chevronRight, size: 14)
                        .foregroundColor(.ctxTextSecondary)

                    Text(file.filePath)
                        .font(.subheadline.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .lineLimit(1)

                    Spacer()

                    DiffFileSummaryBadge(file: file)
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(.plain)

            if isExpanded {
                DiffTextView(diffText: file.diffText)
            }
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.5), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 0.8)
        )
    }
}

private struct DiffFileSummaryBadge: View {
    let file: DiffFile

    var body: some View {
        HStack(spacing: 6) {
            if file.isNew {
                Text("New")
                    .font(.caption2.weight(.semibold))
                    .foregroundColor(.ctxDiffAdded)
            } else if file.isDeleted {
                Text("Deleted")
                    .font(.caption2.weight(.semibold))
                    .foregroundColor(.ctxDiffRemoved)
            }

            if file.addedLines > 0 {
                Text("+\(file.addedLines)")
                    .font(.caption2.weight(.semibold))
                    .foregroundColor(.ctxDiffAdded)
            }

            if file.deletedLines > 0 {
                Text("-\(file.deletedLines)")
                    .font(.caption2.weight(.semibold))
                    .foregroundColor(.ctxDiffRemoved)
            }

            if file.addedLines == 0 && file.deletedLines == 0 && !file.isNew && !file.isDeleted {
                Text("Binary")
                    .font(.caption2.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
            }
        }
    }
}

private func parseUnifiedDiff(_ diffText: String) -> DiffParseResult {
    let lines = diffText.split(omittingEmptySubsequences: false) { $0.isNewline }
    var files: [DiffFile] = []
    var currentLines: [String] = []
    var currentPath: String?
    var currentAdded = 0
    var currentDeleted = 0
    var isNew = false
    var isDeleted = false
    var fileIndex = 0
    var totalAdditions = 0
    var totalDeletions = 0

    func finalize() {
        guard let path = currentPath else { return }
        let id = "\(fileIndex)-\(path)"
        let diffText = currentLines.joined(separator: "\n")
        let file = DiffFile(
            id: id,
            filePath: path,
            diffText: diffText,
            addedLines: currentAdded,
            deletedLines: currentDeleted,
            isNew: isNew,
            isDeleted: isDeleted
        )
        files.append(file)
        totalAdditions += currentAdded
        totalDeletions += currentDeleted
    }

    func normalizePath(_ raw: String) -> String {
        var path = raw
        path = path.trimmingCharacters(in: CharacterSet(charactersIn: "\""))
        if path.hasPrefix("a/") || path.hasPrefix("b/") {
            path.removeFirst(2)
        }
        return path
    }

    func parseDiffHeaderPaths(_ line: String) -> (String, String) {
        let prefix = "diff --git "
        guard line.hasPrefix(prefix) else {
            return ("", "")
        }
        let remainder = String(line.dropFirst(prefix.count))
        var index = remainder.startIndex

        func nextToken() -> String? {
            while index < remainder.endIndex, remainder[index].isWhitespace {
                index = remainder.index(after: index)
            }
            if index >= remainder.endIndex {
                return nil
            }
            if remainder[index] == "\"" {
                index = remainder.index(after: index)
                var token = ""
                while index < remainder.endIndex {
                    let ch = remainder[index]
                    if ch == "\"" {
                        index = remainder.index(after: index)
                        break
                    }
                    if ch == "\\" {
                        let next = remainder.index(after: index)
                        if next < remainder.endIndex {
                            token.append(remainder[next])
                            index = remainder.index(after: next)
                            continue
                        }
                        index = next
                        break
                    }
                    token.append(ch)
                    index = remainder.index(after: index)
                }
                return token
            }
            var token = ""
            while index < remainder.endIndex, !remainder[index].isWhitespace {
                token.append(remainder[index])
                index = remainder.index(after: index)
            }
            return token
        }

        let oldRaw = nextToken() ?? ""
        let newRaw = nextToken() ?? ""
        return (normalizePath(oldRaw), normalizePath(newRaw))
    }

    for line in lines {
        let text = String(line)
        if text.hasPrefix("diff --git ") {
            if currentPath != nil {
                finalize()
                fileIndex += 1
            }
            currentLines = [text]
            currentAdded = 0
            currentDeleted = 0
            isNew = false
            isDeleted = false
            let (_, newPath) = parseDiffHeaderPaths(text)
            currentPath = newPath.isEmpty ? "(unknown)" : newPath
            continue
        }

        guard currentPath != nil else { continue }
        currentLines.append(text)

        if text.hasPrefix("new file mode") || text == "--- /dev/null" {
            isNew = true
        }
        if text.hasPrefix("deleted file mode") || text == "+++ /dev/null" {
            isDeleted = true
        }

        if text.hasPrefix("+"), !text.hasPrefix("+++") {
            currentAdded += 1
        } else if text.hasPrefix("-"), !text.hasPrefix("---") {
            currentDeleted += 1
        }
    }

    if currentPath != nil {
        finalize()
    }

    return DiffParseResult(files: files, additions: totalAdditions, deletions: totalDeletions)
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
