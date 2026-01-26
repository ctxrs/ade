import SwiftUI
import UIKit
import _Concurrency

struct SettingsView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var pushManager: PushNotificationManager
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
    @State private var providerOptions: [String: DaemonAPIClient.ProviderOptions] = [:]
    @State private var providerCheckBusy: Set<String> = []
    @State private var providerAuthBusy: Set<String> = []
    @State private var providerVerifyBusy: Set<String> = []
    @State private var providerInstallBusy: Set<String> = []
    @State private var providerInstallInfo: [String: InstallInfo] = [:]
    @State private var providerVerifyResults: [String: JSONValue] = [:]

    @State private var routingEntry: ModelRoutingEntry?
    @State private var routingProviderId = ""
    @State private var routingModelId = ""
    @State private var routingError: String?

    @State private var pushError: String?
    @State private var pushStatusMessage: String?
    @State private var pushWorking = false

    private var isUITestPreview: Bool {
        UITestOverrides.isActive(.settings)
    }

    private let supabaseTokenStore = KeychainTokenStore(service: "rs.ctx.mobile", account: "supabaseToken")
    private let deviceIdentityStore = DeviceIdentityStore()

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

                    SettingsSectionView(title: "Account") {
                        NavigationLink {
                            AccountSettingsView(
                                userLabel: userLabel,
                                workspaceLabel: workspaceLabel,
                                planLabel: planLabel,
                                entitlementsLoading: entitlementsLoading,
                                entitlementsError: entitlementsError,
                                hasSupabaseToken: hasSupabaseToken
                            )
                        } label: {
                            SettingsNavigationRowView(
                                title: "Account",
                                subtitle: accountSummary,
                                statusPills: []
                            )
                        }
                        .buttonStyle(.plain)
                    }

                    SettingsSectionView(title: "Daemon") {
                        NavigationLink {
                            DaemonSettingsView(
                                telemetryBinding: telemetryBinding,
                                telemetryReady: telemetryReady,
                                telemetrySaving: telemetrySaving,
                                dictationLabel: dictationLabel,
                                titleGenerationLabel: titleGenerationLabel,
                                resourceGovernanceLabel: resourceGovernanceLabel,
                                isLoadingSettings: isLoadingSettings,
                                settingsError: settingsError
                            )
                        } label: {
                            SettingsNavigationRowView(
                                title: "Daemon Settings",
                                subtitle: daemonSummary,
                                statusPills: []
                            )
                        }
                        .buttonStyle(.plain)
                    }

                    SettingsSectionView(title: "Mobile Access") {
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
                    }

                    SettingsSectionView(title: "Notifications") {
                        NavigationLink {
                            NotificationsSettingsView(
                                pushBinding: pushBinding,
                                pushToggleDisabled: pushToggleDisabled,
                                pushEntitlementLabel: pushEntitlementLabel,
                                pushEntitlementTint: pushEntitlementTint,
                                pushStatusLabel: pushStatusLabel,
                                pushStatusTint: pushStatusTint,
                                pushStatusMessage: pushStatusMessage,
                                pushError: pushError,
                                managerError: pushManager.lastError
                            )
                        } label: {
                            SettingsNavigationRowView(
                                title: "Notifications",
                                subtitle: notificationsSummary,
                                statusPills: [
                                    StatusPill(text: pushEntitlementLabel, tint: pushEntitlementTint)
                                ]
                            )
                        }
                        .buttonStyle(.plain)
                    }

                    SettingsSectionView(title: "Model Routing") {
                        NavigationLink {
                            ModelRoutingSettingsView(
                                routingEnabled: routingEnabled,
                                routingProviderChoices: routingProviderChoices,
                                routingModelChoices: routingModelChoices,
                                routingProviderId: $routingProviderId,
                                routingModelId: $routingModelId,
                                routingProviderLabel: routingProviderLabel,
                                routingModelLabel: routingModelLabel,
                                routingEntry: routingEntry,
                                routingError: $routingError,
                                routingCanSave: routingCanSave,
                                providerLabel: providerLabel,
                                onSave: { _Concurrency.Task { await saveRoutingDefaults() } },
                                onClear: { _Concurrency.Task { await clearRoutingDefaults() } }
                            )
                        } label: {
                            SettingsNavigationRowView(
                                title: "Model Routing",
                                subtitle: routingSummary,
                                statusPills: []
                            )
                        }
                        .buttonStyle(.plain)
                    }

                    SettingsSectionView(title: "Providers") {
                        NavigationLink {
                            ProvidersSettingsView(
                                workspaceLabel: workspaceLabel,
                                isLoadingProviders: isLoadingProviders,
                                providersError: providersError,
                                visibleProviders: visibleProviders,
                                providerDetailText: providerDetailText,
                                providerStatusPill: providerStatusPill,
                                providerActions: providerActions
                            )
                        } label: {
                            SettingsNavigationRowView(
                                title: "Providers",
                                subtitle: providersSummary,
                                statusPills: []
                            )
                        }
                        .buttonStyle(.plain)
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
        .onChange(of: pushManager.pushToken) { _ in
            guard !isUITestPreview else { return }
            guard pushManager.isEnabled else { return }
            _Concurrency.Task { await syncPushRegistration(enabled: true) }
        }
        .onChange(of: routingProviderId) { newValue in
            guard !isUITestPreview else { return }
            guard let workspaceId = selectedWorkspaceId, !newValue.isEmpty else { return }
            _Concurrency.Task { await ensureProviderOptions(newValue, workspaceId: workspaceId, force: false) }
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

    private var selectedWorkspaceId: String? {
        selectedWorkspace?.id
    }

    private var daemonKey: String? {
        let raw = connection.secureConfig?.baseURL ?? connection.baseURLText
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
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

    private var accountSummary: String {
        "\(userLabel) · \(planLabel)"
    }

    private var daemonSummary: String {
        let parts = [dictationLabel, titleGenerationLabel, resourceGovernanceLabel]
            .filter { !$0.isEmpty }
        return parts.joined(separator: " · ")
    }

    private var notificationsSummary: String {
        pushStatusLabel
    }

    private var routingSummary: String {
        if !routingEnabled { return "Select a workspace" }
        if routingProviderId.isEmpty && routingModelId.isEmpty { return "Not configured" }
        if routingModelId.isEmpty { return routingProviderLabel }
        return "\(routingProviderLabel) · \(routingModelLabel)"
    }

    private var providersSummary: String {
        let count = visibleProviders.count
        if count == 0 { return "No providers" }
        return "\(count) provider" + (count == 1 ? "" : "s")
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

    private var visibleProviders: [ProviderStatus] {
        providers.filter { $0.details?["ui_hidden"] != "true" }
    }

    private var routingEnabled: Bool {
        if isUITestPreview { return true }
        return connection.apiClient != nil && selectedWorkspaceId != nil && daemonKey != nil
    }

    private var routingProviderChoices: [String] {
        let installed = visibleProviders.filter { $0.installed && $0.health == "ok" }
        return installed.map { $0.providerId }.filter { !$0.isEmpty }
    }

    private var routingProviderLabel: String {
        routingProviderId.isEmpty ? "Select…" : providerLabel(routingProviderId)
    }

    private var routingModelChoices: [String] {
        extractModelIds(from: providerOptions[routingProviderId]?.models)
    }

    private var routingModelLabel: String {
        routingModelId.isEmpty ? "Select…" : routingModelId
    }

    private var routingCanSave: Bool {
        routingEnabled && !routingProviderId.isEmpty && !routingModelId.isEmpty
    }

    private var pushToggleDisabled: Bool {
        if connection.apiClient == nil { return true }
        if pushWorking { return true }
        if let entitlements, !entitlements.isFeatureEnabled("push_notifications") { return true }
        return false
    }

    private var pushStatusLabel: String {
        if !pushManager.isEnabled {
            return "Disabled"
        }
        if pushManager.authorizationStatus == .denied {
            return "Permission denied"
        }
        if pushManager.pushToken == nil {
            return "Waiting for token"
        }
        return "Enabled"
    }

    private var pushStatusTint: Color {
        if !pushManager.isEnabled {
            return .ctxTextMuted
        }
        if pushManager.authorizationStatus == .denied {
            return .ctxWarning
        }
        if pushManager.pushToken == nil {
            return .ctxAccent
        }
        return .ctxAccent
    }

    private var pushBinding: Binding<Bool> {
        Binding(
            get: { pushManager.isEnabled },
            set: { newValue in
                pushManager.setEnabled(newValue)
                _Concurrency.Task { await syncPushRegistration(enabled: newValue) }
            }
        )
    }

    private var pushEntitlementLabel: String {
        if entitlementsLoading {
            return "Loading"
        }
        if let entitlements {
            return entitlements.isFeatureEnabled("push_notifications") ? "Pro enabled" : "Pro required"
        }
        return hasSupabaseToken ? "Unknown" : "Sign in required"
    }

    private var pushEntitlementTint: Color {
        if entitlementsLoading {
            return .ctxAccent
        }
        if let entitlements {
            return entitlements.isFeatureEnabled("push_notifications") ? .ctxAccent : .ctxWarning
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
    private func applyUITestFixtures() {
        settings = UITestFixtures.settings
        settingsError = nil
        telemetryEnabled = UITestFixtures.settings.telemetry?.enabled ?? true
        telemetryReady = true
        isLoadingSettings = false
        mobileStatus = UITestFixtures.mobileAccessStatus
        mobileStatusError = nil
        isLoadingMobileStatus = false
        entitlements = UITestFixtures.entitlements
        entitlementsError = nil
        entitlementsLoading = false
        hasSupabaseToken = true
        supabaseEmail = UITestFixtures.supabaseEmail
        providers = UITestFixtures.providers
        providersError = nil
        isLoadingProviders = false
        routingEntry = UITestFixtures.routingEntry
        routingProviderId = UITestFixtures.routingEntry.providerId
        routingModelId = UITestFixtures.routingEntry.modelId
        routingError = nil
    }

    @MainActor
    private func refreshAll() async {
        if isUITestPreview {
            applyUITestFixtures()
            return
        }
        await refreshSettings()
        await refreshProviders()
        await refreshMobileStatus()
        await refreshEntitlements()
        await pushManager.refreshAuthorizationStatus()
        refreshRoutingDefaults()
        if pushManager.isEnabled {
            await syncPushRegistration(enabled: true)
        }
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
            if let workspaceId = selectedWorkspaceId {
                for provider in providers {
                    _Concurrency.Task {
                        await ensureProviderOptions(provider.providerId, workspaceId: workspaceId, force: false)
                    }
                }
            }
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

    private func refreshRoutingDefaults() {
        routingEntry = nil
        routingProviderId = ""
        routingModelId = ""
        routingError = nil
        guard let daemonKey, let workspaceId = selectedWorkspaceId else { return }
        guard let entry = ModelRoutingDefaults.load(daemonKey: daemonKey, workspaceId: workspaceId) else { return }
        routingEntry = entry
        routingProviderId = entry.providerId
        routingModelId = entry.modelId
    }

    @MainActor
    private func ensureProviderOptions(_ providerId: String, workspaceId: String, force: Bool) async {
        guard let client = connection.apiClient else { return }
        if !force, providerOptions[providerId] != nil { return }
        if providerCheckBusy.contains(providerId) { return }
        providerCheckBusy.insert(providerId)
        defer { providerCheckBusy.remove(providerId) }
        do {
            let options = try await client.getProviderOptions(workspaceId: workspaceId, providerId: providerId)
            providerOptions[providerId] = options
        } catch {
            providersError = "Unable to load provider details."
        }
    }

    @MainActor
    private func saveRoutingDefaults() async {
        routingError = nil
        guard let daemonKey, let workspaceId = selectedWorkspaceId else {
            routingError = "Select a workspace to save routing."
            return
        }
        guard !routingProviderId.isEmpty, !routingModelId.isEmpty else {
            routingError = "Select a harness and model."
            return
        }
        ModelRoutingDefaults.save(
            daemonKey: daemonKey,
            workspaceId: workspaceId,
            providerId: routingProviderId,
            modelId: routingModelId
        )
        refreshRoutingDefaults()
    }

    @MainActor
    private func clearRoutingDefaults() async {
        routingError = nil
        guard let daemonKey, let workspaceId = selectedWorkspaceId else { return }
        ModelRoutingDefaults.clear(daemonKey: daemonKey, workspaceId: workspaceId)
        refreshRoutingDefaults()
    }

    private func providerLabel(_ providerId: String) -> String {
        providerId
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
            .capitalized
    }

    private func providerDetailText(_ provider: ProviderStatus) -> String {
        let providerId = provider.providerId
        if let installInfo = providerInstallInfo[providerId] {
            switch installInfo.state {
            case .running:
                if let lastEvent = installInfo.lastEvent?.message, !lastEvent.isEmpty {
                    return lastEvent
                }
                return "Installing..."
            case .failed:
                if let error = installInfo.error, !error.isEmpty {
                    return "Install failed: \(error)"
                }
                return "Install failed."
            case .succeeded:
                break
            }
        }

        if provider.installed == false {
            return "Not installed."
        }

        if let options = providerOptions[providerId] {
            if options.probeOk == false {
                return options.probeError?.isEmpty == false ? options.probeError! : "Probe failed."
            }
            if options.authRequired {
                return "Auth required."
            }
        }

        if let version = provider.version, !version.isEmpty {
            return version
        }
        if let path = provider.detectedPath, !path.isEmpty {
            return path
        }
        return "Installed."
    }

    private func providerStatusPill(_ provider: ProviderStatus) -> StatusPill {
        let providerId = provider.providerId
        if let installInfo = providerInstallInfo[providerId] {
            switch installInfo.state {
            case .running:
                return StatusPill(text: "Installing", tint: .ctxAccent)
            case .failed:
                return StatusPill(text: "Install failed", tint: .ctxError)
            case .succeeded:
                break
            }
        }

        if provider.installed == false {
            return StatusPill(text: "Not installed", tint: .ctxWarning)
        }

        if providerCheckBusy.contains(providerId) {
            return StatusPill(text: "Checking", tint: .ctxAccent)
        }
        if providerAuthBusy.contains(providerId) {
            return StatusPill(text: "Authenticating", tint: .ctxAccent)
        }
        if providerVerifyBusy.contains(providerId) {
            return StatusPill(text: "Verifying", tint: .ctxAccent)
        }

        if let options = providerOptions[providerId] {
            if options.probeOk == false {
                return StatusPill(text: "Probe failed", tint: .ctxError)
            }
            if options.authRequired {
                return StatusPill(text: "Auth required", tint: .ctxWarning)
            }
        }

        if let verifyStatus = verifyStatus(for: providerId) {
            return verifyStatus
        }

        return StatusPill(text: provider.health.replacingOccurrences(of: "_", with: " ").capitalized, tint: healthTint(for: provider))
    }

    @ViewBuilder
    private func providerActions(for provider: ProviderStatus) -> some View {
        let providerId = provider.providerId
        let isInstalled = provider.installed
        let workspaceId = selectedWorkspaceId
        let options = providerOptions[providerId]
        let authMethods = extractAuthMethods(from: options?.authMethods)
        let authRequired = options?.authRequired == true || !authMethods.isEmpty
        let installBusy = providerInstallBusy.contains(providerId) || providerInstallInfo[providerId]?.state == .running
        let authBusy = providerAuthBusy.contains(providerId)
        let verifyBusy = providerVerifyBusy.contains(providerId)
        let checkBusy = providerCheckBusy.contains(providerId)

        if !isInstalled || authRequired || workspaceId != nil {
            HStack(spacing: 10) {
                if !isInstalled {
                    Button {
                        _Concurrency.Task { await installProvider(providerId) }
                    } label: {
                        Text(installBusy ? "Installing..." : "Install")
                    }
                    .buttonStyle(CtxGhostButtonStyle())
                    .disabled(installBusy || connection.apiClient == nil)
                } else {
                    if authRequired {
                        if authMethods.count > 1 {
                            Menu {
                                ForEach(authMethods) { method in
                                    Button(method.label) {
                                        _Concurrency.Task { await authenticateProvider(providerId, methodId: method.id) }
                                    }
                                }
                            } label: {
                                Text(authBusy ? "Authenticating..." : "Authenticate")
                            }
                            .disabled(authBusy || workspaceId == nil)
                            .buttonStyle(CtxGhostButtonStyle())
                        } else {
                            Button {
                                _Concurrency.Task { await authenticateProvider(providerId, methodId: authMethods.first?.id) }
                            } label: {
                                Text(authBusy ? "Authenticating..." : "Authenticate")
                            }
                            .buttonStyle(CtxGhostButtonStyle())
                            .disabled(authBusy || workspaceId == nil)
                        }
                    }

                    Button {
                        _Concurrency.Task { await verifyProvider(providerId) }
                    } label: {
                        Text(verifyBusy ? "Verifying..." : "Verify")
                    }
                    .buttonStyle(CtxGhostButtonStyle())
                    .disabled(verifyBusy || workspaceId == nil)
                }

                if let workspaceId {
                    Button {
                        _Concurrency.Task { await ensureProviderOptions(providerId, workspaceId: workspaceId, force: true) }
                    } label: {
                        Text(checkBusy ? "Checking..." : "Refresh")
                    }
                    .buttonStyle(CtxGhostButtonStyle())
                    .disabled(checkBusy)
                }
            }
        }
    }

    @MainActor
    private func installProvider(_ providerId: String) async {
        guard let client = connection.apiClient else {
            providersError = "Connect to a daemon to install providers."
            return
        }
        if providerInstallBusy.contains(providerId) { return }
        providerInstallBusy.insert(providerId)
        defer { providerInstallBusy.remove(providerId) }
        providersError = nil
        do {
            let response = try await client.installProvider(providerId: providerId)
            await pollInstallStatus(installId: response.installId, providerId: providerId)
            await refreshProviders()
        } catch {
            providersError = "Failed to start install."
        }
    }

    @MainActor
    private func pollInstallStatus(installId: String, providerId: String) async {
        guard let client = connection.apiClient else { return }
        for _ in 0..<30 {
            do {
                let info = try await client.getInstall(installId: installId)
                providerInstallInfo[providerId] = info
                if info.state != .running {
                    return
                }
            } catch {
                providersError = "Failed to check install status."
                return
            }
            try? await _Concurrency.Task.sleep(nanoseconds: 1_000_000_000)
        }
    }

    @MainActor
    private func authenticateProvider(_ providerId: String, methodId: String?) async {
        guard let client = connection.apiClient else {
            providersError = "Connect to a daemon to authenticate providers."
            return
        }
        guard let workspaceId = selectedWorkspaceId else {
            providersError = "Select a workspace to authenticate."
            return
        }
        if providerAuthBusy.contains(providerId) { return }
        providerAuthBusy.insert(providerId)
        defer { providerAuthBusy.remove(providerId) }
        providersError = nil
        do {
            let response = try await client.authenticateProviderForWorkspace(
                workspaceId: workspaceId,
                providerId: providerId,
                methodId: methodId
            )
            _ = handleAuthResponse(response)
            await ensureProviderOptions(providerId, workspaceId: workspaceId, force: true)
        } catch {
            providersError = "Failed to authenticate provider."
        }
    }

    @MainActor
    private func verifyProvider(_ providerId: String) async {
        guard let client = connection.apiClient else {
            providersError = "Connect to a daemon to verify providers."
            return
        }
        guard let workspaceId = selectedWorkspaceId else {
            providersError = "Select a workspace to verify."
            return
        }
        if providerVerifyBusy.contains(providerId) { return }
        providerVerifyBusy.insert(providerId)
        defer { providerVerifyBusy.remove(providerId) }
        providersError = nil
        do {
            let response = try await client.verifyProviderForWorkspace(workspaceId: workspaceId, providerId: providerId)
            providerVerifyResults[providerId] = response
            await ensureProviderOptions(providerId, workspaceId: workspaceId, force: true)
        } catch {
            providersError = "Failed to verify provider."
        }
    }

    @MainActor
    private func syncPushRegistration(enabled: Bool) async {
        guard let client = connection.apiClient else {
            pushError = "Connect to a daemon to manage push approvals."
            return
        }
        pushWorking = true
        pushError = nil
        pushStatusMessage = nil
        do {
            let identity = try await deviceIdentityStore.loadOrCreate()
            if enabled, pushManager.authorizationStatus == .denied {
                pushError = "Notification permission denied."
                pushWorking = false
                return
            }
            if enabled, pushManager.pushToken == nil {
                pushStatusMessage = "Waiting for APNs token."
                pushWorking = false
                return
            }
            let payload = DaemonAPIClient.RegisterMobileDeviceRequest(
                deviceId: identity.deviceId,
                deviceLabel: UIDevice.current.name,
                platform: UIDevice.current.systemName,
                pushToken: enabled ? pushManager.pushToken : nil,
                pushProvider: enabled && pushManager.pushToken != nil ? "apns" : nil,
                publicKey: identity.publicKey,
                appVersion: appVersionString()
            )
            _ = try await client.registerMobileDevice(payload)
            pushStatusMessage = enabled ? "Push approvals enabled." : "Push approvals disabled."
        } catch {
            pushError = "Failed to update push registration."
        }
        pushWorking = false
    }

    private func appVersionString() -> String? {
        let info = Bundle.main.infoDictionary
        return info?["CFBundleShortVersionString"] as? String
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

    private struct ProviderAuthMethod: Identifiable {
        let id: String
        let label: String
    }

    private func extractAuthMethods(from value: JSONValue?) -> [ProviderAuthMethod] {
        guard let value else { return [] }
        switch value {
        case .array(let items):
            let methods = items.compactMap { item -> ProviderAuthMethod? in
                switch item {
                case .string(let id):
                    let trimmed = id.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !trimmed.isEmpty else { return nil }
                    return ProviderAuthMethod(id: trimmed, label: trimmed)
                case .object(let object):
                    let id = stringValue(from: object, keys: ["methodId", "method_id", "id", "type", "name"])
                    let label = stringValue(from: object, keys: ["label", "name"]) ?? id
                    guard let id, !id.isEmpty else { return nil }
                    return ProviderAuthMethod(id: id, label: label ?? id)
                default:
                    return nil
                }
            }
            return uniqueAuthMethods(methods)
        case .object(let object):
            if let choices = object["choices"] {
                return extractAuthMethods(from: choices)
            }
            if let id = stringValue(from: object, keys: ["methodId", "method_id", "id", "type", "name"]) {
                let label = stringValue(from: object, keys: ["label", "name"]) ?? id
                return [ProviderAuthMethod(id: id, label: label)]
            }
            return []
        default:
            return []
        }
    }

    private func uniqueAuthMethods(_ methods: [ProviderAuthMethod]) -> [ProviderAuthMethod] {
        var seen = Set<String>()
        var out: [ProviderAuthMethod] = []
        for method in methods {
            let trimmed = method.id.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, !seen.contains(trimmed) else { continue }
            seen.insert(trimmed)
            out.append(method)
        }
        return out
    }

    private func verifyStatus(for providerId: String) -> StatusPill? {
        let value = providerVerifyResults[providerId] ?? providerOptions[providerId]?.verify
        guard let value else { return nil }
        if case .bool(let ok) = value {
            return StatusPill(text: ok ? "Verified" : "Verify failed", tint: ok ? .ctxAccent : .ctxError)
        }
        if case .string(let status) = value {
            let trimmed = status.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty else { return nil }
            let lower = trimmed.lowercased()
            if ["ok", "success", "verified", "healthy"].contains(lower) {
                return StatusPill(text: "Verified", tint: .ctxAccent)
            }
            if ["failed", "error", "invalid"].contains(lower) {
                return StatusPill(text: "Verify failed", tint: .ctxError)
            }
            return StatusPill(text: trimmed, tint: .ctxTextSecondary)
        }
        if case .object(let object) = value {
            if let ok = boolValue(from: object, keys: ["ok", "success", "verified"]) {
                return StatusPill(text: ok ? "Verified" : "Verify failed", tint: ok ? .ctxAccent : .ctxError)
            }
            if let status = stringValue(from: object, keys: ["status", "state", "result"]) {
                let lower = status.lowercased()
                if ["ok", "success", "verified", "healthy"].contains(lower) {
                    return StatusPill(text: "Verified", tint: .ctxAccent)
                }
                if ["failed", "error", "invalid"].contains(lower) {
                    return StatusPill(text: "Verify failed", tint: .ctxError)
                }
                return StatusPill(text: status, tint: .ctxTextSecondary)
            }
            if let message = stringValue(from: object, keys: ["message", "error"]) {
                return StatusPill(text: message, tint: .ctxWarning)
            }
        }
        return nil
    }

    private func handleAuthResponse(_ response: JSONValue) -> Bool {
        guard let urlString = extractAuthURL(from: response),
              let url = URL(string: urlString) else {
            return false
        }
        UIApplication.shared.open(url)
        return true
    }

    private func extractAuthURL(from response: JSONValue) -> String? {
        switch response {
        case .string(let value):
            return value
        case .object(let object):
            return stringValue(from: object, keys: ["url", "authUrl", "auth_url", "openUrl", "open_url"])
        default:
            return nil
        }
    }

    private func stringValue(from object: [String: JSONValue], keys: [String]) -> String? {
        for key in keys {
            if let value = object[key], case .string(let string) = value {
                let trimmed = string.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty { return trimmed }
            }
        }
        return nil
    }

    private func boolValue(from object: [String: JSONValue], keys: [String]) -> Bool? {
        for key in keys {
            if let value = object[key], case .bool(let bool) = value {
                return bool
            }
        }
        return nil
    }

    private func healthTint(for provider: ProviderStatus) -> Color {
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

private struct SettingsSectionView<Content: View>: View {
    let title: String
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title)
                .font(.headline)
                .foregroundColor(.ctxTextPrimary)
            GlassPanel {
                content()
            }
        }
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
            LucideIcon(name: .chevronRight, size: 12)
                .foregroundColor(.ctxTextSecondary)
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

private struct SettingsMenuRowView: View {
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
            LucideIcon(name: .chevronDown, size: 12)
                .foregroundColor(.ctxTextSecondary)
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

private struct SettingsStatusRowView: View {
    let title: String
    let value: String
    let tint: Color

    var body: some View {
        HStack {
            Text(title)
                .foregroundColor(.ctxTextSecondary)
            Spacer()
            GlassPill(text: value, tint: tint)
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

private struct AccountSettingsView: View {
    let userLabel: String
    let workspaceLabel: String
    let planLabel: String
    let entitlementsLoading: Bool
    let entitlementsError: String?
    let hasSupabaseToken: Bool

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Account")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .padding(.top, 12)

                    SettingsSectionView(title: "Account") {
                        VStack(alignment: .leading, spacing: 16) {
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
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Account")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct DaemonSettingsView: View {
    let telemetryBinding: Binding<Bool>
    let telemetryReady: Bool
    let telemetrySaving: Bool
    let dictationLabel: String
    let titleGenerationLabel: String
    let resourceGovernanceLabel: String
    let isLoadingSettings: Bool
    let settingsError: String?

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Daemon Settings")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .padding(.top, 12)

                    SettingsSectionView(title: "Daemon Settings") {
                        VStack(alignment: .leading, spacing: 16) {
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
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Daemon Settings")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct NotificationsSettingsView: View {
    let pushBinding: Binding<Bool>
    let pushToggleDisabled: Bool
    let pushEntitlementLabel: String
    let pushEntitlementTint: Color
    let pushStatusLabel: String
    let pushStatusTint: Color
    let pushStatusMessage: String?
    let pushError: String?
    let managerError: String?

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Notifications")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .padding(.top, 12)

                    SettingsSectionView(title: "Notifications") {
                        VStack(alignment: .leading, spacing: 14) {
                            Toggle("Push approvals", isOn: pushBinding)
                                .tint(.ctxAccent)
                                .disabled(pushToggleDisabled)
                            SettingsStatusRowView(
                                title: "Entitlement",
                                value: pushEntitlementLabel,
                                tint: pushEntitlementTint
                            )
                            SettingsStatusRowView(
                                title: "Status",
                                value: pushStatusLabel,
                                tint: pushStatusTint
                            )
                            if let pushStatusMessage {
                                Text(pushStatusMessage)
                                    .font(.caption)
                                    .foregroundColor(.ctxTextSecondary)
                            }
                            if let pushError {
                                Text(pushError)
                                    .font(.caption)
                                    .foregroundColor(.ctxError)
                            } else if let managerError {
                                Text(managerError)
                                    .font(.caption)
                                    .foregroundColor(.ctxError)
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Notifications")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct ModelRoutingSettingsView: View {
    let routingEnabled: Bool
    let routingProviderChoices: [String]
    let routingModelChoices: [String]
    @Binding var routingProviderId: String
    @Binding var routingModelId: String
    let routingProviderLabel: String
    let routingModelLabel: String
    let routingEntry: ModelRoutingEntry?
    @Binding var routingError: String?
    let routingCanSave: Bool
    let providerLabel: (String) -> String
    let onSave: () -> Void
    let onClear: () -> Void

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Model Routing")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .padding(.top, 12)

                    SettingsSectionView(title: "Model Routing") {
                        VStack(alignment: .leading, spacing: 14) {
                            if !routingEnabled {
                                Text("Select a workspace to set routing defaults.")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                            }
                            Menu {
                                ForEach(routingProviderChoices, id: \.self) { providerId in
                                    Button(providerLabel(providerId)) {
                                        routingProviderId = providerId
                                        routingModelId = ""
                                        routingError = nil
                                    }
                                }
                            } label: {
                                SettingsMenuRowView(title: "Harness", value: routingProviderLabel)
                            }
                            .disabled(!routingEnabled || routingProviderChoices.isEmpty)

                            Menu {
                                ForEach(routingModelChoices, id: \.self) { modelId in
                                    Button(modelId) {
                                        routingModelId = modelId
                                        routingError = nil
                                    }
                                }
                            } label: {
                                SettingsMenuRowView(title: "Model", value: routingModelLabel)
                            }
                            .disabled(!routingEnabled || routingModelChoices.isEmpty || routingProviderId.isEmpty)

                            if let routingEntry, !routingEntry.updatedAt.isEmpty {
                                Text("Updated \(routingEntry.updatedAt)")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                            }
                            if let routingError {
                                Text(routingError)
                                    .font(.caption)
                                    .foregroundColor(.ctxError)
                            }

                            HStack(spacing: 12) {
                                Button {
                                    onSave()
                                } label: {
                                    Text("Save routing")
                                }
                                .buttonStyle(CtxPrimaryButtonStyle())
                                .disabled(!routingCanSave)

                                if routingEntry != nil {
                                    Button {
                                        onClear()
                                    } label: {
                                        Text("Clear")
                                    }
                                    .buttonStyle(CtxGhostButtonStyle())
                                    .disabled(!routingEnabled)
                                }
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Model Routing")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct ProvidersSettingsView<Actions: View>: View {
    let workspaceLabel: String
    let isLoadingProviders: Bool
    let providersError: String?
    let visibleProviders: [ProviderStatus]
    let providerDetailText: (ProviderStatus) -> String
    let providerStatusPill: (ProviderStatus) -> StatusPill
    @ViewBuilder let providerActions: (ProviderStatus) -> Actions

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Providers")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .padding(.top, 12)

                    SettingsSectionView(title: "Providers") {
                        VStack(alignment: .leading, spacing: 14) {
                            if isLoadingProviders {
                                ProgressView()
                                    .tint(.ctxAccent)
                            } else if let providersError {
                                Text(providersError)
                                    .font(.caption)
                                    .foregroundColor(.ctxError)
                            } else if visibleProviders.isEmpty {
                                Text("No providers detected.")
                                    .font(.caption)
                                    .foregroundColor(.ctxTextMuted)
                            } else {
                                SettingsRowView(title: "Workspace", value: workspaceLabel)
                                ForEach(visibleProviders, id: \.providerId) { provider in
                                    ProviderRowView(
                                        provider: provider,
                                        detailText: providerDetailText(provider),
                                        statusPill: providerStatusPill(provider)
                                    ) {
                                        providerActions(provider)
                                    }
                                }
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Providers")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct WorkspaceSwitchView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @EnvironmentObject private var workspaceSelection: WorkspaceSelectionStore
    @EnvironmentObject private var workbenchSelection: WorkbenchSelectionStore
    @EnvironmentObject private var workspaceVisibility: WorkspaceVisibilityStore
    @EnvironmentObject private var rootNavigation: RootNavigationStore
    @Environment(\.dismiss) private var dismiss

    @State private var workspaces: [WorkspaceSummary] = []
    @State private var isLoading = false
    @State private var errorMessage: String?
    @State private var deleteAlert: WorkspaceDeleteAlert?
    @State private var deleteInFlight: Set<String> = []

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
                                WorkspaceSwitchRowView(
                                    workspace: workspace,
                                    isSelected: workspace.id == workspaceSelection.workspaceId,
                                    deleteDisabled: deleteInFlight.contains(workspace.id),
                                    onSelect: { selectWorkspace(workspace) },
                                    onDelete: { deleteAlert = WorkspaceDeleteAlert(workspace: workspace) }
                                )
                            }
                        }
                    }

                    GlassPanel {
                        Button {
                            _Concurrency.Task {
                                await connection.disconnect()
                                rootNavigation.route = .launcher
                                dismiss()
                            }
                        } label: {
                            HStack {
                                Image(systemName: "qrcode.viewfinder")
                                Text("Add workspace")
                                Spacer()
                            }
                        }
                        .buttonStyle(CtxPrimaryButtonStyle())
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 24)
            }
        }
        .navigationTitle("Workspace")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .alert(item: $deleteAlert) { alert in
            Alert(
                title: Text("Remove Workspace"),
                message: Text("Remove \"\(alert.name)\" from this device? This does not delete the workspace."),
                primaryButton: .destructive(Text("Remove")) {
                    _Concurrency.Task { await removeWorkspace(alert.workspace) }
                },
                secondaryButton: .cancel()
            )
        }
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
            let daemonKey = connection.baseURLText
            workspaceVisibility.load(daemonKey: daemonKey)
            let hiddenIds = workspaceVisibility.hiddenWorkspaceIds
            workspaces = try await client.listWorkspaces()
            workspaces = workspaces.filter { !hiddenIds.contains($0.id) }
        } catch {
            workspaces = []
            errorMessage = daemonErrorMessage(error, fallback: "Failed to load workspaces.")
        }
        isLoading = false
    }

    private func selectWorkspace(_ workspace: WorkspaceSummary) {
        workspaceSelection.setWorkspace(workspace, daemonKey: connection.baseURLText)
        workbenchSelection.setContext(daemonKey: connection.baseURLText, workspaceId: workspace.id)
        workbenchSelection.setSelection(taskId: nil, sessionId: nil)
        dismiss()
    }

    @MainActor
    private func removeWorkspace(_ workspace: WorkspaceSummary) async {
        if deleteInFlight.contains(workspace.id) { return }
        deleteInFlight.insert(workspace.id)
        defer { deleteInFlight.remove(workspace.id) }
        let daemonKey = connection.baseURLText
        workspaceVisibility.hide(workspaceId: workspace.id, daemonKey: daemonKey)
        await loadWorkspaces()
        if workspace.id == workspaceSelection.workspaceId {
            workspaceSelection.clear(daemonKey: daemonKey)
            workbenchSelection.setContext(daemonKey: daemonKey, workspaceId: nil)
            workbenchSelection.clearSelection()
        }
    }
}

private struct WorkspaceSwitchRowView: View {
    let workspace: WorkspaceSummary
    let isSelected: Bool
    let deleteDisabled: Bool
    let onSelect: () -> Void
    let onDelete: () -> Void

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

            Button {
                onDelete()
            } label: {
                Image(systemName: "trash")
                    .foregroundColor(.ctxTextSecondary)
            }
            .buttonStyle(.plain)
            .disabled(deleteDisabled)
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(isSelected ? 0.75 : 0.6), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(isSelected ? Color.ctxAccent.opacity(0.6) : Color.ctxLine, lineWidth: 1)
        )
        .contentShape(Rectangle())
        .onTapGesture {
            onSelect()
        }
    }

    private var workspaceDetailText: String {
        let url = URL(fileURLWithPath: workspace.rootPath)
        return url.lastPathComponent.isEmpty ? workspace.rootPath : url.lastPathComponent
    }
}

private struct WorkspaceDeleteAlert: Identifiable {
    let id = UUID()
    let workspace: WorkspaceSummary
    var name: String { workspace.name }
}

private struct ProviderRowView<Actions: View>: View {
    let provider: ProviderStatus
    let detailText: String
    let statusPill: StatusPill
    @ViewBuilder let actions: () -> Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
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
                GlassPill(text: statusPill.text, tint: statusPill.tint)
            }
            actions()
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
        .environmentObject(PushNotificationManager.shared)
        .environmentObject(WorkspaceSelectionStore())
        .environmentObject(WorkbenchSelectionStore())
        .environmentObject(WorkspaceVisibilityStore())
        .environmentObject(RootNavigationStore())
}
