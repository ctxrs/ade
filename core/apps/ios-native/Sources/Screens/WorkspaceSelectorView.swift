import SwiftUI

struct WorkspaceSelectorView: View {
    @Environment(\.dismiss) private var dismiss
    @Binding var selectedWorkspace: String
    @Binding var selectedDetail: String
    @State private var query = ""

    private let workspaces: [WorkspaceItem] = [
        WorkspaceItem(name: "Atlas", detail: "ios-native-ui", status: "Connected"),
        WorkspaceItem(name: "Context Monorepo", detail: "workbench-shell", status: "Idle"),
        WorkspaceItem(name: "Remote Devbox", detail: "ctx-linux", status: "Sleeping"),
        WorkspaceItem(name: "Docs Sandbox", detail: "ctx-docs", status: "Active")
    ]

    var body: some View {
        NavigationStack {
            ZStack {
                CtxBackgroundView()
                VStack(alignment: .leading, spacing: 18) {
                    Text("Select workspace")
                        .font(.largeTitle.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                        .padding(.top, 12)

                    HStack(spacing: 12) {
                        Image(systemName: "magnifyingglass")
                            .foregroundColor(.ctxTextSecondary)
                        TextField("Search workspaces", text: $query)
                            .foregroundColor(.ctxTextPrimary)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                    }
                    .padding(12)
                    .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .stroke(Color.ctxLine, lineWidth: 1)
                    )

                    ScrollView(showsIndicators: false) {
                        VStack(spacing: 12) {
                            ForEach(filteredWorkspaces) { workspace in
                                Button {
                                    selectedWorkspace = workspace.name
                                    selectedDetail = workspace.detail
                                    dismiss()
                                } label: {
                                    WorkspaceCardView(workspace: workspace, isSelected: workspace.name == selectedWorkspace)
                                }
                            }
                        }
                    }
                    Spacer()
                    Button {
                        dismiss()
                    } label: {
                        Text("Cancel")
                    }
                    .buttonStyle(CtxGhostButtonStyle())
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 20)
            }
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        dismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .foregroundColor(.ctxTextPrimary)
                    }
                }
            }
        }
    }

    private var filteredWorkspaces: [WorkspaceItem] {
        guard !query.isEmpty else { return workspaces }
        return workspaces.filter { workspace in
            workspace.name.localizedCaseInsensitiveContains(query) ||
            workspace.detail.localizedCaseInsensitiveContains(query)
        }
    }
}

private struct WorkspaceItem: Identifiable {
    let id = UUID()
    let name: String
    let detail: String
    let status: String
}

private struct WorkspaceCardView: View {
    let workspace: WorkspaceItem
    let isSelected: Bool

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Color.ctxSurfaceRaised)
                Image(systemName: "folder.fill")
                    .foregroundColor(.ctxAccent)
            }
            .frame(width: 44, height: 44)

            VStack(alignment: .leading, spacing: 4) {
                Text(workspace.name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(workspace.detail)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }

            Spacer()

            GlassPill(text: workspace.status, tint: workspace.status == "Connected" ? .ctxAccent : .ctxTextSecondary)
        }
        .padding(14)
        .background(Color.ctxSurface.opacity(isSelected ? 0.8 : 0.6), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(isSelected ? Color.ctxAccent.opacity(0.6) : Color.ctxLine, lineWidth: isSelected ? 1.2 : 1)
        )
    }
}

#Preview {
    WorkspaceSelectorView(selectedWorkspace: .constant("Atlas"), selectedDetail: .constant("ios-native-ui"))
}
