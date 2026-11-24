import SwiftUI
import AVFoundation

/// Écran de connexion : scanner le QR code de l'onglet Dessin d'OZ Mapping, ou taper l'adresse.
struct ConnectView: View {
    @Binding var address: String
    var onConnect: () -> Void
    @State private var typed: String = ""
    @State private var scanning = false
    @State private var error: String?

    var body: some View {
        VStack(spacing: 22) {
            Spacer()
            Text("OZ Mapping").font(.system(size: 40, weight: .bold))
            Text("Sur l'ordinateur : onglet Dessin, un QR code s'affiche.\nScannez-le, ou tapez l'adresse indiquée dessous.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
            Button {
                scanning = true
            } label: {
                Label("Scanner le QR code", systemImage: "qrcode.viewfinder")
                    .font(.title3.weight(.semibold))
                    .frame(maxWidth: 360, minHeight: 54)
            }
            .buttonStyle(.borderedProminent)
            .tint(Color(red: 0.35, green: 0.82, blue: 0.6))
            HStack {
                TextField("Adresse, ex. 192.168.1.20:47823", text: $typed)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 360)
                    .onSubmit(connect)
                Button("Se connecter", action: connect).buttonStyle(.bordered)
            }
            if let error { Text(error).foregroundStyle(.red) }
            Spacer()
            Text("L'iPad et l'ordinateur doivent être sur le même Wi-Fi.").font(.footnote).foregroundStyle(.secondary)
        }
        .padding(30)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(red: 0.04, green: 0.05, blue: 0.07))
        .onAppear { typed = address }
        .sheet(isPresented: $scanning) {
            QRScanner { code in
                scanning = false
                typed = code
                connect()
            }
            .ignoresSafeArea()
        }
    }

    private func connect() {
        guard Address.url(from: typed) != nil else {
            error = "Adresse illisible"
            return
        }
        error = nil
        address = typed
        onConnect()
    }
}

/// Lecteur de QR code (caméra arrière).
struct QRScanner: UIViewControllerRepresentable {
    var found: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerController {
        let c = ScannerController()
        c.found = found
        return c
    }

    func updateUIViewController(_ c: ScannerController, context: Context) {}
}

final class ScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var found: ((String) -> Void)?
    private let session = AVCaptureSession()
    private var preview: AVCaptureVideoPreviewLayer?
    private var done = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device),
              session.canAddInput(input) else {
            let label = UILabel()
            label.text = "Caméra indisponible : tapez l'adresse."
            label.textColor = .white
            label.frame = view.bounds
            label.textAlignment = .center
            view.addSubview(label)
            return
        }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        if session.canAddOutput(output) {
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            output.metadataObjectTypes = [.qr]
        }
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        view.layer.addSublayer(layer)
        preview = layer
        DispatchQueue.global(qos: .userInitiated).async { [session] in session.startRunning() }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
        if let conn = preview?.connection, conn.isVideoOrientationSupported {
            switch view.window?.windowScene?.interfaceOrientation {
            case .landscapeLeft: conn.videoOrientation = .landscapeLeft
            case .landscapeRight: conn.videoOrientation = .landscapeRight
            case .portraitUpsideDown: conn.videoOrientation = .portraitUpsideDown
            default: conn.videoOrientation = .portrait
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        session.stopRunning()
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard !done,
              let code = (objects.first as? AVMetadataMachineReadableCodeObject)?.stringValue else { return }
        done = true
        found?(code)
    }
}
