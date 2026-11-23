import AVFoundation
import SwiftUI
import UIKit
#if DEBUG && IRIS_DRIVE_QR_FRAME_TEST
import CryptoKit
import ImageIO
import Vision
#endif

struct QRCodeScannerSheet: View {
    let onCode: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var error = ""

    var body: some View {
        NavigationStack {
            ZStack(alignment: .bottom) {
                QRCodeScannerView(
                    onCode: { code in
                        onCode(code)
                        dismiss()
                    },
                    onError: { error = $0 }
                )
                .ignoresSafeArea()

                if !error.isEmpty {
                    Text(error)
                        .font(.footnote)
                        .foregroundStyle(.white)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(.black.opacity(0.72), in: Capsule())
                        .padding(.bottom, 18)
                }
            }
            .navigationTitle("Scan QR")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close") {
                        dismiss()
                    }
                }
            }
        }
    }
}

private struct QRCodeScannerView: UIViewRepresentable {
    let onCode: (String) -> Void
    let onError: (String) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onCode: onCode, onError: onError)
    }

    func makeUIView(context: Context) -> ScannerPreviewView {
        let view = ScannerPreviewView()
        view.isAccessibilityElement = true
        view.accessibilityLabel = "QR scanner camera"
        view.accessibilityIdentifier = "qrScannerCamera"
        context.coordinator.configure(in: view)
        return view
    }

    func updateUIView(_ view: ScannerPreviewView, context: Context) {
        context.coordinator.attachPreview(to: view)
    }

    static func dismantleUIView(_ uiView: ScannerPreviewView, coordinator: Coordinator) {
        coordinator.stop()
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        private let onCode: (String) -> Void
        private let onError: (String) -> Void
        private let sessionQueue = DispatchQueue(label: "to.iris.drive.qrscanner")
        private var session: AVCaptureSession?
        private var didConfigure = false
        private var didFinish = false

        init(onCode: @escaping (String) -> Void, onError: @escaping (String) -> Void) {
            self.onCode = onCode
            self.onError = onError
        }

        func configure(in view: ScannerPreviewView) {
            guard !didConfigure else {
                attachPreview(to: view)
                return
            }
            didConfigure = true

            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .authorized:
                configureSession(in: view)
            case .notDetermined:
                AVCaptureDevice.requestAccess(for: .video) { [weak self, weak view] granted in
                    guard let self, let view else { return }
                    DispatchQueue.main.async {
                        granted ? self.configureSession(in: view) : self.onError("Camera access denied.")
                    }
                }
            default:
                onError("Camera access denied.")
            }
        }

        func attachPreview(to view: ScannerPreviewView) {
            view.previewLayer.session = session
            view.previewLayer.videoGravity = .resizeAspectFill
        }

        func stop() {
            sessionQueue.async { [session] in
                if session?.isRunning == true {
                    session?.stopRunning()
                }
            }
        }

        private func configureSession(in view: ScannerPreviewView) {
            let session = AVCaptureSession()
            guard let device = AVCaptureDevice.default(for: .video),
                  let input = try? AVCaptureDeviceInput(device: device),
                  session.canAddInput(input)
            else {
                onError("Camera scanner unavailable.")
                return
            }
            session.addInput(input)

            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else {
                onError("Camera scanner unavailable.")
                return
            }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: sessionQueue)
            output.metadataObjectTypes = [.qr]

            self.session = session
            attachPreview(to: view)
            sessionQueue.async {
                session.startRunning()
                #if DEBUG && IRIS_DRIVE_QR_FRAME_TEST
                if let digest = ProcessInfo.processInfo.environment["IRIS_DRIVE_QR_FRAME_SHA256"] {
                    self.readTestFrame(digest, until: .now() + 45)
                }
                #endif
            }
        }

        #if DEBUG && IRIS_DRIVE_QR_FRAME_TEST
        private func readTestFrame(_ digest: String, until deadline: DispatchTime) {
            guard session?.isRunning == true, !didFinish else { return }
            guard digest.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil else { return }
            let file = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                .appendingPathComponent("qr-frame-\(digest).png")
            guard DispatchTime.now() < deadline else {
                try? FileManager.default.removeItem(at: file)
                DispatchQueue.main.async { self.onError("Test QR frame timed out.") }
                return
            }
            if let size = try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize,
               (1...4_194_304).contains(size), let data = try? Data(contentsOf: file),
               SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == digest {
                defer { try? FileManager.default.removeItem(at: file) }
                guard let image = CGImageSourceCreateWithData(data as CFData, nil),
                      let properties = CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any],
                      let width = properties[kCGImagePropertyPixelWidth] as? Int,
                      let height = properties[kCGImagePropertyPixelHeight] as? Int,
                      (1...4096).contains(width), (1...4096).contains(height) else { return }
                let request = VNDetectBarcodesRequest()
                request.symbologies = [.qr]
                do {
                    try VNImageRequestHandler(data: data).perform([request])
                    if let codes = request.results, codes.count == 1 {
                        finish(codes[0].payloadStringValue)
                    }
                } catch {
                    DispatchQueue.main.async { self.onError("Test QR frame could not be read.") }
                }
                return
            }
            sessionQueue.asyncAfter(deadline: .now() + .milliseconds(100)) { [weak self] in
                self?.readTestFrame(digest, until: deadline)
            }
        }
        #endif

        func metadataOutput(
            _ output: AVCaptureMetadataOutput,
            didOutput metadataObjects: [AVMetadataObject],
            from connection: AVCaptureConnection
        ) {
            #if DEBUG && IRIS_DRIVE_QR_FRAME_TEST
            if ProcessInfo.processInfo.environment["IRIS_DRIVE_QR_FRAME_SHA256"] != nil { return }
            #endif
            finish(metadataObjects
                .compactMap({ $0 as? AVMetadataMachineReadableCodeObject })
                .first(where: { $0.type == .qr })?.stringValue)
        }

        private func finish(_ value: String?) {
            guard !didFinish, let code = value?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !code.isEmpty else { return }
            didFinish = true
            session?.stopRunning()
            DispatchQueue.main.async {
                self.onCode(code)
            }
        }
    }
}

private final class ScannerPreviewView: UIView {
    override class var layerClass: AnyClass {
        AVCaptureVideoPreviewLayer.self
    }

    var previewLayer: AVCaptureVideoPreviewLayer {
        layer as! AVCaptureVideoPreviewLayer
    }
}
