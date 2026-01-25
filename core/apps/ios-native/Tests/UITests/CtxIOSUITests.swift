import XCTest

final class CtxIOSUITests: XCTestCase {
    override func setUp() {
        super.setUp()
        continueAfterFailure = false
    }

    private func argValue(_ args: [String], flag: String) -> String? {
        guard let index = args.firstIndex(of: flag), index + 1 < args.count else {
            return nil
        }
        let value = args[index + 1].trimmingCharacters(in: .whitespacesAndNewlines)
        return value.isEmpty ? nil : value
    }

    private func configFromInfoPlist() -> (baseURL: String, token: String)? {
        let bundle = Bundle(for: CtxIOSUITests.self)
        guard let rawBaseURL = bundle.object(forInfoDictionaryKey: "CTX_IOS_BASE_URL") as? String,
              let rawToken = bundle.object(forInfoDictionaryKey: "CTX_IOS_TOKEN") as? String else {
            return nil
        }
        let baseURL = rawBaseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let token = rawToken.trimmingCharacters(in: .whitespacesAndNewlines)
        if baseURL.isEmpty || token.isEmpty { return nil }
        if baseURL.contains("$(") || token.contains("$(") { return nil }
        return (baseURL: baseURL, token: token)
    }

    private func resolveConfig() -> (baseURL: String, token: String)? {
        let env = ProcessInfo.processInfo.environment
        let args = ProcessInfo.processInfo.arguments
        let baseURL = env["CTX_IOS_BASE_URL"] ?? env["CTX_BASE_URL"] ?? argValue(args, flag: "--ctx-base-url")
        let token = env["CTX_IOS_TOKEN"] ?? env["CTX_TOKEN"] ?? argValue(args, flag: "--ctx-token")
        if let baseURL, let token {
            return (baseURL: baseURL, token: token)
        }
        return configFromInfoPlist()
    }

    private func waitForComposerInput(in app: XCUIApplication, timeout: TimeInterval) -> XCUIElement {
        let textField = app.textFields["chat.composer.input"]
        if textField.waitForExistence(timeout: timeout) {
            return textField
        }
        let textView = app.textViews["chat.composer.input"]
        XCTAssertTrue(textView.waitForExistence(timeout: timeout))
        return textView
    }

    private func waitForPromptInput(in app: XCUIApplication, timeout: TimeInterval) -> XCUIElement {
        let textField = app.textFields["newtask.prompt"]
        if textField.waitForExistence(timeout: timeout) {
            return textField
        }
        let textView = app.textViews["newtask.prompt"]
        XCTAssertTrue(textView.waitForExistence(timeout: timeout))
        return textView
    }

    private func replaceText(in element: XCUIElement, value: String, app: XCUIApplication) {
        element.tap()
        if let current = element.value as? String, !current.isEmpty {
            element.press(forDuration: 0.8)
            let selectAll = app.menuItems["Select All"]
            if selectAll.waitForExistence(timeout: 1.5) {
                selectAll.tap()
            }
        }
        element.typeText(value)
    }

    private func connectIfNeeded(app: XCUIApplication, config: (baseURL: String, token: String)) {
        if app.textFields["newtask.prompt"].exists || app.textViews["newtask.prompt"].exists {
            return
        }
        let daemonField = app.textFields["connection.daemon"]
        if daemonField.waitForExistence(timeout: 6) {
            replaceText(in: daemonField, value: config.baseURL, app: app)
            let tokenField = app.secureTextFields["connection.token"]
            if tokenField.exists {
                replaceText(in: tokenField, value: config.token, app: app)
            } else {
                let fallbackTokenField = app.textFields["connection.token"]
                if fallbackTokenField.waitForExistence(timeout: 2) {
                    replaceText(in: fallbackTokenField, value: config.token, app: app)
                }
            }
            app.buttons["connection.connect"].tap()
        }
    }

    private func attachScreenshot(_ name: String) {
        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let outputDir = ProcessInfo.processInfo.environment["CTX_SCREENSHOT_DIR"] {
            let url = URL(fileURLWithPath: outputDir).appendingPathComponent("\(name).png")
            try? screenshot.pngRepresentation.write(to: url)
        }
    }

    private func tapElement(_ element: XCUIElement, in app: XCUIApplication) {
        if element.isHittable {
            element.tap()
        } else {
            let coordinate = element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            coordinate.tap()
        }
    }

    private func openDrawerIfNeeded(in app: XCUIApplication) {
        let scrim = app.otherElements["drawer.scrim"]
        if !scrim.exists {
            let drawerButton = app.buttons["drawer.open"]
            XCTAssertTrue(drawerButton.waitForExistence(timeout: 10))
            tapElement(drawerButton, in: app)
            XCTAssertTrue(scrim.waitForExistence(timeout: 5))
        }
    }

    private func closeDrawerIfOpen(in app: XCUIApplication) {
        let scrim = app.otherElements["drawer.scrim"]
        if scrim.exists {
            tapElement(scrim, in: app)
        }
    }

    private func openSettingsScreen(in app: XCUIApplication) {
        XCTAssertTrue(app.buttons["drawer.open"].waitForExistence(timeout: 20))
        openDrawerIfNeeded(in: app)
        var settingsLink = app.buttons["drawer.workspace.switch"]
        if !settingsLink.waitForExistence(timeout: 6) {
            settingsLink = app.otherElements["drawer.workspace.switch"]
            let scrollView = app.scrollViews.firstMatch
            if scrollView.exists {
                scrollView.swipeUp()
            }
        }
        if !settingsLink.exists {
            settingsLink = app.buttons["drawer.settings"]
        }
        if !settingsLink.exists {
            settingsLink = app.otherElements["drawer.settings"]
        }
        if !settingsLink.exists {
            settingsLink = app.staticTexts["Settings"]
        }
        XCTAssertTrue(settingsLink.waitForExistence(timeout: 20))
        tapElement(settingsLink, in: app)

        let settingsTitle = app.staticTexts["Settings"]
        XCTAssertTrue(settingsTitle.waitForExistence(timeout: 10))
    }

    private func startNewTask(app: XCUIApplication, prompt: String) {
        let promptField = waitForPromptInput(in: app, timeout: 30)
        app.activate()
        tapElement(promptField, in: app)
        if app.keyboards.firstMatch.waitForExistence(timeout: 2) {
            app.typeText(prompt)
        } else {
            tapElement(promptField, in: app)
            app.typeText(prompt)
        }

        let startButton = app.buttons["newtask.send"]
        XCTAssertTrue(startButton.waitForExistence(timeout: 10))
        let startEnabled = NSPredicate(format: "isEnabled == true")
        expectation(for: startEnabled, evaluatedWith: startButton)
        waitForExpectations(timeout: 20)
        startButton.tap()

        let messageList = app.scrollViews["chat.messages"]
        XCTAssertTrue(messageList.waitForExistence(timeout: 30))
    }

    func testConnectionScreenshot() {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "connection"
        app.launchEnvironment["CTX_AUTO_CONNECT"] = "0"
        app.launch()

        let connectButton = app.buttons["connection.connect"]
        XCTAssertTrue(connectButton.waitForExistence(timeout: 10))
        attachScreenshot("connection-view")
    }

    func testSettingsScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "settings"
        app.launch()

        let title = app.staticTexts["Settings"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        attachScreenshot("settings-view")
    }

    func testNewTaskScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "new_task"
        app.launch()

        _ = waitForPromptInput(in: app, timeout: 30)
        attachScreenshot("new-task-view")
    }

    func testTaskListScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "task_list"
        app.launch()

        let drawer = app.descendants(matching: .any).matching(identifier: "drawer.container").firstMatch
        XCTAssertTrue(drawer.waitForExistence(timeout: 20))
        attachScreenshot("task-list")
    }

    func testDiffPanelScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "diff"
        app.launch()

        let diffTitle = app.staticTexts["Diff"]
        XCTAssertTrue(diffTitle.waitForExistence(timeout: 10))
        attachScreenshot("diff-panel")
    }

    func testChatScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "chat"
        app.launch()

        let messageList = app.scrollViews["chat.messages"]
        XCTAssertTrue(messageList.waitForExistence(timeout: 20))
        attachScreenshot("chat-view")
    }

    func testMobileAccessScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "mobile_access"
        app.launch()

        let title = app.staticTexts["Mobile Access"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        attachScreenshot("mobile-access")
    }

    func testWorkspaceSelectorScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "workspace_selector"
        app.launch()

        let workspaceTitle = app.staticTexts["Select workspace"]
        XCTAssertTrue(workspaceTitle.waitForExistence(timeout: 10))
        attachScreenshot("workspace-selector")
    }

    func testDiagnosticsScreenshot() throws {
        let app = XCUIApplication()
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launchEnvironment["CTX_UI_TEST_SCREEN"] = "diagnostics"
        app.launch()

        let title = app.staticTexts["Diagnostics"]
        XCTAssertTrue(title.waitForExistence(timeout: 10))
        attachScreenshot("diagnostics")
    }
}
