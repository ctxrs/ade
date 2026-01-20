import AVKit
import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers
import UIKit
import _Concurrency

enum ComposerMode: String, CaseIterable, Identifiable {
    case `default`
    case research
    case plan
    case review

    var id: String { rawValue }

    var label: String {
        switch self {
        case .default:
            return "Default"
        case .research:
            return "Research"
        case .plan:
            return "Plan"
        case .review:
            return "Review"
        }
    }
}

enum ComposerVerbosity: String, CaseIterable, Identifiable {
    case terse
    case `default`
    case verbose

    var id: String { rawValue }

    var label: String {
        switch self {
        case .terse:
            return "Terse"
        case .default:
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
    @State private var lastDraftSessionId: String?
    @State private var pendingAttachments: [MessageAttachment] = []
    @State private var selectedPhotos: [PhotosPickerItem] = []
    @State private var expandedToolGroups: Set<String> = []
    @State private var expandedTools: Set<String> = []
    @State private var isNearBottom = true
    @FocusState private var isComposerFocused: Bool
    private let scrollAnchorId = "chat.scroll.anchor"

    init(
        viewModel: ChatViewModel,
        showsBackground: Bool = true,
        providerId: String? = nil,
        modelOptions: [String] = [],
        isModelLoading: Bool = false,
        selectedModelId: Binding<String> = .constant(""),
        selectedEffortId: Binding<String> = .constant(""),
        selectedMode: Binding<ComposerMode> = .constant(.default),
        selectedVerbosity: Binding<ComposerVerbosity> = .constant(.default)
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
            lastDraftSessionId = viewModel.sessionId
            composerText = ChatDraftStore.load(sessionId: viewModel.sessionId)
        }
        .onDisappear { viewModel.stopPolling() }
        .onChange(of: viewModel.sessionId) { _, newValue in
            ChatDraftStore.save(composerText, sessionId: lastDraftSessionId)
            lastDraftSessionId = newValue
            composerText = ChatDraftStore.load(sessionId: newValue)
        }
        .onChange(of: composerText) { _, newValue in
            ChatDraftStore.save(newValue, sessionId: viewModel.sessionId)
        }
        .onChange(of: selectedPhotos) { newItems in
            _Concurrency.Task { await loadAttachments(from: newItems) }
        }
        .onChange(of: modelOptions) { _ in
            syncEffortSelection()
        }
        .onChange(of: selectedModelId) { _, newValue in
            syncEffortSelection()
            viewModel.updateSessionModel(newValue)
        }
        .onChange(of: selectedMode) { _, newValue in
            viewModel.updateSessionMode(newValue)
        }
    }

    private var messageList: some View {
        GeometryReader { geometry in
            let horizontalPadding = CtxChatStyle.horizontalPadding
            let availableWidth = max(0, geometry.size.width - (horizontalPadding * 2))
            let maxBubbleWidth = min(CtxChatStyle.userBubbleMaxWidth, availableWidth * CtxChatStyle.userBubbleWidthFraction)
            let assetContext = viewModel.assetContext
            let scrollViewHeight = geometry.size.height
            let displayItems = viewModel.threadItems.filter { item in
                if case .toolGroup = item.kind {
                    return selectedVerbosity != .terse
                }
                return true
            }
            ScrollViewReader { proxy in
                ZStack(alignment: .bottomTrailing) {
                    ScrollView {
                        LazyVStack(spacing: CtxChatStyle.messageSpacing) {
                            if viewModel.isArchivedSession, viewModel.historyHasMore || viewModel.isHistoryLoading {
                                HStack {
                                    if viewModel.isHistoryLoading {
                                        ProgressView()
                                            .tint(.ctxAccent)
                                    } else {
                                        Button("Load earlier messages") {
                                            viewModel.loadEarlierMessages()
                                        }
                                        .font(.caption.weight(.semibold))
                                        .foregroundColor(.ctxTextPrimary)
                                        .padding(.vertical, 6)
                                        .padding(.horizontal, 12)
                                        .background(
                                            Color.ctxSurface.opacity(0.6),
                                            in: RoundedRectangle(cornerRadius: 12, style: .continuous)
                                        )
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 12, style: .continuous)
                                                .stroke(Color.ctxLine, lineWidth: 1)
                                        )
                                    }
                                }
                                .frame(maxWidth: .infinity)
                                .padding(.top, 2)
                            }

                            if !viewModel.queueMessages.isEmpty {
                                QueuePanel(
                                    messages: viewModel.queueMessages,
                                    onRemove: { messageId in
                                        viewModel.removeQueuedMessage(messageId)
                                    }
                                )
                                .padding(.top, 2)
                            }

                            ForEach(displayItems) { item in
                                switch item.kind {
                                case .message(let message):
                                    MessageRow(
                                        message: message,
                                        maxBubbleWidth: maxBubbleWidth,
                                        assetContext: assetContext
                                    )
                                        .id(item.id)
                                case .toolGroup(let group):
                                    let expanded = selectedVerbosity == .verbose || expandedToolGroups.contains(group.id)
                                    ToolGroupRow(
                                        group: group,
                                        expanded: expanded,
                                        toolsLoading: viewModel.toolLoadingTurnIds.contains(group.turnId),
                                        toolDetails: viewModel.toolDetailsByCallId,
                                        verbosity: selectedVerbosity,
                                        expandedTools: expandedTools,
                                        onToggleGroup: {
                                            if selectedVerbosity == .verbose { return }
                                            if expandedToolGroups.contains(group.id) {
                                                expandedToolGroups.remove(group.id)
                                            } else {
                                                expandedToolGroups.insert(group.id)
                                            }
                                        },
                                        onToggleTool: { toolId in
                                            if expandedTools.contains(toolId) {
                                                expandedTools.remove(toolId)
                                            } else {
                                                expandedTools.insert(toolId)
                                            }
                                        },
                                        onRequestTools: {
                                            viewModel.loadTools(for: group.turnId)
                                        }
                                    )
                                    .id(item.id)
                                case .askUser(let question):
                                    AskUserQuestionCard(
                                        question: question,
                                        active: question.toolCallId == viewModel.activeAskToolCallId,
                                        onSubmit: { answers in
                                            await viewModel.submitAskUserQuestion(toolCallId: question.toolCallId, answers: answers)
                                        },
                                        onCancel: {
                                            await viewModel.cancelAskUserQuestion(toolCallId: question.toolCallId)
                                        }
                                    )
                                    .id(item.id)
                                }
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

                        GeometryReader { proxy in
                            Color.clear.preference(
                                key: ScrollToBottomPreferenceKey.self,
                                value: proxy.frame(in: .named("chatScroll")).maxY
                            )
                            .id(scrollAnchorId)
                        }
                        .frame(height: 1)
                    }
                    .coordinateSpace(name: "chatScroll")
                    .scrollDismissesKeyboard(.interactively)
                    .accessibilityIdentifier("chat.messages")
                    .onPreferenceChange(ScrollToBottomPreferenceKey.self) { bottomY in
                        let threshold: CGFloat = 24
                        let nearBottom = bottomY <= scrollViewHeight + threshold
                        if nearBottom != isNearBottom {
                            isNearBottom = nearBottom
                        }
                    }
                    .onAppear {
                        scrollToBottom(proxy: proxy, animated: false)
                    }
                    .onChange(of: viewModel.threadItemsRevision) { _, _ in
                        if viewModel.isArchivedSession && viewModel.isHistoryLoading {
                            return
                        }
                        scrollToBottom(proxy: proxy, animated: true)
                    }
                    .onChange(of: viewModel.turnStatus) { _, _ in
                        scrollToBottom(proxy: proxy, animated: true)
                    }
                    .onChange(of: isComposerFocused) { _, focused in
                        guard focused else { return }
                        scrollToBottom(proxy: proxy, animated: true)
                    }

                    if !isNearBottom {
                        ScrollToBottomButton {
                            scrollToBottom(proxy: proxy, animated: true)
                        }
                        .padding(.trailing, CtxChatStyle.composerOuterPadding)
                        .padding(.bottom, scrollToBottomButtonBottomPadding)
                        .transition(.opacity)
                    }
                }
            }
        }
    }

    private var isSendEnabled: Bool {
        !composerText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !pendingAttachments.isEmpty
    }

    private var scrollToBottomButtonBottomPadding: CGFloat {
        CtxChatStyle.composerOuterPadding + CtxChatStyle.composerPrimarySize + 28
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
        ChatDraftStore.save("", sessionId: viewModel.sessionId)
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
        let target = scrollAnchorId
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
    let isArchived: Bool
    @StateObject private var viewModel: ChatViewModel
    @Binding var activePanel: WorkbenchPanel?
    @Binding var artifactCount: Int
    @State private var availableModels: [String] = []
    @State private var isLoadingModels = false
    @State private var selectedModelId: String
    @State private var selectedEffortId: String = ""
    @State private var selectedMode: ComposerMode = .default
    @State private var selectedVerbosity: ComposerVerbosity = .default

    init(session: SessionSummary, isArchived: Bool, activePanel: Binding<WorkbenchPanel?>, artifactCount: Binding<Int>) {
        self.session = session
        self.isArchived = isArchived
        _activePanel = activePanel
        _artifactCount = artifactCount
        _selectedModelId = State(initialValue: session.modelId)
        _viewModel = StateObject(
            wrappedValue: ChatViewModel(
                initialSessionId: session.id,
                initialWorkspaceId: session.workspaceId
            )
        )
    }

    private var isArtifactsPresented: Binding<Bool> {
        Binding(
            get: { activePanel == .artifacts },
            set: { newValue in
                if newValue {
                    activePanel = .artifacts
                } else if activePanel == .artifacts {
                    activePanel = nil
                }
            }
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
                viewModel.setArchived(isArchived)
                viewModel.selectSession(session.id, workspaceId: session.workspaceId)
                artifactCount = viewModel.artifacts.count
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: session.id) { newSessionId in
                artifactCount = 0
                viewModel.setArchived(isArchived)
                viewModel.selectSession(newSessionId, workspaceId: session.workspaceId)
                selectedModelId = session.modelId
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: session.workspaceId) { _ in
                viewModel.setArchived(isArchived)
                viewModel.selectSession(session.id, workspaceId: session.workspaceId)
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: isArchived) { _, newValue in
                viewModel.setArchived(newValue)
            }
            .onChange(of: session.providerId) { _ in
                _Concurrency.Task { await loadModels() }
            }
            .onChange(of: session.modelId) { _, newModelId in
                if !newModelId.isEmpty {
                    selectedModelId = newModelId
                }
            }
            .onChange(of: viewModel.artifacts.count) { _, newCount in
                artifactCount = newCount
            }
            .sheet(isPresented: isArtifactsPresented) {
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
    @State private var isExpanded: Bool
    @State private var copied = false

    private static let userMessageLineLimit = 4
    private static let userMessageLengthLimit = 280

    init(message: ChatMessage, maxBubbleWidth: CGFloat, assetContext: DaemonAssetContext) {
        self.message = message
        self.maxBubbleWidth = maxBubbleWidth
        self.assetContext = assetContext
        let isLong = message.role == .user && Self.isLongUserMessage(message.text)
        _isExpanded = State(initialValue: !isLong)
    }

    var body: some View {
        let isAssistantSide = message.role == .assistant || message.role == .system
        bubble
            .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("chat.message.\(isAssistantSide ? "assistant" : "user")")
    }

    private var bubble: some View {
        VStack(alignment: .leading, spacing: 8) {
            if message.role == .system {
                Text("System")
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
            }
            if !message.text.isEmpty {
                if message.role == .user {
                    userMessageText
                } else {
                    MarkdownContentView(text: message.text, isMuted: message.role == .system)
                        .accessibilityIdentifier("chat.message.text.assistant")
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

    private var userMessageText: some View {
        let trimmedText = message.text.trimmingCharacters(in: .whitespacesAndNewlines)
        let isLong = Self.isLongUserMessage(message.text)
        let showCopyButton = !trimmedText.isEmpty
        return Text(message.text)
            .font(CtxChatStyle.bodyFont)
            .foregroundColor(.ctxTextPrimary)
            .lineSpacing(CtxChatStyle.bodyLineSpacing)
            .lineLimit(isExpanded ? nil : Self.userMessageLineLimit)
            .padding(.trailing, showCopyButton ? 26 : 0)
            .mask(userMessageMask(isLong: isLong))
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .onTapGesture {
                guard isLong else { return }
                withAnimation(.easeOut(duration: 0.2)) {
                    isExpanded.toggle()
                }
            }
            .overlay(alignment: .topTrailing) {
                if showCopyButton {
                    userCopyButton
                }
            }
            .accessibilityIdentifier("chat.message.text.user")
    }

    private func userMessageMask(isLong: Bool) -> some View {
        Group {
            if isExpanded || !isLong {
                Rectangle()
            } else {
                LinearGradient(
                    gradient: Gradient(stops: [
                        .init(color: .black, location: 0),
                        .init(color: .black, location: 0.7),
                        .init(color: .clear, location: 1),
                    ]),
                    startPoint: .top,
                    endPoint: .bottom
                )
            }
        }
    }

    private var userCopyButton: some View {
        Button(action: copyUserMessage) {
            LucideIcon(name: copied ? .check : .copy, size: 14)
                .foregroundColor(.ctxTextMuted)
                .frame(width: 24, height: 24)
                .background(Color.white.opacity(0.05), in: RoundedRectangle(cornerRadius: 6, style: .continuous))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(copied ? "Copied" : "Copy message")
    }

    private func copyUserMessage() {
        let trimmed = message.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        UIPasteboard.general.string = trimmed
        copied = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            copied = false
        }
    }

    private static func isLongUserMessage(_ text: String) -> Bool {
        if text.count > userMessageLengthLimit {
            return true
        }
        let lineCount = text.split(whereSeparator: \.isNewline).count
        return lineCount > userMessageLineLimit
    }
}

private struct MarkdownContentView: View {
    let text: String
    var isMuted: Bool = false

    var body: some View {
        let segments = parseMarkdownSegments(text)
        VStack(alignment: .leading, spacing: 8) {
            ForEach(segments) { segment in
                switch segment.kind {
                case .text(let value):
                    MarkdownTextBlock(text: value, isMuted: isMuted)
                case .code(let code, let language):
                    CodeBlockView(code: code, language: language)
                }
            }
        }
    }
}

private struct MarkdownTextBlock: View {
    let text: String
    let isMuted: Bool

    var body: some View {
        let color = isMuted ? Color.ctxTextMuted : Color.ctxTextPrimary
        if let attributed = styledMarkdownAttributedString(text, isMuted: isMuted) {
            Text(attributed)
                .font(CtxChatStyle.bodyFont)
                .lineSpacing(CtxChatStyle.bodyLineSpacing)
        } else {
            Text(text)
                .font(CtxChatStyle.bodyFont)
                .foregroundColor(color)
                .lineSpacing(CtxChatStyle.bodyLineSpacing)
        }
    }
}

private func styledMarkdownAttributedString(_ text: String, isMuted: Bool) -> AttributedString? {
    let normalized = text
        .replacingOccurrences(of: "\r\n", with: "\n")
        .replacingOccurrences(of: "\n", with: "  \n")
    guard var attributed = try? AttributedString(
        markdown: normalized,
        options: AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
    ) else {
        return nil
    }
    let inlineColor = UIColor(isMuted ? Color.ctxTextMuted : Color.ctxTextPrimary)
    var baseContainer = AttributeContainer()
    baseContainer.foregroundColor = inlineColor
    attributed.mergeAttributes(baseContainer)
    let background = UIColor(Color.ctxSurfaceRaised)
    let monoFont = UIFont.monospacedSystemFont(ofSize: 14, weight: .regular)

    for run in attributed.runs {
        guard let intent = run.inlinePresentationIntent, intent.contains(.code) else {
            continue
        }
        var container = AttributeContainer()
        container.foregroundColor = inlineColor
        container.backgroundColor = background
        container.font = monoFont
        attributed[run.range].mergeAttributes(container)
    }
    return attributed
}

private struct MarkdownSegment: Identifiable {
    enum Kind {
        case text(String)
        case code(String, String?)
    }

    let id = UUID()
    let kind: Kind
}

private struct CodeBlockView: View {
    let code: String
    let language: String?
    @State private var copied = false

    var body: some View {
        let trimmedCode = code.trimmingCharacters(in: .newlines)
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                if let language, !language.isEmpty {
                    Text(language.uppercased())
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.ctxTextMuted)
                } else {
                    Text("CODE")
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.ctxTextMuted)
                }
                Spacer(minLength: 0)
                Button {
                    UIPasteboard.general.string = trimmedCode
                    copied = true
                    DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                        copied = false
                    }
                } label: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc")
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(copied ? "Copied" : "Copy code")
            }
            ScrollView(.horizontal, showsIndicators: false) {
                Text(trimmedCode.isEmpty ? " " : trimmedCode)
                    .font(.system(size: 14, weight: .regular, design: .monospaced))
                    .foregroundColor(.ctxTextPrimary)
                    .lineSpacing(4)
                    .padding(.bottom, 2)
            }
        }
        .padding(12)
        .background(Color.ctxSurfaceRaised, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct ChatTurnStatusRow: View {
    let status: ChatViewModel.TurnStatusSnapshot
    @State private var copied = false

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
        let assistantMessage = status.assistantMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        let showCopyButton = status.status == .completed && !assistantMessage.isEmpty
        HStack(spacing: 6) {
            Text(label)
            Text("·")
            Text(elapsed)
            if showCopyButton {
                Text("·")
                Button(action: copyAssistantMessage) {
                    LucideIcon(name: copied ? .check : .copy, size: 12)
                        .foregroundColor(.ctxTextSecondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(copied ? "Copied" : "Copy response")
            }
        }
        .font(.system(size: 12, weight: .semibold))
        .foregroundColor(.ctxTextSecondary)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func copyAssistantMessage() {
        let trimmed = status.assistantMessage.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        UIPasteboard.general.string = trimmed
        copied = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            copied = false
        }
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
        case .assistant, .system:
            content
                .padding(.vertical, 2)
        case .user:
            content
                .padding(.vertical, CtxChatStyle.userBubbleVerticalPadding)
                .padding(.horizontal, CtxChatStyle.userBubbleHorizontalPadding)
                .frame(maxWidth: maxBubbleWidth, alignment: .leading)
                .background(
                    Color.ctxBubbleUser,
                    in: RoundedRectangle(cornerRadius: CtxChatStyle.userBubbleCornerRadius, style: .continuous)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: CtxChatStyle.userBubbleCornerRadius, style: .continuous)
                        .stroke(Color.ctxLine, lineWidth: 1)
                )
        }
    }
}

private struct QueuePanel: View {
    let messages: [MessageSummary]
    let onRemove: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Queued messages (\(messages.count))")
                .font(.caption.weight(.semibold))
                .foregroundColor(.ctxTextSecondary)
            ForEach(messages) { message in
                HStack(alignment: .top, spacing: 8) {
                    Text(message.content)
                        .font(.footnote)
                        .foregroundColor(.ctxTextPrimary)
                        .lineLimit(2)
                    Spacer(minLength: 0)
                    Button("Remove") {
                        onRemove(message.id)
                    }
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxAccent)
                }
            }
        }
        .padding(12)
        .background(Color.ctxSurface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct ToolGroupRow: View {
    let group: ChatToolGroup
    let expanded: Bool
    let toolsLoading: Bool
    let toolDetails: [String: SessionTurnTool]
    let verbosity: ComposerVerbosity
    let expandedTools: Set<String>
    let onToggleGroup: () -> Void
    let onToggleTool: (String) -> Void
    let onRequestTools: () -> Void

    var body: some View {
        let total = max(group.toolTotal, group.tools.count)
        let label = toolGroupLabel(total: total, running: group.toolRunning, failed: group.toolFailed, thought: group.thought)
        let hasDetails = total > 0 || !group.thought.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty

        VStack(alignment: .leading, spacing: 8) {
            Button(action: {
                if hasDetails {
                    onToggleGroup()
                }
            }) {
                HStack(spacing: 8) {
                    Text(label)
                        .font(.footnote.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                    Spacer(minLength: 0)
                    if hasDetails {
                        Image(systemName: expanded ? "chevron.up" : "chevron.down")
                            .font(.caption.weight(.semibold))
                            .foregroundColor(.ctxTextMuted)
                    }
                }
            }
            .buttonStyle(.plain)

            if hasDetails && expanded {
                VStack(alignment: .leading, spacing: 12) {
                    if total > 0 && group.tools.isEmpty && toolsLoading {
                        ProgressView()
                            .tint(.ctxAccent)
                    }
                    ForEach(group.tools) { tool in
                        let detail = toolDetails[tool.toolCallId]
                        let isExpanded = verbosity == .verbose || expandedTools.contains(tool.id)
                        ToolRow(
                            tool: tool,
                            detail: detail,
                            expanded: isExpanded,
                            onToggle: {
                                if verbosity == .verbose { return }
                                onToggleTool(tool.id)
                            }
                        )
                    }
                    if !group.thought.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("Thought")
                                .font(.caption.weight(.semibold))
                                .foregroundColor(.ctxTextMuted)
                            Text(group.thought)
                                .font(.footnote)
                                .foregroundColor(.ctxTextPrimary)
                                .lineSpacing(3)
                        }
                        .padding(10)
                        .background(Color.ctxSurfaceRaised, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    }
                }
            }
        }
        .padding(12)
        .background(Color.ctxSurface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
        .task(id: expanded) {
            if expanded && total > 0 {
                onRequestTools()
            }
        }
    }
}

private struct ToolRow: View {
    let tool: ChatToolSummary
    let detail: SessionTurnTool?
    let expanded: Bool
    let onToggle: () -> Void

    var body: some View {
        let title = tool.title ?? humanToolKind(tool.toolKind)
        let statusLabel = humanToolStatus(tool.status)
        let statusColor = toolStatusColor(tool.status)
        let summary = toolSummaryLine(tool.toolKind, input: detail?.inputJson ?? tool.inputPreview)
        let inputPayload = detail?.inputJson ?? tool.inputPreview
        let output = detail?.outputText ?? ""

        VStack(alignment: .leading, spacing: 8) {
            Button(action: onToggle) {
                HStack(alignment: .top, spacing: 10) {
                    ToolIcon(kind: tool.toolKind)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(title.isEmpty ? "Tool" : title)
                            .font(.footnote.weight(.semibold))
                            .foregroundColor(.ctxTextPrimary)
                        HStack(spacing: 6) {
                            ToolStatusBadge(text: statusLabel, tint: statusColor)
                            if !summary.isEmpty {
                                Text(summary)
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                                    .lineLimit(1)
                            }
                        }
                    }
                    Spacer(minLength: 0)
                    Text(formatShortTime(tool.updatedAt))
                        .font(.caption2)
                        .foregroundColor(.ctxTextMuted)
                    Image(systemName: expanded ? "chevron.up" : "chevron.down")
                        .font(.caption.weight(.semibold))
                        .foregroundColor(.ctxTextMuted)
                }
            }
            .buttonStyle(.plain)

            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    if let inputPayload, !formatJSONValue(inputPayload).isEmpty {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Input")
                                .font(.caption.weight(.semibold))
                                .foregroundColor(.ctxTextMuted)
                            Text(formatJSONValue(inputPayload))
                                .font(.system(size: 12, weight: .regular, design: .monospaced))
                                .foregroundColor(.ctxTextPrimary)
                                .lineSpacing(3)
                        }
                    }
                    if !output.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("Output")
                                .font(.caption.weight(.semibold))
                                .foregroundColor(.ctxTextMuted)
                            if looksLikeMarkdown(output) {
                                MarkdownContentView(text: output)
                            } else {
                                Text(output)
                                    .font(.footnote)
                                    .foregroundColor(.ctxTextPrimary)
                                    .lineSpacing(3)
                            }
                        }
                    }
                }
                .padding(.top, 4)
            }
        }
        .padding(10)
        .background(Color.ctxSurfaceRaised, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct ToolIcon: View {
    let kind: String?

    var body: some View {
        let normalized = (kind ?? "").lowercased()
        let icon: LucideIconName = normalized.contains("image") ? .image : .terminal
        LucideIcon(name: icon, size: 16)
            .foregroundColor(.ctxTextPrimary)
            .frame(width: 28, height: 28)
            .background(Color.ctxBackgroundDeep, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

private struct ToolStatusBadge: View {
    let text: String
    let tint: Color

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundColor(tint)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(tint.opacity(0.12), in: Capsule())
    }
}

private struct AskUserQuestionOption: Identifiable {
    let id = UUID()
    let label: String
    let description: String?
    let isOther: Bool
}

private struct AskUserQuestionItem: Identifiable {
    let id = UUID()
    let header: String
    let question: String
    let options: [AskUserQuestionOption]
    let multiSelect: Bool
    let otherLabel: String?
}

private struct AskUserQuestionCard: View {
    let question: AskUserQuestionCardModel
    let active: Bool
    let onSubmit: ([String: String]) async -> Void
    let onCancel: () async -> Void

    @State private var selectedByQuestion: [String: Set<String>] = [:]
    @State private var otherByQuestion: [String: String] = [:]
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        let questions = normalizeAskUserQuestions(question.input)
        if questions.isEmpty { return AnyView(EmptyView()) }

        let answersSignature = answersKey(question.answers)
        let draftAnswers = buildAskUserAnswers(questions, selectedByQuestion: selectedByQuestion, otherByQuestion: otherByQuestion)
        let canSubmit = questions.allSatisfy { draftAnswers[$0.question]?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false }
        let outcomeLabel = question.outcome == "cancelled" ? "Cancelled" : question.answered ? "Submitted" : "Submit"

        return AnyView(
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text("Approval Needed")
                        .font(.footnote.weight(.semibold))
                        .foregroundColor(active ? .ctxAccent : .ctxTextSecondary)
                    Spacer()
                    if question.answered {
                        Text(outcomeLabel)
                            .font(.caption.weight(.semibold))
                            .foregroundColor(.ctxTextMuted)
                    }
                }

                ForEach(questions) { q in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(q.header)
                            .font(.caption.weight(.semibold))
                            .foregroundColor(.ctxTextMuted)
                        Text(q.question)
                            .font(.footnote)
                            .foregroundColor(.ctxTextPrimary)
                        ForEach(q.options) { option in
                            AskUserOptionRow(
                                option: option,
                                selected: selectedByQuestion[q.question]?.contains(option.label) == true,
                                disabled: question.answered || busy,
                                onToggle: {
                                    toggleOption(option.label, for: q)
                                }
                            )
                            if option.isOther, selectedByQuestion[q.question]?.contains(option.label) == true {
                                TextField(option.label, text: Binding(
                                    get: { otherByQuestion[q.question] ?? "" },
                                    set: { otherByQuestion[q.question] = $0 }
                                ))
                                .textFieldStyle(.roundedBorder)
                                .disabled(question.answered || busy)
                            }
                        }
                    }
                }

                if let error {
                    Text(error)
                        .font(.caption)
                        .foregroundColor(.ctxError)
                }

                if !question.answered {
                    HStack(spacing: 12) {
                        Button("Cancel") {
                            _Concurrency.Task {
                                await handleCancel()
                            }
                        }
                        .buttonStyle(CompactGhostButtonStyle())
                        .disabled(busy)

                        Button(outcomeLabel) {
                            _Concurrency.Task {
                                await handleSubmit(answers: draftAnswers, canSubmit: canSubmit)
                            }
                        }
                        .buttonStyle(CompactPrimaryButtonStyle())
                        .disabled(!canSubmit || busy)
                    }
                }
            }
            .padding(14)
            .background(Color.ctxSurface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(Color.ctxLine, lineWidth: 1)
            )
            .onAppear {
                applyAnswers(questions: questions, answers: question.answers)
            }
            .onChange(of: answersSignature) { _, _ in
                applyAnswers(questions: questions, answers: question.answers)
            }
        )
    }

    private func handleSubmit(answers: [String: String], canSubmit: Bool) async {
        guard canSubmit else { return }
        busy = true
        error = nil
        await onSubmit(answers)
        busy = false
    }

    private func handleCancel() async {
        busy = true
        error = nil
        await onCancel()
        busy = false
    }

    private func toggleOption(_ label: String, for questionItem: AskUserQuestionItem) {
        var selected = selectedByQuestion[questionItem.question] ?? []
        if questionItem.multiSelect {
            if selected.contains(label) {
                selected.remove(label)
            } else {
                selected.insert(label)
            }
        } else {
            selected = [label]
        }
        selectedByQuestion[questionItem.question] = selected
        if let otherLabel = questionItem.otherLabel, !selected.contains(otherLabel) {
            otherByQuestion[questionItem.question] = ""
        }
    }

    private func applyAnswers(questions: [AskUserQuestionItem], answers: [String: String]?) {
        guard let answers else { return }
        let derived = deriveAskUserSelection(questions: questions, answers: answers)
        selectedByQuestion = derived.selectedByQuestion
        otherByQuestion = derived.otherByQuestion
    }
}

private struct AskUserOptionRow: View {
    let option: AskUserQuestionOption
    let selected: Bool
    let disabled: Bool
    let onToggle: () -> Void

    var body: some View {
        Button(action: onToggle) {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundColor(selected ? .ctxAccent : .ctxTextMuted)
                VStack(alignment: .leading, spacing: 2) {
                    Text(option.label)
                        .font(.footnote)
                        .foregroundColor(.ctxTextPrimary)
                    if let description = option.description, !description.isEmpty {
                        Text(description)
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    }
                }
                Spacer(minLength: 0)
            }
        }
        .buttonStyle(.plain)
        .disabled(disabled)
    }
}

private struct CompactPrimaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.footnote.weight(.semibold))
            .foregroundColor(.white)
            .padding(.vertical, 8)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity)
            .background(Color.ctxAccent, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .opacity(configuration.isPressed ? 0.8 : 1)
    }
}

private struct CompactGhostButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.footnote.weight(.semibold))
            .foregroundColor(.ctxTextPrimary)
            .padding(.vertical, 8)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity)
            .background(Color.ctxSurfaceRaised, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(Color.ctxLine, lineWidth: 1)
            )
            .opacity(configuration.isPressed ? 0.8 : 1)
    }
}

private func artifactAssetPath(_ artifactId: String) -> String {
    let escaped = artifactId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? artifactId
    return "/api/artifacts/\(escaped)"
}

struct DaemonAssetContext {
    let baseURL: URL?
    let token: String?
    let client: DaemonAPIClient?

    func blobPath(_ blobId: String) -> String {
        let escaped = blobId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? blobId
        return "/api/blobs/\(escaped)"
    }

    func artifactPath(_ artifactId: String) -> String {
        artifactAssetPath(artifactId)
    }

    func blobURL(_ blobId: String) -> URL? {
        url(path: blobPath(blobId))
    }

    func artifactURL(_ artifactId: String) -> URL? {
        url(path: artifactPath(artifactId))
    }

    private func url(path: String) -> URL? {
        guard let baseURL else { return nil }
        let trimmedPath = path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        if trimmedPath.isEmpty { return baseURL }
        guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: true) else {
            return nil
        }
        let basePath = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.path = basePath.isEmpty ? "/\(trimmedPath)" : "/\(basePath)/\(trimmedPath)"
        components.query = nil
        components.fragment = nil
        guard var resolved = components.url,
              var resolvedComponents = URLComponents(url: resolved, resolvingAgainstBaseURL: true) else {
            return nil
        }
        if let token, !token.isEmpty {
            var queryItems = resolvedComponents.queryItems ?? []
            queryItems.append(URLQueryItem(name: "token", value: token))
            resolvedComponents.queryItems = queryItems
            resolved = resolvedComponents.url ?? resolved
        }
        return resolved
    }
}

private let chatIsoFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
}()

private let chatTimeFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateStyle = .none
    formatter.timeStyle = .short
    return formatter
}()

private func parseIso(_ iso: String?) -> Date? {
    guard let iso else { return nil }
    return chatIsoFormatter.date(from: iso) ?? ISO8601DateFormatter().date(from: iso)
}

private func formatIsoTimestamp(_ date: Date) -> String {
    chatIsoFormatter.string(from: date)
}

private func formatShortTime(_ iso: String) -> String {
    guard let date = parseIso(iso) else { return "" }
    return chatTimeFormatter.string(from: date)
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

private func parseMarkdownSegments(_ text: String) -> [MarkdownSegment] {
    var segments: [MarkdownSegment] = []
    var cursor = text.startIndex
    while let fenceRange = text.range(of: "```", range: cursor..<text.endIndex) {
        let before = String(text[cursor..<fenceRange.lowerBound])
        if !before.isEmpty {
            segments.append(MarkdownSegment(kind: .text(before)))
        }

        let afterFence = fenceRange.upperBound
        let newlineRange = text.range(of: "\n", range: afterFence..<text.endIndex)
        let language: String?
        let codeStart: String.Index
        if let newlineRange {
            let langCandidate = text[afterFence..<newlineRange.lowerBound]
            let trimmed = langCandidate.trimmingCharacters(in: .whitespacesAndNewlines)
            language = trimmed.isEmpty ? nil : trimmed
            codeStart = newlineRange.upperBound
        } else {
            language = nil
            codeStart = afterFence
        }

        guard let endRange = text.range(of: "```", range: codeStart..<text.endIndex) else {
            let code = String(text[codeStart..<text.endIndex])
            segments.append(MarkdownSegment(kind: .code(code, language)))
            cursor = text.endIndex
            break
        }

        let code = String(text[codeStart..<endRange.lowerBound])
        segments.append(MarkdownSegment(kind: .code(code, language)))
        cursor = endRange.upperBound
    }

    if cursor < text.endIndex {
        let tail = String(text[cursor..<text.endIndex])
        if !tail.isEmpty {
            segments.append(MarkdownSegment(kind: .text(tail)))
        }
    }

    if segments.isEmpty {
        segments.append(MarkdownSegment(kind: .text(text)))
    }
    return segments
}

private func looksLikeMarkdown(_ text: String) -> Bool {
    if text.contains("```") { return true }
    if text.range(of: #"^#{1,6}\s"#, options: .regularExpression) != nil { return true }
    if text.range(of: #"^\s*[-*]\s+"#, options: .regularExpression) != nil { return true }
    if text.range(of: #"\[[^\]]+\]\([^)]+\)"#, options: .regularExpression) != nil { return true }
    return false
}

private func roleForMessage(_ role: MessageRole) -> ChatMessage.Role {
    switch role {
    case .assistant:
        return .assistant
    case .user:
        return .user
    case .system:
        return .system
    }
}

private func toolGroupLabel(total: Int, running: Int, failed: Int, thought: String) -> String {
    var parts: [String] = []
    if total > 0 {
        parts.append("\(total) tool\(total == 1 ? "" : "s")")
    }
    if running > 0 {
        parts.append("\(running) running")
    }
    if failed > 0 {
        parts.append("\(failed) failed")
    }
    if parts.isEmpty && !thought.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        parts.append("Thought")
    }
    let joined = parts.joined(separator: " · ")
    return joined.isEmpty ? "Activity" : joined
}

private func humanToolKind(_ toolKind: String?) -> String {
    let raw = toolKind?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    guard !raw.isEmpty else { return "Tool" }
    let cleaned = raw.replacingOccurrences(of: "_", with: " ")
    return cleaned.split(separator: " ").map { $0.capitalized }.joined(separator: " ")
}

private func humanToolStatus(_ status: String?) -> String {
    let normalized = (status ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    switch normalized {
    case "pending", "queued":
        return "Pending"
    case "running", "in_progress", "inprogress":
        return "Running"
    case "failed", "error":
        return "Failed"
    case "completed", "success", "done":
        return "Completed"
    default:
        return normalized.isEmpty ? "Unknown" : normalized.capitalized
    }
}

private func toolStatusColor(_ status: String?) -> Color {
    let normalized = (status ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    switch normalized {
    case "failed", "error":
        return .ctxError
    case "running", "in_progress", "inprogress":
        return .ctxWarning
    case "completed", "success", "done":
        return .ctxAccent
    case "pending", "queued":
        return .ctxTextMuted
    default:
        return .ctxTextMuted
    }
}

private func toolSummaryLine(_ toolKind: String?, input: JSONValue?) -> String {
    let kind = (toolKind ?? "").lowercased()
    let object = objectValue(from: input)

    if kind == "execute" {
        if let command = stringValue(from: object?["command"]) {
            return truncateMiddle(command, maxLen: 120)
        }
        if let commandArray = arrayValue(from: object?["command"]) {
            let parts = commandArray.compactMap { stringValue(from: $0) }
            if !parts.isEmpty {
                return truncateMiddle(parts.joined(separator: " "), maxLen: 120)
            }
        }
    }

    if kind == "search" {
        let query = stringValue(from: object?["query"] ?? object?["pattern"] ?? object?["regex"] ?? object?["text"]) ?? ""
        let path = formatToolPathSummary(input)
        if !query.isEmpty && !path.isEmpty {
            return truncateMiddle("\(query) in \(path)", maxLen: 120)
        }
        return truncateMiddle(query.isEmpty ? path : query, maxLen: 120)
    }

    if kind == "list" || kind == "list_files" || kind == "read" || kind == "read_file" {
        return formatToolPathSummary(input)
    }

    if kind == "edit" || kind == "write" || kind == "apply_patch" {
        return formatToolPathSummary(input)
    }

    if kind == "fetch" || kind == "http" || kind == "curl" {
        let method = stringValue(from: object?["method"])?.uppercased() ?? "GET"
        let url = stringValue(from: object?["url"] ?? object?["uri"] ?? object?["href"]) ?? ""
        if url.isEmpty { return method }
        return truncateMiddle("\(method) \(url)", maxLen: 120)
    }

    return ""
}

private func formatToolPathSummary(_ input: JSONValue?) -> String {
    let result = extractToolPaths(input)
    guard !result.paths.isEmpty else { return "" }
    let total = result.total ?? result.paths.count
    let head = truncateMiddle(result.paths[0], maxLen: 120)
    let more = max(0, total - 1)
    return more > 0 ? "\(head) +\(more) more" : head
}

private func extractToolPaths(_ input: JSONValue?) -> (paths: [String], total: Int?) {
    guard let object = objectValue(from: input) else { return ([], nil) }
    var paths: [String] = []

    if let path = stringValue(from: object["path"]) {
        paths.append(path)
    }
    if let pathArray = arrayValue(from: object["paths"]) {
        paths.append(contentsOf: pathArray.compactMap { stringValue(from: $0) })
    }
    let total = numberValue(from: object["paths_total"] ?? object["pathsTotal"]).map { Int($0) }
    return (paths: paths, total: total)
}

private func truncateMiddle(_ text: String, maxLen: Int) -> String {
    if text.count <= maxLen { return text }
    let head = max(10, Int(Double(maxLen) * 0.6))
    let tail = max(10, maxLen - head - 3)
    let startIdx = text.index(text.startIndex, offsetBy: head)
    let endIdx = text.index(text.endIndex, offsetBy: -tail)
    return "\(text[text.startIndex..<startIdx])...\(text[endIdx..<text.endIndex])"
}

private func formatJSONValue(_ value: JSONValue) -> String {
    switch value {
    case .string(let string):
        return string
    case .number(let number):
        return String(number)
    case .bool(let flag):
        return flag ? "true" : "false"
    case .null:
        return "null"
    default:
        guard let data = try? JSONEncoder().encode(value),
              let obj = try? JSONSerialization.jsonObject(with: data),
              let pretty = try? JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted]),
              let text = String(data: pretty, encoding: .utf8) else {
            return ""
        }
        return text
    }
}

private func normalizeAskUserQuestions(_ input: JSONValue) -> [AskUserQuestionItem] {
    let rawQuestions = extractAskUserQuestions(from: input)
    guard !rawQuestions.isEmpty else { return [] }
    var items: [AskUserQuestionItem] = []
    for (idx, raw) in rawQuestions.enumerated() {
        guard let object = objectValue(from: raw) else { continue }
        let question = stringValue(from: object["question"]) ?? ""
        if question.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { continue }
        let header = stringValue(from: object["header"]) ?? "Question \(idx + 1)"
        let options = normalizeAskUserOptions(object["options"])
        let multiSelect = boolValue(from: object["multiSelect"] ?? object["multi_select"]) ?? false
        let otherResult = extractOtherOption(questionObject: object, options: options)
        var displayOptions = otherResult.options
        if let otherLabel = otherResult.otherLabel {
            displayOptions.append(AskUserQuestionOption(label: otherLabel, description: nil, isOther: true))
        }
        items.append(
            AskUserQuestionItem(
                header: header,
                question: question,
                options: displayOptions,
                multiSelect: multiSelect,
                otherLabel: otherResult.otherLabel
            )
        )
    }
    return items
}

private func extractAskUserQuestions(from input: JSONValue) -> [JSONValue] {
    if case .array(let array) = input {
        return array
    }
    if let object = objectValue(from: input) {
        if case .array(let questions) = object["questions"] {
            return questions
        }
        if let nested = objectValue(from: object["input"]),
           case .array(let questions) = nested["questions"] {
            return questions
        }
    }
    return []
}

private func normalizeAskUserOptions(_ value: JSONValue?) -> [AskUserQuestionOption] {
    guard let value, case .array(let array) = value else { return [] }
    var options: [AskUserQuestionOption] = []
    for item in array {
        switch item {
        case .string(let label):
            let trimmed = label.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmed.isEmpty {
                options.append(AskUserQuestionOption(label: trimmed, description: nil, isOther: false))
            }
        case .object(let object):
            let label = stringValue(from: object["label"])?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if label.isEmpty { continue }
            let description = stringValue(from: object["description"])
            options.append(AskUserQuestionOption(label: label, description: description, isOther: false))
        default:
            continue
        }
    }
    return options
}

private func extractOtherOption(
    questionObject: [String: JSONValue],
    options: [AskUserQuestionOption]
) -> (options: [AskUserQuestionOption], otherLabel: String?) {
    let allowOther = boolValue(from: questionObject["allowOther"] ?? questionObject["allow_other"] ?? questionObject["allowOtherOption"]) ?? false
    let explicitLabel = stringValue(from: questionObject["otherOptionLabel"] ?? questionObject["other_option_label"] ?? questionObject["otherLabel"])
    var otherLabel = explicitLabel?.trimmingCharacters(in: .whitespacesAndNewlines)
    var cleanedOptions: [AskUserQuestionOption] = []

    for option in options {
        let label = option.label.trimmingCharacters(in: .whitespacesAndNewlines)
        let lower = label.lowercased()
        if otherLabel == nil && (lower == "other" || lower == "type something" || lower == "type something.") {
            otherLabel = label
            continue
        }
        cleanedOptions.append(option)
    }

    if otherLabel == nil && allowOther {
        otherLabel = "Type something."
    }
    return (cleanedOptions, otherLabel)
}

private func splitAnswerParts(_ answer: String, multiSelect: Bool) -> [String] {
    let trimmed = answer.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.isEmpty { return [] }
    if !multiSelect { return [trimmed] }
    return trimmed.split(separator: ",").map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.filter { !$0.isEmpty }
}

private func deriveAskUserSelection(
    questions: [AskUserQuestionItem],
    answers: [String: String]
) -> (selectedByQuestion: [String: Set<String>], otherByQuestion: [String: String]) {
    var selectedByQuestion: [String: Set<String>] = [:]
    var otherByQuestion: [String: String] = [:]

    for q in questions {
        let answer = answers[q.question] ?? ""
        let parts = splitAnswerParts(answer, multiSelect: q.multiSelect)
        if parts.isEmpty { continue }
        let optionLabels = Set(q.options.map { $0.label })
        var selected = Set<String>()
        var otherParts: [String] = []

        for part in parts {
            if optionLabels.contains(part) {
                selected.insert(part)
            } else {
                otherParts.append(part)
            }
        }

        if !otherParts.isEmpty {
            otherByQuestion[q.question] = q.multiSelect ? otherParts.joined(separator: ", ") : otherParts[0]
            if let otherLabel = q.otherLabel {
                selected.insert(otherLabel)
            }
        }
        if !selected.isEmpty {
            selectedByQuestion[q.question] = selected
        }
    }
    return (selectedByQuestion, otherByQuestion)
}

private func buildAskUserAnswers(
    _ questions: [AskUserQuestionItem],
    selectedByQuestion: [String: Set<String>],
    otherByQuestion: [String: String]
) -> [String: String] {
    var out: [String: String] = [:]
    for q in questions {
        let selected = selectedByQuestion[q.question] ?? []
        let other = otherByQuestion[q.question]?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let trimmedSelected: [String]
        if let otherLabel = q.otherLabel {
            trimmedSelected = selected.filter { $0 != otherLabel }
        } else {
            trimmedSelected = Array(selected)
        }

        if q.multiSelect {
            var combined = trimmedSelected
            if !other.isEmpty { combined.append(other) }
            if !combined.isEmpty { out[q.question] = combined.joined(separator: ", ") }
        } else {
            if !other.isEmpty {
                out[q.question] = other
            } else if let first = trimmedSelected.first {
                out[q.question] = first
            }
        }
    }
    return out
}

private func answersKey(_ answers: [String: String]?) -> String {
    guard let answers else { return "" }
    return answers.keys.sorted().map { "\($0)=\(answers[$0] ?? "")" }.joined(separator: "|")
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
                            assetContext: DaemonAssetContext(baseURL: nil, token: nil, client: nil)
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
                RemoteAssetImage(
                    url: url,
                    token: assetContext.token,
                    assetPath: assetContext.blobPath(blobId),
                    client: assetContext.client,
                    content: { image in
                        image
                            .resizable()
                            .scaledToFill()
                    },
                    placeholder: { placeholder },
                    loading: { loading }
                )
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

private enum RemoteAssetImagePhase {
    case empty
    case success(UIImage)
    case failure
}

private struct RemoteAssetImage<Content: View, Placeholder: View, Loading: View>: View {
    let url: URL
    let token: String?
    let assetPath: String?
    let client: DaemonAPIClient?
    let content: (Image) -> Content
    let placeholder: () -> Placeholder
    let loading: () -> Loading

    @State private var phase: RemoteAssetImagePhase = .empty

    var body: some View {
        Group {
            switch phase {
            case .success(let image):
                content(Image(uiImage: image))
            case .failure:
                placeholder()
            case .empty:
                loading()
            }
        }
        .task(id: cacheKey) {
            await load()
        }
    }

    private var cacheKey: String {
        let tokenPart = token ?? ""
        return "\(url.absoluteString)|\(tokenPart)"
    }

    private func load() async {
        await MainActor.run {
            phase = .empty
        }
        if let assetPath, let cached = await ArtifactContentCache.shared.cachedData(for: assetPath) {
            if let image = UIImage(data: cached) {
                await MainActor.run { phase = .success(image) }
                return
            }
        }
        if let client, let assetPath {
            do {
                let data = try await client.fetchAssetData(path: assetPath)
                guard let image = UIImage(data: data) else {
                    await MainActor.run { phase = .failure }
                    return
                }
                await ArtifactContentCache.shared.store(data, for: assetPath, fileExtension: url.pathExtension)
                await MainActor.run { phase = .success(image) }
                return
            } catch {
                // Fall through to direct fetch if secure request fails.
            }
        }
        var request = URLRequest(url: url)
        if let token, !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        request.cachePolicy = .reloadIgnoringLocalCacheData
        do {
            let (data, response) = try await URLSession.shared.data(for: request)
            guard let http = response as? HTTPURLResponse,
                  (200..<300).contains(http.statusCode),
                  let image = UIImage(data: data) else {
                await MainActor.run { phase = .failure }
                return
            }
            if let assetPath {
                await ArtifactContentCache.shared.store(data, for: assetPath, fileExtension: url.pathExtension)
            }
            await MainActor.run { phase = .success(image) }
        } catch {
            await MainActor.run { phase = .failure }
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
    @State private var cachedVideoURL: URL?

    var body: some View {
        ZStack {
            if artifact.missing == true {
                missingView
            } else if isVideoArtifact(artifact) {
                let assetPath = assetContext.artifactPath(artifact.id.stringValue)
                let resolvedURL = cachedVideoURL ?? assetContext.artifactURL(artifact.id.stringValue)
                if let url = resolvedURL {
                    VideoPlayer(player: player)
                        .onAppear { updatePlayer(url) }
                        .onChange(of: resolvedURL) { _, next in
                            if let next { updatePlayer(next) }
                        }
                        .onDisappear {
                            player?.pause()
                            player = nil
                        }
                        .aspectRatio(16 / 9, contentMode: .fit)
                        .task(id: assetPath) {
                            cachedVideoURL = await ArtifactContentCache.shared.cachedURL(for: assetPath)
                        }
                } else {
                    placeholder
                }
            } else if isImageArtifact(artifact),
                      let url = assetContext.artifactURL(artifact.id.stringValue) {
                RemoteAssetImage(
                    url: url,
                    token: assetContext.token,
                    assetPath: assetContext.artifactPath(artifact.id.stringValue),
                    client: assetContext.client,
                    content: { image in
                        image
                            .resizable()
                            .scaledToFit()
                    },
                    placeholder: { placeholder },
                    loading: { loading }
                )
                .padding(12)
            } else {
                placeholder
            }
        }
        .background(Color.ctxSurfaceRaised)
        .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
    }

    private func updatePlayer(_ url: URL) {
        if let existing = player,
           let asset = existing.currentItem?.asset as? AVURLAsset,
           asset.url == url {
            return
        }
        player = AVPlayer(url: url)
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
                RemoteAssetImage(
                    url: url,
                    token: assetContext.token,
                    assetPath: assetContext.artifactPath(artifact.id.stringValue),
                    client: assetContext.client,
                    content: { image in
                        image
                            .resizable()
                            .scaledToFill()
                    },
                    placeholder: { placeholder },
                    loading: { loading }
                )
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
    @State private var textFieldWidth: CGFloat = 0
    @State private var isFullscreenPresented = false

    private var modelChoices: [String] {
        modelOptions.isEmpty ? ["default"] : modelOptions
    }

    private var resolvedModelId: String {
        let fallback = modelChoices.first ?? "default"
        return selectedModelId.isEmpty ? fallback : selectedModelId
    }

    private var estimatedLineCount: Int {
        let fallback = max(1, text.components(separatedBy: .newlines).count)
        let width = max(0, textFieldWidth - 4)
        guard width > 0 else { return fallback }
        let font = UIFont.systemFont(ofSize: 16)
        let textToMeasure = text.isEmpty ? " " : text
        let bounding = (textToMeasure as NSString).boundingRect(
            with: CGSize(width: width, height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading],
            attributes: [.font: font],
            context: nil
        )
        let lineHeight = font.lineHeight
        return max(1, Int(ceil(bounding.height / lineHeight)))
    }

    private var showsFullscreenToggle: Bool {
        estimatedLineCount >= 3
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
                    .background(
                        GeometryReader { proxy in
                            Color.clear.preference(key: ComposerTextFieldWidthKey.self, value: proxy.size.width)
                        }
                    )
                    .onPreferenceChange(ComposerTextFieldWidthKey.self) { width in
                        if width > 0, width != textFieldWidth {
                            textFieldWidth = width
                        }
                    }
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

                HStack(spacing: 4) {
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
                    .accessibilityIdentifier("chat.composer.attach")

                    if isWorking && !isSendEnabled {
                        ComposerCircleButton(icon: .square, style: .fill, accessibilityId: "chat.composer.stop") {
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
                        .frame(width: CtxChatStyle.composerPrimarySize, height: CtxChatStyle.composerPrimarySize)
                        .accessibilityIdentifier("chat.composer.mic")
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
            if showsFullscreenToggle || contextSummary != nil {
                HStack(spacing: 6) {
                    if showsFullscreenToggle {
                        Button(action: { isFullscreenPresented = true }) {
                            LucideIcon(name: .expand, size: 14)
                                .foregroundColor(.ctxTextMuted)
                                .frame(width: 22, height: 22)
                                .contentShape(Rectangle())
                        }
                        .accessibilityIdentifier("chat.composer.expand")
                    }

                    if let contextSummary {
                        ContextWindowIndicator(summary: contextSummary)
                            .allowsHitTesting(false)
                    }
                }
                .padding(.top, 2)
                .padding(.trailing, 12)
            }
        }
        .fullScreenCover(isPresented: $isFullscreenPresented) {
                ComposerFullscreenView(
                    text: $text,
                    isSendEnabled: isSendEnabled,
                    isWorking: isWorking,
                    onSend: onSend,
                    onInterrupt: onInterrupt,
                    onClose: { isFullscreenPresented = false }
                )
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

private struct ComposerFullscreenView: View {
    @Binding var text: String
    var isSendEnabled: Bool
    var isWorking: Bool
    var onSend: () -> Void
    var onInterrupt: () -> Void
    var onClose: () -> Void
    @FocusState private var isFocused: Bool

    var body: some View {
        let showStop = isWorking && !isSendEnabled
        VStack(spacing: 16) {
            HStack {
                Button(action: onClose) {
                    LucideIcon(name: .x, size: 18)
                        .foregroundColor(.ctxTextPrimary)
                        .frame(width: 32, height: 32)
                        .contentShape(Rectangle())
                }
                .accessibilityIdentifier("chat.composer.fullscreen.close")

                Spacer(minLength: 0)
            }
            .padding(.horizontal, 20)
            .padding(.top, 12)

            TextEditor(text: $text)
                .font(CtxChatStyle.bodyFont)
                .foregroundColor(.ctxTextPrimary)
                .tint(.ctxAccent)
                .scrollContentBackground(.hidden)
                .focused($isFocused)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(Color.ctxSurfaceRaised)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(Color.ctxLine, lineWidth: 1)
                )
                .padding(.horizontal, 16)

            HStack {
                Spacer(minLength: 0)
                if showStop {
                    ComposerCircleButton(icon: .square, style: .fill, accessibilityId: "chat.composer.fullscreen.stop") {
                        onInterrupt()
                    }
                } else {
                    ComposerCircleButton(icon: .arrowUp, accessibilityId: "chat.composer.fullscreen.send") {
                        onSend()
                    }
                    .disabled(!isSendEnabled)
                    .opacity(isSendEnabled ? 1 : 0.4)
                }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 24)
        }
        .background(Color.ctxBackground.ignoresSafeArea())
        .onAppear {
            isFocused = true
        }
    }
}

private struct ScrollToBottomButton: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            LucideIcon(name: .arrowDown, size: 16)
                .foregroundColor(.ctxTextPrimary)
                .frame(width: 36, height: 36)
                .background(Circle().fill(Color.ctxSurfaceRaised))
                .overlay(
                    Circle().stroke(Color.ctxLine, lineWidth: 1)
                )
        }
        .accessibilityIdentifier("chat.scrollToBottom")
        .shadow(color: Color.ctxShadow, radius: 6, x: 0, y: 3)
    }
}

private struct ComposerTextFieldWidthKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

private struct ScrollToBottomPreferenceKey: PreferenceKey {
    static var defaultValue: CGFloat = 0

    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
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
    let iconStyle: LucideIconStyle
    let accessibilityId: String?
    let action: () -> Void

    init(
        icon: LucideIconName,
        style: LucideIconStyle = .stroke,
        accessibilityId: String? = nil,
        action: @escaping () -> Void
    ) {
        self.icon = icon
        self.iconStyle = style
        self.accessibilityId = accessibilityId
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            LucideIcon(name: icon, size: 18, style: iconStyle)
                .foregroundColor(.white)
                .frame(width: CtxChatStyle.composerPrimarySize, height: CtxChatStyle.composerPrimarySize)
                .background(
                    Circle().fill(Color.ctxAccent)
                )
        }
        .accessibilityIdentifier(accessibilityId ?? "")
    }
}

struct ChatMessage: Identifiable, Sendable {
    enum Role: Sendable {
        case assistant
        case user
        case system
    }

    let id: String
    let role: Role
    let text: String
    let attachments: [MessageAttachment]
    let createdAt: String
}

struct ChatToolSummary: Identifiable, Sendable {
    let id: String
    let toolCallId: String
    let turnId: String
    let toolKind: String?
    let title: String?
    let status: String?
    let inputPreview: JSONValue?
    let outputText: String?
    let createdAt: String
    let updatedAt: String
}

struct ChatToolGroup: Identifiable, Sendable {
    let id: String
    let turnId: String
    let createdAt: String
    let toolTotal: Int
    let toolPending: Int
    let toolRunning: Int
    let toolCompleted: Int
    let toolFailed: Int
    let thought: String
    let tools: [ChatToolSummary]
}

struct AskUserQuestionCardModel: Identifiable, Sendable {
    let id: String
    let turnId: String
    let toolCallId: String
    let createdAt: String
    let input: JSONValue
    let answers: [String: String]?
    let outcome: String?
    let answered: Bool
}

struct ChatThreadItem: Identifiable, Sendable {
    enum Kind: Sendable {
        case message(ChatMessage)
        case toolGroup(ChatToolGroup)
        case askUser(AskUserQuestionCardModel)
    }

    let id: String
    let createdAt: String
    let orderSeq: Int?
    let kind: Kind
}

private struct AskUserAnswerState: Sendable {
    let outcome: String
    let answers: [String: String]
}

private struct ChatThreadBuildKey: Hashable, Sendable {
    let sessionId: String
    let snapshotRev: Int
    let lastEventSeq: Int
    let messageCount: Int
    let turnCount: Int
    let eventCount: Int
    let toolSummaryCount: Int
    let optimisticCount: Int
    let lastMessageId: String?
    let lastMessageHash: Int
}

private struct ChatThreadBuildInput: Sendable {
    let key: ChatThreadBuildKey
    let messages: [ChatMessage]
    let turns: [SessionTurn]
    let events: [SessionEvent]
    let toolSummaries: [SessionTurnToolSummary]
    let optimisticAnswers: [String: AskUserAnswerState]
}

private struct ChatThreadBuildOutput: Sendable {
    let key: ChatThreadBuildKey
    let items: [ChatThreadItem]
    let activeAskToolCallId: String?
}

private enum ChatThreadMergeKind: String, Sendable {
    case history
    case head
    case append
}

private struct ChatThreadMergeKey: Hashable, Sendable {
    let sessionId: String
    let snapshotRev: Int
    let lastEventSeq: Int
    let existingCount: Int
    let incomingCount: Int
    let existingTailId: String?
    let incomingTailId: String?
    let kind: ChatThreadMergeKind
}

private actor ChatThreadPipeline {
    private let isoFormatter: ISO8601DateFormatter
    private let fallbackFormatter: ISO8601DateFormatter
    private var threadCache: [ChatThreadBuildKey: ChatThreadBuildOutput] = [:]
    private var threadCacheOrder: [ChatThreadBuildKey] = []
    private var messageMergeCache: [ChatThreadMergeKey: [ChatMessage]] = [:]
    private var turnMergeCache: [ChatThreadMergeKey: [SessionTurn]] = [:]
    private var mergeCacheOrder: [ChatThreadMergeKey] = []
    private let threadCacheLimit = 6
    private let mergeCacheLimit = 8

    init() {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        isoFormatter = formatter
        fallbackFormatter = ISO8601DateFormatter()
    }

    func buildThreadItems(input: ChatThreadBuildInput) -> ChatThreadBuildOutput {
        if let cached = threadCache[input.key] {
            return cached
        }

        let answersByToolCallId = collectAskUserQuestionAnswers(
            events: input.events,
            optimistic: input.optimisticAnswers
        )
        let toolSummaries = buildToolSummaries(from: input.toolSummaries, events: input.events)
        let toolsByTurn = Dictionary(grouping: toolSummaries, by: { $0.turnId })

        var items: [ChatThreadItem] = []

        for message in input.messages {
            let item = ChatThreadItem(
                id: "msg-\(message.id)",
                createdAt: message.createdAt,
                orderSeq: nil,
                kind: .message(message)
            )
            items.append(item)
        }

        for turn in input.turns {
            let turnId = turn.turnId.stringValue
            let tools = toolsByTurn[turnId] ?? []
            let thought = turn.thoughtPartial ?? ""
            let hasTools = turn.toolTotal > 0 || !tools.isEmpty
            let hasThought = !thought.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            if !hasTools && !hasThought {
                continue
            }
            let group = ChatToolGroup(
                id: "turn-\(turnId)",
                turnId: turnId,
                createdAt: turn.startedAt,
                toolTotal: turn.toolTotal,
                toolPending: turn.toolPending,
                toolRunning: turn.toolRunning,
                toolCompleted: turn.toolCompleted,
                toolFailed: turn.toolFailed,
                thought: thought,
                tools: tools
            )
            let item = ChatThreadItem(
                id: group.id,
                createdAt: group.createdAt,
                orderSeq: turn.startSeq,
                kind: .toolGroup(group)
            )
            items.append(item)
        }

        for event in input.events {
            if let askItem = buildAskUserQuestionItem(event: event, answersByToolCallId: answersByToolCallId) {
                let item = ChatThreadItem(
                    id: askItem.id,
                    createdAt: askItem.createdAt,
                    orderSeq: event.seq,
                    kind: .askUser(askItem)
                )
                items.append(item)
            }
        }

        items.sort { left, right in
            let leftDate = parseIso(left.createdAt) ?? .distantPast
            let rightDate = parseIso(right.createdAt) ?? .distantPast
            if leftDate != rightDate {
                return leftDate < rightDate
            }
            let leftSeq = left.orderSeq ?? 0
            let rightSeq = right.orderSeq ?? 0
            if leftSeq != rightSeq {
                return leftSeq < rightSeq
            }
            return left.id < right.id
        }

        let activeAskToolCallId = items.compactMap { item -> String? in
            if case .askUser(let ask) = item.kind, !ask.answered {
                return ask.toolCallId
            }
            return nil
        }.first

        let output = ChatThreadBuildOutput(
            key: input.key,
            items: items,
            activeAskToolCallId: activeAskToolCallId
        )
        storeThreadCache(output)
        return output
    }

    func mergeMessages(
        existing: [ChatMessage],
        incoming: [ChatMessage],
        key: ChatThreadMergeKey
    ) -> [ChatMessage] {
        if let cached = messageMergeCache[key] {
            return cached
        }
        var byId: [String: ChatMessage] = [:]
        for message in existing {
            byId[message.id] = message
        }
        for message in incoming {
            byId[message.id] = message
        }
        let merged = byId.values.sorted { left, right in
            let leftDate = parseIso(left.createdAt) ?? .distantPast
            let rightDate = parseIso(right.createdAt) ?? .distantPast
            if leftDate != rightDate {
                return leftDate < rightDate
            }
            return left.id < right.id
        }
        storeMergeCache(key: key, messages: merged)
        return merged
    }

    func mergeTurns(
        existing: [SessionTurn],
        incoming: [SessionTurn],
        key: ChatThreadMergeKey
    ) -> [SessionTurn] {
        if let cached = turnMergeCache[key] {
            return cached
        }
        var byId: [String: SessionTurn] = [:]
        for turn in existing {
            byId[turn.turnId.stringValue] = turn
        }
        for turn in incoming {
            byId[turn.turnId.stringValue] = turn
        }
        let merged = byId.values.sorted { left, right in
            let leftSeq = left.startSeq ?? 0
            let rightSeq = right.startSeq ?? 0
            if leftSeq != rightSeq {
                return leftSeq < rightSeq
            }
            let leftDate = parseIso(left.updatedAt) ?? parseIso(left.startedAt) ?? .distantPast
            let rightDate = parseIso(right.updatedAt) ?? parseIso(right.startedAt) ?? .distantPast
            return leftDate < rightDate
        }
        storeMergeCache(key: key, turns: merged)
        return merged
    }

    private func parseIso(_ iso: String?) -> Date? {
        guard let iso else { return nil }
        return isoFormatter.date(from: iso) ?? fallbackFormatter.date(from: iso)
    }

    private func storeThreadCache(_ output: ChatThreadBuildOutput) {
        threadCache[output.key] = output
        noteThreadKey(output.key)
        if threadCacheOrder.count > threadCacheLimit {
            let overflow = threadCacheOrder.count - threadCacheLimit
            for key in threadCacheOrder.prefix(overflow) {
                threadCache.removeValue(forKey: key)
            }
            threadCacheOrder.removeFirst(overflow)
        }
    }

    private func noteThreadKey(_ key: ChatThreadBuildKey) {
        if let index = threadCacheOrder.firstIndex(of: key) {
            threadCacheOrder.remove(at: index)
        }
        threadCacheOrder.append(key)
    }

    private func storeMergeCache(key: ChatThreadMergeKey, messages: [ChatMessage]? = nil, turns: [SessionTurn]? = nil) {
        if let messages {
            messageMergeCache[key] = messages
        }
        if let turns {
            turnMergeCache[key] = turns
        }
        noteMergeKey(key)
        if mergeCacheOrder.count > mergeCacheLimit {
            let overflow = mergeCacheOrder.count - mergeCacheLimit
            for key in mergeCacheOrder.prefix(overflow) {
                messageMergeCache.removeValue(forKey: key)
                turnMergeCache.removeValue(forKey: key)
            }
            mergeCacheOrder.removeFirst(overflow)
        }
    }

    private func noteMergeKey(_ key: ChatThreadMergeKey) {
        if let index = mergeCacheOrder.firstIndex(of: key) {
            mergeCacheOrder.remove(at: index)
        }
        mergeCacheOrder.append(key)
    }

    private func buildToolSummaries(
        from summaries: [SessionTurnToolSummary],
        events: [SessionEvent]
    ) -> [ChatToolSummary] {
        if !summaries.isEmpty {
            return summaries.map { summary in
                ChatToolSummary(
                    id: "tool-\(summary.turnId.stringValue)-\(summary.toolCallId)",
                    toolCallId: summary.toolCallId,
                    turnId: summary.turnId.stringValue,
                    toolKind: summary.toolKind,
                    title: summary.title,
                    status: summary.status,
                    inputPreview: summary.inputPreview,
                    outputText: nil,
                    createdAt: summary.createdAt,
                    updatedAt: summary.updatedAt
                )
            }
        }
        return buildToolSummariesFromEvents(events)
    }

    private func buildToolSummariesFromEvents(_ events: [SessionEvent]) -> [ChatToolSummary] {
        var byId: [String: ChatToolSummary] = [:]
        for event in events {
            guard event.eventType == "tool_call" || event.eventType == "tool_call_update" || event.eventType == "tool_result" else {
                continue
            }
            guard let turnId = event.turnId?.stringValue else { continue }
            guard let payload = objectValue(from: event.payloadJson) else { continue }
            let update = objectValue(from: payload["acp_update"]) ?? payload
            guard let toolCallId = extractToolCallId(payload: payload, update: update) else { continue }

            let toolCall = objectValue(from: update["toolCall"] ?? update["tool_call"]) ?? [:]
            let toolKind = stringValue(from: update["kind"] ?? toolCall["kind"])
            let title = stringValue(from: update["title"] ?? toolCall["title"] ?? toolCall["name"])
            let rawStatus = stringValue(from: update["status"] ?? toolCall["status"])
            let status = rawStatus ?? (event.eventType == "tool_result" ? "completed" : nil)
            let inputPreview = update["input_preview"] ?? update["input"] ?? update["rawInput"] ?? update["raw_input"] ?? toolCall["input"]
            let outputText = stringValue(from: update["output_text"] ?? update["output"] ?? update["result"] ?? payload["output_text"] ?? payload["output"])

            let existing = byId[toolCallId]
            let summary = ChatToolSummary(
                id: existing?.id ?? "tool-\(turnId)-\(toolCallId)",
                toolCallId: toolCallId,
                turnId: existing?.turnId ?? turnId,
                toolKind: toolKind ?? existing?.toolKind,
                title: title ?? existing?.title,
                status: status ?? existing?.status,
                inputPreview: inputPreview ?? existing?.inputPreview,
                outputText: outputText ?? existing?.outputText,
                createdAt: existing?.createdAt ?? event.createdAt,
                updatedAt: event.createdAt
            )
            byId[toolCallId] = summary
        }
        return Array(byId.values)
    }

    private func extractToolCallId(payload: [String: JSONValue], update: [String: JSONValue]) -> String? {
        if let id = stringValue(from: payload["tool_call_id"]) { return id }
        if let id = stringValue(from: update["toolCallId"] ?? update["tool_call_id"]) { return id }
        if let rawInput = objectValue(from: update["rawInput"] ?? update["raw_input"]),
           let id = stringValue(from: rawInput["call_id"]) {
            return id
        }
        if let toolCall = objectValue(from: update["toolCall"] ?? update["tool_call"]),
           let id = stringValue(from: toolCall["id"] ?? toolCall["tool_call_id"]) {
            return id
        }
        return nil
    }

    private func buildAskUserQuestionItem(
        event: SessionEvent,
        answersByToolCallId: [String: AskUserAnswerState]
    ) -> AskUserQuestionCardModel? {
        guard event.eventType == "notice" else { return nil }
        guard let payload = objectValue(from: event.payloadJson) else { return nil }
        guard stringValue(from: payload["kind"]) == "ask_user_question" else { return nil }
        guard let toolCallId = stringValue(from: payload["tool_call_id"]), !toolCallId.isEmpty else { return nil }
        let input = payload["input"] ?? payload["input_json"] ?? event.payloadJson
        let answerState = answersByToolCallId[toolCallId]
        let turnId = event.turnId?.stringValue ?? "unknown"

        return AskUserQuestionCardModel(
            id: "ask-\(turnId)-\(toolCallId)",
            turnId: turnId,
            toolCallId: toolCallId,
            createdAt: event.createdAt,
            input: input,
            answers: answerState?.answers,
            outcome: answerState?.outcome,
            answered: answerState != nil
        )
    }

    private func collectAskUserQuestionAnswers(
        events: [SessionEvent],
        optimistic: [String: AskUserAnswerState]
    ) -> [String: AskUserAnswerState] {
        var map: [String: AskUserAnswerState] = [:]
        for event in events {
            guard let parsed = extractAskUserQuestionAnswer(event) else { continue }
            map[parsed.toolCallId] = parsed.state
        }
        for (toolCallId, state) in optimistic {
            if map[toolCallId] == nil {
                map[toolCallId] = state
            } else if (map[toolCallId]?.answers.isEmpty ?? true) && !state.answers.isEmpty {
                map[toolCallId] = state
            }
        }
        return map
    }

    private func extractAskUserQuestionAnswer(_ event: SessionEvent) -> (toolCallId: String, state: AskUserAnswerState)? {
        guard event.eventType == "notice" else { return nil }
        guard let payload = objectValue(from: event.payloadJson) else { return nil }
        guard stringValue(from: payload["kind"]) == "ask_user_question_answered" else { return nil }
        guard let toolCallId = stringValue(from: payload["tool_call_id"]), !toolCallId.isEmpty else { return nil }
        let outcome = stringValue(from: payload["outcome"]) ?? "submitted"
        let answers = normalizeAskUserAnswerMap(payload["answers"] ?? payload["answer"])
        let normalizedOutcome = outcome == "cancelled" ? "cancelled" : "submitted"
        return (toolCallId, AskUserAnswerState(outcome: normalizedOutcome, answers: answers))
    }

    private func normalizeAskUserAnswerMap(_ value: JSONValue?) -> [String: String] {
        guard let object = objectValue(from: value) else { return [:] }
        var out: [String: String] = [:]
        for (key, val) in object {
            let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, let text = stringValue(from: val) else { continue }
            out[trimmed] = text
        }
        return out
    }
}

@MainActor
final class ChatViewModel: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var threadItems: [ChatThreadItem] = []
    @Published var queueMessages: [MessageSummary] = []
    @Published var toolLoadingTurnIds: Set<String> = []
    @Published var toolDetailsByCallId: [String: SessionTurnTool] = [:]
    @Published var errorMessage: String?
    @Published var artifacts: [Artifact] = []
    @Published var isArtifactsLoading = false
    @Published var artifactsError: String?
    @Published var turnStatus: TurnStatusSnapshot?
    @Published private(set) var isAssistantWorking = false
    @Published private(set) var contextWindowInfo: ContextWindowInfo?
    @Published private(set) var assetBaseURL: URL?
    @Published private(set) var assetToken: String?
    @Published private(set) var threadItemsRevision: Int = 0
    @Published private(set) var activeAskToolCallId: String?
    @Published private(set) var isArchivedSession = false
    @Published private(set) var historyHasMore = false
    @Published private(set) var isHistoryLoading = false

    private struct StreamingAssistantState {
        let turnId: String
        let messageId: String
        var text: String
        var isActive: Bool
        let createdAt: String
    }

    struct TurnStatusSnapshot: Equatable {
        let status: SessionTurnStatus
        let startedAt: String
        let updatedAt: String
        let assistantMessage: String
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
    @Published private(set) var sessionId: String?
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
    private var latestTurns: [SessionTurn] = []
    private var latestEvents: [SessionEvent] = []
    private var latestToolSummaries: [SessionTurnToolSummary] = []
    private var optimisticAskAnswers: [String: AskUserAnswerState] = [:]
    private var lastSetModelId: String?
    private var lastSetModeId: String?
    private var historyCursor: Int?
    private var historyInitialized = false
    private var headMessages: [ChatMessage] = []
    private var historyMessages: [ChatMessage] = []
    private var headTurns: [SessionTurn] = []
    private var historyTurns: [SessionTurn] = []
    private let threadPipeline = ChatThreadPipeline()
    private var threadBuildToken: Int = 0
    private var lastThreadBuildKey: ChatThreadBuildKey?
    private var sessionStateRev: Int?
    private var threadSnapshotRev: Int { sessionStateRev ?? 0 }

    init(client: DaemonAPIClient? = nil, initialSessionId: String? = nil, initialWorkspaceId: String? = nil) {
        self.client = client
        self.sessionId = initialSessionId
        self.workspaceId = initialWorkspaceId
        if client == nil {
            messages = Self.sampleMessages
            latestTurns = []
            latestEvents = []
            latestToolSummaries = []
            turnStatus = sampleTurnStatus()
            rebuildThreadItems()
        }
    }

    var assetContext: DaemonAssetContext {
        DaemonAssetContext(baseURL: assetBaseURL, token: assetToken, client: client)
    }

    func setClient(_ client: DaemonAPIClient?) {
        self.client = client
        if client != nil {
            messages = []
            threadItems = []
            queueMessages = []
            toolDetailsByCallId = [:]
            toolLoadingTurnIds = []
            latestTurns = []
            latestEvents = []
            latestToolSummaries = []
            optimisticAskAnswers = [:]
            activeAskToolCallId = nil
            threadItemsRevision = 0
            lastThreadBuildKey = nil
            threadBuildToken += 1
            sessionStateRev = nil
            lastSetModelId = nil
            lastSetModeId = nil
            historyCursor = nil
            historyInitialized = false
            historyHasMore = false
            isHistoryLoading = false
            headMessages = []
            historyMessages = []
            headTurns = []
            historyTurns = []
            setPendingAssistantResponse(false)
            streamingAssistantState = nil
            errorMessage = nil
            artifacts = []
            artifactsError = nil
            isArtifactsLoading = false
            turnStatus = sampleTurnStatus()
            contextWindowInfo = nil
            lastEventSeq = nil
            refreshAssetContext()
            _Concurrency.Task { @MainActor in
                secureContext = await client?.secureConnectionContext()
            }
            if pollTask != nil && !isArchivedSession {
                startStream()
            }
            _Concurrency.Task {
                _ = await refreshMessages()
                await refreshQueue()
                await refreshArtifacts()
            }
            updateWorkingState()
        } else {
            stopStream()
            sessionId = nil
            workspaceId = nil
            lastEventSeq = nil
            messages = Self.sampleMessages
            latestTurns = []
            latestEvents = []
            latestToolSummaries = []
            optimisticAskAnswers = [:]
            threadItems = []
            queueMessages = []
            toolDetailsByCallId = [:]
            toolLoadingTurnIds = []
            threadItemsRevision = 0
            activeAskToolCallId = nil
            lastThreadBuildKey = nil
            threadBuildToken += 1
            sessionStateRev = nil
            lastSetModelId = nil
            lastSetModeId = nil
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
            rebuildThreadItems()
            updateWorkingState()
        }
    }

    func setArchived(_ value: Bool) {
        if isArchivedSession == value { return }
        isArchivedSession = value
        if value {
            _Concurrency.Task {
                await ArtifactContentCache.shared.setActiveScope(nil)
            }
            stopPolling()
            stopStream()
        } else {
            let scope = sessionId
            _Concurrency.Task {
                await ArtifactContentCache.shared.setActiveScope(scope)
            }
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

    private func sampleTurnStatus() -> TurnStatusSnapshot? {
        guard ProcessInfo.processInfo.environment["CTX_UI_TEST_MODE"] == "1" else { return nil }
        let now = Date()
        let assistantMessage = messages.reversed().first(where: { $0.role == .assistant })?.text ?? ""
        return TurnStatusSnapshot(
            status: .completed,
            startedAt: formatIsoTimestamp(now.addingTimeInterval(-42)),
            updatedAt: formatIsoTimestamp(now),
            assistantMessage: assistantMessage
        )
    }

    func selectSession(_ sessionId: String?, workspaceId: String? = nil) {
        sessionGeneration += 1
        self.sessionId = sessionId
        self.workspaceId = workspaceId
        _Concurrency.Task {
            await ArtifactContentCache.shared.setActiveScope(sessionId)
        }
        lastEventSeq = nil
        setPendingAssistantResponse(false)
        streamingAssistantState = nil
        refreshInFlight = false
        refreshPending = false
        messages = []
        threadItems = []
        queueMessages = []
        toolDetailsByCallId = [:]
        toolLoadingTurnIds = []
        latestTurns = []
        latestEvents = []
        latestToolSummaries = []
        optimisticAskAnswers = [:]
        activeAskToolCallId = nil
        threadItemsRevision = 0
        lastThreadBuildKey = nil
        threadBuildToken += 1
        sessionStateRev = nil
        lastSetModelId = nil
        lastSetModeId = nil
        historyCursor = nil
        historyInitialized = false
        historyHasMore = false
        isHistoryLoading = false
        headMessages = []
        historyMessages = []
        headTurns = []
        historyTurns = []
        errorMessage = nil
        artifacts = []
        artifactsError = nil
        artifactsRefreshInFlight = false
        artifactsRefreshPending = false
        turnStatus = nil
        contextWindowInfo = nil
        updateWorkingState()
        _Concurrency.Task {
            await hydrateFromCacheIfNeeded(sessionId: sessionId, generation: sessionGeneration)
            if !isArchivedSession {
                await primeStreamCursor()
            }
            _ = await refreshMessages()
            await refreshQueue()
            await refreshArtifacts()
            if !isArchivedSession {
                await sendStreamSubscriptionIfNeeded()
            }
        }
    }

    private func hydrateFromCacheIfNeeded(sessionId: String?, generation: Int) async {
        guard !isArchivedSession else { return }
        guard messages.isEmpty && latestTurns.isEmpty else { return }
        guard let sessionId else { return }
        guard let cached = await ATSHeadCache.shared.load(sessionId: sessionId) else { return }
        guard generation == sessionGeneration, sessionId == self.sessionId else { return }
        applyCachedHead(cached)
    }

    private func applyCachedHead(_ cached: CachedSessionHead) {
        let nextMessages = cached.messages.map { message in
            ChatMessage(
                id: message.id.stringValue,
                role: roleForMessage(message.role),
                text: message.content,
                attachments: message.attachments ?? [],
                createdAt: message.createdAt
            )
        }
        let nextWithStreaming = applyStreamingAssistantState(to: nextMessages)
        messages = nextWithStreaming
        latestTurns = cached.turns
        latestEvents = cached.events ?? []
        latestToolSummaries = cached.toolSummaries ?? []
        lastEventSeq = cached.lastEventSeq
        updateSessionStateRev(cached.stateRev)
        workspaceId = workspaceId ?? cached.session.workspaceId.stringValue
        if let turn = mostRecentTurn(in: cached.turns) {
            updateTurnStatus(from: turn)
            updateContextWindow(from: turn)
        }
        lastSetModelId = cached.session.modelId
        rebuildThreadItems()
        updateWorkingState()
    }

    func startPolling() {
        guard !isArchivedSession else { return }
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
        let local = ChatMessage(
            id: UUID().uuidString,
            role: .user,
            text: text,
            attachments: attachments,
            createdAt: formatIsoTimestamp(Date())
        )
        appendMessage(local)
        setPendingAssistantResponse(true)

        _Concurrency.Task {
            guard let client else { return }
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            do {
                _ = try await client.postMessage(sessionId: resolved, content: text, delivery: .immediate, attachments: attachments)
                _ = await refreshMessages()
                await refreshQueue()
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
            let snapshot = try await client.getSessionSnapshot(sessionId: resolved, limit: 200, includeEvents: true)
            let head = snapshot.head
            guard generation == sessionGeneration, resolved == sessionId else { return .skipped }
            let nextMessages = head.messages.map { message in
                ChatMessage(
                    id: message.id.stringValue,
                    role: roleForMessage(message.role),
                    text: message.content,
                    attachments: message.attachments ?? [],
                    createdAt: message.createdAt
                )
            }
            let nextWithStreaming = isArchivedSession ? nextMessages : applyStreamingAssistantState(to: nextMessages)
            if isArchivedSession {
                headMessages = nextMessages
                headTurns = head.turns
                if !historyInitialized {
                    historyCursor = head.turns.first?.startSeq
                    historyHasMore = head.hasMoreTurns && historyCursor != nil
                    historyInitialized = true
                }
                latestTurns = historyTurns + headTurns
                let mergedMessages = await mergeMessages(historyMessages, headMessages, kind: .head)
                guard generation == sessionGeneration, resolved == sessionId else { return .skipped }
                messages = mergedMessages
                lastEventSeq = head.lastEventSeq
            } else {
                messages = nextWithStreaming
                latestTurns = head.turns
            }
            latestEvents = head.events ?? []
            latestToolSummaries = head.toolSummaries ?? []
            updateSessionStateRev(head.stateRev)
            if let turn = mostRecentTurn(in: head.turns) {
                updateTurnStatus(from: turn)
                updateContextWindow(from: turn)
            }
            lastSetModelId = head.session.modelId
            rebuildThreadItems()
            await refreshQueue()
            if !isArchivedSession, pendingAssistantResponse, nextWithStreaming.contains(where: { $0.role == .assistant }) {
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

    func loadEarlierMessages() {
        guard isArchivedSession, historyHasMore, !isHistoryLoading else { return }
        guard historyCursor != nil else { return }
        let generation = sessionGeneration
        let beforeSeq = historyCursor
        let limit = 200
        isHistoryLoading = true
        _Concurrency.Task {
            defer { isHistoryLoading = false }
            guard let client else { return }
            let resolved = await resolveSessionId()
            guard let resolved, generation == sessionGeneration else { return }
            if let cached = await SessionHistoryPageCache.shared.load(
                sessionId: resolved,
                beforeSeq: beforeSeq,
                limit: limit
            ) {
                guard generation == sessionGeneration else { return }
                await applyHistoryPage(cached, generation: generation)
                return
            }
            do {
                let page = try await client.getSessionHistory(sessionId: resolved, beforeSeq: beforeSeq, limit: limit)
                guard generation == sessionGeneration else { return }
                await SessionHistoryPageCache.shared.store(page, sessionId: resolved, beforeSeq: beforeSeq, limit: limit)
                await applyHistoryPage(page, generation: generation)
            } catch {
                if generation == sessionGeneration {
                    errorMessage = "Failed to load older messages."
                }
            }
        }
    }

    private func applyHistoryPage(_ page: SessionHistoryPage, generation: Int) async {
        let nextMessages = page.messages.map { message in
            ChatMessage(
                id: message.id.stringValue,
                role: roleForMessage(message.role),
                text: message.content,
                attachments: message.attachments ?? [],
                createdAt: message.createdAt
            )
        }
        let mergedHistoryMessages = await mergeMessages(historyMessages, nextMessages, kind: .history)
        let mergedHistoryTurns = await mergeTurns(historyTurns, page.turns, kind: .history)
        guard generation == sessionGeneration else { return }
        historyMessages = mergedHistoryMessages
        historyTurns = mergedHistoryTurns
        historyCursor = page.nextCursor
        historyHasMore = page.hasMore
        latestTurns = historyTurns + headTurns
        let mergedMessages = await mergeMessages(historyMessages, headMessages, kind: .head)
        guard generation == sessionGeneration else { return }
        messages = mergedMessages
        rebuildThreadItems()
    }

    private func refreshQueue() async {
        guard let client else { return }
        let resolved = await resolveSessionId()
        guard let resolved else { return }
        do {
            queueMessages = try await client.listQueue(sessionId: resolved)
        } catch {
            queueMessages = []
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
            prefetchArtifactsIfNeeded()
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

    private func decodeArtifacts(from payload: JSONValue) -> [Artifact]? {
        guard case let .object(object) = payload,
              let artifactsValue = object["artifacts"] else { return nil }
        let encoder = JSONEncoder()
        guard let data = try? encoder.encode(artifactsValue) else { return nil }
        return try? streamDecoder.decode([Artifact].self, from: data)
    }

    private func applyArtifactsFromStream(_ artifacts: [Artifact]) {
        self.artifacts = artifacts
        artifactsError = nil
        isArtifactsLoading = false
        prefetchArtifactsIfNeeded()
    }

    private func prefetchArtifactsIfNeeded() {
        guard !isArchivedSession,
              let sessionId,
              let client else { return }
        let targets = artifacts.compactMap { artifact -> ArtifactPrefetchTarget? in
            if artifact.missing == true { return nil }
            if !(isImageArtifact(artifact) || isVideoArtifact(artifact)) { return nil }
            if artifact.bytes <= 0 { return nil }
            let path = artifactAssetPath(artifact.id.stringValue)
            let primaryExt = artifactFileExtension(artifact.absolutePath)
            let fallbackExt = artifactFileExtension(artifact.name ?? "")
            let ext = primaryExt.isEmpty ? fallbackExt : primaryExt
            return ArtifactPrefetchTarget(
                key: path,
                path: path,
                size: artifact.bytes,
                fileExtension: ext.isEmpty ? nil : ext
            )
        }
        guard !targets.isEmpty else { return }
        _Concurrency.Task {
            await ArtifactContentCache.shared.setActiveScope(sessionId)
            await ArtifactContentCache.shared.prefetch(targets: targets, client: client)
        }
    }

    private func appendMessage(_ message: ChatMessage) {
        if isArchivedSession {
            let generation = sessionGeneration
            let headSnapshot = headMessages
            _Concurrency.Task {
                let mergedHead = await mergeMessages(headSnapshot, [message], kind: .append)
                guard generation == sessionGeneration else { return }
                headMessages = mergedHead
                let mergedMessages = await mergeMessages(historyMessages, mergedHead, kind: .head)
                guard generation == sessionGeneration else { return }
                messages = mergedMessages
                rebuildThreadItems()
            }
            return
        }
        var next = messages
        next.append(message)
        messages = next
        rebuildThreadItems()
    }

    func updateSessionModel(_ modelId: String) {
        guard let client else { return }
        let trimmed = modelId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        if lastSetModelId == trimmed { return }
        lastSetModelId = trimmed
        _Concurrency.Task {
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            _ = try? await client.setSessionModel(sessionId: resolved, modelId: trimmed)
        }
    }

    func updateSessionMode(_ mode: ComposerMode) {
        guard let client else { return }
        let next = mode.rawValue
        if lastSetModeId == next { return }
        lastSetModeId = next
        _Concurrency.Task {
            let resolved = await resolveSessionId()
            guard let resolved else { return }
            try? await client.setSessionMode(sessionId: resolved, modeId: next)
        }
    }

    func loadTools(for turnId: String) {
        guard let client else { return }
        if toolLoadingTurnIds.contains(turnId) { return }
        toolLoadingTurnIds.insert(turnId)
        _Concurrency.Task {
            defer { toolLoadingTurnIds.remove(turnId) }
            do {
                let resolved = await resolveSessionId()
                guard let resolved else { return }
                let tools = try await client.listTurnTools(sessionId: resolved, turnId: turnId)
                var next = toolDetailsByCallId
                for tool in tools {
                    next[tool.toolCallId] = tool
                }
                toolDetailsByCallId = next
            } catch {
                // Best-effort; keep existing detail cache.
            }
        }
    }

    func removeQueuedMessage(_ messageId: String) {
        guard let client else { return }
        _Concurrency.Task {
            do {
                try await client.deleteMessage(messageId: messageId)
                await refreshQueue()
            } catch {
                // Best-effort removal; ignore errors.
            }
        }
    }

    func submitAskUserQuestion(toolCallId: String, answers: [String: String]) async {
        guard let client else { return }
        let resolved = await resolveSessionId()
        guard let resolved else { return }
        optimisticAskAnswers[toolCallId] = AskUserAnswerState(outcome: "submitted", answers: answers)
        rebuildThreadItems()
        do {
            try await client.submitAskUserQuestion(
                sessionId: resolved,
                toolCallId: toolCallId,
                outcome: "submitted",
                answers: answers
            )
            _ = await refreshMessages()
        } catch {
            optimisticAskAnswers.removeValue(forKey: toolCallId)
            rebuildThreadItems()
        }
    }

    func cancelAskUserQuestion(toolCallId: String) async {
        guard let client else { return }
        let resolved = await resolveSessionId()
        guard let resolved else { return }
        optimisticAskAnswers[toolCallId] = AskUserAnswerState(outcome: "cancelled", answers: [:])
        rebuildThreadItems()
        do {
            try await client.submitAskUserQuestion(
                sessionId: resolved,
                toolCallId: toolCallId,
                outcome: "cancelled",
                answers: [:]
            )
            _ = await refreshMessages()
        } catch {
            optimisticAskAnswers.removeValue(forKey: toolCallId)
            rebuildThreadItems()
        }
    }

    private func applyStreamingAssistantState(to base: [ChatMessage]) -> [ChatMessage] {
        guard let state = streamingAssistantState, !state.text.isEmpty else { return base }
        var next = base

        if let last = next.last, last.role == .assistant {
            if last.text == state.text {
                return next
            }
            if state.text.count >= last.text.count || state.isActive {
                next[next.count - 1] = ChatMessage(
                    id: last.id,
                    role: .assistant,
                    text: state.text,
                    attachments: last.attachments,
                    createdAt: last.createdAt
                )
            }
            return next
        }

        next.append(
            ChatMessage(
                id: state.messageId,
                role: .assistant,
                text: state.text,
                attachments: [],
                createdAt: state.createdAt
            )
        )
        return next
    }

    private func mergeMessages(
        _ existing: [ChatMessage],
        _ incoming: [ChatMessage],
        kind: ChatThreadMergeKind
    ) async -> [ChatMessage] {
        let key = ChatThreadMergeKey(
            sessionId: sessionId ?? "unknown",
            snapshotRev: threadSnapshotRev,
            lastEventSeq: lastEventSeq ?? 0,
            existingCount: existing.count,
            incomingCount: incoming.count,
            existingTailId: existing.last?.id,
            incomingTailId: incoming.last?.id,
            kind: kind
        )
        return await threadPipeline.mergeMessages(existing: existing, incoming: incoming, key: key)
    }

    private func mergeTurns(
        _ existing: [SessionTurn],
        _ incoming: [SessionTurn],
        kind: ChatThreadMergeKind
    ) async -> [SessionTurn] {
        let key = ChatThreadMergeKey(
            sessionId: sessionId ?? "unknown",
            snapshotRev: threadSnapshotRev,
            lastEventSeq: lastEventSeq ?? 0,
            existingCount: existing.count,
            incomingCount: incoming.count,
            existingTailId: existing.last?.turnId.stringValue,
            incomingTailId: incoming.last?.turnId.stringValue,
            kind: kind
        )
        return await threadPipeline.mergeTurns(existing: existing, incoming: incoming, key: key)
    }

    private func rebuildThreadItems() {
        let lastMessage = messages.last
        let key = ChatThreadBuildKey(
            sessionId: sessionId ?? "unknown",
            snapshotRev: threadSnapshotRev,
            lastEventSeq: lastEventSeq ?? 0,
            messageCount: messages.count,
            turnCount: latestTurns.count,
            eventCount: latestEvents.count,
            toolSummaryCount: latestToolSummaries.count,
            optimisticCount: optimisticAskAnswers.count,
            lastMessageId: lastMessage?.id,
            lastMessageHash: lastMessage?.text.hashValue ?? 0
        )
        if key == lastThreadBuildKey {
            return
        }
        lastThreadBuildKey = key
        threadBuildToken += 1
        let token = threadBuildToken
        let generation = sessionGeneration
        let input = ChatThreadBuildInput(
            key: key,
            messages: messages,
            turns: latestTurns,
            events: latestEvents,
            toolSummaries: latestToolSummaries,
            optimisticAnswers: optimisticAskAnswers
        )
        _Concurrency.Task {
            let output = await threadPipeline.buildThreadItems(input: input)
            guard token == threadBuildToken, generation == sessionGeneration else { return }
            guard output.key == lastThreadBuildKey else { return }
            threadItems = output.items
            activeAskToolCallId = output.activeAskToolCallId
            threadItemsRevision += 1
        }
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
            streamingAssistantState = StreamingAssistantState(
                turnId: turnId,
                messageId: turnId,
                text: assistantPartial,
                isActive: isActive,
                createdAt: turn.updatedAt
            )
        } else {
            streamingAssistantState?.text = assistantPartial
            streamingAssistantState?.isActive = isActive
        }

        messages = applyStreamingAssistantState(to: messages)
        rebuildThreadItems()
        updateWorkingState()
    }

    private func updateTurnStatus(from turn: SessionTurn) {
        turnStatus = TurnStatusSnapshot(
            status: turn.status,
            startedAt: turn.startedAt,
            updatedAt: turn.updatedAt,
            assistantMessage: assistantMessageContent(for: turn)
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

    private func updateSessionStateRev(_ nextRev: Int?) {
        guard let nextRev else { return }
        if let current = sessionStateRev {
            sessionStateRev = max(current, nextRev)
        } else {
            sessionStateRev = nextRev
        }
    }

    private func assistantMessageContent(for turn: SessionTurn) -> String {
        if let partial = turn.assistantPartial?.trimmingCharacters(in: .whitespacesAndNewlines),
           !partial.isEmpty {
            return partial
        }
        if let streaming = streamingAssistantState?.text.trimmingCharacters(in: .whitespacesAndNewlines),
           !streaming.isEmpty {
            return streaming
        }
        return messages.reversed().first(where: { $0.role == .assistant })?.text ?? ""
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
            let params = DaemonAPIClient.WorkspaceActiveSnapshotParams(limit: 50)
            let snapshot = try await client.getWorkspaceActiveSnapshot(workspaceId: workspace.id, params: params)
            await ATSHeadCache.shared.store(heads: snapshot.active.tasks.map { $0.primarySessionHead })
            var tasks = snapshot.active.tasks.map(WorkspaceTaskSummary.init)
            if tasks.isEmpty {
                let workspaceTasks = try await client.listWorkspaceTasks(workspaceId: workspace.id)
                if let task = workspaceTasks.first {
                    let sessions = try await client.listTaskSessions(taskId: task.id.stringValue)
                    let summaries = sessions.map(SessionSnapshotSummary.init)
                    let sortAt = task.lastActivityAt ?? task.updatedAt ?? task.createdAt
                    tasks = [WorkspaceTaskSummary(task: task, sessions: summaries, sortAt: sortAt)]
                }
            }
            guard let task = tasks.first else { return nil }
            guard let resolved = resolvePrimarySessionId(for: task) else { return nil }
            sessionId = resolved
            return resolved
        } catch {
            return nil
        }
    }

    private func resolvePrimarySessionId(for task: WorkspaceTaskSummary) -> String? {
        guard let primaryId = task.task.primarySessionId else { return nil }
        return primaryId.stringValue
    }

    private func resolveWorkspaceId() async -> String? {
        if let workspaceId { return workspaceId }
        guard let client else { return nil }
        if let sessionId {
            if let snapshot = try? await client.getSessionSnapshot(sessionId: sessionId, limit: 1, includeEvents: false) {
                let head = snapshot.head
                let resolved = head.session.workspaceId.stringValue
                workspaceId = resolved
                lastEventSeq = head.lastEventSeq
                updateSessionStateRev(head.stateRev)
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
        guard !isArchivedSession else { return }
        if !force, lastEventSeq != nil, workspaceId != nil { return }
        if let snapshot = try? await client.getSessionSnapshot(sessionId: sessionId, limit: 1, includeEvents: false) {
            let head = snapshot.head
            lastEventSeq = head.lastEventSeq
            updateSessionStateRev(head.stateRev)
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
        guard !isArchivedSession else { return }
        await primeStreamCursor()
        let subscription = WorkspaceActiveSnapshotSessionSubscription(
            sessionId: CtxID(sessionId),
            afterSeq: lastEventSeq
        )
        let message = WorkspaceActiveSnapshotClientMessage(
            type: "subscribe",
            sessionIds: [],
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
            guard let event = try? streamDecoder.decode(WorkspaceActiveSnapshotEvent.self, from: decrypted) else { return }
            handleStreamEvent(event)
            return
        }
        guard let event = try? streamDecoder.decode(WorkspaceActiveSnapshotEvent.self, from: data) else { return }
        handleStreamEvent(event)
    }

    private func handleStreamEvent(_ event: WorkspaceActiveSnapshotEvent) {
        guard let currentSessionId = sessionId else { return }
        switch event {
        case .sessionHeadDelta(_, _, let delta):
            let eventSessionId = delta.sessionId.stringValue
            if eventSessionId == currentSessionId {
                _Concurrency.Task { await ATSHeadCache.shared.apply(delta: delta) }
                lastEventSeq = delta.lastEventSeq
                if let nextRev = delta.stateRev {
                    if let current = sessionStateRev, nextRev > current + 1 {
                        _Concurrency.Task {
                            _ = await refreshMessages()
                            await refreshArtifacts()
                        }
                    }
                    updateSessionStateRev(nextRev)
                }
                if let turn = delta.turn {
                    applyTurnDelta(turn)
                }
                if let event = delta.event, event.eventType == "artifacts_set" {
                    if let updated = decodeArtifacts(from: event.payloadJson) {
                        applyArtifactsFromStream(updated)
                    } else {
                        _Concurrency.Task { await refreshArtifacts() }
                    }
                }
                if let message = delta.message {
                    if message.role == .assistant {
                        setPendingAssistantResponse(false)
                        streamingAssistantState = nil
                        updateWorkingState()
                    }
                    _Concurrency.Task {
                        _ = await refreshMessages()
                        await refreshQueue()
                    }
                } else if let turn = delta.turn, turn.status != .queued, turn.status != .running {
                    _Concurrency.Task { _ = await refreshMessages() }
                } else if let event = delta.event, event.eventType == "notice" {
                    _Concurrency.Task { _ = await refreshMessages() }
                }
            }
        case .sessionSummary(_, _, let summary):
            let eventSessionId = summary.session.id.stringValue
            if eventSessionId == currentSessionId {
                if let lastEventSeq = summary.lastEventSeq {
                    self.lastEventSeq = lastEventSeq
                }
                if let stateRev = summary.stateRev {
                    if let current = sessionStateRev, stateRev != current {
                        updateSessionStateRev(stateRev)
                        _Concurrency.Task {
                            _ = await refreshMessages()
                            await refreshQueue()
                            await refreshArtifacts()
                        }
                        return
                    }
                    updateSessionStateRev(stateRev)
                }
                _Concurrency.Task {
                    _ = await refreshMessages()
                    await refreshQueue()
                    await refreshArtifacts()
                }
            }
        case .sessionGap(_, _, let sessionId, let afterSeq, _):
            if sessionId.stringValue == currentSessionId {
                lastEventSeq = afterSeq
                sessionStateRev = nil
                streamingAssistantState = nil
                updateWorkingState()
                _Concurrency.Task {
                    await primeStreamCursor(force: true)
                    _ = await refreshMessages()
                    await refreshQueue()
                    await refreshArtifacts()
                }
            }
        default:
            break
        }
    }

    private static let sampleMessages: [ChatMessage] = [
        ChatMessage(
            id: UUID().uuidString,
            role: .assistant,
            text: "Welcome to ctx. What would you like to build today?",
            attachments: [],
            createdAt: formatIsoTimestamp(Date().addingTimeInterval(-240))
        ),
        ChatMessage(
            id: UUID().uuidString,
            role: .user,
            text: "A native chat screen with SwiftUI components.\nMake the user message bubble full-width and easy to copy.\nMatch the web styling and clamp long messages.\nAdd tap-to-expand for the full text on demand.\nKeep the status row copy affordance visible.",
            attachments: [],
            createdAt: formatIsoTimestamp(Date().addingTimeInterval(-180))
        ),
        ChatMessage(
            id: UUID().uuidString,
            role: .assistant,
            text: "Great. I will set up message bubbles and a composer that feels like ChatGPT.",
            attachments: [],
            createdAt: formatIsoTimestamp(Date().addingTimeInterval(-120))
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

private func stringValue(from value: JSONValue?) -> String? {
    switch value {
    case .string(let string):
        return string
    case .number(let number):
        return String(number)
    case .bool(let flag):
        return flag ? "true" : "false"
    default:
        return nil
    }
}

private func boolValue(from value: JSONValue?) -> Bool? {
    switch value {
    case .bool(let flag):
        return flag
    case .string(let string):
        let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if trimmed == "true" { return true }
        if trimmed == "false" { return false }
        return nil
    case .number(let number):
        return number != 0
    default:
        return nil
    }
}

private func arrayValue(from value: JSONValue?) -> [JSONValue]? {
    if case .array(let array) = value {
        return array
    }
    return nil
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
