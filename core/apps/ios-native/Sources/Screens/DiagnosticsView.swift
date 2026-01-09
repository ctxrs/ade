import SwiftUI
import UIKit

struct DiagnosticsView: View {
    @EnvironmentObject private var connection: ConnectionStore
    @State private var diagnostics: Diagnostics?
    @State private var isLoading = false
    @State private var errorMessage: String?
    @State private var didCopy = false

    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Diagnostics")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)

                    HStack(spacing: 12) {
                        Button {
                            _Concurrency.Task { await refreshDiagnostics() }
                        } label: {
                            HStack {
                                Image(systemName: "arrow.clockwise")
                                Text("Refresh")
                            }
                        }
                        .buttonStyle(CtxGhostButtonStyle())

                        Button {
                            copyDiagnostics()
                        } label: {
                            HStack {
                                Image(systemName: "doc.on.doc")
                                Text(didCopy ? "Copied" : "Copy JSON")
                            }
                        }
                        .buttonStyle(CtxGhostButtonStyle())
                        .disabled(diagnostics == nil)
                    }

                    if isLoading {
                        Text("Loading diagnostics...")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    }

                    if let errorMessage {
                        Text(errorMessage)
                            .font(.caption)
                            .foregroundColor(.ctxError)
                    }

                    if diagnostics == nil, connection.apiClient == nil {
                        Text("Connect to a daemon to view diagnostics.")
                            .font(.caption)
                            .foregroundColor(.ctxTextMuted)
                    }

                    if let diagnostics {
                        GlassPanel {
                            VStack(alignment: .leading, spacing: 12) {
                                Text("Daemon")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                DiagnosticsRowView(title: "Daemon URL", value: diagnostics.daemon.daemonUrl)
                                DiagnosticsRowView(title: "Version", value: diagnostics.daemon.version)
                                DiagnosticsRowView(title: "PID", value: String(diagnostics.daemon.pid))
                                DiagnosticsRowView(title: "Data root", value: diagnostics.daemon.dataRoot)
                                DiagnosticsRowView(title: "Auth required", value: diagnostics.daemon.authRequired ? "Yes" : "No")
                            }
                        }

                        GlassPanel {
                            VStack(alignment: .leading, spacing: 12) {
                                Text("Platform")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                DiagnosticsRowView(title: "OS", value: diagnostics.platform.os)
                                DiagnosticsRowView(title: "Arch", value: diagnostics.platform.arch)
                            }
                        }

                        GlassPanel {
                            VStack(alignment: .leading, spacing: 12) {
                                Text("Providers")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                if diagnostics.providers.isEmpty {
                                    Text("No provider diagnostics available.")
                                        .font(.caption)
                                        .foregroundColor(.ctxTextMuted)
                                } else {
                                    ForEach(diagnostics.providers, id: \.providerId) { provider in
                                        DiagnosticsProviderRowView(provider: provider)
                                    }
                                }
                            }
                        }

                        GlassPanel {
                            VStack(alignment: .leading, spacing: 12) {
                                Text("Logs")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                DiagnosticsRowView(title: "Directory", value: diagnostics.logs.dir)
                                if diagnostics.logs.files.isEmpty {
                                    Text("No log files found.")
                                        .font(.caption)
                                        .foregroundColor(.ctxTextMuted)
                                } else {
                                    ForEach(diagnostics.logs.files, id: \.name) { file in
                                        DiagnosticsLogRowView(file: file)
                                    }
                                }
                            }
                        }

                        GlassPanel {
                            VStack(alignment: .leading, spacing: 12) {
                                Text("Diagnostics JSON")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                ScrollView(.horizontal, showsIndicators: false) {
                                    Text(diagnosticsJSON)
                                        .font(.system(.caption, design: .monospaced))
                                        .foregroundColor(.ctxTextSecondary)
                                        .padding(8)
                                }
                                .frame(maxWidth: .infinity, minHeight: 160, alignment: .leading)
                                .background(Color.ctxSurface.opacity(0.35), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                                .overlay(
                                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                                        .stroke(Color.ctxLine, lineWidth: 1)
                                )
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 40)
            }
        }
        .navigationTitle("Diagnostics")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.visible, for: .navigationBar)
        .task {
            await refreshDiagnostics()
        }
        .onChange(of: connection.isConnected) { _ in
            _Concurrency.Task { await refreshDiagnostics() }
        }
    }

    private var diagnosticsJSON: String {
        guard let diagnostics else { return "" }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.keyEncodingStrategy = .convertToSnakeCase
        guard let data = try? encoder.encode(diagnostics) else {
            return ""
        }
        return String(data: data, encoding: .utf8) ?? ""
    }

    @MainActor
    private func refreshDiagnostics() async {
        guard let client = connection.apiClient else {
            diagnostics = nil
            errorMessage = nil
            return
        }
        isLoading = true
        errorMessage = nil
        didCopy = false
        do {
            diagnostics = try await client.getDiagnostics()
        } catch {
            diagnostics = nil
            errorMessage = "Unable to load diagnostics."
        }
        isLoading = false
    }

    private func copyDiagnostics() {
        guard !diagnosticsJSON.isEmpty else { return }
        UIPasteboard.general.string = diagnosticsJSON
        didCopy = true
    }
}

private struct DiagnosticsRowView: View {
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

private struct DiagnosticsProviderRowView: View {
    let provider: ProviderStatus

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(displayName)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Spacer()
                GlassPill(text: healthLabel, tint: healthTint)
            }
            if let version = provider.version, !version.isEmpty {
                Text(version)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            if provider.diagnostics.isEmpty {
                Text("No diagnostics reported.")
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            } else {
                ForEach(provider.diagnostics.indices, id: \.self) { index in
                    Text(provider.diagnostics[index])
                        .font(.caption)
                        .foregroundColor(.ctxTextSecondary)
                }
            }
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }

    private var displayName: String {
        provider.providerId.replacingOccurrences(of: "_", with: " ").capitalized
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

private struct DiagnosticsLogRowView: View {
    let file: Diagnostics.Logs.LogFile

    var body: some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 4) {
                Text(file.name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(details)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }

    private var details: String {
        var parts = ["\(file.bytes) bytes"]
        if let modified = file.modifiedUtc, !modified.isEmpty {
            parts.append("modified \(modified)")
        }
        return parts.joined(separator: " - ")
    }
}

#Preview {
    DiagnosticsView()
        .environmentObject(ConnectionStore())
}
