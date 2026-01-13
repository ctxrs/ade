import SwiftUI

struct HarnessCatalogEntry {
    let id: String
    let assetName: String
    let invertInDark: Bool
}

enum HarnessCatalog {
    static let entries: [String: HarnessCatalogEntry] = [
        "claude": .init(id: "claude", assetName: "harness_claude", invertInDark: false),
        "codex": .init(id: "codex", assetName: "harness_codex", invertInDark: true),
        "qwen": .init(id: "qwen", assetName: "harness_qwen", invertInDark: false),
        "cursor": .init(id: "cursor", assetName: "harness_cursor", invertInDark: true),
        "amp": .init(id: "amp", assetName: "harness_amp", invertInDark: false),
        "droid": .init(id: "droid", assetName: "harness_droid", invertInDark: true),
        "gemini": .init(id: "gemini", assetName: "harness_gemini", invertInDark: false),
        "copilot": .init(id: "copilot", assetName: "harness_copilot", invertInDark: true),
        "opencode": .init(id: "opencode", assetName: "harness_opencode", invertInDark: true),
        "cline": .init(id: "cline", assetName: "harness_cline", invertInDark: false),
        "mistral": .init(id: "mistral", assetName: "harness_mistral", invertInDark: false),
        "auggie": .init(id: "auggie", assetName: "harness_auggie", invertInDark: true),
        "goose": .init(id: "goose", assetName: "harness_goose", invertInDark: false),
        "kimi": .init(id: "kimi", assetName: "harness_kimi", invertInDark: false),
        "kiro": .init(id: "kiro", assetName: "harness_kiro", invertInDark: false),
        "codebuff": .init(id: "codebuff", assetName: "harness_codebuff", invertInDark: false),
        "charm": .init(id: "charm", assetName: "harness_charm", invertInDark: false),
        "rovo": .init(id: "rovo", assetName: "harness_rovo", invertInDark: false),
        "aider": .init(id: "aider", assetName: "harness_aider", invertInDark: false),
        "continue": .init(id: "continue", assetName: "harness_continue", invertInDark: false),
        "openhands": .init(id: "openhands", assetName: "harness_openhands", invertInDark: false),
        "swe-agent": .init(id: "swe-agent", assetName: "harness_swe-agent", invertInDark: false),
        "cagent": .init(id: "cagent", assetName: "harness_cagent", invertInDark: false),
        "kilo": .init(id: "kilo", assetName: "harness_kilo", invertInDark: false),
        "cody": .init(id: "cody", assetName: "harness_cody", invertInDark: false),
        "junie": .init(id: "junie", assetName: "harness_junie", invertInDark: false)
    ]

    static func entry(for id: String) -> HarnessCatalogEntry? {
        entries[id.lowercased()]
    }
}

struct HarnessInvertModifier: ViewModifier {
    let shouldInvert: Bool

    func body(content: Content) -> some View {
        if shouldInvert {
            content.colorInvert()
        } else {
            content
        }
    }
}
