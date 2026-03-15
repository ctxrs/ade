#!/usr/bin/env swift

import AppKit
import CoreGraphics
import Foundation

struct Scenario: Decodable {
    let actions: [Action]
}

enum Action: Decodable {
    case move(x: Double, y: Double, durationMs: Int)
    case drag(x: Double, y: Double, durationMs: Int)
    case click(button: String)
    case type(text: String, cps: Double?)
    case wait(durationMs: Int)
    case key(name: String)

    enum CodingKeys: String, CodingKey {
        case kind
        case x
        case y
        case durationMs = "duration_ms"
        case button
        case text
        case cps
        case name
    }

    enum Kind: String, Decodable {
        case move
        case drag
        case click
        case type
        case wait
        case key
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(Kind.self, forKey: .kind)
        switch kind {
        case .move:
            self = .move(
                x: try container.decode(Double.self, forKey: .x),
                y: try container.decode(Double.self, forKey: .y),
                durationMs: try container.decodeIfPresent(Int.self, forKey: .durationMs) ?? 400
            )
        case .drag:
            self = .drag(
                x: try container.decode(Double.self, forKey: .x),
                y: try container.decode(Double.self, forKey: .y),
                durationMs: try container.decodeIfPresent(Int.self, forKey: .durationMs) ?? 400
            )
        case .click:
            self = .click(
                button: try container.decodeIfPresent(String.self, forKey: .button) ?? "left"
            )
        case .type:
            self = .type(
                text: try container.decode(String.self, forKey: .text),
                cps: try container.decodeIfPresent(Double.self, forKey: .cps)
            )
        case .wait:
            self = .wait(durationMs: try container.decode(Int.self, forKey: .durationMs))
        case .key:
            self = .key(name: try container.decode(String.self, forKey: .name))
        }
    }
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

func checkAccessibility() {
    let options: NSDictionary = [kAXTrustedCheckOptionPrompt.takeRetainedValue() as String: true]
    if !AXIsProcessTrustedWithOptions(options) {
        fail("Accessibility permission is required for macOS demo conductor.")
    }
}

func sleepMs(_ durationMs: Int) {
    usleep(useconds_t(durationMs * 1000))
}

func postMouseEvent(type: CGEventType, point: CGPoint, button: CGMouseButton) {
    guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: button) else {
        fail("Failed to create mouse event \(type.rawValue)")
    }
    event.post(tap: .cghidEventTap)
}

func moveCursorSmoothly(to target: CGPoint, durationMs: Int) {
    let start = NSEvent.mouseLocation
    let steps = max(1, durationMs / 16)
    for step in 1...steps {
        let progress = Double(step) / Double(steps)
        let eased = 1 - pow(1 - progress, 3)
        let point = CGPoint(
            x: start.x + (target.x - start.x) * eased,
            y: start.y + (target.y - start.y) * eased
        )
        postMouseEvent(type: .mouseMoved, point: point, button: .left)
        sleepMs(16)
    }
}

func dragCursorSmoothly(to target: CGPoint, durationMs: Int) {
    let start = NSEvent.mouseLocation
    postMouseEvent(type: .leftMouseDown, point: start, button: .left)
    let steps = max(1, durationMs / 16)
    for step in 1...steps {
        let progress = Double(step) / Double(steps)
        let eased = 1 - pow(1 - progress, 3)
        let point = CGPoint(
            x: start.x + (target.x - start.x) * eased,
            y: start.y + (target.y - start.y) * eased
        )
        postMouseEvent(type: .leftMouseDragged, point: point, button: .left)
        sleepMs(16)
    }
    postMouseEvent(type: .leftMouseUp, point: target, button: .left)
}

func click(button: String) {
    let point = NSEvent.mouseLocation
    let mouseButton: CGMouseButton = button == "right" ? .right : .left
    let downType: CGEventType = button == "right" ? .rightMouseDown : .leftMouseDown
    let upType: CGEventType = button == "right" ? .rightMouseUp : .leftMouseUp
    postMouseEvent(type: downType, point: point, button: mouseButton)
    sleepMs(40)
    postMouseEvent(type: upType, point: point, button: mouseButton)
}

func postKey(character: UniChar) {
    guard let keyDown = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true),
          let keyUp = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false) else {
        fail("Failed to create keyboard event")
    }
    var char = character
    keyDown.keyboardSetUnicodeString(stringLength: 1, unicodeString: &char)
    keyUp.keyboardSetUnicodeString(stringLength: 1, unicodeString: &char)
    keyDown.post(tap: .cghidEventTap)
    keyUp.post(tap: .cghidEventTap)
}

func postVirtualKey(_ keyCode: CGKeyCode) {
    guard let keyDown = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: true),
          let keyUp = CGEvent(keyboardEventSource: nil, virtualKey: keyCode, keyDown: false) else {
        fail("Failed to create virtual keyboard event")
    }
    keyDown.post(tap: .cghidEventTap)
    keyUp.post(tap: .cghidEventTap)
}

func keyCode(for name: String) -> CGKeyCode? {
    switch name.lowercased() {
    case "return", "enter":
        return 36
    case "tab":
        return 48
    case "space":
        return 49
    case "escape", "esc":
        return 53
    default:
        return nil
    }
}

func typeText(_ text: String, cps: Double?) {
    let charsPerSecond = max(1.0, cps ?? 12.0)
    let delayMs = Int(1000.0 / charsPerSecond)
    for codeUnit in text.utf16 {
        postKey(character: codeUnit)
        sleepMs(delayMs)
    }
}

func usage() -> Never {
    let text = """
    macos_demo_conductor.swift

    Usage:
      swift core/apps/desktop/scripts/macos_demo_conductor.swift --scenario <file>
    """
    print(text)
    exit(0)
}

let args = CommandLine.arguments
guard let scenarioIndex = args.firstIndex(of: "--scenario") else {
    usage()
}
guard scenarioIndex + 1 < args.count else {
    fail("--scenario requires a path")
}

let scenarioPath = args[scenarioIndex + 1]
let scenarioData = try Data(contentsOf: URL(fileURLWithPath: scenarioPath))
let scenario = try JSONDecoder().decode(Scenario.self, from: scenarioData)

checkAccessibility()

for action in scenario.actions {
    switch action {
    case let .move(x, y, durationMs):
        moveCursorSmoothly(to: CGPoint(x: x, y: y), durationMs: durationMs)
    case let .drag(x, y, durationMs):
        dragCursorSmoothly(to: CGPoint(x: x, y: y), durationMs: durationMs)
    case let .click(button):
        click(button: button)
    case let .type(text, cps):
        typeText(text, cps: cps)
    case let .wait(durationMs):
        sleepMs(durationMs)
    case let .key(name):
        guard let keyCode = keyCode(for: name) else {
            fail("Unsupported key action: \(name)")
        }
        postVirtualKey(keyCode)
    }
}
