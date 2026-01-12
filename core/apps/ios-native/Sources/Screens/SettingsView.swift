import SwiftUI
import _Concurrency

struct SettingsView: View {
    @EnvironmentObject private var connection: ConnectionStore
    let selectedWorkspace: WorkspaceSummary?

    @State private var settings: PublicSettings?
    @State private var isLoadingSettings = false
    @State private var settingsError: String?
    @State private var telemetryEnabled = true
    @State private var telemetryReady = false
    @State private var telemetrySaving = false

    @State private var mobileStatus: MobileAccessStatus?
    @State private var isLoadingMobileStatus = false
    @State private var mobileStatusError: String?

    @State private var entitlements: EntitlementsSnapshot?
    @State private var entitlementsLoading = false
    @State private var entitlementsError: String?
    @State private var hasSupabaseToken = false
    @State private var supabaseEmail: String?

    @State private var providers: [ProviderStatus] = []
    @State private var isLoadingProviders = false
    @State private var providersError: String?

    private let supabaseTokenStore = KeychainTokenStore(service: "rs.ctx.mobile", account: "supabaseToken")

    init(selectedWorkspace: WorkspaceSummary? = nil) {
        self.selectedWorkspace = selectedWorkspace
    }

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Settings")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 16) {
                            Text("Account")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            SettingsRowView(title: "User", value: userLabel)
                            NavigationLink {
                                WorkspaceSwitchView()
                            } label: {
                                SettingsLinkRowView(title: "Workspace", value: workspaceLabel)
                            }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("settings.workspace.link")
                            SettingsRowView(title: "Plan", value: planLabel)
                        }
                    }

                    if entitlementsLoading {
                        Text("Loading entitlements...")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    } else if let entitlementsError {
                        Text(entitlementsError)
                            .font(.caption)
                            .foregroundColor(.ctxError)
                    } else if !hasSupabaseToken {
                        Text("Add a Supabase token to load entitlements.")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 16) {
                            Text("Daemon Settings")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            Toggle("Usage analytics", isOn: telemetryBinding)
                                .tint(.ctxAccent)
                                .disabled(!telemetryReady || telemetrySaving)
                            SettingsRowView(title: "Dictation", value: dictationLabel)
                            SettingsRowView(title: "Title generation", value: titleGenerationLabel)
                            SettingsRowView(title: "Resource limits", value: resourceGovernanceLabel)
                        }
                        .foregroundColor(.ctxTextSecondary)
                    }

                    if isLoadingSettings {
                        Text("Loading daemon settings...")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    } else if let settingsError {
                        Text(settingsError)
                            .font(.caption)
                            .foregroundColor(.ctxError)
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Mobile Access")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            NavigationLink {
                                MobileAccessView()
                            } label: {
                                SettingsNavigationRowView(
                                    title: "Mobile Access",
                                    subtitle: "Manage remote tunnel access",
                                    statusPills: [
                                        StatusPill(text: mobileEntitlementLabel, tint: mobileEntitlementTint),
                                        StatusPill(text: mobileStatusLabel, tint: mobileStatusTint),
                                    ]
                                )
                            }
                            .buttonStyle(.plain)

                            if isLoadingMobileStatus {
                                Text("Loading mobile access status...")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                            } else if let mobileStatusError {
                                Text(mobileStatusError)
                                    .font(.caption)
                                    .foregroundColor(.ctxError)
                            }
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Providers")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            if isLoadingProviders {
                                ProgressView()
                                    .tint(.ctxAccent)
                            } else if let providersError {
                                Text(providersError)
                                    .font(.caption)
                                    .foregroundColor(.ctxError)
                            } else if providers.isEmpty {
                                Text("No providers detected.")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                            } else {
                                ForEach(providers, id: \.providerId) { provider in
                                    ProviderRowView(provider: provider)
                                }
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .onAppear {
            _Concurrency.Task {
                await refreshAll()
            }
        }
        .onChange(of: connection.isConnected) { _ in
            _Concurrency.Task { await refreshAll() }
        }
    }

    private var userLabel: String {
        if let email = supabaseEmail, !email.isEmpty {
            return email
        }
        return hasSupabaseToken ? "Signed in" : "Not signed in"
    }

    private var workspaceLabel: String {
        selectedWorkspace?.name ?? "No workspace"
    }

    private var planLabel: String {
        if entitlementsLoading {
            return "Loading"
        }
        if let entitlements {
            return planLabel(for: entitlements.planType)
        }
        return hasSupabaseToken ? "Unknown" : "Sign in required"
    }

    private var dictationLabel: String {
        guard let dictation = settings?.dictation else {
            return settingsFallbackLabel
        }
        if !dictation.enabled {
            return "Disabled"
        }
        let provider = dictation.provider
            .replacingOccurrences(of: "_", with: " ")
            .capitalized
        return provider.isEmpty ? "Enabled" : "Enabled - \(provider)"
    }

    private var titleGenerationLabel: String {
        guard let titleGeneration = settings?.titleGeneration else {
            return settingsFallbackLabel
        }
        if titleGeneration.apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return "Missing API key"
        }
        return titleGeneration.model.isEmpty ? "Configured" : "Configured - \(titleGeneration.model)"
    }

    private var resourceGovernanceLabel: String {
        guard let resourceGovernance = settings?.resourceGovernance else {
            return settingsFallbackLabel
        }
        if !resourceGovernance.enabled {
            return "Disabled"
        }
        let mode = resourceGovernance.mode.capitalized
        if let state = resourceGovernance.status?.state {
            let formatted = state.replacingOccurrences(of: "_", with: " ").capitalized
            return "\(mode) - \(formatted)"
        }
        return mode.isEmpty ? "Enabled" : mode
    }

    private var settingsFallbackLabel: String {
        if connection.apiClient == nil {
            return "Disconnected"
        }
        if isLoadingSettings {
            return "Loading"
        }
        return "Unavailable"
    }

    private var mobileStatusLabel: String {
        if isLoadingMobileStatus {
            return "Loading"
        }
        if let status = mobileStatus {
            return status.enabled ? "Enabled" : "Disabled"
        }
        if connection.apiClient == nil {
            return "Disconnected"
        }
        return "Unknown"
    }

    private var mobileStatusTint: Color {
        if isLoadingMobileStatus {
            return .ctxAccent
        }
        if let status = mobileStatus {
            return status.enabled ? .ctxAccent : .ctxTextMuted
        }
        return .ctxTextMuted
    }

    private var mobileEntitlementLabel: String {
        if entitlementsLoading {
            return "Loading"
        }
        if let entitlements {
            return entitlements.isFeatureEnabled("remote_mobile_access") ? "Pro enabled" : "Pro required"
        }
        return hasSupabaseToken ? "Unknown" : "Sign in required"
    }

    private var mobileEntitlementTint: Color {
        if entitlementsLoading {
            return .ctxAccent
        }
        if let entitlements {
            return entitlements.isFeatureEnabled("remote_mobile_access") ? .ctxAccent : .ctxWarning
        }
        return .ctxTextMuted
    }

    private func planLabel(for plan: EntitlementsSnapshot.PlanType) -> String {
        switch plan {
        case .freeLocal:
            return "Free/Local"
        case .pro:
            return "Pro"
        case .team:
            return "Team"
        case .enterprise:
            return "Enterprise"
        }
    }

    private var telemetryBinding: Binding<Bool> {
        Binding(
            get: { telemetryEnabled },
            set: { newValue in
                telemetryEnabled = newValue
                guard telemetryReady, !telemetrySaving, !isLoadingSettings else { return }
                _Concurrency.Task { await updateTelemetry(enabled: newValue) }
            }
        )
    }

    @MainActor
    private func refreshAll() async {
        await refreshSettings()
        await refreshProviders()
        await refreshMobileStatus()
        await refreshEntitlements()
    }

    @MainActor
    private func refreshSettings() async {
        guard let client = connection.apiClient else {
            settings = nil
            settingsError = nil
            telemetryReady = false
            return
        }
        isLoadingSettings = true
        settingsError = nil
        do {
            let next = try await client.getSettings()
            settings = next
            telemetryEnabled = next.telemetry?.enabled ?? true
            telemetryReady = true
        } catch {
            settings = nil
            telemetryReady = false
            settingsError = "Unable to load settings."
        }
        isLoadingSettings = false
    }

    @MainActor
    private func updateTelemetry(enabled: Bool) async {
        guard let client = connection.apiClient else { return }
        guard let telemetry = settings?.telemetry else { return }
        settingsError = nil
        telemetrySaving = true
        let update = SettingsUpdate(telemetry: TelemetrySettingsUpdate(enabled: enabled, endpoint: telemetry.endpoint))
        do {
            let next = try await client.updateSettings(update)
            settings = next
            telemetryEnabled = next.telemetry?.enabled ?? enabled
        } catch {
            settingsError = "Unable to update telemetry settings."
            telemetryEnabled = settings?.telemetry?.enabled ?? enabled
        }
        telemetrySaving = false
    }

    @MainActor
    private func refreshProviders() async {
        guard let client = connection.apiClient else {
            providers = []
            providersError = nil
            return
        }
        isLoadingProviders = true
        providersError = nil
        do {
            providers = try await client.listProviders()
        } catch {
            providers = []
            providersError = "Unable to load providers."
        }
        isLoadingProviders = false
    }

    @MainActor
    private func refreshMobileStatus() async {
        guard let client = connection.apiClient else {
            mobileStatus = nil
            mobileStatusError = nil
            return
        }
        isLoadingMobileStatus = true
        mobileStatusError = nil
        do {
            mobileStatus = try await client.getMobileAccessStatus()
        } catch {
            mobileStatusError = "Unable to fetch mobile access status."
        }
        isLoadingMobileStatus = false
    }

    @MainActor
    private func refreshEntitlements() async {
        entitlementsError = nil
        let token: String?
        do {
            token = try await supabaseTokenStore.loadToken()
        } catch {
            hasSupabaseToken = false
            entitlements = nil
            supabaseEmail = nil
            entitlementsError = "Failed to load Supabase token."
            entitlementsLoading = false
            return
        }
        guard let token else {
            hasSupabaseToken = false
            entitlements = nil
            supabaseEmail = nil
            entitlementsLoading = false
            return
        }
        let trimmed = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            hasSupabaseToken = false
            entitlements = nil
            supabaseEmail = nil
            entitlementsLoading = false
            return
        }
        hasSupabaseToken = true
        let tokenInfo = SupabaseEntitlementsClient.tokenInfo(from: trimmed)
        supabaseEmail = tokenInfo?.email
        entitlementsLoading = true
        do {
            entitlements = try await SupabaseEntitlementsClient.fetchEntitlements(token: trimmed)
        } catch {
            entitlements = nil
            entitlementsError = "Unable to load entitlements."
        }
        entitlementsLoading = false
    }
}

private struct SettingsRowView: View {
    let title: String
    let value: String

    var body: some View {
        HStack {
            Text(title)
                .foregroundColor(.ctxTextSecondary)
            Spacer()
            Text(value)
                .foregroundColor(.ctxTextPrimary)
                .multilineTextAlignment(.trailing)
        }
        .font(.subheadline)
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct SettingsLinkRowView: View {
    let title: String
    let value: String

    var body: some View {
        HStack {
            Text(title)
                .foregroundColor(.ctxTextSecondary)
            Spacer()
            Text(value)
                .foregroundColor(.ctxTextPrimary)
                .multilineTextAlignment(.trailing)
            Image(systemName: "chevron.right")
                .foregroundColor(.ctxTextSecondary)
                .font(.caption)
        }
        .font(.subheadline)
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

struct WorkspaceSwitchView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var workspaceSelection: WorkspaceSelectionStore
    @EnvironmentObject private var workbenchSelection: WorkbenchSelectionStore
    @Environment(\.dismiss) private var dismiss

    @State private var workspaces: [WorkspaceSummary] = []
    @State private var isLoading = false
    @State private var errorMessage: String?

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 16) {
                    Text("Workspace")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .padding(.top, 12)

                    if isLoading {
                        Text("Loading workspaces...")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    } else if let errorMessage {
                        Text(errorMessage)
                            .font(.caption)
                            .foregroundColor(.ctxError)
                    }

                    if workspaces.isEmpty, !isLoading {
                        Text("No workspaces available.")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    } else {
                        VStack(spacing: 12) {
                            ForEach(workspaces) { workspace in
                                Button {
                                    selectWorkspace(workspace)
                                } label: {
                                    WorkspaceSwitchRowView(
                                        workspace: workspace,
                                        isSelected: workspace.id == workspaceSelection.workspaceId
                                    )
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 24)
            }
        }
        .navigationTitle("Workspace")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .task {
            await loadWorkspaces()
        }
        .onChange(of: connection.isConnected) { _ in
            _Concurrency.Task { await loadWorkspaces() }
        }
    }

    @MainActor
    private func loadWorkspaces() async {
        guard let client = connection.apiClient else {
            workspaces = []
            errorMessage = "Connect to a daemon to load workspaces."
            return
        }
        isLoading = true
        errorMessage = nil
        do {
            workspaces = try await client.listWorkspaces()
        } catch {
            workspaces = []
            errorMessage = "Failed to load workspaces."
        }
        isLoading = false
    }

    private func selectWorkspace(_ workspace: WorkspaceSummary) {
        workspaceSelection.setWorkspace(workspace, daemonKey: connection.baseURLText)
        workbenchSelection.setContext(daemonKey: connection.baseURLText, workspaceId: workspace.id)
        workbenchSelection.setSelection(taskId: nil, trackId: nil, sessionId: nil)
        dismiss()
    }
}

private struct WorkspaceSwitchRowView: View {
    let workspace: WorkspaceSummary
    let isSelected: Bool

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Color.ctxSurfaceRaised)
                Image(systemName: "folder")
                    .foregroundColor(.ctxAccent)
            }
            .frame(width: 36, height: 36)

            VStack(alignment: .leading, spacing: 2) {
                Text(workspace.name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(workspaceDetailText)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }

            Spacer()

            if isSelected {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundColor(.ctxAccent)
            }
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(isSelected ? 0.75 : 0.6), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(isSelected ? Color.ctxAccent.opacity(0.6) : Color.ctxLine, lineWidth: 1)
        )
    }

    private var workspaceDetailText: String {
        let url = URL(fileURLWithPath: workspace.rootPath)
        return url.lastPathComponent.isEmpty ? workspace.rootPath : url.lastPathComponent
    }
}

private struct ProviderRowView: View {
    let provider: ProviderStatus

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "bolt.circle")
                .foregroundColor(.ctxAccent)
            VStack(alignment: .leading, spacing: 4) {
                Text(displayName)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(detailText)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
            GlassPill(text: healthLabel, tint: healthTint)
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }

    private var displayName: String {
        provider.providerId.replacingOccurrences(of: "_", with: " ").capitalized
    }

    private var detailText: String {
        if provider.installed == false {
            return "Not installed"
        }
        if let version = provider.version, !version.isEmpty {
            return version
        }
        if let path = provider.detectedPath, !path.isEmpty {
            return path
        }
        return "Installed"
    }

    private var healthLabel: String {
        provider.health.replacingOccurrences(of: "_", with: " ").capitalized
    }

    private var healthTint: Color {
        let health = provider.health.lowercased()
        if health == "ok" || health == "healthy" {
            return .ctxAccent
        }
        if health == "warning" || health == "unsupported version" || health == "unsupported_version" || health == "degraded" {
            return .ctxWarning
        }
        return .ctxError
    }
}

private struct StatusPill {
    let text: String
    let tint: Color
}

private struct SettingsNavigationRowView: View {
    let title: String
    let subtitle: String
    let statusPills: [StatusPill]

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "antenna.radiowaves.left.and.right")
                .foregroundColor(.ctxAccent)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
            HStack(spacing: 6) {
                ForEach(statusPills.indices, id: \.self) { index in
                    let pill = statusPills[index]
                    GlassPill(text: pill.text, tint: pill.tint)
                }
            }
            Image(systemName: "chevron.right")
                .foregroundColor(.ctxTextSecondary)
                .font(.caption)
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

#Preview {
    SettingsView()
        .environmentObject(ConnectionStore())
}
