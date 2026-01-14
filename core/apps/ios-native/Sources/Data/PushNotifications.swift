import Foundation
import UserNotifications
import UIKit
import _Concurrency

@MainActor
final class PushNotificationManager: NSObject, ObservableObject {
    static let shared = PushNotificationManager()

    @Published private(set) var pushToken: String?
    @Published private(set) var authorizationStatus: UNAuthorizationStatus = .notDetermined
    @Published private(set) var lastError: String?
    @Published var isEnabled: Bool

    private let enabledKey = "ctx.ios.push.enabled.v1"
    private let tokenKey = "ctx.ios.push.token.v1"

    override init() {
        let storedEnabled = UserDefaults.standard.bool(forKey: enabledKey)
        let storedToken = UserDefaults.standard.string(forKey: tokenKey)
        self.isEnabled = storedEnabled
        self.pushToken = storedToken
        super.init()
        _Concurrency.Task { await refreshAuthorizationStatus() }
    }

    func refreshAuthorizationStatus() async {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        authorizationStatus = settings.authorizationStatus
    }

    func setEnabled(_ enabled: Bool) {
        isEnabled = enabled
        UserDefaults.standard.set(enabled, forKey: enabledKey)
        if enabled {
            _Concurrency.Task { _ = await requestAuthorizationAndRegister() }
        } else {
            UIApplication.shared.unregisterForRemoteNotifications()
            clearToken()
        }
    }

    func requestAuthorizationAndRegister() async -> Bool {
        do {
            let granted = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .badge, .sound])
            await refreshAuthorizationStatus()
            if granted {
                UIApplication.shared.registerForRemoteNotifications()
            }
            return granted
        } catch {
            lastError = "Failed to request notification access."
            return false
        }
    }

    func updateDeviceToken(_ data: Data) {
        let token = data.map { String(format: "%02x", $0) }.joined()
        pushToken = token
        lastError = nil
        UserDefaults.standard.set(token, forKey: tokenKey)
    }

    func updateRegistrationError(_ error: Error) {
        lastError = error.localizedDescription
    }

    func clearToken() {
        pushToken = nil
        UserDefaults.standard.removeObject(forKey: tokenKey)
    }
}

final class PushNotificationAppDelegate: NSObject, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        _Concurrency.Task { @MainActor in
            PushNotificationManager.shared.updateDeviceToken(deviceToken)
        }
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        _Concurrency.Task { @MainActor in
            PushNotificationManager.shared.updateRegistrationError(error)
        }
    }

    func applicationDidBecomeActive(_ application: UIApplication) {
        _Concurrency.Task { @MainActor in
            await PushNotificationManager.shared.refreshAuthorizationStatus()
        }
    }
}
