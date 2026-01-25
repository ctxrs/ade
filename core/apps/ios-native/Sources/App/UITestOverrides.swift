import Foundation

enum UITestScreen: String {
    case connection
    case chat
    case diff
    case settings
    case mobileAccess
    case diagnostics
    case workspaceSelector
    case taskList
    case newTask
}

enum UITestOverrides {
    static let isUITestMode: Bool = {
        ProcessInfo.processInfo.environment["CTX_UI_TEST_MODE"] == "1"
    }()

    static let screen: UITestScreen? = {
        guard isUITestMode else { return nil }
        let raw = ProcessInfo.processInfo.environment["CTX_UI_TEST_SCREEN"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        guard let raw, !raw.isEmpty else { return nil }
        switch raw {
        case "connection":
            return .connection
        case "chat":
            return .chat
        case "diff":
            return .diff
        case "settings":
            return .settings
        case "mobile_access", "mobile-access", "mobileaccess":
            return .mobileAccess
        case "diagnostics":
            return .diagnostics
        case "workspace_selector", "workspace-selector", "workspaceselector":
            return .workspaceSelector
        case "task_list", "task-list", "tasklist":
            return .taskList
        case "new_task", "new-task", "newtask":
            return .newTask
        default:
            return nil
        }
    }()

    static func isActive(_ screen: UITestScreen) -> Bool {
        isUITestMode && self.screen == screen
    }
}
