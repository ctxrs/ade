import SwiftUI

enum LucideIconName {
    case image
    case gitBranch
    case monitor
    case terminal
    case ellipsis
    case atSign
    case slash
    case chevronDown
    case chevronRight
    case mic
    case arrowDown
    case arrowUp
    case square
    case expand
    case x
    case copy
    case check
    case settings
    case plus
}

enum LucideIconStyle {
    case stroke
    case fill
}

struct LucideIcon: View {
    let name: LucideIconName
    var size: CGFloat = 16
    var style: LucideIconStyle = .stroke

    var body: some View {
        let scale = size / 24
        let lineWidth = size * 2 / 24
        let path = lucidePath(name)
            .applying(CGAffineTransform(scaleX: scale, y: scale))
        if style == .fill {
            path
                .fill()
                .frame(width: size, height: size)
        } else {
            path
                .stroke(style: StrokeStyle(lineWidth: lineWidth, lineCap: .round, lineJoin: .round))
                .frame(width: size, height: size)
        }
    }

    private func lucidePath(_ name: LucideIconName) -> Path {
        var path = Path()
        switch name {
        case .image:
            path.addRoundedRect(in: CGRect(x: 3, y: 3, width: 18, height: 18), cornerSize: CGSize(width: 2, height: 2))
            path.addEllipse(in: CGRect(x: 7, y: 7, width: 4, height: 4))
            path.move(to: CGPoint(x: 21, y: 15))
            path.addLine(to: CGPoint(x: 17.914, y: 11.914))
            path.addLine(to: CGPoint(x: 6, y: 21))
        case .gitBranch:
            path.move(to: CGPoint(x: 6, y: 3))
            path.addLine(to: CGPoint(x: 6, y: 15))
            path.addEllipse(in: CGRect(x: 15, y: 3, width: 6, height: 6))
            path.addEllipse(in: CGRect(x: 3, y: 15, width: 6, height: 6))
            path.move(to: CGPoint(x: 18, y: 9))
            path.addArc(center: CGPoint(x: 9, y: 9), radius: 9, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
        case .monitor:
            path.addRoundedRect(in: CGRect(x: 2, y: 3, width: 20, height: 14), cornerSize: CGSize(width: 2, height: 2))
            path.move(to: CGPoint(x: 8, y: 21))
            path.addLine(to: CGPoint(x: 16, y: 21))
            path.move(to: CGPoint(x: 12, y: 17))
            path.addLine(to: CGPoint(x: 12, y: 21))
        case .terminal:
            path.move(to: CGPoint(x: 12, y: 19))
            path.addLine(to: CGPoint(x: 20, y: 19))
            path.move(to: CGPoint(x: 4, y: 17))
            path.addLine(to: CGPoint(x: 10, y: 11))
            path.addLine(to: CGPoint(x: 4, y: 5))
        case .ellipsis:
            path.addEllipse(in: CGRect(x: 4, y: 11, width: 2, height: 2))
            path.addEllipse(in: CGRect(x: 11, y: 11, width: 2, height: 2))
            path.addEllipse(in: CGRect(x: 18, y: 11, width: 2, height: 2))
        case .atSign:
            path.addEllipse(in: CGRect(x: 8, y: 8, width: 8, height: 8))
            path.move(to: CGPoint(x: 16, y: 8))
            path.addLine(to: CGPoint(x: 16, y: 13))
            path.addArc(center: CGPoint(x: 19, y: 13), radius: 3, startAngle: .degrees(180), endAngle: .degrees(0), clockwise: false)
            path.addLine(to: CGPoint(x: 22, y: 12))
            path.addArc(center: CGPoint(x: 12, y: 12), radius: 10, startAngle: .degrees(0), endAngle: .degrees(130), clockwise: true)
        case .slash:
            path.move(to: CGPoint(x: 22, y: 2))
            path.addLine(to: CGPoint(x: 2, y: 22))
        case .chevronDown:
            path.move(to: CGPoint(x: 6, y: 9))
            path.addLine(to: CGPoint(x: 12, y: 15))
            path.addLine(to: CGPoint(x: 18, y: 9))
        case .chevronRight:
            path.move(to: CGPoint(x: 9, y: 6))
            path.addLine(to: CGPoint(x: 15, y: 12))
            path.addLine(to: CGPoint(x: 9, y: 18))
        case .mic:
            path.move(to: CGPoint(x: 12, y: 19))
            path.addLine(to: CGPoint(x: 12, y: 22))
            path.move(to: CGPoint(x: 19, y: 10))
            path.addLine(to: CGPoint(x: 19, y: 12))
            path.addArc(center: CGPoint(x: 12, y: 12), radius: 7, startAngle: .degrees(0), endAngle: .degrees(180), clockwise: false)
            path.addLine(to: CGPoint(x: 5, y: 10))
            path.addRoundedRect(in: CGRect(x: 9, y: 2, width: 6, height: 13), cornerSize: CGSize(width: 3, height: 3))
        case .arrowDown:
            path.move(to: CGPoint(x: 5, y: 12))
            path.addLine(to: CGPoint(x: 12, y: 19))
            path.addLine(to: CGPoint(x: 19, y: 12))
            path.move(to: CGPoint(x: 12, y: 5))
            path.addLine(to: CGPoint(x: 12, y: 19))
        case .arrowUp:
            path.move(to: CGPoint(x: 5, y: 12))
            path.addLine(to: CGPoint(x: 12, y: 5))
            path.addLine(to: CGPoint(x: 19, y: 12))
            path.move(to: CGPoint(x: 12, y: 19))
            path.addLine(to: CGPoint(x: 12, y: 5))
        case .square:
            path.addRoundedRect(in: CGRect(x: 3, y: 3, width: 18, height: 18), cornerSize: CGSize(width: 2, height: 2))
        case .expand:
            path.move(to: CGPoint(x: 15, y: 15))
            path.addLine(to: CGPoint(x: 21, y: 21))
            path.move(to: CGPoint(x: 15, y: 9))
            path.addLine(to: CGPoint(x: 21, y: 3))
            path.move(to: CGPoint(x: 21, y: 16))
            path.addLine(to: CGPoint(x: 21, y: 21))
            path.addLine(to: CGPoint(x: 16, y: 21))
            path.move(to: CGPoint(x: 21, y: 8))
            path.addLine(to: CGPoint(x: 21, y: 3))
            path.addLine(to: CGPoint(x: 16, y: 3))
            path.move(to: CGPoint(x: 3, y: 16))
            path.addLine(to: CGPoint(x: 3, y: 21))
            path.addLine(to: CGPoint(x: 8, y: 21))
            path.move(to: CGPoint(x: 3, y: 21))
            path.addLine(to: CGPoint(x: 9, y: 15))
            path.move(to: CGPoint(x: 3, y: 8))
            path.addLine(to: CGPoint(x: 3, y: 3))
            path.addLine(to: CGPoint(x: 8, y: 3))
            path.move(to: CGPoint(x: 9, y: 9))
            path.addLine(to: CGPoint(x: 3, y: 3))
        case .x:
            path.move(to: CGPoint(x: 6, y: 6))
            path.addLine(to: CGPoint(x: 18, y: 18))
            path.move(to: CGPoint(x: 6, y: 18))
            path.addLine(to: CGPoint(x: 18, y: 6))
        case .copy:
            path.addRoundedRect(in: CGRect(x: 9, y: 9, width: 13, height: 13), cornerSize: CGSize(width: 2, height: 2))
            path.addRoundedRect(in: CGRect(x: 3, y: 3, width: 13, height: 13), cornerSize: CGSize(width: 2, height: 2))
        case .check:
            path.move(to: CGPoint(x: 4, y: 12))
            path.addLine(to: CGPoint(x: 9, y: 17))
            path.addLine(to: CGPoint(x: 20, y: 6))
        case .settings:
            let center = CGPoint(x: 12, y: 12)
            let teeth = 8
            let outerRadius: CGFloat = 9
            let innerRadius: CGFloat = 6.5
            let angleStep = CGFloat.pi * 2 / CGFloat(teeth * 2)

            for index in 0..<(teeth * 2) {
                let radius = index.isMultiple(of: 2) ? outerRadius : innerRadius
                let angle = CGFloat(index) * angleStep - (.pi / 2)
                let point = CGPoint(
                    x: center.x + cos(angle) * radius,
                    y: center.y + sin(angle) * radius
                )
                if index == 0 {
                    path.move(to: point)
                } else {
                    path.addLine(to: point)
                }
            }
            path.closeSubpath()
            path.addEllipse(in: CGRect(x: 9, y: 9, width: 6, height: 6))
        case .plus:
            path.move(to: CGPoint(x: 12, y: 5))
            path.addLine(to: CGPoint(x: 12, y: 19))
            path.move(to: CGPoint(x: 5, y: 12))
            path.addLine(to: CGPoint(x: 19, y: 12))
        }
        return path
    }
}
