import SwiftUI

extension Color {
    static let ctxBackground = Color(red: 0.118, green: 0.118, blue: 0.118) // #1e1e1e
    static let ctxBackgroundDeep = Color(red: 0.09, green: 0.09, blue: 0.09)
    static let ctxSurface = Color(red: 0.145, green: 0.145, blue: 0.149) // #252526
    static let ctxSurfaceRaised = Color(red: 0.176, green: 0.176, blue: 0.176) // #2d2d2d
    static let ctxBubbleAssistant = ctxSurface
    static let ctxBubbleUser = Color.white.opacity(0.02)
    static let ctxTextPrimary = Color(red: 0.831, green: 0.831, blue: 0.831) // #d4d4d4
    static let ctxTextSecondary = Color(red: 0.831, green: 0.831, blue: 0.831).opacity(0.65)
    static let ctxTextMuted = Color(red: 0.831, green: 0.831, blue: 0.831).opacity(0.45)
    static let ctxAccent = Color(red: 0.216, green: 0.580, blue: 1.0) // #3794ff
    static let ctxAccentMuted = Color(red: 0.17, green: 0.44, blue: 0.84)
    static let ctxWarning = Color(red: 0.984, green: 0.749, blue: 0.141) // #fbbf24
    static let ctxError = Color(red: 0.937, green: 0.267, blue: 0.267) // #ef4444
    static let ctxDiffAdded = Color(red: 0.733, green: 0.969, blue: 0.816) // #bbf7d0
    static let ctxDiffRemoved = Color(red: 0.996, green: 0.792, blue: 0.792) // #fecaca
    static let ctxLine = Color.white.opacity(0.08)
    static let ctxGlassStroke = Color.white.opacity(0.14)
    static let ctxShadow = Color.black.opacity(0.2)
}

enum CtxChatStyle {
    static let horizontalPadding: CGFloat = 16
    static let messageSpacing: CGFloat = 14
    static let messageTopPadding: CGFloat = 10
    static let messageBottomPadding: CGFloat = 8

    static let bodyFont: Font = .system(size: 16, weight: .regular)
    static let bodyLineSpacing: CGFloat = 3

    static let userBubbleCornerRadius: CGFloat = 12
    static let userBubbleWidthFraction: CGFloat = 1.0
    static let userBubbleMaxWidth: CGFloat = 960
    static let userBubbleVerticalPadding: CGFloat = 8
    static let userBubbleHorizontalPadding: CGFloat = 10

    static let composerCornerRadius: CGFloat = 28
    static let composerOuterPadding: CGFloat = 12
    static let composerInnerHorizontalPadding: CGFloat = 16
    static let composerInnerVerticalPadding: CGFloat = 14

    static let composerToolSize: CGFloat = 36
    static let composerPrimarySize: CGFloat = 40
}
