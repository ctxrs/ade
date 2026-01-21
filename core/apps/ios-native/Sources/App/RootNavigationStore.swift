import SwiftUI

enum RootRoute {
    case launcher
    case workbench
}

@MainActor
final class RootNavigationStore: ObservableObject {
    @Published var route: RootRoute = .launcher
}
