import SwiftUI

struct CtxBackgroundView: View {
    var body: some View {
        ZStack {
            LinearGradient(
                colors: [Color.ctxBackground, Color.ctxBackgroundDeep],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            RadialGradient(
                colors: [Color.ctxSurfaceRaised.opacity(0.6), Color.clear],
                center: .bottomTrailing,
                startRadius: 40,
                endRadius: 420
            )
        }
        .ignoresSafeArea()
    }
}

struct GlassPanel<Content: View>: View {
    let cornerRadius: CGFloat
    let paddingAmount: CGFloat
    let content: Content

    init(cornerRadius: CGFloat = 22, padding: CGFloat = 16, @ViewBuilder content: () -> Content) {
        self.cornerRadius = cornerRadius
        self.paddingAmount = padding
        self.content = content()
    }

    var body: some View {
        content
            .padding(paddingAmount)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .stroke(Color.ctxGlassStroke, lineWidth: 0.8)
            )
            .shadow(color: Color.ctxShadow, radius: 16, x: 0, y: 8)
    }
}

struct CtxField: View {
    let title: String
    let placeholder: String
    @Binding var text: String
    var isSecure: Bool = false
    var accessibilityId: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.footnote.weight(.semibold))
                .foregroundColor(.ctxTextSecondary)
            Group {
                if isSecure {
                    SecureField(placeholder, text: $text)
                        .accessibilityIdentifier(accessibilityId ?? "")
                } else {
                    TextField(placeholder, text: $text)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .accessibilityIdentifier(accessibilityId ?? "")
                }
            }
            .foregroundColor(.ctxTextPrimary)
            .padding(12)
            .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(Color.ctxLine, lineWidth: 1)
            )
        }
    }
}

struct CtxPrimaryButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .foregroundColor(.ctxTextPrimary)
            .padding(.vertical, 12)
            .padding(.horizontal, 18)
            .frame(maxWidth: .infinity)
            .background(
                LinearGradient(
                    colors: [Color.ctxAccent, Color.ctxAccentMuted],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                ),
                in: RoundedRectangle(cornerRadius: 16, style: .continuous)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .stroke(Color.ctxGlassStroke, lineWidth: 0.8)
            )
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
            .opacity(configuration.isPressed ? 0.9 : 1)
    }
}

struct CtxGhostButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.subheadline.weight(.semibold))
            .foregroundColor(.ctxTextPrimary)
            .padding(.vertical, 10)
            .padding(.horizontal, 16)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .stroke(Color.ctxLine, lineWidth: 1)
            )
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
            .opacity(configuration.isPressed ? 0.9 : 1)
    }
}

struct GlassPill: View {
    let text: String
    let tint: Color

    var body: some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .foregroundColor(tint)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(.ultraThinMaterial, in: Capsule())
            .overlay(
                Capsule()
                    .stroke(tint.opacity(0.5), lineWidth: 0.8)
            )
    }
}
