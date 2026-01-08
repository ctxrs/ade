import SwiftUI

struct WorkbenchShellView: View {
    @State private var isDrawerOpen = false
    @State private var showWorkspaceSelector = false
    @State private var messageDraft = ""
    @State private var workspaceName = "Atlas"
    @State private var workspaceDetail = "ios-native-ui"

    var body: some View {
        GeometryReader { proxy in
            let drawerWidth = min(320, proxy.size.width * 0.78)

            ZStack(alignment: .leading) {
                CtxBackgroundView()

                WorkbenchHomeView(
                    isDrawerOpen: $isDrawerOpen,
                    showWorkspaceSelector: $showWorkspaceSelector,
                    messageDraft: $messageDraft,
                    workspaceName: workspaceName,
                    workspaceDetail: workspaceDetail
                )
                .blur(radius: isDrawerOpen ? 8 : 0)
                .overlay {
                    if isDrawerOpen {
                        Color.black.opacity(0.35)
                            .ignoresSafeArea()
                            .onTapGesture { isDrawerOpen = false }
                    }
                }

                WorkbenchDrawerView(
                    workspaceName: workspaceName,
                    workspaceDetail: workspaceDetail,
                    drawerWidth: drawerWidth,
                    onClose: { isDrawerOpen = false },
                    onWorkspaceTap: { showWorkspaceSelector = true }
                )
                .offset(x: isDrawerOpen ? 0 : -drawerWidth - 24)
            }
            .animation(.spring(response: 0.35, dampingFraction: 0.85), value: isDrawerOpen)
        }
        .sheet(isPresented: $showWorkspaceSelector) {
            WorkspaceSelectorView(selectedWorkspace: $workspaceName, selectedDetail: $workspaceDetail)
        }
        .navigationBarBackButtonHidden(true)
        .toolbar(.hidden, for: .navigationBar)
    }
}

private struct WorkbenchHomeView: View {
    @Binding var isDrawerOpen: Bool
    @Binding var showWorkspaceSelector: Bool
    @Binding var messageDraft: String
    let workspaceName: String
    let workspaceDetail: String

    var body: some View {
        VStack(spacing: 18) {
            WorkbenchTopBar(
                workspaceName: workspaceName,
                workspaceDetail: workspaceDetail,
                onMenuTap: { isDrawerOpen = true },
                onWorkspaceTap: { showWorkspaceSelector = true }
            )
            .padding(.horizontal, 20)
            .padding(.top, 12)

            ScrollView(showsIndicators: false) {
                VStack(alignment: .leading, spacing: 18) {
                    GlassPanel {
                        VStack(alignment: .leading, spacing: 12) {
                            HStack {
                                Text("Active session")
                                    .font(.headline)
                                    .foregroundColor(.ctxTextPrimary)
                                Spacer()
                                GlassPill(text: "Streaming", tint: .ctxAccent)
                            }
                            Text("Reviewing navigation shell layout, waiting for the next instruction.")
                                .font(.subheadline)
                                .foregroundColor(.ctxTextSecondary)
                            HStack(spacing: 12) {
                                WorkbenchStatView(title: "Turns", value: "12")
                                WorkbenchStatView(title: "Files", value: "3")
                                WorkbenchStatView(title: "Latency", value: "210ms")
                            }
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 14) {
                            Text("Quick actions")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            VStack(spacing: 10) {
                                QuickActionRowView(
                                    title: "Open workbench session",
                                    subtitle: "Jump back into the last run",
                                    icon: "sparkles"
                                )
                                QuickActionRowView(
                                    title: "Review diagnostics",
                                    subtitle: "Health checks and streaming logs",
                                    icon: "waveform.path.ecg"
                                )
                                QuickActionRowView(
                                    title: "Manage settings",
                                    subtitle: "Providers, tokens, and privacy",
                                    icon: "slider.horizontal.3"
                                )
                            }
                        }
                    }

                    GlassPanel {
                        VStack(alignment: .leading, spacing: 12) {
                            Text("Recent workspaces")
                                .font(.headline)
                                .foregroundColor(.ctxTextPrimary)
                            VStack(spacing: 12) {
                                WorkspaceRowView(title: "Context Monorepo", subtitle: "worktrees/ios-native-ui", status: "Active")
                                WorkspaceRowView(title: "Remote Devbox", subtitle: "ctx-remote-linux", status: "Sleeping")
                            }
                        }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.bottom, 100)
            }
        }
        .safeAreaInset(edge: .bottom) {
            MessageComposerView(text: $messageDraft)
        }
    }
}

private struct WorkbenchTopBar: View {
    let workspaceName: String
    let workspaceDetail: String
    let onMenuTap: () -> Void
    let onWorkspaceTap: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button(action: onMenuTap) {
                Image(systemName: "line.3.horizontal")
                    .font(.title2)
            }
            .foregroundColor(.ctxTextPrimary)

            Button(action: onWorkspaceTap) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(workspaceName)
                        .font(.headline)
                        .foregroundColor(.ctxTextPrimary)
                    Text(workspaceDetail)
                        .font(.caption)
                        .foregroundColor(.ctxTextMuted)
                }
            }

            Spacer()

            GlassPill(text: "Connected", tint: .ctxAccent)
        }
    }
}

private struct WorkbenchDrawerView: View {
    let workspaceName: String
    let workspaceDetail: String
    let drawerWidth: CGFloat
    let onClose: () -> Void
    let onWorkspaceTap: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack {
                Text("ctx")
                    .font(.title2.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Spacer()
                Button(action: onClose) {
                    Image(systemName: "xmark")
                        .font(.headline)
                }
                .foregroundColor(.ctxTextSecondary)
            }

            Button(action: onWorkspaceTap) {
                HStack(spacing: 12) {
                    Image(systemName: "rectangle.grid.1x2")
                        .foregroundColor(.ctxAccent)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(workspaceName)
                            .foregroundColor(.ctxTextPrimary)
                            .font(.subheadline.weight(.semibold))
                        Text(workspaceDetail)
                            .foregroundColor(.ctxTextMuted)
                            .font(.caption)
                    }
                    Spacer()
                    Image(systemName: "chevron.right")
                        .foregroundColor(.ctxTextSecondary)
                        .font(.caption)
                }
                .padding(12)
            }
            .buttonStyle(CtxGhostButtonStyle())

            VStack(alignment: .leading, spacing: 12) {
                Text("Sessions")
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
                DrawerSessionRowView(title: "Navigation shell build", subtitle: "2m ago", status: "Active")
                DrawerSessionRowView(title: "Workbench polish", subtitle: "Yesterday", status: "Completed")
                DrawerSessionRowView(title: "Connection test", subtitle: "2 days ago", status: "Paused")
            }

            VStack(alignment: .leading, spacing: 12) {
                Text("Shortcuts")
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.ctxTextMuted)
                NavigationLink {
                    SettingsView()
                } label: {
                    DrawerLinkRowView(title: "Settings", icon: "gearshape")
                }
                NavigationLink {
                    DiagnosticsView()
                } label: {
                    DrawerLinkRowView(title: "Diagnostics", icon: "waveform.path.ecg")
                }
            }

            Spacer()

            HStack(spacing: 12) {
                GlassPill(text: "Daemon healthy", tint: .ctxAccent)
                Spacer()
                Image(systemName: "antenna.radiowaves.left.and.right")
                    .foregroundColor(.ctxTextSecondary)
            }
        }
        .padding(20)
        .frame(width: drawerWidth)
        .frame(maxHeight: .infinity, alignment: .top)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .stroke(Color.ctxGlassStroke, lineWidth: 0.8)
        )
        .shadow(color: Color.ctxShadow, radius: 24, x: 0, y: 12)
        .padding(.top, 12)
        .padding(.bottom, 24)
        .padding(.leading, 12)
    }
}

private struct WorkbenchStatView: View {
    let title: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption)
                .foregroundColor(.ctxTextMuted)
            Text(value)
                .font(.headline)
                .foregroundColor(.ctxTextPrimary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(10)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct QuickActionRowView: View {
    let title: String
    let subtitle: String
    let icon: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .foregroundColor(.ctxAccent)
                .frame(width: 28)
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
        .background(Color.ctxSurface.opacity(0.55), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct WorkspaceRowView: View {
    let title: String
    let subtitle: String
    let status: String

    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                Circle()
                    .fill(Color.ctxSurfaceRaised)
                Image(systemName: "folder")
                    .foregroundColor(.ctxTextSecondary)
            }
            .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
            GlassPill(text: status, tint: status == "Active" ? .ctxAccent : .ctxTextSecondary)
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct DrawerSessionRowView: View {
    let title: String
    let subtitle: String
    let status: String

    var body: some View {
        HStack(spacing: 12) {
            Circle()
                .fill(status == "Active" ? Color.ctxAccent : Color.ctxSurfaceRaised)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.ctxTextPrimary)
                Text(subtitle)
                    .font(.caption)
                    .foregroundColor(.ctxTextMuted)
            }
            Spacer()
            Text(status)
                .font(.caption.weight(.semibold))
                .foregroundColor(status == "Active" ? .ctxAccent : .ctxTextSecondary)
        }
        .padding(12)
        .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.ctxLine, lineWidth: 1)
        )
    }
}

private struct DrawerLinkRowView: View {
    let title: String
    let icon: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .foregroundColor(.ctxAccent)
            Text(title)
                .foregroundColor(.ctxTextPrimary)
                .font(.subheadline.weight(.semibold))
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

private struct MessageComposerView: View {
    @Binding var text: String

    var body: some View {
        GlassPanel(cornerRadius: 24, padding: 12) {
            HStack(alignment: .bottom, spacing: 12) {
                Image(systemName: "sparkles")
                    .foregroundColor(.ctxAccent)
                TextField("Message ctx...", text: $text, axis: .vertical)
                    .foregroundColor(.ctxTextPrimary)
                    .lineLimit(1...4)
                    .textInputAutocapitalization(.sentences)
                Button {
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.title2)
                        .foregroundColor(text.isEmpty ? .ctxTextMuted : .ctxAccent)
                }
                .disabled(text.isEmpty)
            }
        }
        .padding(.horizontal, 20)
        .padding(.bottom, 14)
        .background(Color.ctxBackground.opacity(0.85))
    }
}

#Preview {
    WorkbenchShellView()
}
