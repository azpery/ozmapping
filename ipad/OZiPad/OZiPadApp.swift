import SwiftUI

/// App iPad d'OZ Mapping : dessin, photos, chimie, commandes du spectacle et médias, en natif.
/// Elle parle à OZ Mapping sur l'ordinateur comme la page web (même protocole), et lit
/// directement l'inclinaison, les secousses et le souffle.
@main
struct OZiPadApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
                .statusBarHidden(true)
                .persistentSystemOverlays(.hidden)
                .preferredColorScheme(.dark)
                .onAppear { UIApplication.shared.isIdleTimerDisabled = true }
        }
    }
}

/// Adresse mémorisée → l'app ; sinon (ou à la demande) l'écran de connexion.
struct RootView: View {
    @AppStorage("address") private var address: String = ""
    @State private var connecting = false

    var body: some View {
        // Lancement de test (simulateur) : l'éditeur de détourage directement.
        if ProcessInfo.processInfo.arguments.contains("-oz-cut-test") {
            CutEditorView(object: Self.testPhoto) { _ in }
                .environmentObject(Link(url: URL(string: "https://127.0.0.1:9")!))
        } else if let url = Address.url(from: address), !connecting {
            SessionView(url: url, onChangeComputer: { connecting = true })
                .id(url)
        } else {
            ConnectView(address: $address, onConnect: { connecting = false })
                .ignoresSafeArea()
        }
    }
}

extension RootView {
    static var testPhoto: UIImage {
        UIGraphicsImageRenderer(size: CGSize(width: 1200, height: 900)).image { c in
            UIColor.darkGray.setFill()
            c.fill(CGRect(x: 0, y: 0, width: 1200, height: 900))
            UIColor.systemRed.setFill()
            c.cgContext.fillEllipse(in: CGRect(x: 400, y: 250, width: 400, height: 400))
        }
    }
}

enum Address {
    /// "192.168.1.20", "192.168.1.20:47823", "https://…" → l'adresse d'OZ Mapping.
    static func url(from text: String) -> URL? {
        var s = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.isEmpty { return nil }
        if !s.contains("://") {
            s = "https://" + s
        }
        guard var c = URLComponents(string: s), c.host != nil else { return nil }
        if c.port == nil { c.port = 47823 }
        c.path = "/"
        c.query = nil
        return c.url
    }

    /// Adresse privée (réseau local) ou cet appareil.
    static func isLocal(_ host: String) -> Bool {
        if host == "localhost" || host.hasSuffix(".local") { return true }
        let p = host.split(separator: ".").compactMap { Int($0) }
        guard p.count == 4 else { return false }
        return p[0] == 10 || p[0] == 127 || (p[0] == 192 && p[1] == 168) || (p[0] == 172 && (16...31).contains(p[1])) || (p[0] == 169 && p[1] == 254)
    }
}
