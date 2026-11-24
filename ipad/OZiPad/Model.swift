import Foundation
import SwiftUI

enum Tab: String, CaseIterable, Identifiable {
    case draw, photos, chem, live, media, room
    var id: String { rawValue }
    var title: String {
        switch self {
        case .draw: return "Dessin"
        case .photos: return "Photos"
        case .chem: return "Chimie"
        case .live: return "Live"
        case .media: return "Médias"
        case .room: return "Salle 3D"
        }
    }
}

/// Réglages du dessin (gardés d'une fois sur l'autre).
struct DrawTool: Codable, Equatable {
    var tool = "free"
    var color = "#2ee6ff"
    var brush = "neon"
    var sym = 1
    var anim = "none"
    var life = 0
    var out = "fade"
    var width = 30.0
    var show = "drawing"

    /// Épaisseur du trait, en hauteur de l'image.
    var strokeWidth: Double { 0.002 + pow(width / 100, 1.6) * 0.09 }
    var eraserSize: Double { 0.015 + width / 100 * 0.08 }
}

/// Réglages de la chimie.
struct ChemTool: Codable, Equatable {
    var ing = "ink"
    var base = "dark"
    var color = "#2ee6ff"
    var size = 35.0
    var renew = 0.0
    var boltBeat = false
    var flip = false
    var motion = true
    var mic = false
    /// Sensibilité au son (0..100) ; absente des réglages gardés par une version précédente.
    var sound: Double? = 50
    /// Son qui fait vibrer le bassin : "app" (l'ordinateur), "ipad" (le micro), "off".
    var listen: String? = "app"
    var listens: String { listen ?? "app" }

    /// Taille d'une goutte (part de la hauteur de l'image) ; le stylet appuyé verse plus.
    func radius(_ p: Double) -> Double { (0.012 + pow(size / 100, 1.5) * 0.16) * (0.6 + 0.8 * p) }
}

/// Choix proposés (les mêmes que la page web).
enum Opt {
    static let tool = [("free", "Main levée"), ("line", "Ligne"), ("rect", "Rectangle"), ("circle", "Cercle"), ("erase", "Gomme")]
    static let toolIcon = ["free": "scribble", "line": "line.diagonal", "rect": "rectangle", "circle": "circle", "erase": "eraser"]
    static let colors = ["#ffffff", "#ff3b6b", "#ff9f1c", "#ffe14d", "#4dff88", "#2ee6ff", "#4d7cff", "#b84dff"]
    static let brush = [("line", "Trait"), ("neon", "Néon"), ("rainbow", "Arc-en-ciel"), ("sparkle", "Étincelles"),
                        ("spray", "Aérographe"), ("dots", "Pointillés")]
    static let life = [(0, "Jamais"), (1, "1 s"), (3, "3 s"), (8, "8 s"), (20, "20 s")]
    static let out = [("fade", "Fondu"), ("shrink", "Rétrécit"), ("tail", "Se rembobine"), ("rise", "S'envole"),
                      ("burst", "Explose"), ("melt", "Coule"), ("trail", "Comète")]
    static let anim = [("none", "Fixe"), ("pulse", "Au tempo"), ("blink", "Clignote"), ("wave", "Ondule"),
                       ("spin", "Tourne"), ("float", "Flotte"), ("trace", "Se dessine")]
    static let sym = [(1, "Aucune"), (2, "Miroir"), (4, "×4"), (6, "Kaléido 6"), (8, "Kaléido 8")]
    static let show = [("drawing", "Le dessin seul"), ("screen", "Toute la sortie")]

    static let stickerAnim = [("none", "Fixe"), ("pulse", "Au tempo"), ("spin", "Tourne"), ("float", "Flotte"),
                              ("wave", "Danse"), ("blink", "Clignote")]

    static let chemFamilies: [(String, [(String, String)])] = [
        ("Verser", [("water", "Eau"), ("milk", "Lait"), ("oil", "Huile"), ("gasoline", "Essence")]),
        ("Couleurs", [("ink", "Encre"), ("coloring", "Colorant")]),
        ("Peintures", [("acrylic", "Acrylique"), ("watercolor", "Aquarelle"), ("gouache", "Gouache"),
                       ("fluo", "Fluo"), ("metallic", "Métallisée"), ("marbling", "Marbrure")]),
        ("Réactifs", [("soap", "Savon"), ("alcohol", "Alcool"), ("stir", "Touiller")]),
        ("Électricité", [("lightning", "Foudre")]),
    ]
    static let chemIcon = [
        "water": "drop.fill", "milk": "cup.and.saucer.fill", "oil": "circle.hexagongrid.fill", "gasoline": "flame.fill",
        "ink": "drop.triangle.fill", "coloring": "eyedropper.full", "acrylic": "paintbrush.fill",
        "watercolor": "paintbrush.pointed.fill", "gouache": "paintpalette.fill", "fluo": "sun.max.fill",
        "metallic": "sparkles", "marbling": "hurricane", "soap": "bubble.left.and.bubble.right.fill",
        "alcohol": "testtube.2", "stir": "arrow.triangle.2.circlepath", "lightning": "bolt.fill",
    ]
    static let chemBase = [("dark", "Eau, dans le noir"), ("lit", "Eau éclairée"), ("milk", "Lait"),
                           ("oil", "Huile de lin"), ("bubble", "Bulle de savon")]
    static let chemBaseIcon = ["dark": "moon.fill", "lit": "lightbulb.fill", "milk": "cup.and.saucer.fill",
                               "oil": "drop.circle.fill", "bubble": "circle.circle"]
    static let chemColors = ["#ff2d55", "#ff9f0a", "#ffd60a", "#30d158", "#2ee6ff", "#0a84ff", "#bf5af2", "#ffffff"]

    static func label<V: Equatable>(_ options: [(V, String)], _ v: V) -> String {
        options.first { $0.0 == v }?.1 ?? ""
    }
}

/// L'app : onglet choisi, réglages, photos du dessin, capteurs. La liaison avec l'ordinateur est
/// dans `Link`.
final class Model: ObservableObject {
    let link: Link
    let sensors = Sensors()

    /// Onglet choisi (le dernier utilisé au lancement).
    @Published var tab: Tab = Tab(rawValue: UserDefaults.standard.string(forKey: "tab") ?? "") ?? .draw {
        didSet {
            UserDefaults.standard.set(tab.rawValue, forKey: "tab")
            if tab != oldValue { tabChanged(from: oldValue) }
        }
    }
    @Published var draw: DrawTool {
        didSet {
            save(draw, "oz-dessin")
            if draw.show != oldValue.show { sendView() }
        }
    }
    @Published var chem: ChemTool {
        didSet { save(chem, "oz-chimie") }
    }
    /// Toile en grand (panneau de réglages caché).
    @Published var full = false
    @Published var motionOn = false
    @Published var micOn = false
    @Published var sensorMsg = ""

    /// Photos du dessin : la liste n'est pas publiée (la toile la modifie pendant un geste,
    /// 60 fois par seconde) ; `stickersRev` change quand elle arrive de l'ordinateur.
    var stickers: [StickerInfo] = []
    @Published private(set) var stickersRev = 0
    @Published var selSticker: Int?
    /// Photo en cours de geste sur la toile : sa position locale l'emporte sur celle reçue.
    var grabId: Int?
    /// Une photo vient d'être envoyée : la choisir dès qu'elle arrive.
    var pendingSelect: Int?

    private var canvasWidth = 1024
    /// On a versé de la chimie : la toile montre alors toute la sortie, même en dessinant
    /// (sinon le bassin disparaîtrait de l'iPad, alors qu'il est projeté sous le dessin).
    @Published var chemUsed = false {
        didSet { if chemUsed != oldValue { sendView() } }
    }
    private var lastMotion = Date.distantPast
    private var lastSound = Date.distantPast

    init(url: URL) {
        link = Link(url: url)
        draw = Self.load("oz-dessin") ?? DrawTool()
        chem = Self.load("oz-chimie") ?? ChemTool()
        link.onConnect = { [weak self] in self?.chemSync() }
        link.onStickers = { [weak self] list in self?.applyStickers(list) }
        sensors.onMotion = { [weak self] gx, gy, shake in self?.motion(gx, gy, shake) }
        sensors.onBlow = { [weak self] level in
            guard let self, self.tab == .chem, self.chem.mic else { return }
            self.link.chem("blow", ["amount": level])
        }
        sensors.onSound = { [weak self] level, bass, high in
            guard let self, self.chem.listens == "ipad" else { return }
            let now = Date()
            guard now.timeIntervalSince(self.lastSound) >= 0.033 else { return }
            self.lastSound = now
            self.link.chem("hear", ["level": level, "bass": bass, "high": high])
        }
        sensors.onStatus = { [weak self] what, ok in
            guard let self else { return }
            if what == "motion" {
                self.motionOn = ok
                if !ok { self.sensorMsg = "Capteurs de mouvement indisponibles." }
            } else {
                self.micOn = ok
                if !ok {
                    self.chem.mic = false
                    self.chem.listen = "app"
                    self.sensorMsg = "Micro refusé (Réglages › OZ Mapping › Micro)."
                }
            }
        }
    }

    func start() {
        link.start()
        if tab == .chem { startSensors() }
        if chem.listens == "ipad" { sensors.startMic() }
    }

    func stop() {
        link.stop()
        sensors.stopAll()
        motionOn = false
        micOn = false
    }

    // ------------------------------------------------------------------ onglets, aperçu

    private func tabChanged(from old: Tab) {
        sendView()
        if tab == .chem { startSensors() } else if old == .chem { stopSensors() }
        // Le bassin écoute l'iPad : le micro reste ouvert dans les autres onglets.
        if chem.listens == "ipad" { sensors.startMic() }
    }

    func setCanvasWidth(_ w: Int) {
        guard w > 0, w != canvasWidth else { return }
        canvasWidth = w
        sendView()
    }

    /// Ce que montre la toile : le dessin seul (léger) ou toute la sortie (effets, visuels…).
    /// Onglet Chimie : toujours toute la sortie (le bassin est un visuel de l'app).
    func sendView() {
        link.setView(w: canvasWidth, screen: draw.show == "screen" || tab == .chem || chemUsed)
    }

    // ------------------------------------------------------------------ photos du dessin

    private func applyStickers(_ list: [StickerInfo]) {
        var list = list
        if let g = grabId, let old = stickers.first(where: { $0.id == g }), let k = list.firstIndex(where: { $0.id == g }) {
            (list[k].x, list[k].y, list[k].s, list[k].r) = (old.x, old.y, old.s, old.r)
        }
        if let n = pendingSelect, list.count > n {
            selSticker = list.last?.id
            pendingSelect = nil
        }
        stickers = list
        if let s = selSticker, !list.contains(where: { $0.id == s }) { selSticker = nil }
        stickersRev += 1
    }

    var selected: StickerInfo? { stickers.first { $0.id == selSticker } }

    /// Déplace / tourne / agrandit la photo choisie (et l'envoie).
    func moveSelected(_ change: (inout StickerInfo) -> Void) {
        guard let k = stickers.firstIndex(where: { $0.id == selSticker }) else { return }
        change(&stickers[k])
        let s = stickers[k]
        link.send(["t": "stmove", "id": s.id, "x": s.x, "y": s.y, "s": s.s, "r": s.r])
        stickersRev += 1
    }

    // ------------------------------------------------------------------ chimie, capteurs

    /// Fond, renouvellement, foudre au rythme : renvoyés à chaque connexion (l'app a pu redémarrer).
    func chemSync() {
        link.chem("base", ["base": chem.base])
        link.chem("renew", ["rate": chem.renew / 100])
        link.chem("boltbeat", ["on": chem.boltBeat])
        link.chem("sound", ["amount": (chem.sound ?? 50) / 100])
        link.chem("listen", ["from": chem.listens])
    }

    /// Le bassin vibre avec le son de l'ordinateur, celui du micro de l'iPad, ou aucun.
    func setListen(_ from: String) {
        chem.listen = from
        link.chem("listen", ["from": from])
        if from == "ipad" { sensors.startMic() } else if !chem.mic { sensors.stopMic(); micOn = false }
    }

    func startSensors() {
        if chem.motion { sensors.startMotion() }
        if chem.mic { sensors.startMic() }
    }

    func stopSensors() {
        sensors.stopMotion()
        motionOn = false
        if chem.listens != "ipad" {
            sensors.stopMic()
            micOn = false
        }
    }

    func setMotion(_ on: Bool) {
        chem.motion = on
        if on { sensors.startMotion() } else { sensors.stopMotion(); motionOn = false }
    }

    func setMic(_ on: Bool) {
        chem.mic = on
        if on { sensors.startMic() } else if chem.listens != "ipad" { sensors.stopMic(); micOn = false }
    }

    private func motion(_ gx: Double, _ gy: Double, _ shake: Double) {
        guard tab == .chem else { return }
        let now = Date()
        guard now.timeIntervalSince(lastMotion) >= 0.06 else { return }
        lastMotion = now
        let k = chem.flip ? -1.0 : 1.0
        link.chem("motion", ["gx": gx * k, "gy": gy * k, "shake": shake])
    }

    // ------------------------------------------------------------------ réglages gardés

    private func save<T: Encodable>(_ v: T, _ key: String) {
        if let d = try? JSONEncoder().encode(v) { UserDefaults.standard.set(d, forKey: key) }
    }

    private static func load<T: Decodable>(_ key: String) -> T? {
        guard let d = UserDefaults.standard.data(forKey: key) else { return nil }
        return try? JSONDecoder().decode(T.self, from: d)
    }
}
