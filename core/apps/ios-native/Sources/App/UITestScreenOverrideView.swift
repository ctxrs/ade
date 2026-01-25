import SwiftUI

struct UITestScreenOverrideView: View {
    let screen: UITestScreen
    @State private var selectedWorkspace = UITestFixtures.workspace.name
    @State private var selectedDetail = "ios-native-ui"

    var body: some View {
        switch screen {
        case .connection:
            NavigationStack {
                ConnectionView()
            }
        case .chat:
            ChatView(viewModel: ChatViewModel(), showsBackground: true)
        case .diff:
            WorkbenchDiffPanelView.uiTestView()
        case .settings:
            NavigationStack {
                SettingsView(selectedWorkspace: UITestFixtures.workspace)
            }
        case .mobileAccess:
            NavigationStack {
                MobileAccessView()
            }
        case .diagnostics:
            NavigationStack {
                DiagnosticsView()
            }
        case .workspaceSelector:
            WorkspaceSelectorView(
                selectedWorkspace: $selectedWorkspace,
                selectedDetail: $selectedDetail
            )
        case .taskList:
            WorkbenchTaskListPreviewView()
        case .newTask:
            WorkbenchNewTaskPreviewView()
        }
    }
}
