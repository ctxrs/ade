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
    }

    func testSendAndReceiveMessage() throws {
        guard let config = resolveConfig() else {
            throw XCTSkip("Pass CTX_IOS_BASE_URL + CTX_IOS_TOKEN (or --ctx-base-url/--ctx-token) to run UI tests.")
        }

        let app = XCUIApplication()
        app.launchEnvironment["CTX_IOS_BASE_URL"] = config.baseURL
        app.launchEnvironment["CTX_IOS_TOKEN"] = config.token
        app.launchEnvironment["CTX_AUTO_CONNECT"] = "1"
        app.launchEnvironment["CTX_RESET_SELECTION"] = "1"
        app.launchEnvironment["CTX_UI_TEST_MODE"] = "1"
        app.launch()

        connectIfNeeded(app: app, config: config)

        let drawerButton = app.buttons["drawer.open"]
        if drawerButton.waitForExistence(timeout: 10) {
            drawerButton.tap()
            let drawer = app.otherElements["drawer.container"]
            if drawer.waitForExistence(timeout: 5) {
                app.buttons["drawer.close"].tap()
            }
        }

        let prompt = "UI Test \(Int(Date().timeIntervalSince1970))"
        let promptField = waitForPromptInput(in: app, timeout: 30)
        if promptField.isHittable {
            promptField.tap()
            promptField.typeText(prompt)
        } else {
            app.activate()
            promptField.tap()
            promptField.typeText(prompt)
        }

        let startButton = app.buttons["newtask.start"]
        XCTAssertTrue(startButton.waitForExistence(timeout: 10))
        let startEnabled = NSPredicate(format: "isEnabled == true")
        expectation(for: startEnabled, evaluatedWith: startButton)
        waitForExpectations(timeout: 20)
        startButton.tap()

        let messageList = app.scrollViews["chat.messages"]
        XCTAssertTrue(messageList.waitForExistence(timeout: 30))
        attachScreenshot("chat-loaded")

        let composer = waitForComposerInput(in: app, timeout: 10)
        if composer.isHittable {
            composer.tap()
            composer.typeText("Ping from UI test")
        } else {
            app.activate()
            composer.tap()
            composer.typeText("Ping from UI test")
        }
        app.buttons["chat.composer.send"].tap()

        XCTAssertTrue(app.staticTexts["Ping from UI test"].waitForExistence(timeout: 20))
        attachScreenshot("chat-after-send")

        let assistantTexts = app.staticTexts.matching(identifier: "chat.message.text.assistant")
        let assistantPredicate = NSPredicate(format: "count >= 1")
        expectation(for: assistantPredicate, evaluatedWith: assistantTexts)
        waitForExpectations(timeout: 90)
        attachScreenshot("chat-received")
    }
}
