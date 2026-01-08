import AVFoundation
import SwiftUI

private struct ConnectionQRResult {
    let baseURL: String
    let token: String
}

struct QRCodeScannerView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var cameraAccess: CameraAccess = .unknown
    @State private var scanError: String?
    @State private var isHandlingScan = false

    let onScan: (ConnectionQRResult) -> Void

    var body: some View {
        ZStack {
            CtxBackgroundView()
            VStack(spacing: 20) {
                HStack {
                    Text("Scan QR code")
                        .font(.title3.weight(.semibold))
                        .foregroundColor(.ctxTextPrimary)
                    Spacer()
                    Button {
                        dismiss()
                    } label: {
                        HStack(spacing: 6) {
                            Image(systemName: "xmark")
                            Text("Close")
                        }
                    }
                    .buttonStyle(CtxGhostButtonStyle())
                }

                ZStack {
                    if cameraAccess == .denied {
                        VStack(spacing: 8) {
                            Text("Camera access is required to scan QR codes.")
                                .font(.subheadline)
                                .foregroundColor(.ctxTextSecondary)
                                .multilineTextAlignment(.center)
                            Text("Enable camera access in Settings to continue.")
                                .font(.footnote)
                                .foregroundColor(.ctxTextMuted)
                        }
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                    } else {
                        QRCodeScannerPreview(isActive: cameraAccess == .allowed, onScan: handleScan)
                            .overlay(
                                RoundedRectangle(cornerRadius: 24, style: .continuous)
                                    .stroke(Color.ctxLine, lineWidth: 2)
                            )
                            .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
                            .overlay(
                                RoundedRectangle(cornerRadius: 24, style: .continuous)
                                    .stroke(Color.ctxGlassStroke, lineWidth: 0.8)
                            )
                    }
                }
                .frame(height: 360)
                .background(Color.ctxSurface.opacity(0.6), in: RoundedRectangle(cornerRadius: 24, style: .continuous))

                Text("Point your camera at the QR code to import the daemon URL and token.")
                    .font(.footnote)
                    .foregroundColor(.ctxTextMuted)
                    .multilineTextAlignment(.center)

                if let scanError {
                    Text(scanError)
                        .font(.footnote)
                        .foregroundColor(.ctxError)
                        .multilineTextAlignment(.center)
                }
            }
            .padding(.horizontal, 22)
            .padding(.top, 20)
        }
        .task {
            await updateCameraAccess()
        }
        .toolbar(.hidden, for: .navigationBar)
    }

    private func handleScan(_ payload: String) {
        guard !isHandlingScan else { return }
        isHandlingScan = true
        let trimmed = payload.trimmingCharacters(in: .whitespacesAndNewlines)
        let (result, error) = parseConnectionPayload(trimmed)
        if let result {
            onScan(result)
            dismiss()
        } else {
            scanError = error ?? "This QR code does not include connection details."
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.2) {
                isHandlingScan = false
            }
        }
    }

    @MainActor
    private func updateCameraAccess() async {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            cameraAccess = .allowed
        case .denied, .restricted:
            cameraAccess = .denied
        case .notDetermined:
            let granted = await withCheckedContinuation { continuation in
                AVCaptureDevice.requestAccess(for: .video) { allowed in
                    continuation.resume(returning: allowed)
                }
            }
            cameraAccess = granted ? .allowed : .denied
        @unknown default:
            cameraAccess = .denied
        }
    }
}

private enum CameraAccess {
    case unknown
    case denied
    case allowed
}

private struct QRCodeScannerPreview: UIViewRepresentable {
    let isActive: Bool
    let onScan: (String) -> Void

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        view.videoPreviewLayer.videoGravity = .resizeAspectFill
        view.videoPreviewLayer.session = context.coordinator.session
        return view
    }

    func updateUIView(_ uiView: PreviewView, context: Context) {
        if isActive {
            context.coordinator.start()
        } else {
            context.coordinator.stop()
        }
    }

    static func dismantleUIView(_ uiView: PreviewView, coordinator: Coordinator) {
        coordinator.stop()
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(onScan: onScan)
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        let session = AVCaptureSession()
        private let onScan: (String) -> Void
        private var isConfigured = false

        init(onScan: @escaping (String) -> Void) {
            self.onScan = onScan
        }

        func start() {
            configureIfNeeded()
            guard !session.isRunning else { return }
            DispatchQueue.global(qos: .userInitiated).async {
                self.session.startRunning()
            }
        }

        func stop() {
            guard session.isRunning else { return }
            DispatchQueue.global(qos: .userInitiated).async {
                self.session.stopRunning()
            }
        }

        func metadataOutput(
            _ output: AVCaptureMetadataOutput,
            didOutput metadataObjects: [AVMetadataObject],
            from connection: AVCaptureConnection
        ) {
            guard let object = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
                  object.type == .qr,
                  let value = object.stringValue else {
                return
            }
            onScan(value)
        }

        private func configureIfNeeded() {
            guard !isConfigured else { return }
            isConfigured = true

            session.beginConfiguration()
            session.sessionPreset = .high

            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input) else {
                session.commitConfiguration()
                return
            }
            session.addInput(input)

            let metadataOutput = AVCaptureMetadataOutput()
            if session.canAddOutput(metadataOutput) {
                session.addOutput(metadataOutput)
                metadataOutput.setMetadataObjectsDelegate(self, queue: .main)
                if metadataOutput.availableMetadataObjectTypes.contains(.qr) {
                    metadataOutput.metadataObjectTypes = [.qr]
                }
            }

            session.commitConfiguration()
        }
    }
}

private final class PreviewView: UIView {
    override class var layerClass: AnyClass {
        AVCaptureVideoPreviewLayer.self
    }

    var videoPreviewLayer: AVCaptureVideoPreviewLayer {
        layer as! AVCaptureVideoPreviewLayer
    }
}

private func parseConnectionPayload(_ payload: String) -> (ConnectionQRResult?, String?) {
    if let jsonResult = parseConnectionJSON(payload) {
        switch jsonResult {
        case .success(let result):
            return (result, nil)
        case .unsupported(let message):
            return (nil, message)
        }
    }

    if let urlResult = parseConnectionURL(payload) {
        return (urlResult, nil)
    }

    return (nil, nil)
}

private enum ConnectionJSONParseResult {
    case success(ConnectionQRResult)
    case unsupported(String)
}

private func parseConnectionJSON(_ payload: String) -> ConnectionJSONParseResult? {
    guard let data = payload.data(using: .utf8),
          let json = try? JSONSerialization.jsonObject(with: data),
          let dict = json as? [String: Any] else {
        return nil
    }

    if let type = dict["type"] as? String, type == "context_mobile_e2ee" {
        return .unsupported("Secure pairing QR codes are not supported yet.")
    }

    let profile = dict["connection_profile"] as? [String: Any] ?? dict
    let connection = profile["connection"] as? [String: Any] ?? [:]
    let auth = profile["auth"] as? [String: Any] ?? [:]

    let baseURL = stringValue(
        connection["base_url"],
        connection["baseUrl"],
        connection["url"],
        connection["baseURL"],
        profile["base_url"],
        profile["baseUrl"],
        profile["baseURL"]
    )

    let token = stringValue(
        auth["api_token"],
        auth["token"],
        profile["api_token"],
        profile["token"]
    )

    guard let baseURL, let token else {
        return nil
    }

    return .success(ConnectionQRResult(baseURL: normalizeBaseURL(baseURL), token: token))
}

private func parseConnectionURL(_ payload: String) -> ConnectionQRResult? {
    guard let url = URL(string: payload),
          let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
        return nil
    }

    let tokenKeys = ["token", "api_token", "access_token"]
    let token = components.queryItems?.first(where: { tokenKeys.contains($0.name.lowercased()) })?.value

    guard let token, !token.isEmpty else {
        return nil
    }

    var baseComponents = components
    baseComponents.query = nil
    baseComponents.fragment = nil

    guard let baseURL = baseComponents.url?.absoluteString else {
        return nil
    }

    return ConnectionQRResult(baseURL: normalizeBaseURL(baseURL), token: token)
}

private func normalizeBaseURL(_ value: String) -> String {
    var trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
    while trimmed.hasSuffix("/") {
        trimmed.removeLast()
    }
    return trimmed
}

private func stringValue(_ values: Any?...) -> String? {
    for value in values {
        if let string = value as? String, !string.isEmpty {
            return string
        }
        if let number = value as? NSNumber {
            return number.stringValue
        }
    }
    return nil
}

#Preview {
    QRCodeScannerView { _ in }
}
