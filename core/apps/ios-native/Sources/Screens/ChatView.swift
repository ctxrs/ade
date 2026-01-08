import SwiftUI

struct ChatView: View {
    @ObservedObject var viewModel: ChatViewModel
    var showsBackground: Bool = true
    @State private var composerText = ""

    var body: some View {
        ZStack {
            if showsBackground {
                Color.ctxBackground.ignoresSafeArea()
            }
            messageList
        }
        .safeAreaInset(edge: .bottom) {
            ComposerBar(
                text: $composerText,
                isSendEnabled: !composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                onSend: sendMessage
            )
        }
        .onAppear { viewModel.startPolling() }
        .onDisappear { viewModel.stopPolling() }
    }

    private var messageList: some View {
        GeometryReader { geometry in
            let horizontalPadding: CGFloat = 14
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
                    .padding(.vertical, 16)
                }
                .onAppear {
                    scrollToBottom(proxy: proxy, animated: false)
                }
                .onChange(of: viewModel.messages.count) { _ in
                    scrollToBottom(proxy: proxy, animated: true)
                }
                .onChange(of: viewModel.isAssistantTyping) { _ in
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
    var onSend: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            ComposerIconButton(systemName: "plus", isPrimary: false, isEnabled: true) {
            }

            ZStack(alignment: .leading) {
                if text.isEmpty {
                    Text("Message")
                        .foregroundColor(.ctxTextSecondary)
                }
                TextField("", text: $text, axis: .vertical)
                    .lineLimit(1...4)
                    .foregroundColor(.ctxTextPrimary)
                    .tint(.ctxAccent)
            }
            .font(.system(size: 16))
            .padding(.vertical, 9)
            .padding(.horizontal, 14)
            .padding(.trailing, 42)
            .background(.ultraThinMaterial, in: Capsule())
            .overlay(
                Capsule()
                    .stroke(Color.white.opacity(0.08), lineWidth: 1)
            )
            .overlay(alignment: .trailing) {
                if !isSendEnabled {
                    ComposerIconButton(systemName: "mic.fill", isPrimary: false, isEnabled: true) {
                    }
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
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.ultraThinMaterial)
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

    private var client: DaemonAPIClient?
    private var sessionId: String?
    private var pollTask: Task<Void, Never>?

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
            Task { await refreshMessages() }
        }
    }

    func startPolling() {
        guard pollTask == nil else { return }
        pollTask = Task {
            while !Task.isCancelled {
                await refreshMessages()
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }

    func stopPolling() {
        pollTask?.cancel()
        pollTask = nil
    }

    func send(_ text: String) {
        let local = ChatMessage(id: UUID(), role: .user, text: text)
        messages.append(local)
        isAssistantTyping = true

        Task {
            guard let client else { return }
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            do {
                _ = try await client.postMessage(sessionId: resolved, content: text, delivery: .immediate, attachments: nil)
                await refreshMessages()
            } catch {
                errorMessage = "Failed to send message."
            }
        }
    }

    private func refreshMessages() async {
        guard let client else { return }
        let resolved = await resolveSessionId()
        guard let resolved else { return }
        do {
            let items = try await client.listMessages(sessionId: resolved)
            messages = items.map { summary in
                ChatMessage(
                    id: UUID(uuidString: summary.id) ?? UUID(),
                    role: summary.role == .user ? .user : .assistant,
                    text: summary.content
                )
            }
            isAssistantTyping = false
        } catch {
            errorMessage = "Failed to load messages."
        }
    }

    private func resolveSessionId() async -> String? {
        if let sessionId { return sessionId }
        guard let client else { return nil }
        do {
            let workspaces = try await client.listWorkspaces()
            guard let workspace = workspaces.first else { return nil }
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
