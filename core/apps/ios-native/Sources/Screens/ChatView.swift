import AVKit
import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import UIKit
import _Concurrency

enum ComposerMode: String, CaseIterable, Identifiable {
    case standard
    case plan

    var id: String { rawValue }

    var label: String {
        switch self {
        case .standard:
            return "Default"
        case .plan:
            return "Plan"
        }
    }
}

enum ComposerVerbosity: String, CaseIterable, Identifiable {
    case terse
    case normal
    case verbose

    var id: String { rawValue }

    var label: String {
        switch self {
        case .terse:
            return "Terse"
        case .normal:
            return "Default"
        case .verbose:
            return "Verbose"
        }
    }
}

struct ChatView: View {
    @ObservedObject var viewModel: ChatViewModel
    var showsBackground: Bool = true
    var providerId: String?
    var modelOptions: [String]
    var isModelLoading: Bool
    @Binding var selectedModelId: String
    @Binding var selectedEffortId: String
    @Binding var selectedMode: ComposerMode
    @Binding var selectedVerbosity: ComposerVerbosity
    @State private var composerText = ""
    @State private var pendingAttachments: [MessageAttachment] = []
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @FocusState private var isComposerFocused: Bool

    init(
        viewModel: ChatViewModel,
        showsBackground: Bool = true,
        providerId: String? = nil,
        modelOptions: [String] = [],
        isModelLoading: Bool = false,
        selectedModelId: Binding<String> = .constant(""),
        selectedEffortId: Binding<String> = .constant(""),
        selectedMode: Binding<ComposerMode> = .constant(.standard),
        selectedVerbosity: Binding<ComposerVerbosity> = .constant(.normal)
    ) {
        self.viewModel = viewModel
        self.showsBackground = showsBackground
        self.providerId = providerId
        self.modelOptions = modelOptions
        self.isModelLoading = isModelLoading
        _selectedModelId = selectedModelId
        _selectedEffortId = selectedEffortId
        _selectedMode = selectedMode
        _selectedVerbosity = selectedVerbosity
    }

    var body: some View {
        ZStack {
            if showsBackground {
                Color.ctxBackground.ignoresSafeArea()
            }
            messageList
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 10) {
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
                    isWorking: viewModel.isAssistantWorking,
                    isFocused: $isComposerFocused,
                    providerId: providerId,
                    modelOptions: modelOptions,
                    isModelLoading: isModelLoading,
                    selectedModelId: $selectedModelId,
                    selectedEffortId: $selectedEffortId,
                    selectedMode: $selectedMode,
                    selectedVerbosity: $selectedVerbosity,
                    contextWindowInfo: viewModel.contextWindowInfo,
                    onSend: sendMessage,
                    onInterrupt: interruptSession
                )
            }
            .padding(.horizontal, CtxChatStyle.composerOuterPadding)
            .padding(.top, 8)
            .padding(.bottom, 8)
            .background(Color.ctxBackground.ignoresSafeArea(edges: .bottom))
        }
        .onAppear {
            viewModel.startPolling()
            syncEffortSelection()
        }
        .onDisappear { viewModel.stopPolling() }
        .onChange(of: selectedPhotos) { newItems in
            _Concurrency.Task { await loadAttachments(from: newItems) }
        }
        .onChange(of: modelOptions) { _ in
            syncEffortSelection()
        }
        .onChange(of: selectedModelId) { _ in
            syncEffortSelection()
        }
    }

    private var messageList: some View {
        GeometryReader { geometry in
            let horizontalPadding = CtxChatStyle.horizontalPadding
            let availableWidth = max(0, geometry.size.width - (horizontalPadding * 2))
            let maxBubbleWidth = min(CtxChatStyle.userBubbleMaxWidth, availableWidth * CtxChatStyle.userBubbleWidthFraction)
            let assetContext = viewModel.assetContext
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: CtxChatStyle.messageSpacing) {
                        ForEach(viewModel.messages) { message in
                            MessageRow(
                                message: message,
                                maxBubbleWidth: maxBubbleWidth,
                                assetContext: assetContext
                            )
                                .id(message.id)
                        }
                        if let status = viewModel.turnStatus {
                            ChatTurnStatusRow(status: status)
                                .padding(.top, 4)
                                .padding(.bottom, 4)
                        }
                    }
                    .padding(.horizontal, horizontalPadding)
                    .padding(.top, CtxChatStyle.messageTopPadding)
                    .padding(.bottom, CtxChatStyle.messageBottomPadding)
                }
                .scrollDismissesKeyboard(.interactively)
                .accessibilityIdentifier("chat.messages")
                .onAppear {
                    scrollToBottom(proxy: proxy, animated: false)
                }
                .onChange(of: viewModel.messages.last?.id) { _, _ in
                    scrollToBottom(proxy: proxy, animated: true)
                }
                .onChange(of: viewModel.messages.last?.text) { _, _ in
                    scrollToBottom(proxy: proxy, animated: true)
                }
                .onChange(of: isComposerFocused) { _, focused in
                    guard focused else { return }
                    scrollToBottom(proxy: proxy, animated: true)
                }
            }
        }
    }

    private var isSendEnabled: Bool {
        !composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pendingAttachments.isEmpty
    }

    private func syncEffortSelection() {
        let modelChoices = modelOptions.isEmpty ? ["default"] : modelOptions
        let catalog = buildModelCatalog(modelChoices)
        let fallbackModelId = modelChoices.first ?? ""
        let currentModelId = selectedModelId.isEmpty ? fallbackModelId : selectedModelId
        guard !currentModelId.isEmpty else { return }

        let parsed = parseModelId(currentModelId, catalog: catalog)
        let baseId = parsed.base.isEmpty ? currentModelId : parsed.base
        let efforts = catalog.effortsByBase[baseId] ?? []

        if efforts.isEmpty {
            if !selectedEffortId.isEmpty {
                selectedEffortId = ""
            }
            if selectedModelId.isEmpty, !fallbackModelId.isEmpty {
                selectedModelId = fallbackModelId
            }
            return
        }

        var nextEffort = selectedEffortId
        if nextEffort.isEmpty || !efforts.contains(nextEffort) {
            if let parsedEffort = parsed.effort, efforts.contains(parsedEffort) {
                nextEffort = parsedEffort
            } else {
                nextEffort = pickDefaultEffort(efforts) ?? ""
            }
        }

        if selectedEffortId != nextEffort {
            selectedEffortId = nextEffort
        }

        let resolvedModelId = deriveFullModelIdForBase(catalog: catalog, baseId: baseId, preferredEffort: nextEffort)
        if !resolvedModelId.isEmpty, selectedModelId != resolvedModelId {
            selectedModelId = resolvedModelId
        }
    }

    private func sendMessage() {
        let trimmed = composerText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !pendingAttachments.isEmpty else { return }
        viewModel.send(trimmed, attachments: pendingAttachments)
        composerText = ""
        pendingAttachments = []
    }

    private func interruptSession() {
        viewModel.interrupt()
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
        guard let target = viewModel.messages.last?.id else { return }
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
    @Binding var isArtifactsPresented: Bool
    @State private var availableModels: [String] = []
    @State private var isLoadingModels = false
    @State private var selectedModelId: String
    @State private var selectedEffortId: String = ""
    @State private var selectedMode: ComposerMode = .standard
    @State private var selectedVerbosity: ComposerVerbosity = .normal

    init(session: SessionSummary, isArtifactsPresented: Binding<Bool>) {
        self.session = session
        _isArtifactsPresented = isArtifactsPresented
        _selectedModelId = State(initialValue: session.modelId)
        _viewModel = StateObject(
            wrappedValue: ChatViewModel(
                initialSessionId: session.id,
                initialWorkspaceId: session.workspaceId
            )
        )
    }

    var body: some View {
        ChatView(
            viewModel: viewModel,
            showsBackground: true,
            providerId: session.providerId,
            modelOptions: availableModels,
            isModelLoading: isLoadingModels,
            selectedModelId: $selectedModelId,
            selectedEffortId: $selectedEffortId,
            selectedMode: $selectedMode,
            selectedVerbosity: $selectedVerbosity
        )
            .onAppear {
                viewModel.setClient(connection.apiClient)
                viewModel.selectSession(session.id, workspaceId: session.workspaceId)
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: session.id) { newSessionId in
                viewModel.selectSession(newSessionId, workspaceId: session.workspaceId)
                selectedModelId = session.modelId
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: session.workspaceId) { _ in
                viewModel.selectSession(session.id, workspaceId: session.workspaceId)
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: session.providerId) { _ in
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: session.modelId) { _, newModelId in
                if !newModelId.isEmpty {
                    selectedModelId = newModelId
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

    @MainActor
    private func loadModels() async {
        guard let client = connection.apiClient,
              let workspaceId = session.workspaceId else {
            availableModels = []
            isLoadingModels = false
            return
        }
        isLoadingModels = true
        do {
            let options = try await client.getProviderOptions(workspaceId: workspaceId, providerId: session.providerId)
            let models = extractModelIds(from: options.models)
            availableModels = models
            if !selectedModelId.isEmpty, models.contains(selectedModelId) {
                isLoadingModels = false
                return
            }
            selectedModelId = models.first ?? selectedModelId
        } catch {
            availableModels = []
        }
        isLoadingModels = false
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
            } else {
                Spacer(minLength: 0)
                bubble
            }
        }
        .frame(maxWidth: .infinity, alignment: message.role == .assistant ? .leading : .trailing)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.message.\(message.role == .assistant ? "assistant" : "user")")
    }

    private var bubble: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !message.text.isEmpty {
                if message.role == .assistant {
                    MarkdownText(text: message.text)
                        .accessibilityIdentifier("chat.message.text.assistant")
                } else {
                    Text(message.text)
                        .font(CtxChatStyle.bodyFont)
                        .foregroundColor(.ctxTextPrimary)
                        .lineSpacing(CtxChatStyle.bodyLineSpacing)
                        .accessibilityIdentifier("chat.message.text.user")
                }
            }
            if !message.attachments.isEmpty {
                AttachmentStrip(
                    attachments: message.attachments,
                    assetContext: assetContext
                )
            }
        }
        .modifier(MessageBubbleStyle(role: message.role, maxBubbleWidth: maxBubbleWidth))
        .accessibilityElement(children: .contain)
    }
}

private struct MarkdownText: View {
    let text: String

    var body: some View {
        if let attributed = try? AttributedString(
            markdown: text,
            options: AttributedString.MarkdownParsingOptions(
                interpretedSyntax: .full,
                failurePolicy: .returnPartiallyParsedIfPossible
            )
        ) {
            Text(attributed)
                .font(CtxChatStyle.bodyFont)
                .foregroundColor(.ctxTextPrimary)
                .lineSpacing(CtxChatStyle.bodyLineSpacing)
        } else {
            Text(text)
                .font(CtxChatStyle.bodyFont)
                .foregroundColor(.ctxTextPrimary)
                .lineSpacing(CtxChatStyle.bodyLineSpacing)
        }
    }
}

private struct ChatTurnStatusRow: View {
    let status: ChatViewModel.TurnStatusSnapshot

    var body: some View {
        let isRunning = status.status == .queued || status.status == .running
        if isRunning {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                statusContent(now: context.date)
            }
        } else {
            statusContent(now: nil)
        }
    }

    @ViewBuilder
    private func statusContent(now: Date?) -> some View {
        let label = humanTurnStatus(status.status)
        let elapsed = formatElapsed(
            startedAt: status.startedAt,
            updatedAt: status.updatedAt,
            now: now
        )
        HStack(spacing: 6) {
            Text(label)
            Text("·")
            Text(elapsed)
        }
        .font(.system(size: 12, weight: .semibold))
        .foregroundColor(.ctxTextSecondary)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct ContextWindowInfo: Equatable {
    let windowTokens: Double?
    let usedTokens: Double?
    let remainingTokens: Double?
    let remainingFraction: Double?
}

private struct MessageBubbleStyle: ViewModifier {
    let role: ChatMessage.Role
    let maxBubbleWidth: CGFloat

    func body(content: Content) -> some View {
        switch role {
        case .assistant:
            content
                .padding(.vertical, 2)
        case .user:
            content
                .padding(.vertical, CtxChatStyle.userBubbleVerticalPadding)
                .padding(.horizontal, CtxChatStyle.userBubbleHorizontalPadding)
                .background(
                    Color.ctxBubbleUser,
                    in: RoundedRectangle(cornerRadius: CtxChatStyle.userBubbleCornerRadius, style: .continuous)
                )
                .frame(maxWidth: maxBubbleWidth, alignment: .trailing)
        }
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

private let chatIsoFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
}()

private func parseIso(_ iso: String?) -> Date? {
    guard let iso else { return nil }
    return chatIsoFormatter.date(from: iso) ?? ISO8601DateFormatter().date(from: iso)
}

private func humanTurnStatus(_ status: SessionTurnStatus) -> String {
    switch status {
    case .completed:
        return "Completed"
    case .interrupted:
        return "Interrupted"
    case .failed:
        return "Error"
    case .queued, .running:
        return "Working"
    }
}

private func formatElapsed(startedAt: String, updatedAt: String, now: Date?) -> String {
    guard let start = parseIso(startedAt) else { return "0s" }
    let end = now ?? parseIso(updatedAt) ?? Date()
    let elapsedMs = max(0, end.timeIntervalSince(start) * 1000)
    return formatElapsedMs(elapsedMs)
}

private func formatElapsedMs(_ ms: TimeInterval) -> String {
    let totalSeconds = max(0, Int(ms / 1000))
    let seconds = totalSeconds % 60
    let minutes = (totalSeconds / 60) % 60
    let hours = totalSeconds / 3600
    if hours > 0 {
        return "\(hours)h \(minutes)m"
    }
    if minutes > 0 {
        return "\(minutes)m \(seconds)s"
    }
    return "\(seconds)s"
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
            .padding(.horizontal, 4)
            .padding(.vertical, 2)
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

private let artifactVideoExtensions: Set<String> = ["mp4", "mov", "webm", "m4v"]

private func artifactFileExtension(_ path: String) -> String {
    let filename = path.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? ""
    let parts = filename.split(separator: ".")
    guard parts.count > 1, let ext = parts.last else { return "" }
    return ext.lowercased()
}

private func isVideoArtifact(_ artifact: Artifact) -> Bool {
    let mime = artifact.mimeType.lowercased()
    if mime.hasPrefix("video/") { return true }
    let ext = artifactFileExtension(artifact.absolutePath)
    return artifactVideoExtensions.contains(ext)
}

private func isImageArtifact(_ artifact: Artifact) -> Bool {
    let mime = artifact.mimeType.lowercased()
    return mime.hasPrefix("image/")
}

private func artifactDisplayName(_ artifact: Artifact) -> String {
    let name = (artifact.name ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    if !name.isEmpty { return name }
    let parts = artifact.absolutePath.split(whereSeparator: { $0 == "/" || $0 == "\\" })
    if let last = parts.last { return String(last) }
    return "artifact"
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

private func artifactMetaText(_ artifact: Artifact) -> String {
    if artifact.missing == true {
        return "Missing"
    }
    let mime = artifact.mimeType.isEmpty ? "application/octet-stream" : artifact.mimeType
    return "\(mime) · \(formatBytes(artifact.bytes))"
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
                            NavigationLink {
                                ArtifactDetailView(artifact: artifact, assetContext: assetContext)
                            } label: {
                                ArtifactRow(artifact: artifact, assetContext: assetContext)
                            }
                            .buttonStyle(.plain)
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

struct ArtifactDetailView: View {
    let artifact: Artifact
    let assetContext: DaemonAssetContext

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(alignment: .leading, spacing: 16) {
                ArtifactDetailMedia(artifact: artifact, assetContext: assetContext)
                    .frame(maxWidth: .infinity)
                    .frame(minHeight: 220)

                VStack(alignment: .leading, spacing: 6) {
                    Text(artifactDisplayName(artifact))
                        .font(.headline)
                        .foregroundColor(.ctxTextPrimary)
                    Text(artifactMetaText(artifact))
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                }
            }
            .padding(16)
        }
        .background(Color.ctxBackground.ignoresSafeArea())
        .navigationTitle(artifactDisplayName(artifact))
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct ArtifactDetailMedia: View {
    let artifact: Artifact
    let assetContext: DaemonAssetContext
    @State private var player: AVPlayer?

    var body: some View {
        ZStack {
            if artifact.missing == true {
                missingView
            } else if isVideoArtifact(artifact),
                      let url = assetContext.artifactURL(artifact.id.stringValue) {
                VideoPlayer(player: player)
                    .onAppear {
                        if player == nil {
                            player = AVPlayer(url: url)
                        }
                    }
                    .onDisappear {
                        player?.pause()
                        player = nil
                    }
                    .aspectRatio(16 / 9, contentMode: .fit)
            } else if isImageArtifact(artifact),
                      let url = assetContext.artifactURL(artifact.id.stringValue) {
                AsyncImage(url: url) { phase in
                    switch phase {
                    case .success(let image):
                        image
                            .resizable()
                            .scaledToFit()
                    case .failure:
                        placeholder
                    default:
                        loading
                    }
                }
                .padding(12)
            } else {
                placeholder
            }
        }
        .background(Color.ctxSurfaceRaised)
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
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
        artifactDisplayName(artifact)
    }

    private var metaText: String {
        artifactMetaText(artifact)
    }
}

struct ArtifactPreview: View {
    let artifact: Artifact
    let assetContext: DaemonAssetContext

    var body: some View {
        ZStack {
            if artifact.missing == true {
                missingView
            } else if isVideo {
                videoPreview
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
        isImageArtifact(artifact)
    }

    private var isVideo: Bool {
        isVideoArtifact(artifact)
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

    private var videoPreview: some View {
        ZStack {
            Color.ctxSurfaceRaised
            Image(systemName: "play.rectangle.fill")
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

struct ComposerBar: View {
    @Binding var text: String
    @Binding var selectedPhotos: [PhotosPickerItem]
    var isSendEnabled: Bool
    var isWorking: Bool
    var isFocused: FocusState<Bool>.Binding
    var providerId: String?
    var modelOptions: [String]
    var isModelLoading: Bool
    @Binding var selectedModelId: String
    @Binding var selectedEffortId: String
    @Binding var selectedMode: ComposerMode
    @Binding var selectedVerbosity: ComposerVerbosity
    var contextWindowInfo: ContextWindowInfo?
    var onSend: () -> Void
    var onInterrupt: () -> Void

    private var modelChoices: [String] {
        modelOptions.isEmpty ? ["default"] : modelOptions
    }

    private var resolvedModelId: String {
        let fallback = modelChoices.first ?? "default"
        return selectedModelId.isEmpty ? fallback : selectedModelId
    }

    var body: some View {
        let catalog = buildModelCatalog(modelChoices)
        let parsedModel = parseModelId(resolvedModelId, catalog: catalog)
        let baseId = parsedModel.base.isEmpty ? resolvedModelId : parsedModel.base
        let baseOptions = catalog.baseIds.isEmpty ? [resolvedModelId] : catalog.baseIds
        let modelLabel = isModelLoading ? "Loading..." : (catalog.displayNameByBase[baseId] ?? baseId)
        let effortOptions = catalog.effortsByBase[baseId] ?? []
        let resolvedEffortId = resolveEffortId(parsed: parsedModel, efforts: effortOptions)
        let effortLabel = formatEffortLabel(resolvedEffortId)
        let contextSummary = contextWindowSummary(from: contextWindowInfo)

        let combinedModelLabel = effortLabel.isEmpty ? modelLabel : "\(modelLabel) · \(effortLabel)"
        let baseBinding = Binding<String>(
            get: { baseId },
            set: { selectBase($0, catalog: catalog) }
        )
        let effortBinding = Binding<String>(
            get: { resolvedEffortId ?? "" },
            set: { selectEffort($0, baseId: baseId, catalog: catalog) }
        )

        VStack(spacing: 12) {
            ZStack(alignment: .topLeading) {
                if text.isEmpty {
                    Text("@ for context, / for commands")
                        .font(CtxChatStyle.bodyFont)
                        .foregroundColor(.ctxTextMuted)
                        .padding(.top, 2)
                }
                TextField("", text: $text, axis: .vertical)
                    .lineLimit(1...6)
                    .accessibilityIdentifier("chat.composer.input")
                    .font(CtxChatStyle.bodyFont)
                    .foregroundColor(.ctxTextPrimary)
                    .tint(.ctxAccent)
                    .focused(isFocused)
            }

            HStack(spacing: 12) {
                HStack(spacing: 8) {
                    ComposerHarnessIcon(providerId: providerId)

                    Menu {
                        Section("Model") {
                            Picker("Model", selection: baseBinding) {
                                ForEach(baseOptions, id: \.self) { base in
                                    let label = catalog.displayNameByBase[base] ?? base
                                    Text(label).tag(base)
                                }
                            }
                            .pickerStyle(.inline)
                        }

                        if !effortOptions.isEmpty {
                            Section("Effort") {
                                Picker("Effort", selection: effortBinding) {
                                    ForEach(effortOptions, id: \.self) { effort in
                                        Text(formatEffortLabel(effort)).tag(effort)
                                    }
                                }
                                .pickerStyle(.inline)
                            }
                        }
                    } label: {
                        HStack(spacing: 6) {
                            Text(combinedModelLabel)
                                .font(.footnote.weight(.semibold))
                                .foregroundColor(.ctxTextPrimary)
                                .lineLimit(1)
                            LucideIcon(name: .chevronDown, size: 12)
                                .foregroundColor(.ctxTextMuted)
                        }
                    }
                    .accessibilityIdentifier("chat.composer.model")
                    .disabled(isModelLoading || baseOptions.isEmpty)
                }

                Spacer(minLength: 0)

                Menu {
                    Picker("Mode", selection: $selectedMode) {
                        ForEach(ComposerMode.allCases) { mode in
                            Text(mode.label).tag(mode)
                        }
                    }
                    .pickerStyle(.inline)

                    Picker("Verbosity", selection: $selectedVerbosity) {
                        ForEach(ComposerVerbosity.allCases) { verbosity in
                            Text(verbosity.label).tag(verbosity)
                        }
                    }
                    .pickerStyle(.inline)
                } label: {
                    ComposerToolIcon(name: .ellipsis)
                }
                .accessibilityIdentifier("chat.composer.options")

                PhotosPicker(selection: $selectedPhotos, matching: .images) {
                    ComposerToolIcon(name: .image)
                }

                if isWorking {
                    ComposerCircleButton(icon: .square, accessibilityId: "chat.composer.stop") {
                        onInterrupt()
                    }
                } else if isSendEnabled {
                    ComposerCircleButton(icon: .arrowUp, accessibilityId: "chat.composer.send") {
                        onSend()
                    }
                } else {
                    Button {} label: {
                        ComposerToolIcon(name: .mic)
                    }
                }
            }
        }
        .padding(.horizontal, CtxChatStyle.composerInnerHorizontalPadding)
        .padding(.vertical, CtxChatStyle.composerInnerVerticalPadding)
        .background(
            Color.ctxSurfaceRaised,
            in: RoundedRectangle(cornerRadius: CtxChatStyle.composerCornerRadius, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: CtxChatStyle.composerCornerRadius, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
        .overlay(alignment: .topTrailing) {
            if let contextSummary {
                ContextWindowIndicator(summary: contextSummary)
                    .padding(.top, 4)
                    .padding(.trailing, 6)
                    .allowsHitTesting(false)
            }
        }
    }

    private func resolveEffortId(parsed: ParsedModelId, efforts: [String]) -> String? {
        guard !efforts.isEmpty else { return nil }
        if !selectedEffortId.isEmpty, efforts.contains(selectedEffortId) {
            return selectedEffortId
        }
        if let parsedEffort = parsed.effort, efforts.contains(parsedEffort) {
            return parsedEffort
        }
        return pickDefaultEffort(efforts)
    }

    private func selectBase(_ baseId: String, catalog: ModelCatalog) {
        let preferredEffort = selectedEffortId.isEmpty ? nil : selectedEffortId
        let nextModelId = deriveFullModelIdForBase(catalog: catalog, baseId: baseId, preferredEffort: preferredEffort)
        selectedModelId = nextModelId
        let parsed = parseModelId(nextModelId, catalog: catalog)
        let nextEffort = resolveEffortId(parsed: parsed, efforts: catalog.effortsByBase[baseId] ?? []) ?? ""
        if selectedEffortId != nextEffort {
            selectedEffortId = nextEffort
        }
    }

    private func selectEffort(_ effortId: String, baseId: String, catalog: ModelCatalog) {
        selectedEffortId = effortId
        let nextModelId = catalog.fullIdByBaseEffort[baseId]?[effortId] ?? composeModelId(base: baseId, effort: effortId)
        selectedModelId = nextModelId
    }
}

private struct ContextWindowSummary: Equatable {
    let percent: Int
    let usedLabel: String
    let windowLabel: String

    var text: String {
        "\(percent)% · \(usedLabel)/\(windowLabel)"
    }
}

private struct ContextWindowIndicator: View {
    let summary: ContextWindowSummary

    var body: some View {
        Text(summary.text)
            .font(.caption2.weight(.semibold))
            .foregroundColor(.ctxTextMuted)
            .accessibilityLabel("Context Window: \(summary.text)")
    }
}

struct ComposerToolIcon: View {
    let name: LucideIconName

    var body: some View {
        LucideIcon(name: name, size: 20)
            .foregroundColor(.ctxTextPrimary)
            .frame(width: CtxChatStyle.composerToolSize, height: CtxChatStyle.composerToolSize)
            .contentShape(Rectangle())
    }
}

struct ComposerHarnessIcon: View {
    let providerId: String?

    var body: some View {
        let base = RoundedRectangle(cornerRadius: 8, style: .continuous)
        if let providerId,
           let harness = HarnessCatalog.entry(for: providerId) {
            Image(harness.assetName)
                .resizable()
                .renderingMode(.original)
                .scaledToFit()
                .frame(width: 20, height: 20)
                .clipShape(base)
                .modifier(HarnessInvertModifier(shouldInvert: harness.invertInDark))
                .background(
                    base
                        .fill(Color.white.opacity(0.08))
                )
        } else {
            base
                .fill(Color.white.opacity(0.10))
                .frame(width: 20, height: 20)
                .overlay(
                    base.stroke(Color(red: 0.071, green: 0.071, blue: 0.071).opacity(0.8), lineWidth: 1)
                )
        }
    }
}

struct ComposerCircleButton: View {
    let icon: LucideIconName
    let accessibilityId: String?
    let action: () -> Void

    init(icon: LucideIconName, accessibilityId: String? = nil, action: @escaping () -> Void) {
        self.icon = icon
        self.accessibilityId = accessibilityId
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            LucideIcon(name: icon, size: 18)
                .foregroundColor(.white)
                .frame(width: CtxChatStyle.composerPrimarySize, height: CtxChatStyle.composerPrimarySize)
                .background(
                    Circle().fill(Color.ctxAccent)
                )
        }
        .accessibilityIdentifier(accessibilityId ?? "")
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
    @Published var errorMessage: String?
    @Published var artifacts: [Artifact] = []
    @Published var isArtifactsLoading = false
    @Published var artifactsError: String?
    @Published var turnStatus: TurnStatusSnapshot?
    @Published private(set) var isAssistantWorking = false
    @Published private(set) var contextWindowInfo: ContextWindowInfo?
    @Published private(set) var assetBaseURL: URL?
    @Published private(set) var assetToken: String?

    private struct StreamingAssistantState {
        let turnId: String
        let messageId: UUID
        var text: String
        var isActive: Bool
    }

    struct TurnStatusSnapshot: Equatable {
        let status: SessionTurnStatus
        let startedAt: String
        let updatedAt: String
    }

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
    private var secureContext: SecureConnectionContext?
    private var sessionId: String?
    private var workspaceId: String?
    private var lastEventSeq: Int?
    private var pollTask: _Concurrency.Task<Void, Never>?
    private var streamTask: _Concurrency.Task<Void, Never>?
    private var streamSocket: URLSessionWebSocketTask?
    private var isStreamConnected = false
    private var streamReconnectDelay: TimeInterval = 1
    private var refreshInFlight = false
    private var refreshPending = false
    private var artifactsRefreshInFlight = false
    private var artifactsRefreshPending = false
    private var pendingAssistantResponse = false
    private var streamingAssistantState: StreamingAssistantState?
    private var consecutivePollFailures = 0
    private var sessionGeneration = 0

    init(client: DaemonAPIClient? = nil, initialSessionId: String? = nil, initialWorkspaceId: String? = nil) {
        self.client = client
        self.sessionId = initialSessionId
        self.workspaceId = initialWorkspaceId
        if client == nil {
            messages = Self.sampleMessages
        }
    }

    var assetContext: DaemonAssetContext {
        DaemonAssetContext(baseURL: assetBaseURL, token: assetToken)
    }

    func setClient(_ client: DaemonAPIClient?) {
        self.client = client
        if client != nil {
            messages = []
            setPendingAssistantResponse(false)
            streamingAssistantState = nil
            errorMessage = nil
            artifacts = []
            artifactsError = nil
            isArtifactsLoading = false
            turnStatus = nil
            contextWindowInfo = nil
            lastEventSeq = nil
            refreshAssetContext()
            _Concurrency.Task { @MainActor in
                secureContext = await client?.secureConnectionContext()
            }
            if pollTask != nil {
                startStream()
            }
            _Concurrency.Task {
                _ = await refreshMessages()
                await refreshArtifacts()
            }
            updateWorkingState()
        } else {
            stopStream()
            sessionId = nil
            workspaceId = nil
            lastEventSeq = nil
            messages = Self.sampleMessages
            setPendingAssistantResponse(false)
            streamingAssistantState = nil
            artifacts = []
            artifactsError = nil
            isArtifactsLoading = false
            turnStatus = nil
            contextWindowInfo = nil
            assetBaseURL = nil
            assetToken = nil
            secureContext = nil
            updateWorkingState()
        }
    }

    private func refreshAssetContext() {
        guard let client else {
            assetBaseURL = nil
            assetToken = nil
            return
        }
        _Concurrency.Task { @MainActor in
            assetBaseURL = await client.daemonBaseURL()
            assetToken = await client.authToken()
        }
    }

    private func setPendingAssistantResponse(_ pending: Bool) {
        pendingAssistantResponse = pending
        updateWorkingState()
    }

    private func updateWorkingState() {
        let statusWorking = turnStatus?.status == .queued || turnStatus?.status == .running
        let streamingWorking = streamingAssistantState?.isActive == true
        isAssistantWorking = pendingAssistantResponse || statusWorking || streamingWorking
    }

    func selectSession(_ sessionId: String?, workspaceId: String? = nil) {
        sessionGeneration += 1
        self.sessionId = sessionId
        self.workspaceId = workspaceId
        lastEventSeq = nil
        setPendingAssistantResponse(false)
        streamingAssistantState = nil
        refreshInFlight = false
        refreshPending = false
        messages = []
        errorMessage = nil
        artifacts = []
        artifactsError = nil
        artifactsRefreshInFlight = false
        artifactsRefreshPending = false
        turnStatus = nil
        contextWindowInfo = nil
        updateWorkingState()
        _Concurrency.Task {
            await primeStreamCursor()
            _ = await refreshMessages()
            await refreshArtifacts()
            await sendStreamSubscriptionIfNeeded()
        }
    }

    func startPolling() {
        guard pollTask == nil else { return }
        startStream()
        pollTask = _Concurrency.Task {
            while !_Concurrency.Task.isCancelled {
                let result = await refreshMessages()
                let delay = nextPollDelay(for: result)
                try? await _Concurrency.Task.sleep(nanoseconds: delay)
            }
        }
    }

    func stopPolling() {
        pollTask?.cancel()
        pollTask = nil
        stopStream()
    }

    func interrupt() {
        guard let client else { return }
        _Concurrency.Task {
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            do {
                try await client.interruptSession(sessionId: resolved)
            } catch {
                // Best-effort interrupt; rely on stream updates to reflect status.
            }
        }
    }

    func send(_ text: String, attachments: [MessageAttachment]) {
        let local = ChatMessage(id: UUID(), role: .user, text: text, attachments: attachments)
        appendMessage(local)
        setPendingAssistantResponse(true)

        _Concurrency.Task {
            guard let client else { return }
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            do {
                _ = try await client.postMessage(sessionId: resolved, content: text, delivery: .immediate, attachments: attachments)
                _ = await refreshMessages()
            } catch {
                setPendingAssistantResponse(false)
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
        let generation = sessionGeneration
        let resolved = await resolveSessionId()
        guard let resolved, generation == sessionGeneration else { return .skipped }
        do {
            let items = try await client.listMessages(sessionId: resolved)
            guard generation == sessionGeneration, resolved == sessionId else { return .skipped }
            let nextMessages = items.map { summary in
                ChatMessage(
                    id: UUID(uuidString: summary.id) ?? UUID(),
                    role: summary.role == .user ? .user : .assistant,
                    text: summary.content,
                    attachments: summary.attachments ?? []
                )
            }
            let nextWithStreaming = applyStreamingAssistantState(to: nextMessages)
            messages = nextWithStreaming
            if pendingAssistantResponse, nextWithStreaming.contains(where: { $0.role == .assistant }) {
                setPendingAssistantResponse(false)
                streamingAssistantState = nil
                updateWorkingState()
            }
            errorMessage = nil
            if refreshPending, generation == sessionGeneration {
                refreshPending = false
                _Concurrency.Task { _ = await refreshMessages() }
            }
            return .success
        } catch {
            if generation == sessionGeneration {
                errorMessage = "Failed to load messages."
            }
            if refreshPending, generation == sessionGeneration {
                refreshPending = false
                _Concurrency.Task { _ = await refreshMessages() }
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
        let generation = sessionGeneration
        let resolved = await resolveSessionId()
        guard let resolved, generation == sessionGeneration else { return }
        do {
            artifacts = try await client.listSessionArtifacts(sessionId: resolved)
            artifactsError = nil
            if artifactsRefreshPending, generation == sessionGeneration {
                artifactsRefreshPending = false
                _Concurrency.Task { await refreshArtifacts() }
            }
        } catch {
            if generation == sessionGeneration {
                artifactsError = "Failed to load artifacts."
            }
            if artifactsRefreshPending, generation == sessionGeneration {
                artifactsRefreshPending = false
                _Concurrency.Task { await refreshArtifacts() }
            }
        }
    }

    private func appendMessage(_ message: ChatMessage) {
        var next = messages
        next.append(message)
        messages = next
    }

    private func applyStreamingAssistantState(to base: [ChatMessage]) -> [ChatMessage] {
        guard let state = streamingAssistantState, !state.text.isEmpty else { return base }
        var next = base

        if let last = next.last, last.role == .assistant {
            if last.text == state.text {
                return next
            }
            if state.text.count >= last.text.count || state.isActive {
                next[next.count - 1] = ChatMessage(id: last.id, role: .assistant, text: state.text, attachments: last.attachments)
            }
            return next
        }

        next.append(ChatMessage(id: state.messageId, role: .assistant, text: state.text, attachments: []))
        return next
    }

    private func applyTurnDelta(_ turn: SessionTurn) {
        let turnId = turn.turnId.stringValue
        let isActive = turn.status == .queued || turn.status == .running
        if isActive {
            setPendingAssistantResponse(true)
        }

        updateTurnStatus(from: turn)
        updateContextWindow(from: turn)

        guard let assistantPartial = turn.assistantPartial, !assistantPartial.isEmpty else {
            if !isActive, streamingAssistantState?.turnId == turnId {
                streamingAssistantState?.isActive = false
            }
            updateWorkingState()
            return
        }

        if streamingAssistantState?.turnId != turnId {
            let stableId = UUID(uuidString: turnId) ?? UUID()
            streamingAssistantState = StreamingAssistantState(
                turnId: turnId,
                messageId: stableId,
                text: assistantPartial,
                isActive: isActive
            )
        } else {
            streamingAssistantState?.text = assistantPartial
            streamingAssistantState?.isActive = isActive
        }

        messages = applyStreamingAssistantState(to: messages)
        updateWorkingState()
    }

    private func updateTurnStatus(from turn: SessionTurn) {
        turnStatus = TurnStatusSnapshot(
            status: turn.status,
            startedAt: turn.startedAt,
            updatedAt: turn.updatedAt
        )
        if turn.status == .completed || turn.status == .failed || turn.status == .interrupted {
            setPendingAssistantResponse(false)
        }
        updateWorkingState()
    }

    private func updateContextWindow(from turn: SessionTurn) {
        guard let info = parseContextWindowInfo(from: turn.metricsJson) else { return }
        contextWindowInfo = info
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
            let tracks = try await client.listTracks(workspaceId: workspace.id, taskId: task.id)
            guard let track = tracks.first else { return nil }
            let sessions = try await client.listSessions(workspaceId: workspace.id, trackId: track.id)
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

    private func primeStreamCursor(force: Bool = false) async {
        guard let client, let sessionId else { return }
        if !force, lastEventSeq != nil, workspaceId != nil { return }
        if let head = try? await client.getSessionHead(sessionId: sessionId, limit: 1, includeEvents: false) {
            lastEventSeq = head.lastEventSeq
            workspaceId = workspaceId ?? head.session.workspaceId.stringValue
            if let turn = mostRecentTurn(in: head.turns) {
                updateTurnStatus(from: turn)
                updateContextWindow(from: turn)
            }
        }
    }

    private func mostRecentTurn(in turns: [SessionTurn]) -> SessionTurn? {
        turns.max { left, right in
            let leftDate = parseIso(left.updatedAt) ?? parseIso(left.startedAt) ?? .distantPast
            let rightDate = parseIso(right.updatedAt) ?? parseIso(right.startedAt) ?? .distantPast
            return leftDate < rightDate
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
        streamTask = _Concurrency.Task {
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
        while !_Concurrency.Task.isCancelled {
            guard client != nil else {
                try? await _Concurrency.Task.sleep(nanoseconds: 1_000_000_000)
                continue
            }
            guard let workspaceId = await resolveWorkspaceId() else {
                try? await _Concurrency.Task.sleep(nanoseconds: 1_000_000_000)
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
            if _Concurrency.Task.isCancelled { break }
            let delay = min(streamReconnectDelay, 15)
            streamReconnectDelay = min(streamReconnectDelay * 2, 15)
            try? await _Concurrency.Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
        }
    }

    private func openStream(workspaceId: String) async throws -> URLSessionWebSocketTask {
        guard let client else { throw DaemonStreamError.invalidURL }
        await primeStreamCursor(force: true)
        let baseURL = await client.daemonBaseURL()
        if let secure = await client.secureConnectionContext() {
            let socket = try streamClient.connectSecureWorkspaceStream(baseURL: baseURL, workspaceId: workspaceId, deviceId: secure.deviceId)
            secureContext = secure
            streamSocket = socket
            streamReconnectDelay = 1
            await sendStreamSubscriptionIfNeeded()
            isStreamConnected = true
            return socket
        }
        let token = await client.authToken()
        let socket = try streamClient.connectWorkspaceStream(baseURL: baseURL, workspaceId: workspaceId, token: token)
        streamSocket = socket
        streamReconnectDelay = 1
        await sendStreamSubscriptionIfNeeded()
        isStreamConnected = true
        return socket
    }

    private func listenToStream(_ socket: URLSessionWebSocketTask) async {
        while !_Concurrency.Task.isCancelled {
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
        if let secureContext {
            let seq = await SecureSequenceStore.shared.next(for: secureContext.deviceId)
            guard let payload = try? streamEncoder.encode(message) else { return }
            guard let envelope = try? MobileE2EE.encryptPayload(
                deviceId: secureContext.deviceId,
                seq: seq,
                key: secureContext.key,
                plaintext: payload
            ) else { return }
            guard let wrapped = try? streamEncoder.encode(envelope),
                  let text = String(data: wrapped, encoding: .utf8) else { return }
            try? await streamClient.send(.string(text), via: socket)
            return
        }
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
        if let secureContext {
            guard let envelope = try? streamDecoder.decode(SecureEnvelope.self, from: data) else { return }
            guard envelope.deviceId == secureContext.deviceId else { return }
            guard let decrypted = try? MobileE2EE.decryptEnvelope(envelope, key: secureContext.key) else { return }
            guard let event = try? streamDecoder.decode(WorkspaceCatchupEvent.self, from: decrypted) else { return }
            handleStreamEvent(event)
            return
        }
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
                if let turn = delta.turn {
                    applyTurnDelta(turn)
                }
                if delta.event?.eventType == "artifacts_set" {
                    _Concurrency.Task { await refreshArtifacts() }
                }
                if let message = delta.message {
                    if message.role == .assistant {
                        setPendingAssistantResponse(false)
                        streamingAssistantState = nil
                        updateWorkingState()
                    }
                    _Concurrency.Task { _ = await refreshMessages() }
                } else if let turn = delta.turn, turn.status != .queued, turn.status != .running {
                    _Concurrency.Task { _ = await refreshMessages() }
                }
            }
        case .sessionSummary(_, _, let summary):
            let eventSessionId = summary.session.id.stringValue
            if eventSessionId == currentSessionId {
                if let lastEventSeq = summary.lastEventSeq {
                    self.lastEventSeq = lastEventSeq
                }
                _Concurrency.Task {
                    _ = await refreshMessages()
                    await refreshArtifacts()
                }
            }
        case .sessionGap(_, _, let sessionId, let afterSeq, _):
            if sessionId.stringValue == currentSessionId {
                lastEventSeq = afterSeq
                streamingAssistantState = nil
                updateWorkingState()
                _Concurrency.Task {
                    await primeStreamCursor(force: true)
                    _ = await refreshMessages()
                    await refreshArtifacts()
                }
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
            text: "Great. I will set up message bubbles and a composer that feels like ChatGPT.",
            attachments: []
        ),
    ]
}

private func extractModelIds(from models: JSONValue?) -> [String] {
    guard let models else { return [] }
    switch models {
    case .array(let values):
        let rawIds: [String] = values.compactMap { value in
            switch value {
            case .string(let string):
                return string
            case .object(let object):
                if case .string(let id) = object["modelId"] ?? object["id"] {
                    return id
                }
                return nil
            default:
                return nil
            }
        }
        return uniqueTrimmed(rawIds)
    case .object(let object):
        if let available = object["availableModels"] {
            return extractModelIds(from: available)
        }
        if let models = object["models"] {
            return extractModelIds(from: models)
        }
        if let choices = object["choices"] {
            return extractModelIds(from: choices)
        }
        if case .string(let current) = object["currentModelId"] {
            return uniqueTrimmed([current])
        }
        let keys = object.keys.filter { !$0.isEmpty && $0 != "choices" }.sorted()
        return keys
    default:
        return []
    }
}

private func uniqueTrimmed(_ values: [String]) -> [String] {
    var seen = Set<String>()
    var out: [String] = []
    for value in values {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !seen.contains(trimmed) else { continue }
        seen.insert(trimmed)
        out.append(trimmed)
    }
    return out
}

private let preferredEffortOrder: [String] = ["none", "minimal", "low", "medium", "high", "xhigh"]

private struct ParsedModelId {
    let full: String
    let base: String
    let effort: String?
}

private struct ModelCatalog {
    let baseIds: [String]
    let displayNameByBase: [String: String]
    let effortsByBase: [String: [String]]
    let fullIdByBaseEffort: [String: [String: String]]
}

private func splitOnLastSlash(_ fullModelId: String) -> (full: String, base: String, suffix: String?) {
    let full = fullModelId.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !full.isEmpty else { return ("", "", nil) }
    guard let idx = full.lastIndex(of: "/"), idx != full.startIndex else {
        return (full, full, nil)
    }
    let base = String(full[..<idx])
    let suffix = String(full[full.index(after: idx)...]).trimmingCharacters(in: .whitespacesAndNewlines)
    return (full, base, suffix.isEmpty ? nil : suffix)
}

private func normalizeEffortIdForCompare(_ value: String) -> String {
    value.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
}

private func isPreferredEffortId(_ value: String) -> Bool {
    let normalized = normalizeEffortIdForCompare(value)
    return preferredEffortOrder.contains(normalized)
}

private func hasTrailingParenSuffix(name: String, suffix: String) -> Bool {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.hasSuffix(")") else { return false }
    guard let openIdx = trimmed.lastIndex(of: "("),
          openIdx < trimmed.index(before: trimmed.endIndex) else {
        return false
    }
    let inner = String(trimmed[trimmed.index(after: openIdx)..<trimmed.index(before: trimmed.endIndex)])
    return normalizeEffortIdForCompare(inner) == normalizeEffortIdForCompare(suffix)
}

private func stripTrailingParenIfEffort(_ name: String, effortIds: [String]) -> String {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard trimmed.hasSuffix(")") else { return trimmed }
    guard let openIdx = trimmed.lastIndex(of: "("),
          openIdx < trimmed.index(before: trimmed.endIndex) else {
        return trimmed
    }
    let inner = String(trimmed[trimmed.index(after: openIdx)..<trimmed.index(before: trimmed.endIndex)])
    let normalizedInner = normalizeEffortIdForCompare(inner)
    let isEffort = effortIds.contains { normalizeEffortIdForCompare($0) == normalizedInner }
    return isEffort ? String(trimmed[..<openIdx]).trimmingCharacters(in: .whitespacesAndNewlines) : trimmed
}

private func orderEffortIds(_ efforts: Set<String>) -> [String] {
    let list = efforts.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
    let orderIndex: (String) -> Int = { value in
        let normalized = normalizeEffortIdForCompare(value)
        return preferredEffortOrder.firstIndex(of: normalized) ?? Int.max
    }
    return list.sorted { left, right in
        let leftIndex = orderIndex(left)
        let rightIndex = orderIndex(right)
        if leftIndex != rightIndex {
            return leftIndex < rightIndex
        }
        return left.localizedCaseInsensitiveCompare(right) == .orderedAscending
    }
}

private func buildModelCatalog(_ modelIds: [String]) -> ModelCatalog {
    var baseIdsSet = Set<String>()
    var rawEffortsByBase: [String: Set<String>] = [:]
    var rawNamesByBase: [String: [String]] = [:]
    var displayNameByBase: [String: String] = [:]
    var fullIdByBaseEffort: [String: [String: String]] = [:]

    for id in modelIds {
        let trimmedId = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedId.isEmpty else { continue }
        let name = trimmedId
        let parts = splitOnLastSlash(trimmedId)
        guard !parts.base.isEmpty else { continue }

        let effort = parts.suffix.flatMap { suffix in
            (isPreferredEffortId(suffix) || hasTrailingParenSuffix(name: name, suffix: suffix)) ? suffix : nil
        }
        let base = effort == nil ? trimmedId : parts.base

        baseIdsSet.insert(base)
        rawNamesByBase[base, default: []].append(name)
        if let effort {
            rawEffortsByBase[base, default: []].insert(effort)
            var map = fullIdByBaseEffort[base] ?? [:]
            map[effort] = trimmedId
            fullIdByBaseEffort[base] = map
        }
    }

    let baseIds = baseIdsSet.sorted { $0.localizedCaseInsensitiveCompare($1) == .orderedAscending }
    var effortsByBase: [String: [String]] = [:]
    for base in baseIds {
        let ordered = rawEffortsByBase[base].map(orderEffortIds) ?? []
        let efforts = ordered.count >= 2 ? ordered : []
        effortsByBase[base] = efforts

        if !efforts.isEmpty {
            displayNameByBase[base] = base
            continue
        }

        let names = rawNamesByBase[base] ?? []
        let stripped = names.map { stripTrailingParenIfEffort($0, effortIds: efforts) }
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        displayNameByBase[base] = stripped.first ?? base
    }

    return ModelCatalog(
        baseIds: baseIds,
        displayNameByBase: displayNameByBase,
        effortsByBase: effortsByBase,
        fullIdByBaseEffort: fullIdByBaseEffort
    )
}

private func parseModelId(_ fullModelId: String, catalog: ModelCatalog?) -> ParsedModelId {
    let parts = splitOnLastSlash(fullModelId)
    guard !parts.full.isEmpty else { return ParsedModelId(full: "", base: "", effort: nil) }
    guard let suffix = parts.suffix else {
        return ParsedModelId(full: parts.full, base: parts.full, effort: nil)
    }

    if let catalog {
        let options = catalog.effortsByBase[parts.base] ?? []
        if options.contains(suffix) {
            return ParsedModelId(full: parts.full, base: parts.base, effort: suffix)
        }
        if catalog.baseIds.contains(parts.full) {
            return ParsedModelId(full: parts.full, base: parts.full, effort: nil)
        }
    }

    if isPreferredEffortId(suffix) {
        return ParsedModelId(full: parts.full, base: parts.base, effort: suffix)
    }
    return ParsedModelId(full: parts.full, base: parts.full, effort: nil)
}

private func composeModelId(base: String, effort: String?) -> String {
    let trimmed = base.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return "" }
    guard let effort, !effort.isEmpty else { return trimmed }
    return "\(trimmed)/\(effort)"
}

private func formatEffortLabel(_ effort: String?) -> String {
    let raw = effort?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    guard !raw.isEmpty else { return "" }
    let normalized = raw.lowercased()
    if normalized == "xhigh" || normalized == "extra_high" || normalized == "extra-high" {
        return "Extra High"
    }
    return normalized.prefix(1).uppercased() + normalized.dropFirst()
}

private func pickDefaultEffort(_ efforts: [String]) -> String? {
    if let medium = efforts.first(where: { normalizeEffortIdForCompare($0) == "medium" }) {
        return medium
    }
    return efforts.first
}

private func deriveFullModelIdForBase(catalog: ModelCatalog, baseId: String, preferredEffort: String?) -> String {
    let efforts = catalog.effortsByBase[baseId] ?? []
    guard !efforts.isEmpty else { return baseId }
    let resolvedEffort = preferredEffort.flatMap { pref in
        efforts.first(where: { normalizeEffortIdForCompare($0) == normalizeEffortIdForCompare(pref) })
    } ?? pickDefaultEffort(efforts)
    guard let effort = resolvedEffort else { return baseId }
    if let fullId = catalog.fullIdByBaseEffort[baseId]?[effort] {
        return fullId
    }
    return composeModelId(base: baseId, effort: effort)
}

private func contextWindowSummary(from info: ContextWindowInfo?) -> ContextWindowSummary? {
    guard let info, let windowTokensRaw = info.windowTokens, windowTokensRaw.isFinite else { return nil }
    let windowTokens = max(1, Int(round(windowTokensRaw)))

    let usedTokens: Double?
    if let used = info.usedTokens, used.isFinite {
        usedTokens = used
    } else if let remaining = info.remainingTokens, remaining.isFinite {
        usedTokens = Double(windowTokens) - remaining
    } else if let remainingFraction = info.remainingFraction, remainingFraction.isFinite {
        usedTokens = Double(windowTokens) * (1 - remainingFraction)
    } else {
        return nil
    }

    let clampedUsed = max(0, min(Double(windowTokens), round(usedTokens ?? 0)))
    let percentValue = Int(round((clampedUsed / Double(windowTokens)) * 100))
    let percent = max(0, min(100, percentValue))
    let usedLabel = formatUsedTokenCount(clampedUsed)
    let windowLabel = formatTokenCount(Double(windowTokens))
    return ContextWindowSummary(percent: percent, usedLabel: usedLabel, windowLabel: windowLabel)
}

private func formatTokenCount(_ value: Double) -> String {
    guard value.isFinite else { return "0" }
    if value >= 1_000_000 {
        let scaled = value / 1_000_000
        let fixed = scaled >= 10 ? String(format: "%.0f", scaled) : String(format: "%.1f", scaled)
        return "\(trimTrailingZero(fixed))m"
    }
    if value >= 1_000 {
        let scaled = value / 1_000
        let fixed = scaled >= 100 ? String(format: "%.0f", scaled) : String(format: "%.1f", scaled)
        return "\(trimTrailingZero(fixed))k"
    }
    return "\(Int(round(value)))"
}

private func formatUsedTokenCount(_ value: Double) -> String {
    guard value.isFinite else { return "0" }
    if value >= 1_000_000 {
        let scaled = value / 1_000_000
        let fixed = scaled >= 10 ? String(format: "%.0f", scaled) : String(format: "%.1f", scaled)
        return "\(trimTrailingZero(fixed))m"
    }
    if value >= 1_000 {
        let rounded = Int(round(value / 1_000))
        return "\(rounded)k"
    }
    return "\(Int(round(value)))"
}

private func trimTrailingZero(_ value: String) -> String {
    value.hasSuffix(".0") ? String(value.dropLast(2)) : value
}

private func parseContextWindowInfo(from metrics: JSONValue?) -> ContextWindowInfo? {
    guard let metrics else { return nil }
    if let object = objectValue(from: metrics) {
        if let nested = object["context_window"]
            ?? object["contextWindow"]
            ?? object["context_window_info"]
            ?? object["contextWindowInfo"] {
            return parseContextWindowObject(from: nested)
        }
        return parseContextWindowObject(from: metrics)
    }
    return nil
}

private func parseContextWindowObject(from value: JSONValue) -> ContextWindowInfo? {
    guard let object = objectValue(from: value) else { return nil }
    let windowTokens = numberValue(from: object["window_tokens"] ?? object["windowTokens"] ?? object["context_window_tokens"] ?? object["contextWindowTokens"])
    let usedTokens = numberValue(from: object["used_tokens"] ?? object["usedTokens"])
    let remainingTokens = numberValue(from: object["remaining_tokens"] ?? object["remainingTokens"])
    let remainingFraction = numberValue(from: object["remaining_fraction"] ?? object["remainingFraction"])
    if windowTokens == nil && usedTokens == nil && remainingTokens == nil && remainingFraction == nil {
        return nil
    }
    return ContextWindowInfo(
        windowTokens: windowTokens,
        usedTokens: usedTokens,
        remainingTokens: remainingTokens,
        remainingFraction: remainingFraction
    )
}

private func numberValue(from value: JSONValue?) -> Double? {
    switch value {
    case .number(let number):
        return number
    case .string(let string):
        return Double(string)
    default:
        return nil
    }
}

private func objectValue(from value: JSONValue?) -> [String: JSONValue]? {
    if case .object(let object) = value {
        return object
    }
    return nil
}

#Preview {
    ChatView(viewModel: ChatViewModel())
}
