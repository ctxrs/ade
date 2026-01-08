import SwiftUI

struct ChatView: View {
    @StateObject private var viewModel = ChatViewModel.sample()
    @State private var composerText = ""

    var body: some View {
        ZStack {
            Color.ctxBackground.ignoresSafeArea()
            messageList
        }
        .safeAreaInset(edge: .bottom) {
            ComposerBar(
                text: $composerText,
                isSendEnabled: !composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                onSend: sendMessage
            )
        }
    }

    private var messageList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 14) {
                    ForEach(viewModel.messages) { message in
                        MessageRow(message: message)
                            .id(message.id)
                    }
                    if viewModel.isAssistantTyping {
                        TypingIndicatorRow()
                            .id("typing-indicator")
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 20)
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

    var body: some View {
        HStack {
            if message.role == .assistant {
                bubble
                Spacer(minLength: 40)
            } else {
                Spacer(minLength: 40)
                bubble
            }
        }
    }

    private var bubble: some View {
        Text(message.text)
            .font(.system(size: 16))
            .foregroundColor(.ctxTextPrimary)
            .lineSpacing(3)
            .padding(.vertical, 10)
            .padding(.horizontal, 14)
            .background(bubbleColor, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Color.white.opacity(0.06), lineWidth: 1)
            )
            .frame(maxWidth: 300, alignment: message.role == .assistant ? .leading : .trailing)
    }

    private var bubbleColor: Color {
        message.role == .assistant ? .ctxBubbleAssistant : .ctxBubbleUser
    }
}

struct TypingIndicatorRow: View {
    var body: some View {
        HStack {
            TypingIndicatorView()
                .padding(.vertical, 10)
                .padding(.horizontal, 14)
                .background(Color.ctxBubbleAssistant, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(Color.white.opacity(0.06), lineWidth: 1)
                )
            Spacer(minLength: 40)
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
        HStack(spacing: 12) {
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
            .padding(.vertical, 10)
            .padding(.horizontal, 14)
            .background(.ultraThinMaterial, in: Capsule())
            .overlay(
                Capsule()
                    .stroke(Color.white.opacity(0.08), lineWidth: 1)
            )
            .frame(maxWidth: .infinity)

            HStack(spacing: 8) {
                ComposerIconButton(systemName: "mic.fill", isPrimary: false, isEnabled: true) {
                }
                ComposerIconButton(systemName: "paperplane.fill", isPrimary: true, isEnabled: isSendEnabled) {
                    onSend()
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
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

final class ChatViewModel: ObservableObject {
    @Published var messages: [ChatMessage]
    @Published var isAssistantTyping: Bool

    init(messages: [ChatMessage], isAssistantTyping: Bool) {
        self.messages = messages
        self.isAssistantTyping = isAssistantTyping
    }

    func send(_ text: String) {
        messages.append(ChatMessage(id: UUID(), role: .user, text: text))
        isAssistantTyping = true
    }

    static func sample() -> ChatViewModel {
        ChatViewModel(
            messages: [
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
                )
            ],
            isAssistantTyping: true
        )
    }
}

#Preview {
    ChatView()
}
