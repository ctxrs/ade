import AVKit
import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import UIKit
import _Concurrency

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
                    isFocused: $isComposerFocused,
                    onSend: sendMessage
                )
            }
            .padding(.horizontal, CtxChatStyle.composerOuterPadding)
            .padding(.top, 8)
            .padding(.bottom, 8)
            .background(Color.ctxBackground.ignoresSafeArea(edges: .bottom))
        }
        .onAppear { viewModel.startPolling() }
        .onDisappear { viewModel.stopPolling() }
        .onChange(of: selectedPhotos) { newItems in
            _Concurrency.Task { await loadAttachments(from: newItems) }
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
            .onChange(of: session.id) { newSessionId in
                viewModel.selectSession(newSessionId, workspaceId: session.workspaceId)
            }
            .onChange(of: session.workspaceId) { _ in
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
                Text(message.text)
                    .font(CtxChatStyle.bodyFont)
                    .foregroundColor(.ctxTextPrimary)
                    .lineSpacing(CtxChatStyle.bodyLineSpacing)
                    .accessibilityIdentifier("chat.message.text.\(message.role == .assistant ? "assistant" : "user")")
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
        return "\(hours)h \(String(format: "%02d", minutes))m"
    }
    if minutes > 0 {
        return "\(minutes)m \(String(format: "%02d", seconds))s"
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
    var isFocused: FocusState<Bool>.Binding
    var onSend: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            ZStack(alignment: .topLeading) {
                if text.isEmpty {
                    Text("Ask anything")
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

            HStack(spacing: 14) {
                PhotosPicker(selection: $selectedPhotos, matching: .images) {
                    ComposerToolIcon(systemName: "plus")
                }

                Spacer(minLength: 0)

                if isSendEnabled {
                    ComposerCircleButton(systemName: "arrow.up", accessibilityId: "chat.composer.send") {
                        onSend()
                    }
                } else {
                    Button {} label: {
                        ComposerToolIcon(systemName: "mic")
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
    }
}

struct ComposerToolIcon: View {
    let systemName: String

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: 20, weight: .semibold))
            .foregroundColor(.ctxTextPrimary)
            .frame(width: CtxChatStyle.composerToolSize, height: CtxChatStyle.composerToolSize)
            .contentShape(Rectangle())
    }
}

struct ComposerCircleButton: View {
    let systemName: String
    let accessibilityId: String?
    let action: () -> Void

    init(systemName: String, accessibilityId: String? = nil, action: @escaping () -> Void) {
        self.systemName = systemName
        self.accessibilityId = accessibilityId
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 18, weight: .semibold))
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
            pendingAssistantResponse = false
            streamingAssistantState = nil
            errorMessage = nil
            artifacts = []
            artifactsError = nil
            isArtifactsLoading = false
            turnStatus = nil
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
        } else {
            stopStream()
            sessionId = nil
            workspaceId = nil
            lastEventSeq = nil
            messages = Self.sampleMessages
            pendingAssistantResponse = false
            streamingAssistantState = nil
            artifacts = []
            artifactsError = nil
            isArtifactsLoading = false
            turnStatus = nil
            assetBaseURL = nil
            assetToken = nil
            secureContext = nil
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

    func selectSession(_ sessionId: String?, workspaceId: String? = nil) {
        sessionGeneration += 1
        self.sessionId = sessionId
        self.workspaceId = workspaceId
        lastEventSeq = nil
        pendingAssistantResponse = false
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

    func send(_ text: String, attachments: [MessageAttachment]) {
        let local = ChatMessage(id: UUID(), role: .user, text: text, attachments: attachments)
        appendMessage(local)
        pendingAssistantResponse = true

        _Concurrency.Task {
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
                pendingAssistantResponse = false
                streamingAssistantState = nil
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
            pendingAssistantResponse = true
        }

        updateTurnStatus(from: turn)

        guard let assistantPartial = turn.assistantPartial, !assistantPartial.isEmpty else {
            if !isActive, streamingAssistantState?.turnId == turnId {
                streamingAssistantState?.isActive = false
            }
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
    }

    private func updateTurnStatus(from turn: SessionTurn) {
        turnStatus = TurnStatusSnapshot(
            status: turn.status,
            startedAt: turn.startedAt,
            updatedAt: turn.updatedAt
        )
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
                        pendingAssistantResponse = false
                        streamingAssistantState = nil
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

#Preview {
    ChatView(viewModel: ChatViewModel())
}
