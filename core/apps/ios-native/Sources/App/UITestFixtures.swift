import Foundation

enum UITestFixtures {
    static let workspace = WorkspaceSummary(
        id: "00000000-0000-0000-0000-000000000001",
        name: "Atlas",
        rootPath: "/Users/example-user/atlas",
        createdAt: "2024-06-01T12:00:00Z"
    )

    static let settings = PublicSettings(
        dictation: DictationSettings(
            enabled: true,
            provider: "livekit",
            livekit: LiveKitDictationSettings(
                baseUrl: "wss://dictation.ctx.dev",
                apiKey: "lk_test_key",
                apiSecretSet: true,
                model: "nova",
                language: "en"
            )
        ),
        telemetry: TelemetrySettings(
            enabled: true,
            endpoint: "https://telemetry.ctx.dev"
        ),
        titleGeneration: TitleGenerationSettings(
            baseUrl: "https://api.openai.com",
            apiKey: "sk-test-key",
            model: "gpt-4.1-mini",
            useJson: true
        ),
        resourceGovernance: ResourceGovernanceSettings(
            enabled: true,
            mode: "adaptive",
            cpuQuotaPct: 80,
            memoryHighMb: 4096,
            memoryMaxMb: 8192,
            effective: ResourceGovernanceLimits(
                cpuQuotaPct: 80,
                memoryHighMb: 4096,
                memoryMaxMb: 8192
            ),
            status: ResourceGovernanceStatus(
                state: "healthy",
                canApplyNow: true,
                requiresRestart: false,
                message: "Within limits"
            )
        ),
        providerGuard: ProviderGuardSettings(
            enabled: true,
            mode: "conservative",
            memoryHighMb: 4096,
            memoryMaxMb: 8192,
            intervalMs: 300_000,
            gracePeriodMs: 600_000
        )
    )

    static let entitlements = EntitlementsSnapshot(
        planType: .pro,
        features: [
            "remote_mobile_access": "enabled",
            "push_notifications": "enabled"
        ],
        expiresAt: "2025-01-01T00:00:00Z",
        graceExpiresAt: "2025-01-08T00:00:00Z"
    )

    static let mobileAccessStatus = MobileAccessStatus(
        enabled: true,
        tunnelId: "tnl_8f3a21",
        publicBaseUrl: "https://atlas.ctx.dev",
        relayBaseUrl: "https://relay.ctx.dev",
        daemonPublicKey: "pk_test_daemon",
        tunnelState: .running,
        lastError: nil
    )

    static let mobileAccessResponse = EnableMobileAccessResponse(
        status: mobileAccessStatus,
        qrPayload: .object([
            "base_url": .string("https://atlas.ctx.dev"),
            "token": .string("ctx_test_token"),
            "device": .string("iPhone 15 Pro")
        ]),
        pairingExpiresAt: "2024-07-02T12:00:00Z"
    )

    static let providers: [ProviderStatus] = [
        ProviderStatus(
            providerId: "codex",
            installed: true,
            detectedPath: "/usr/local/bin/codex",
            version: "0.18.1",
            health: "ok",
            diagnostics: ["ok"],
            details: ["ui_label": "OpenAI Codex"]
        ),
        ProviderStatus(
            providerId: "claude",
            installed: true,
            detectedPath: "/usr/local/bin/claude",
            version: "1.2.0",
            health: "ok",
            diagnostics: ["ok"],
            details: ["ui_label": "Claude Code"]
        )
    ]

    static let diagnostics = Diagnostics(
        daemon: Diagnostics.Daemon(
            version: "0.6.1",
            pid: 12345,
            dataRoot: "/Users/example-user/.ctx",
            daemonUrl: "http://127.0.0.1:4399",
            authRequired: true
        ),
        platform: Diagnostics.Platform(
            os: "macOS 14.6",
            arch: "arm64"
        ),
        logs: Diagnostics.Logs(
            dir: "/Users/example-user/.ctx/logs",
            files: [
                Diagnostics.Logs.LogFile(
                    name: "daemon.log",
                    bytes: 120_456,
                    modifiedUtc: "2024-07-01T12:00:00Z"
                ),
                Diagnostics.Logs.LogFile(
                    name: "ios-client.log",
                    bytes: 42_880,
                    modifiedUtc: "2024-07-01T11:30:00Z"
                )
            ]
        ),
        providers: providers,
        managedInstalls: nil
    )

    static let routingEntry = ModelRoutingEntry(
        providerId: "codex",
        modelId: "gpt-5.2-codex",
        updatedAt: "2024-07-02T08:15:00Z"
    )

    static let supabaseEmail = "jane@ctx.dev"

    static let newTaskModels: [String] = [
        "gpt-5.2-codex",
        "gpt-4.1-mini",
        "claude-3.5-sonnet"
    ]

    static let taskListWorkspaces: [WorkspaceSummary] = [
        workspace,
        WorkspaceSummary(
            id: "00000000-0000-0000-0000-000000000002",
            name: "Remote Devbox",
            rootPath: "/Users/example-user/devbox",
            createdAt: "2024-05-10T09:00:00Z"
        ),
        WorkspaceSummary(
            id: "00000000-0000-0000-0000-000000000003",
            name: "Docs Sandbox",
            rootPath: "/Users/example-user/docs",
            createdAt: "2024-04-18T16:20:00Z"
        )
    ]

    static let taskListActiveTasks: [WorkspaceTaskSummary] = {
        let workspaceId = CtxID(workspace.id)
        let worktreeId = CtxID("00000000-0000-0000-0000-000000000301")

        let sessionA = makeSessionSummary(
            id: "00000000-0000-0000-0000-000000000201",
            taskId: "00000000-0000-0000-0000-000000000101",
            workspaceId: workspaceId.rawValue,
            worktreeId: worktreeId.rawValue,
            providerId: "codex",
            modelId: "gpt-5.2-codex",
            status: "running",
            title: "Drawer polish",
            lastMessagePreview: "Refining the workbench drawer spacing.",
            isWorking: true
        )

        let sessionB = makeSessionSummary(
            id: "00000000-0000-0000-0000-000000000202",
            taskId: "00000000-0000-0000-0000-000000000102",
            workspaceId: workspaceId.rawValue,
            worktreeId: worktreeId.rawValue,
            providerId: "claude",
            modelId: "claude-3.5-sonnet",
            status: "failed",
            title: "Diff parsing",
            lastMessagePreview: "Diff parser crashed on renamed file.",
            isWorking: false
        )

        let sessionC = makeSessionSummary(
            id: "00000000-0000-0000-0000-000000000203",
            taskId: "00000000-0000-0000-0000-000000000103",
            workspaceId: workspaceId.rawValue,
            worktreeId: worktreeId.rawValue,
            providerId: "codex",
            modelId: "gpt-4.1-mini",
            status: "completed",
            title: "Copy review",
            lastMessagePreview: "Settings copy updated and approved.",
            isWorking: false
        )

        let taskA = Task(
            id: CtxID("00000000-0000-0000-0000-000000000101"),
            workspaceId: workspaceId,
            title: "Polish drawer list",
            description: "Tune spacing and task indicators",
            status: "running",
            createdAt: "2024-06-30T14:00:00Z",
            updatedAt: "2024-07-01T09:20:00Z",
            archivedAt: nil,
            assistantSeenAt: "2024-07-01T09:00:00Z",
            lastActivityAt: "2024-07-01T09:20:00Z",
            lastAssistantMessageAt: nil,
            hasActiveSession: true,
            primarySessionId: sessionA.session.id,
            primaryWorktreeId: worktreeId
        )

        let taskB = Task(
            id: CtxID("00000000-0000-0000-0000-000000000102"),
            workspaceId: workspaceId,
            title: "Fix diff rendering",
            description: "Handle rename hunks",
            status: "failed",
            createdAt: "2024-06-28T10:30:00Z",
            updatedAt: "2024-07-01T08:40:00Z",
            archivedAt: nil,
            assistantSeenAt: "2024-07-01T08:30:00Z",
            lastActivityAt: "2024-07-01T08:40:00Z",
            lastAssistantMessageAt: nil,
            hasActiveSession: false,
            primarySessionId: sessionB.session.id,
            primaryWorktreeId: worktreeId
        )

        let taskC = Task(
            id: CtxID("00000000-0000-0000-0000-000000000103"),
            workspaceId: workspaceId,
            title: "Sync settings copy",
            description: "Align with desktop copy",
            status: "completed",
            createdAt: "2024-06-20T09:00:00Z",
            updatedAt: "2024-07-01T10:20:00Z",
            archivedAt: nil,
            assistantSeenAt: "2024-07-01T10:00:00Z",
            lastActivityAt: "2024-07-01T10:20:00Z",
            lastAssistantMessageAt: "2024-07-01T10:15:00Z",
            hasActiveSession: false,
            primarySessionId: sessionC.session.id,
            primaryWorktreeId: worktreeId
        )

        return [
            WorkspaceTaskSummary(task: taskA, sessions: [sessionA], sortAt: "2024-07-01T09:20:00Z"),
            WorkspaceTaskSummary(task: taskB, sessions: [sessionB], sortAt: "2024-07-01T08:40:00Z"),
            WorkspaceTaskSummary(task: taskC, sessions: [sessionC], sortAt: "2024-07-01T10:20:00Z")
        ]
    }()

    static let taskListArchivedTasks: [WorkspaceTaskSummary] = {
        let workspaceId = CtxID(workspace.id)
        let worktreeId = CtxID("00000000-0000-0000-0000-000000000302")

        let session = makeSessionSummary(
            id: "00000000-0000-0000-0000-000000000204",
            taskId: "00000000-0000-0000-0000-000000000104",
            workspaceId: workspaceId.rawValue,
            worktreeId: worktreeId.rawValue,
            providerId: "gemini",
            modelId: "gemini-2.5-pro",
            status: "completed",
            title: "Archived follow-up",
            lastMessagePreview: "Task archived after merge.",
            isWorking: false
        )

        let task = Task(
            id: CtxID("00000000-0000-0000-0000-000000000104"),
            workspaceId: workspaceId,
            title: "Archive old session",
            description: nil,
            status: "completed",
            createdAt: "2024-06-10T11:00:00Z",
            updatedAt: "2024-06-15T10:00:00Z",
            archivedAt: "2024-06-15T10:00:00Z",
            assistantSeenAt: "2024-06-15T09:00:00Z",
            lastActivityAt: "2024-06-15T10:00:00Z",
            lastAssistantMessageAt: "2024-06-15T09:30:00Z",
            hasActiveSession: false,
            primarySessionId: session.session.id,
            primaryWorktreeId: worktreeId
        )

        return [
            WorkspaceTaskSummary(task: task, sessions: [session], sortAt: "2024-06-15T10:00:00Z")
        ]
    }()

    private static func makeSessionSummary(
        id: String,
        taskId: String,
        workspaceId: String,
        worktreeId: String,
        providerId: String,
        modelId: String,
        status: String,
        title: String,
        lastMessagePreview: String?,
        isWorking: Bool
    ) -> SessionSnapshotSummary {
        let session = Session(
            id: CtxID(id),
            taskId: CtxID(taskId),
            workspaceId: CtxID(workspaceId),
            worktreeId: CtxID(worktreeId),
            parentSessionId: nil,
            relationship: nil,
            providerId: providerId,
            modelId: modelId,
            title: title,
            agentRole: "assistant",
            status: status,
            envTarget: nil,
            createdAt: "2024-07-01T08:00:00Z",
            updatedAt: "2024-07-01T09:30:00Z"
        )
        let activity = isWorking
            ? SessionActivityState(isWorking: true, lastTurnStatus: .running)
            : nil
        return SessionSnapshotSummary(
            session: session,
            lastMessageAt: "2024-07-01T09:30:00Z",
            lastMessagePreview: lastMessagePreview,
            lastEventSeq: 128,
            stateRev: 4,
            activity: activity,
            unread: nil
        )
    }

}
