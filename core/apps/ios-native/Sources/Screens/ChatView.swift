import Foundation
import SwiftUI

struct ChatView: View {
    @ObservedObject var viewModel: ChatViewModel
    var showsBackground: Bool = true
    @State private var composerText = ""
    @FocusState private var isComposerFocused: Bool

    var body: some View {
        ZStack {
            if showsBackground {
                Color.ctxBackground.ignoresSafeArea()
            }
            messageList
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            ComposerBar(
                text: $composerText,
                isSendEnabled: !composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                isFocused: $isComposerFocused,
                onSend: sendMessage
            )
        }
        .onAppear { viewModel.startPolling() }
        .onDisappear { viewModel.stopPolling() }
    }

    private var messageList: some View {
        GeometryReader { geometry in
            let horizontalPadding: CGFloat = 16
            let availableWidth = max(0, geometry.size.width - (horizontalPadding * 2))
            let maxBubbleWidth = min(360, availableWidth * 0.78)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 12) {
                        ForEach(viewModel.messages) { message in
                            MessageRow(message: message, maxBubbleWidth: maxBubbleWidth)
                                .id(message.id)
                        }
                        if viewModel.isAssistantTyping {
                            TypingIndicatorRow(maxBubbleWidth: maxBubbleWidth)
                                .id("typing-indicator")
                        }
                    }
                    .padding(.horizontal, horizontalPadding)
                    .padding(.top, 16)
                    .padding(.bottom, 12)
                }
                .scrollDismissesKeyboard(.interactively)
                .onAppear {
                    scrollToBottom(proxy: proxy, animated: false)
                }
                .onChange(of: viewModel.messages.count) { _ in
                    scrollToBottom(proxy: proxy, animated: true)
                }
                .onChange(of: viewModel.isAssistantTyping) { _ in
                    scrollToBottom(proxy: proxy, animated: true)
                }
                .onChange(of: isComposerFocused) { focused in
                    guard focused else { return }
                    scrollToBottom(proxy: proxy, animated: true)
                }
            }
        }
    }

    private func sendMessage() {
        let trimmed = composerText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        viewModel.send(trimmed)
        composerText = ""
    }

    private func scrollToBottom(proxy: ScrollViewProxy, animated: Bool) {
        let target: AnyHashable?
        if viewModel.isAssistantTyping {
            target = "typing-indicator"
        } else {
            target = viewModel.messages.last?.id
        }
        guard let target else { return }
        if animated {
            withAnimation(.easeOut(duration: 0.2)) {
                proxy.scrollTo(target, anchor: .bottom)
            }
        } else {
            proxy.scrollTo(target, anchor: .bottom)
        }
    }
}

struct ChatDetailView: View {
    @EnvironmentObject private var connection: ConnectionStore
    let session: SessionSummary
    @StateObject private var viewModel = ChatViewModel()

    var body: some View {
        ChatView(viewModel: viewModel, showsBackground: true)
            .onAppear {
                viewModel.setClient(connection.apiClient)
                viewModel.selectSession(session.id, workspaceId: session.workspaceId)
            }
            .navigationTitle(session.title)
            .navigationBarTitleDisplayMode(.inline)
    }
}

struct MessageRow: View {
    let message: ChatMessage
    let maxBubbleWidth: CGFloat

    var body: some View {
        HStack {
            if message.role == .assistant {
                bubble
                Spacer(minLength: 0)
            } else {
                Spacer(minLength: 0)
                bubble
            }
        }
    }

    private var bubble: some View {
        Text(message.text)
            .font(.system(size: 16, weight: .regular))
            .foregroundColor(.ctxTextPrimary)
            .lineSpacing(4)
            .padding(.vertical, 11)
            .padding(.horizontal, 15)
            .background(bubbleColor, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 20, style: .continuous)
                    .stroke(Color.white.opacity(0.06), lineWidth: 1)
            )
            .frame(maxWidth: maxBubbleWidth, alignment: message.role == .assistant ? .leading : .trailing)
    }

    private var bubbleColor: Color {
        message.role == .assistant ? .ctxBubbleAssistant : .ctxBubbleUser
    }
}

struct TypingIndicatorRow: View {
    let maxBubbleWidth: CGFloat

    var body: some View {
        HStack {
            TypingIndicatorView()
                .padding(.vertical, 11)
                .padding(.horizontal, 15)
                .background(Color.ctxBubbleAssistant, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 20, style: .continuous)
                        .stroke(Color.white.opacity(0.06), lineWidth: 1)
                )
                .frame(maxWidth: maxBubbleWidth, alignment: .leading)
            Spacer(minLength: 0)
        }
    }
}

struct TypingIndicatorView: View {
    @State private var animate = false

    var body: some View {
        HStack(spacing: 6) {
            ForEach(0..<3) { index in
                Circle()
                    .fill(Color.ctxTextSecondary)
                    .frame(width: 6, height: 6)
                    .scaleEffect(animate ? 1 : 0.6)
                    .opacity(animate ? 1 : 0.4)
                    .animation(
                        .easeInOut(duration: 0.8)
                            .repeatForever()
                            .delay(Double(index) * 0.2),
                        value: animate
                    )
            }
        }
        .onAppear {
            animate = true
        }
    }
}

struct ComposerBar: View {
    @Binding var text: String
    var isSendEnabled: Bool
    var isFocused: FocusState<Bool>.Binding
    var onSend: () -> Void

    var body: some View {
        HStack(alignment: .bottom, spacing: 10) {
            ComposerIconButton(systemName: "plus", isPrimary: false, isEnabled: true) {}

            ZStack(alignment: .leading) {
                if text.isEmpty {
                    Text("Message")
                        .foregroundColor(.ctxTextSecondary)
                }
                TextField("", text: $text, axis: .vertical)
                    .lineLimit(1...4)
                    .foregroundColor(.ctxTextPrimary)
                    .tint(.ctxAccent)
                    .focused(isFocused)
            }
            .font(.system(size: 16))
            .padding(.vertical, 10)
            .padding(.horizontal, 14)
            .padding(.trailing, 46)
            .background(Color.ctxSurface, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Color.ctxLine, lineWidth: 1)
            )
            .overlay(alignment: .trailing) {
                if !isSendEnabled {
                    ComposerIconButton(systemName: "mic.fill", isPrimary: false, isEnabled: true) {}
                        .padding(.trailing, 6)
                } else {
                    ComposerIconButton(systemName: "paperplane.fill", isPrimary: true, isEnabled: isSendEnabled) {
                        onSend()
                    }
                    .padding(.trailing, 6)
                }
            }
            .frame(maxWidth: .infinity)
        }
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 10)
        .background(.ultraThinMaterial)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(Color.ctxLine)
                .frame(height: 1)
        }
    }
}

struct ComposerIconButton: View {
    let systemName: String
    let isPrimary: Bool
    let isEnabled: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 16, weight: .semibold))
                .foregroundColor(foregroundColor)
                .frame(width: 36, height: 36)
                .background(backgroundView)
                .overlay(
                    Circle()
                        .stroke(Color.white.opacity(isPrimary ? 0 : 0.08), lineWidth: 1)
                )
        }
        .disabled(!isEnabled)
        .opacity(isEnabled ? 1 : 0.45)
    }

    private var foregroundColor: Color {
        if isPrimary && isEnabled {
            return .white
        }
        return .ctxTextPrimary
    }

    @ViewBuilder
    private var backgroundView: some View {
        if isPrimary && isEnabled {
            Circle().fill(Color.ctxAccent)
        } else {
            Circle().fill(.thinMaterial)
        }
    }
}

struct ChatMessage: Identifiable, Equatable {
    enum Role {
        case assistant
        case user
    }

    let id: UUID
    let role: Role
    let text: String
}

@MainActor
final class ChatViewModel: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var isAssistantTyping = false
    @Published var errorMessage: String?

    private enum RefreshResult {
        case success
        case skipped
        case failed
    }

    private let streamClient = DaemonStreamClient()
    private let streamEncoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return encoder
    }()
    private let streamDecoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    private var client: DaemonAPIClient?
    private var sessionId: String?
    private var workspaceId: String?
    private var lastEventSeq: Int?
    private var pollTask: Task<Void, Never>?
    private var streamTask: Task<Void, Never>?
    private var streamSocket: URLSessionWebSocketTask?
    private var isStreamConnected = false
    private var streamReconnectDelay: TimeInterval = 1
    private var refreshInFlight = false
    private var refreshPending = false
    private var pendingAssistantResponse = false
    private var consecutivePollFailures = 0

    init(client: DaemonAPIClient? = nil) {
        self.client = client
        if client == nil {
            messages = Self.sampleMessages
            isAssistantTyping = true
        }
    }

    func setClient(_ client: DaemonAPIClient?) {
        self.client = client
        if client != nil {
            messages = []
            isAssistantTyping = false
            pendingAssistantResponse = false
            errorMessage = nil
            workspaceId = nil
            lastEventSeq = nil
            if pollTask != nil {
                startStream()
            }
            Task { _ = await refreshMessages() }
        } else {
            stopStream()
            sessionId = nil
            workspaceId = nil
            lastEventSeq = nil
            messages = Self.sampleMessages
            isAssistantTyping = true
            pendingAssistantResponse = true
        }
    }

    func selectSession(_ sessionId: String?, workspaceId: String? = nil) {
        self.sessionId = sessionId
        self.workspaceId = workspaceId
        lastEventSeq = nil
        pendingAssistantResponse = false
        isAssistantTyping = false
        Task {
            await primeStreamCursor()
            _ = await refreshMessages()
            await sendStreamSubscriptionIfNeeded()
        }
    }

    func startPolling() {
        guard pollTask == nil else { return }
        startStream()
        pollTask = Task {
            while !Task.isCancelled {
                let result = await refreshMessages()
                let delay = nextPollDelay(for: result)
                try? await Task.sleep(nanoseconds: delay)
            }
        }
    }

    func stopPolling() {
        pollTask?.cancel()
        pollTask = nil
        stopStream()
    }

    func send(_ text: String) {
        let local = ChatMessage(id: UUID(), role: .user, text: text)
        messages.append(local)
        pendingAssistantResponse = true
        isAssistantTyping = true

        Task {
            guard let client else { return }
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            do {
                _ = try await client.postMessage(sessionId: resolved, content: text, delivery: .immediate, attachments: nil)
                _ = await refreshMessages()
            } catch {
                pendingAssistantResponse = false
                errorMessage = "Failed to send message."
            }
        }
    }

    private func refreshMessages() async -> RefreshResult {
        guard let client else { return .skipped }
        if refreshInFlight {
            refreshPending = true
            return .skipped
        }
        refreshInFlight = true
        defer { refreshInFlight = false }
        let resolved = await resolveSessionId()
        guard let resolved else { return .skipped }
        do {
            let items = try await client.listMessages(sessionId: resolved)
            let nextMessages = items.map { summary in
                ChatMessage(
                    id: UUID(uuidString: summary.id) ?? UUID(),
                    role: summary.role == .user ? .user : .assistant,
                    text: summary.content
                )
            }
            messages = nextMessages
            updateTypingIndicator(with: nextMessages)
            errorMessage = nil
            if refreshPending {
                refreshPending = false
                Task { _ = await refreshMessages() }
            }
            return .success
        } catch {
            errorMessage = "Failed to load messages."
            if refreshPending {
                refreshPending = false
                Task { _ = await refreshMessages() }
            }
            return .failed
        }
    }

    private func resolveSessionId() async -> String? {
        if let sessionId {
            if workspaceId == nil {
                _ = await resolveWorkspaceId()
            }
            return sessionId
        }
        guard let client else { return nil }
        do {
            let workspaces = try await client.listWorkspaces()
            guard let workspace = workspaces.first else { return nil }
            workspaceId = workspace.id
            let tasks = try await client.listTasks(workspaceId: workspace.id)
            guard let task = tasks.first else { return nil }
            let tracks = try await client.listTracks(taskId: task.id)
            guard let track = tracks.first else { return nil }
            let sessions = try await client.listSessions(forTrack: track.id)
            guard let session = sessions.first else { return nil }
            sessionId = session.id
            return session.id
        } catch {
            return nil
        }
    }

    private func resolveWorkspaceId() async -> String? {
        if let workspaceId { return workspaceId }
        guard let client else { return nil }
        if let sessionId {
            if let head = try? await client.getSessionHead(sessionId: sessionId, limit: 1, includeEvents: false) {
                let resolved = head.session.workspaceId.stringValue
                workspaceId = resolved
                lastEventSeq = head.lastEventSeq
                return resolved
            }
        }
        if let workspaces = try? await client.listWorkspaces(),
           let workspace = workspaces.first {
            workspaceId = workspace.id
            return workspace.id
        }
        return nil
    }

    private func primeStreamCursor() async {
        guard let client, let sessionId else { return }
        if lastEventSeq != nil, workspaceId != nil { return }
        if let head = try? await client.getSessionHead(sessionId: sessionId, limit: 1, includeEvents: false) {
            lastEventSeq = head.lastEventSeq
            workspaceId = workspaceId ?? head.session.workspaceId.stringValue
        }
    }

    private func updateTypingIndicator(with messages: [ChatMessage]) {
        if pendingAssistantResponse, messages.last?.role == .assistant {
            pendingAssistantResponse = false
        }
        if pendingAssistantResponse {
            if let lastRole = messages.last?.role {
                isAssistantTyping = lastRole != .assistant
            } else {
                isAssistantTyping = true
            }
        } else {
            isAssistantTyping = false
        }
    }

    private func nextPollDelay(for result: RefreshResult) -> UInt64 {
        let baseDelay: TimeInterval
        if pendingAssistantResponse {
            baseDelay = 1.2
        } else if isStreamConnected {
            baseDelay = 10
        } else {
            baseDelay = 4
        }

        switch result {
        case .failed:
            consecutivePollFailures += 1
        case .success, .skipped:
            consecutivePollFailures = 0
        }

        let backoff = min(baseDelay * pow(1.6, Double(consecutivePollFailures)), 20)
        let jitter = Double.random(in: 0...0.3)
        let delay = max(0.5, backoff + jitter)
        return UInt64(delay * 1_000_000_000)
    }

    private func startStream() {
        guard streamTask == nil else { return }
        streamTask = Task {
            await streamLoop()
        }
    }

    private func stopStream() {
        streamTask?.cancel()
        streamTask = nil
        if let streamSocket {
            streamClient.disconnect(streamSocket)
        }
        streamSocket = nil
        isStreamConnected = false
        streamReconnectDelay = 1
    }

    private func streamLoop() async {
        while !Task.isCancelled {
            guard client != nil else {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                continue
            }
            guard let workspaceId = await resolveWorkspaceId() else {
                try? await Task.sleep(nanoseconds: 1_000_000_000)
                continue
            }
            do {
                let socket = try await openStream(workspaceId: workspaceId)
                await listenToStream(socket)
                if let streamSocket {
                    streamClient.disconnect(streamSocket)
                }
                streamSocket = nil
                isStreamConnected = false
            } catch {
                isStreamConnected = false
            }
            if Task.isCancelled { break }
            let delay = min(streamReconnectDelay, 15)
            streamReconnectDelay = min(streamReconnectDelay * 2, 15)
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
    }

    private func openStream(workspaceId: String) async throws -> URLSessionWebSocketTask {
        guard let client else { throw DaemonStreamError.invalidURL }
        await primeStreamCursor()
        let baseURL = await client.daemonBaseURL()
        let token = await client.authToken()
        let socket = try streamClient.connectWorkspaceStream(baseURL: baseURL, workspaceId: workspaceId, token: token)
        streamSocket = socket
        streamReconnectDelay = 1
        await sendStreamSubscriptionIfNeeded()
        isStreamConnected = true
        return socket
    }

    private func listenToStream(_ socket: URLSessionWebSocketTask) async {
        while !Task.isCancelled {
            do {
                let message = try await streamClient.receive(from: socket)
                await handleStreamMessage(message)
            } catch {
                break
            }
        }
    }

    private func sendStreamSubscriptionIfNeeded() async {
        guard let socket = streamSocket else { return }
        guard let sessionId else { return }
        await primeStreamCursor()
        let subscription = WorkspaceCatchupSessionSubscription(
            sessionId: CtxID(sessionId),
            afterSeq: lastEventSeq
        )
        let message = WorkspaceCatchupClientMessage(
            type: "subscribe",
            sessionIds: nil,
            sessions: [subscription]
        )
        guard let payload = try? streamEncoder.encode(message),
              let text = String(data: payload, encoding: .utf8) else { return }
        try? await streamClient.send(.string(text), via: socket)
    }

    private func handleStreamMessage(_ message: URLSessionWebSocketTask.Message) async {
        let data: Data?
        switch message {
        case .data(let payload):
            data = payload
        case .string(let text):
            data = text.data(using: .utf8)
        @unknown default:
            data = nil
        }
        guard let data else { return }
        guard let event = try? streamDecoder.decode(WorkspaceCatchupEvent.self, from: data) else { return }
        handleStreamEvent(event)
    }

    private func handleStreamEvent(_ event: WorkspaceCatchupEvent) {
        guard let currentSessionId = sessionId else { return }
        switch event {
        case .sessionHeadDelta(_, _, let delta):
            let eventSessionId = delta.sessionId.stringValue
            if eventSessionId == currentSessionId {
                lastEventSeq = delta.lastEventSeq
                Task { _ = await refreshMessages() }
            }
        case .sessionSummary(_, _, let summary):
            let eventSessionId = summary.session.id.stringValue
            if eventSessionId == currentSessionId {
                if let lastEventSeq = summary.lastEventSeq {
                    self.lastEventSeq = lastEventSeq
                }
                Task { _ = await refreshMessages() }
            }
        case .sessionGap(_, _, let sessionId, let afterSeq, _):
            if sessionId.stringValue == currentSessionId {
                lastEventSeq = afterSeq
                Task { _ = await refreshMessages() }
            }
        default:
            break
        }
    }

    private static let sampleMessages: [ChatMessage] = [
        ChatMessage(
            id: UUID(),
            role: .assistant,
            text: "Welcome to ctx. What would you like to build today?"
        ),
        ChatMessage(
            id: UUID(),
            role: .user,
            text: "A native chat screen with SwiftUI components."
        ),
        ChatMessage(
            id: UUID(),
            role: .assistant,
            text: "Great. I will set up message bubbles, typing states, and a composer that feels like ChatGPT."
        ),
    ]
}

#Preview {
    ChatView(viewModel: ChatViewModel())
}
