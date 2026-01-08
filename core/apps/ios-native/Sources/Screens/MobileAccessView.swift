import SwiftUI
import UIKit

struct MobileAccessView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @State private var status: MobileAccessStatus?
    @State private var lastEnableResponse: EnableMobileAccessResponse?
    @State private var supabaseToken = ""
    @State private var entitlements: EntitlementsSnapshot?
    @State private var entitlementsLoading = false
    @State private var entitlementsError: String?
    @State private var hasSupabaseToken = false
    @State private var isLoadingStatus = false
    @State private var isWorking = false
    @State private var errorMessage: String?
    @State private var tokenMessage: String?
    @State private var didCopyPayload = false

    private let supabaseTokenStore = KeychainTokenStore(service: "rs.ctx.mobile", account: "supabaseToken")

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Mobile Access")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)

                    Text("Manage remote tunnel access and pairing for this daemon.")
                        .font(.subheadline)
                        .foregroundColor(.ctxTextMuted)

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            HStack {
                                Text("Status")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                Spacer()
                                if isLoadingStatus {
                                    ProgressView()
                                        .tint(.ctxAccent)
                                }
                            }

                            StatusPillRow(
                                title: "Entitlement",
                                value: entitlementLabel,
                                tint: entitlementTint
                            )

                            if let status {
                                StatusPillRow(
                                    title: "Mobile access",
                                    value: status.enabled ? "Enabled" : "Disabled",
                                    tint: status.enabled ? .ctxAccent : .ctxTextMuted
                                )
                                StatusPillRow(
                                    title: "Tunnel",
                                    value: status.tunnelState.rawValue.capitalized,
                                    tint: tint(for: status.tunnelState)
                                )
                                StatusValueRow(title: "Public URL", value: status.publicBaseUrl ?? "-")
                                StatusValueRow(title: "Tunnel ID", value: status.tunnelId ?? "-")
                                if let lastError = status.lastError, !lastError.isEmpty {
                                    Text(lastError)
                                        .font(.footnote)
                                        .foregroundColor(.ctxError)
                                }
                            } else if connection.apiClient == nil {
                                Text("Connect to a daemon to view mobile access status.")
                                    .font(.footnote)
                                    .foregroundColor(.ctxTextSecondary)
                            } else {
                                Text("Status unavailable.")
                                    .font(.footnote)
                                    .foregroundColor(.ctxTextSecondary)
                            }

                            if let errorMessage {
                                Text(errorMessage)
                                    .font(.footnote)
                                    .foregroundColor(.ctxError)
                            }

                            if entitlementsLoading {
                                Text("Loading entitlements...")
                                    .font(.footnote)
                                    .foregroundColor(.ctxTextMuted)
                            } else if let entitlementsError {
                                Text(entitlementsError)
                                    .font(.footnote)
                                    .foregroundColor(.ctxError)
                            } else if !hasSupabaseToken {
                                Text("Supabase token required to check entitlements.")
                                    .font(.footnote)
                                    .foregroundColor(.ctxTextMuted)
                            }

                            Button {
                                Task { await refreshStatus() }
                            } label: {
                                HStack {
                                    Image(systemName: "arrow.clockwise")
                                    Text("Refresh status")
                                }
                            }
                            .buttonStyle(CtxGhostButtonStyle())
                            .disabled(isLoadingStatus || isWorking)
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Manage Access")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            CtxField(title: "Supabase token", placeholder: "Paste token", text: $supabaseToken, isSecure: true)
                            Text("Required to enable or disable managed mobile access.")
                                .font(.caption)
                                .foregroundColor(.ctxTextMuted)

                            if let tokenMessage {
                                Text(tokenMessage)
                                    .font(.footnote)
                                    .foregroundColor(.ctxTextSecondary)
                            }

                            if status?.enabled == true {
                                Button {
                                    Task { await disableAccess() }
                                } label: {
                                    HStack {
                                        Text(isWorking ? "Disabling..." : "Disable Mobile Access")
                                        Spacer()
                                        Image(systemName: "xmark.circle.fill")
                                    }
                                }
                                .buttonStyle(CtxGhostButtonStyle())
                                .disabled(isWorking)
                            } else {
                                Button {
                                    Task { await enableAccess() }
                                } label: {
                                    HStack {
                                        Text(isWorking ? "Enabling..." : "Enable Mobile Access")
                                        Spacer()
                                        Image(systemName: "bolt.circle.fill")
                                    }
                                }
                                .buttonStyle(CtxPrimaryButtonStyle())
                                .disabled(isWorking)
                            }
                        }
                    }

                    if let response = lastEnableResponse {
                        GlassPanel {
                            VStack(alignment: .leading, spacing: 12) {
                                Text("Pairing Window")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                StatusValueRow(title: "Expires", value: response.pairingExpiresAt)
                                Button {
                                    copyPayload(response.qrPayload)
                                } label: {
                                    HStack {
                                        Image(systemName: "doc.on.doc")
                                        Text(didCopyPayload ? "Pairing payload copied" : "Copy pairing payload")
                                        Spacer()
                                    }
                                }
                                .buttonStyle(CtxGhostButtonStyle())
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Mobile Access")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .task {
            await loadSupabaseToken()
            await refreshEntitlements()
            await refreshStatus()
        }
        .onChange(of: connection.isConnected) { _ in
            Task { await refreshStatus() }
        }
    }

    private var entitlementLabel: String {
        if entitlementsLoading {
            return "Loading"
        }
        if let entitlements {
            return entitlements.isFeatureEnabled("remote_mobile_access") ? "Pro enabled" : "Pro required"
        }
        return hasSupabaseToken ? "Unknown" : "Sign in required"
    }

    private var entitlementTint: Color {
        if entitlementsLoading {
            return .ctxAccent
        }
        if let entitlements {
            return entitlements.isFeatureEnabled("remote_mobile_access") ? .ctxAccent : .ctxWarning
        }
        return .ctxTextMuted
    }

    @MainActor
    private func refreshStatus() async {
        guard let client = connection.apiClient else {
            status = nil
            return
        }
        isLoadingStatus = true
        errorMessage = nil
        do {
            status = try await client.getMobileAccessStatus()
        } catch {
            errorMessage = describeError(error, fallback: "Failed to load mobile access status.")
        }
        isLoadingStatus = false
    }

    @MainActor
    private func enableAccess() async {
        guard let client = connection.apiClient else {
            errorMessage = "Connect to a daemon to enable mobile access."
            return
        }
        guard let token = sanitizedSupabaseToken() else { return }
        isWorking = true
        errorMessage = nil
        await persistSupabaseToken(token)
        do {
            let response = try await client.enableMobileAccess(supabaseToken: token)
            status = response.status
            lastEnableResponse = response
            didCopyPayload = false
            await refreshEntitlements()
        } catch {
            errorMessage = describeError(error, fallback: "Failed to enable mobile access.")
        }
        isWorking = false
    }

    @MainActor
    private func disableAccess() async {
        guard let client = connection.apiClient else {
            errorMessage = "Connect to a daemon to disable mobile access."
            return
        }
        guard let token = sanitizedSupabaseToken() else { return }
        isWorking = true
        errorMessage = nil
        await persistSupabaseToken(token)
        do {
            try await client.disableMobileAccess(supabaseToken: token)
            lastEnableResponse = nil
            await refreshStatus()
            await refreshEntitlements()
        } catch {
            errorMessage = describeError(error, fallback: "Failed to disable mobile access.")
        }
        isWorking = false
    }

    @MainActor
    private func loadSupabaseToken() async {
        do {
            if let stored = try await supabaseTokenStore.loadToken() {
                supabaseToken = stored
                hasSupabaseToken = !stored.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            }
        } catch {
            tokenMessage = "Failed to load Supabase token."
        }
    }

    @MainActor
    private func refreshEntitlements() async {
        entitlementsError = nil
        let trimmed = supabaseToken.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            hasSupabaseToken = false
            entitlements = nil
            entitlementsLoading = false
            return
        }
        hasSupabaseToken = true
        entitlementsLoading = true
        do {
            entitlements = try await SupabaseEntitlementsClient.fetchEntitlements(token: trimmed)
        } catch {
            entitlements = nil
            entitlementsError = "Unable to load entitlements."
        }
        entitlementsLoading = false
    }

    @MainActor
    private func persistSupabaseToken(_ token: String) async {
        do {
            try await supabaseTokenStore.saveToken(token)
            tokenMessage = "Supabase token saved."
        } catch {
            tokenMessage = "Failed to save Supabase token."
        }
    }

    private func sanitizedSupabaseToken() -> String? {
        let trimmed = supabaseToken.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            errorMessage = "Supabase token is required."
            return nil
        }
        return trimmed
    }

    private func tint(for state: MobileTunnelState) -> Color {
        switch state {
        case .idle:
            return .ctxTextMuted
        case .running:
            return .ctxAccent
        case .error:
            return .ctxError
        }
    }

    private func copyPayload(_ payload: JSONValue) {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        guard let data = try? encoder.encode(payload),
              let string = String(data: data, encoding: .utf8) else {
            errorMessage = "Failed to encode pairing payload."
            return
        }
        UIPasteboard.general.string = string
        didCopyPayload = true
    }

    private func describeError(_ error: Error, fallback: String) -> String {
        if let apiError = error as? DaemonAPIError {
            switch apiError {
            case .missingToken:
                return "Daemon token is missing. Reconnect to the daemon."
            case .invalidURL:
                return "Daemon URL is invalid."
            case .invalidResponse:
                return "Daemon returned an invalid response."
            case .requestFailed(_, let message):
                return message.isEmpty ? fallback : message
            }
        }
        let message = error.localizedDescription
        return message.isEmpty ? fallback : message
    }
}

private struct StatusValueRow: View {
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

private struct StatusPillRow: View {
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

#Preview {
    MobileAccessView()
        .environmentObject(ConnectionStore())
}
