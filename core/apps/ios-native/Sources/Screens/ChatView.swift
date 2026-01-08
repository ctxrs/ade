import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import UIKit

struct ChatView: View {
    @ObservedObject var viewModel: ChatViewModel
    var showsBackground: Bool = true
    @State private var composerText = ""
    @State private var pendingAttachments: [MessageAttachment] = []
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @FocusState private var isComposerFocused: Bool

    var body: some View {
        ZStack {
            if showsBackground {
                Color.ctxBackground.ignoresSafeArea()
            }
            messageList
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                if !pendingAttachments.isEmpty {
                    ComposerAttachmentsRow(
                        attachments: pendingAttachments,
                        onRemove: { index in
                            pendingAttachments.remove(at: index)
                        }
                    )
                }
                ComposerBar(
                    text: $composerText,
                    selectedPhotos: $selectedPhotos,
                    isSendEnabled: isSendEnabled,
                    isFocused: $isComposerFocused,
                    onSend: sendMessage
                )
            }
        }
        .onAppear { viewModel.startPolling() }
        .onDisappear { viewModel.stopPolling() }
        .onChange(of: selectedPhotos) { newItems in
            Task { await loadAttachments(from: newItems) }
        }
    }

    private var messageList: some View {
        GeometryReader { geometry in
            let horizontalPadding: CGFloat = 16
            let availableWidth = max(0, geometry.size.width - (horizontalPadding * 2))
            let maxBubbleWidth = min(360, availableWidth * 0.78)
            let assetContext = viewModel.assetContext
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 12) {
                        ForEach(viewModel.messages) { message in
                            MessageRow(
                                message: message,
                                maxBubbleWidth: maxBubbleWidth,
                                assetContext: assetContext
                            )
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

    private var isSendEnabled: Bool {
        !composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pendingAttachments.isEmpty
    }

    private func sendMessage() {
        let trimmed = composerText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !pendingAttachments.isEmpty else { return }
        viewModel.send(trimmed, attachments: pendingAttachments)
        composerText = ""
        pendingAttachments = []
    }

    @MainActor
    private func loadAttachments(from items: [PhotosPickerItem]) async {
        guard !items.isEmpty else { return }
        var nextAttachments: [MessageAttachment] = []
        for item in items {
            guard let data = try? await item.loadTransferable(type: Data.self) else { continue }
            let contentType = item.supportedContentTypes.first
            let mimeType = contentType?.preferredMIMEType ?? "image/*"
            let fileExtension = contentType?.preferredFilenameExtension
            let name = suggestedAttachmentName(extension: fileExtension)
            nextAttachments.append(
                MessageAttachment(
                    kind: .image,
                    mimeType: mimeType,
                    dataBase64: data.base64EncodedString(),
                    blobId: nil,
                    name: name
                )
            )
        }
        if !nextAttachments.isEmpty {
            pendingAttachments.append(contentsOf: nextAttachments)
        }
        selectedPhotos = []
    }

    private func suggestedAttachmentName(extension fileExtension: String?) -> String? {
        let suffix = (fileExtension?.isEmpty == false) ? ".\(fileExtension ?? "")" : ""
        let shortId = String(UUID().uuidString.prefix(8))
        return "photo-\(shortId)\(suffix)"
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
    @StateObject private var viewModel: ChatViewModel
    @State private var isArtifactsPresented = false

    init(session: SessionSummary) {
        self.session = session
        _viewModel = StateObject(
            wrappedValue: ChatViewModel(
                initialSessionId: session.id,
                initialWorkspaceId: session.workspaceId
            )
        )
    }

    var body: some View {
        ChatView(viewModel: viewModel, showsBackground: true)
            .onAppear {
                viewModel.setClient(connection.apiClient)
                viewModel.selectSession(session.id, workspaceId: session.workspaceId)
            }
            .navigationTitle(session.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button {
                        isArtifactsPresented = true
                    } label: {
                        Image(systemName: "photo.stack")
                    }
                }
            }
            .sheet(isPresented: $isArtifactsPresented) {
                ArtifactsListView(
                    artifacts: viewModel.artifacts,
                    assetContext: viewModel.assetContext,
                    isLoading: viewModel.isArtifactsLoading,
                    errorMessage: viewModel.artifactsError
                )
            }
    }
}

struct MessageRow: View {
    let message: ChatMessage
    let maxBubbleWidth: CGFloat
    let assetContext: DaemonAssetContext

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
        VStack(alignment: .leading, spacing: 8) {
            if !message.text.isEmpty {
                Text(message.text)
                    .font(.system(size: 16, weight: .regular))
                    .foregroundColor(.ctxTextPrimary)
                    .lineSpacing(4)
            }
            if !message.attachments.isEmpty {
                AttachmentStrip(
                    attachments: message.attachments,
                    assetContext: assetContext
                )
            }
        }
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

struct DaemonAssetContext {
    let baseURL: URL?
    let token: String?

    func blobURL(_ blobId: String) -> URL? {
        let escaped = blobId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? blobId
        return url(path: "/api/blobs/\(escaped)")
    }

    func artifactURL(_ artifactId: String) -> URL? {
        let escaped = artifactId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? artifactId
        return url(path: "/api/artifacts/\(escaped)")
    }

    private func url(path: String) -> URL? {
        guard let baseURL else { return nil }
        let normalizedPath = path.hasPrefix("/") ? path : "/\(path)"
        guard let base = URL(string: normalizedPath, relativeTo: baseURL),
              var components = URLComponents(url: base, resolvingAgainstBaseURL: true) else {
            return nil
        }
        if let token, !token.isEmpty {
            var queryItems = components.queryItems ?? []
            queryItems.append(URLQueryItem(name: "token", value: token))
            components.queryItems = queryItems
        }
        return components.url
    }
}

struct ComposerAttachmentsRow: View {
    let attachments: [MessageAttachment]
    let onRemove: (Int) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 10) {
                ForEach(Array(attachments.enumerated()), id: \.offset) { index, attachment in
                    ZStack(alignment: .topTrailing) {
                        AttachmentPreview(
                            attachment: attachment,
                            assetContext: DaemonAssetContext(baseURL: nil, token: nil)
                        )
                        .frame(width: 72, height: 72)
                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

                        Button {
                            onRemove(index)
                        } label: {
                            Image(systemName: "xmark")
                                .font(.caption2.weight(.bold))
                                .foregroundColor(.white)
                                .padding(5)
                                .background(Color.black.opacity(0.6), in: Circle())
                        }
                        .padding(4)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
        .background(.ultraThinMaterial)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(Color.ctxLine)
                .frame(height: 1)
        }
    }
}

struct AttachmentStrip: View {
    let attachments: [MessageAttachment]
    let assetContext: DaemonAssetContext

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 10) {
                ForEach(Array(attachments.enumerated()), id: \.offset) { _, attachment in
                    AttachmentPreview(attachment: attachment, assetContext: assetContext)
                        .frame(width: 140, height: 140)
                        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                }
            }
        }
    }
}

struct AttachmentPreview: View {
    let attachment: MessageAttachment
    let assetContext: DaemonAssetContext
    @State private var inlineImage: UIImage?

    var body: some View {
        ZStack {
            if let inlineImage {
                Image(uiImage: inlineImage)
                    .resizable()
                    .scaledToFill()
            } else if attachment.kind == .imageRef,
                      let blobId = attachment.blobId,
                      let url = assetContext.blobURL(blobId) {
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image
                            .resizable()
                            .scaledToFill()
                    case .failure:
                        placeholder
                    default:
                        loading
                    }
                }
            } else {
                placeholder
            }
        }
        .background(Color.ctxSurfaceRaised)
        .clipped()
        .task(id: attachmentTaskKey) {
            guard attachment.kind == .image,
                  let base64 = attachment.dataBase64,
                  let data = Data(base64Encoded: base64),
                  let image = UIImage(data: data) else {
                inlineImage = nil
                return
            }
            inlineImage = image
        }
    }

    private var loading: some View {
        ProgressView()
            .tint(.ctxAccent)
    }

    private var attachmentTaskKey: String {
        let payload = attachment.dataBase64 ?? attachment.blobId ?? ""
        return "\(attachment.kind.rawValue)-\(payload)"
    }

    private var placeholder: some View {
        ZStack {
            Color.ctxSurfaceRaised
            Image(systemName: "photo")
                .foregroundColor(.ctxTextMuted)
        }
    }
}

struct ArtifactsListView: View {
    @Environment(\.dismiss) private var dismiss
    let artifacts: [Artifact]
    let assetContext: DaemonAssetContext
    let isLoading: Bool
    let errorMessage: String?

    var body: some View {
        NavigationStack {
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 14) {
                    if isLoading {
                        ProgressView()
                            .tint(.ctxAccent)
                    } else if let errorMessage {
                        Text(errorMessage)
                            .font(.caption)
                            .foregroundColor(.ctxError)
                    } else if artifacts.isEmpty {
                        Text("No artifacts yet.")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    } else {
                        ForEach(artifacts) { artifact in
                            ArtifactRow(artifact: artifact, assetContext: assetContext)
                        }
                    }
                }
                .padding(16)
            }
            .background(Color.ctxBackground.ignoresSafeArea())
            .navigationTitle("Artifacts")
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

struct ArtifactRow: View {
    let artifact: Artifact
    let assetContext: DaemonAssetContext

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            ArtifactPreview(artifact: artifact, assetContext: assetContext)
                .frame(width: 72, height: 72)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))

            VStack(alignment: .leading, spacing: 6) {
                Text(displayName)
                    .font(.headline)
                    .foregroundColor(.ctxTextPrimary)
                    .lineLimit(1)
                Text(metaText)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }

            Spacer(minLength: 0)
        }
        .padding(12)
        .background(Color.ctxSurface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.white.opacity(0.06), lineWidth: 1)
        )
    }

    private var displayName: String {
        let name = (artifact.name ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !name.isEmpty { return name }
        let parts = artifact.absolutePath.split(whereSeparator: { $0 == "/" || $0 == "\\" })
        if let last = parts.last { return String(last) }
        return "artifact"
    }

    private var metaText: String {
        if artifact.missing == true {
            return "Missing"
        }
        let mime = artifact.mimeType.isEmpty ? "application/octet-stream" : artifact.mimeType
        return "\(mime) · \(formatBytes(artifact.bytes))"
    }

    private func formatBytes(_ bytes: Int) -> String {
        guard bytes > 0 else { return "0 B" }
        let units = ["B", "KB", "MB", "GB"]
        var value = Double(bytes)
        var idx = 0
        while value >= 1024 && idx < units.count - 1 {
            value /= 1024
            idx += 1
        }
        let formatter = value >= 10 || idx == 0 ? "%.0f" : "%.1f"
        return String(format: formatter, value) + " " + units[idx]
    }
}

struct ArtifactPreview: View {
    let artifact: Artifact
    let assetContext: DaemonAssetContext

    var body: some View {
        ZStack {
            if artifact.missing == true {
                missingView
            } else if isImage, let url = assetContext.artifactURL(artifact.id.stringValue) {
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image
                            .resizable()
                            .scaledToFill()
                    case .failure:
                        placeholder
                    default:
                        loading
                    }
                }
            } else {
                placeholder
            }
        }
        .background(Color.ctxSurfaceRaised)
        .clipped()
    }

    private var isImage: Bool {
        artifact.mimeType.lowercased().hasPrefix("image/")
    }

    private var loading: some View {
        ProgressView()
            .tint(.ctxAccent)
    }

    private var placeholder: some View {
        ZStack {
            Color.ctxSurfaceRaised
            Image(systemName: "doc")
                .foregroundColor(.ctxTextMuted)
        }
    }

    private var missingView: some View {
        ZStack {
            Color.ctxSurfaceRaised
            Text("Missing")
                .font(.caption2)
                .foregroundColor(.ctxTextMuted)
        }
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
    @Binding var selectedPhotos: [PhotosPickerItem]
    var isSendEnabled: Bool
    var isFocused: FocusState<Bool>.Binding
    var onSend: () -> Void

    var body: some View {
        HStack(alignment: .bottom, spacing: 10) {
            PhotosPicker(selection: $selectedPhotos, matching: .images) {
                ComposerIconButton(systemName: "plus", isPrimary: false, isEnabled: true) {}
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

struct ChatMessage: Identifiable {
    enum Role {
        case assistant
        case user
    }

    let id: UUID
    let role: Role
    let text: String
    let attachments: [MessageAttachment]
}

@MainActor
final class ChatViewModel: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var isAssistantTyping = false
    @Published var errorMessage: String?
    @Published var artifacts: [Artifact] = []
    @Published var isArtifactsLoading = false
    @Published var artifactsError: String?
    @Published private(set) var assetBaseURL: URL?
    @Published private(set) var assetToken: String?

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
    private var artifactsRefreshInFlight = false
    private var artifactsRefreshPending = false
    private var pendingAssistantResponse = false
    private var consecutivePollFailures = 0

    init(client: DaemonAPIClient? = nil, initialSessionId: String? = nil, initialWorkspaceId: String? = nil) {
        self.client = client
        self.sessionId = initialSessionId
        self.workspaceId = initialWorkspaceId
        if client == nil {
            messages = Self.sampleMessages
            isAssistantTyping = true
        }
    }

    var assetContext: DaemonAssetContext {
        DaemonAssetContext(baseURL: assetBaseURL, token: assetToken)
    }

    func setClient(_ client: DaemonAPIClient?) {
        self.client = client
        if client != nil {
            messages = []
            isAssistantTyping = false
            pendingAssistantResponse = false
            errorMessage = nil
            artifacts = []
            artifactsError = nil
            isArtifactsLoading = false
            lastEventSeq = nil
            refreshAssetContext()
            if pollTask != nil {
                startStream()
            }
            Task {
                _ = await refreshMessages()
                await refreshArtifacts()
            }
        } else {
            stopStream()
            sessionId = nil
            workspaceId = nil
            lastEventSeq = nil
            messages = Self.sampleMessages
            isAssistantTyping = true
            pendingAssistantResponse = true
            artifacts = []
            artifactsError = nil
            isArtifactsLoading = false
            assetBaseURL = nil
            assetToken = nil
        }
    }

    private func refreshAssetContext() {
        guard let client else {
            assetBaseURL = nil
            assetToken = nil
            return
        }
        Task { @MainActor in
            assetBaseURL = await client.daemonBaseURL()
            assetToken = await client.authToken()
        }
    }

    func selectSession(_ sessionId: String?, workspaceId: String? = nil) {
        self.sessionId = sessionId
        self.workspaceId = workspaceId
        lastEventSeq = nil
        pendingAssistantResponse = false
        isAssistantTyping = false
        artifacts = []
        artifactsError = nil
        Task {
            await primeStreamCursor()
            _ = await refreshMessages()
            await refreshArtifacts()
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

    func send(_ text: String, attachments: [MessageAttachment]) {
        let local = ChatMessage(id: UUID(), role: .user, text: text, attachments: attachments)
        messages.append(local)
        pendingAssistantResponse = true
        isAssistantTyping = true

        Task {
            guard let client else { return }
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            do {
                _ = try await client.postMessage(sessionId: resolved, content: text, delivery: .immediate, attachments: attachments)
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
                    text: summary.content,
                    attachments: summary.attachments ?? []
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

    private func refreshArtifacts() async {
        guard let client else { return }
        if artifactsRefreshInFlight {
            artifactsRefreshPending = true
            return
        }
        artifactsRefreshInFlight = true
        isArtifactsLoading = true
        defer {
            artifactsRefreshInFlight = false
            isArtifactsLoading = false
        }
        let resolved = await resolveSessionId()
        guard let resolved else { return }
        do {
            artifacts = try await client.listSessionArtifacts(sessionId: resolved)
            artifactsError = nil
            if artifactsRefreshPending {
                artifactsRefreshPending = false
                Task { await refreshArtifacts() }
            }
        } catch {
            artifactsError = "Failed to load artifacts."
            if artifactsRefreshPending {
                artifactsRefreshPending = false
                Task { await refreshArtifacts() }
            }
        }
    }

    private func resolveSessionId() async -> String? {
        if let sessionId {
            if workspaceId == nil {
                _ = await resolveWorkspaceId()
            }
            return sessionId
        }
        if workspaceId != nil {
            return nil
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
                if delta.event?.eventType == "artifacts_set" {
                    Task { await refreshArtifacts() }
                }
                Task { _ = await refreshMessages() }
            }
        case .sessionSummary(_, _, let summary):
            let eventSessionId = summary.session.id.stringValue
            if eventSessionId == currentSessionId {
                if let lastEventSeq = summary.lastEventSeq {
                    self.lastEventSeq = lastEventSeq
                }
                Task {
                    _ = await refreshMessages()
                    await refreshArtifacts()
                }
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
            text: "Welcome to ctx. What would you like to build today?",
            attachments: []
        ),
        ChatMessage(
            id: UUID(),
            role: .user,
            text: "A native chat screen with SwiftUI components.",
            attachments: []
        ),
        ChatMessage(
            id: UUID(),
            role: .assistant,
            text: "Great. I will set up message bubbles, typing states, and a composer that feels like ChatGPT.",
            attachments: []
        ),
    ]
}

#Preview {
    ChatView(viewModel: ChatViewModel())
}
