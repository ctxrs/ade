import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @State private var notificationsEnabled = true
    @State private var hapticsEnabled = true
    @State private var analyticsEnabled = false
    @State private var mobileStatus: MobileAccessStatus?
    @State private var isLoadingMobileStatus = false
    @State private var mobileStatusError: String?

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
                            SettingsRowView(title: "User", value: "codex@ctx.local")
                            SettingsRowView(title: "Workspace", value: "Atlas")
                            SettingsRowView(title: "Plan", value: "Local-first")
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 16) {
                            Text("Preferences")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            Toggle("Push notifications", isOn: $notificationsEnabled)
                                .tint(.ctxAccent)
                            Toggle("Haptics", isOn: $hapticsEnabled)
                                .tint(.ctxAccent)
                            Toggle("Share anonymous analytics", isOn: $analyticsEnabled)
                                .tint(.ctxAccent)
                        }
                        .foregroundColor(.ctxTextSecondary)
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
                                    status: mobileStatusLabel,
                                    statusTint: mobileStatusTint
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
                            ProviderRowView(name: "Codex", detail: "GPT-5 • Connected")
                            ProviderRowView(name: "Claude", detail: "Idle")
                            ProviderRowView(name: "Gemini", detail: "Not configured")
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
        .task {
            await refreshMobileStatus()
        }
        .onChange(of: connection.isConnected) { _ in
            Task { await refreshMobileStatus() }
        }
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

private struct ProviderRowView: View {
    let name: String
    let detail: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "bolt.circle")
                .foregroundColor(.ctxAccent)
            VStack(alignment: .leading, spacing: 4) {
                Text(name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(detail)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
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

private struct SettingsNavigationRowView: View {
    let title: String
    let subtitle: String
    let status: String
    let statusTint: Color

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
            GlassPill(text: status, tint: statusTint)
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
