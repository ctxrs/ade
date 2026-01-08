import SwiftUI

struct DiagnosticsView: View {
    var body: some View {
        ZStack {
            CtxBackgroundView()
            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 20) {
                    Text("Diagnostics")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("System health")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            HealthRowView(title: "Daemon", status: "Healthy", tint: .ctxAccent)
                            HealthRowView(title: "Streaming", status: "Stable", tint: .ctxAccent)
                            HealthRowView(title: "Background sync", status: "Delayed", tint: .ctxWarning)
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Recent events")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            EventRowView(title: "Connected to Atlas", time: "2m ago")
                            EventRowView(title: "Session streamed 24 frames", time: "5m ago")
                            EventRowView(title: "Token refresh pending", time: "12m ago")
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Diagnostics actions")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            DiagnosticActionRowView(title: "Export logs", subtitle: "Bundle device + daemon logs")
                            DiagnosticActionRowView(title: "Run connection test", subtitle: "Ping daemon + verify TLS")
                            DiagnosticActionRowView(title: "Reset streaming session", subtitle: "Restart active session")
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
    }
}

private struct HealthRowView: View {
    let title: String
    let status: String
    let tint: Color

    var body: some View {
        HStack {
            Text(title)
                .foregroundColor(.ctxTextSecondary)
            Spacer()
            GlassPill(text: status, tint: tint)
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

private struct EventRowView: View {
    let title: String
    let time: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "clock")
                .foregroundColor(.ctxTextSecondary)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(time)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct DiagnosticActionRowView: View {
    let title: String
    let subtitle: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "bolt.circle")
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
    DiagnosticsView()
}
