import Foundation
import ImageIO
import UIKit

typealias JSON = [String: Any]

/// Clé de comparaison d'une valeur JSON (identifiant de visuel, de scénario…).
func jsonKey(_ v: Any?) -> String {
    guard let v, !(v is NSNull) else { return "" }
    if let s = v as? String { return s }
    if let d = try? JSONSerialization.data(withJSONObject: v, options: [.fragmentsAllowed, .sortedKeys]) {
        return String(decoding: d, as: UTF8.self)
    }
    return "\(v)"
}

private func num(_ j: JSON, _ k: String, _ def: Double = 0) -> Double { (j[k] as? NSNumber)?.doubleValue ?? def }
private func flag(_ j: JSON, _ k: String) -> Bool { (j[k] as? NSNumber)?.boolValue ?? false }
private func str(_ j: JSON, _ k: String) -> String { j[k] as? String ?? "" }
private func list(_ j: JSON, _ k: String) -> [JSON] { j[k] as? [JSON] ?? [] }

/// Commandes de l'onglet Live : les mêmes que celles d'un contrôleur MIDI (voir midi.rs).
enum A {
    static func col(_ c: Int) -> Any { ["PlayColumn": ["column": c]] }
    static func layer(_ l: Int) -> Any { ["SelectLayer": ["layer": l]] }
    static func vis(_ l: Int) -> Any { ["LayerVisible": ["layer": l]] }
    static func opacity(_ l: Int) -> Any { ["Opacity": ["layer": l]] }
    static func preset(_ n: String) -> Any { ["Preset": ["name": n]] }
    static func media(_ i: Int) -> Any { ["LaunchMedia": ["index": i]] }
    static func visual(_ p: Any) -> Any { ["Visual": ["pattern": p]] }
    static func fx(_ k: Int) -> Any { ["Effect": ["param": k]] }
    static func scen(_ k: Any) -> Any { ["LedScenario": ["kind": k]] }
    static func hold(_ id: Int) -> Any { ["DmxHold": ["fixture": id]] }
    static func tog(_ id: Int) -> Any { ["DmxToggle": ["fixture": id]] }
    static func cue(_ i: Int) -> Any { ["LaserCue": ["index": i]] }
    static func mode(_ m: String) -> Any { ["Mode": ["mode": m]] }
    static func output(_ k: Int) -> Any { ["IpadOutput": ["output": k]] }
    static func groupVis(_ g: Int) -> Any { ["GroupVisible": ["group": g]] }
    static func groupFx(_ g: Int, _ fx: String) -> Any { ["GroupFx": ["group": g, "fx": fx]] }
}

/// État de l'app de l'ordinateur pour l'onglet Live (message "live").
struct LiveState {
    struct Layer { var name: String; var opacity: Double; var visible: Bool; var sketch: Bool; var group: Int }
    struct Group { var id: Int; var name: String; var visible: Bool; var fx: String }
    struct Column { var name: String; var filled: Bool; var active: Bool }
    struct Item { var id: Any; var key: String; var name: String; var family: String }
    struct Param { var name: String; var v: Double }
    struct Media { var name: String; var photo: Bool; var folder: String }
    struct Dmx { var id: Int; var name: String; var kind: String; var on: Bool }

    var master, bpm, phase, visualMix, speed, ledBrightness: Double
    var blackout, white, bypass, grid, recording, chain, auto, mix, react, paused, reverse: Bool
    var sel, content: Int
    var playing: Int?
    var layers: [Layer]
    var groups: [Group]
    var columns: [Column]
    var presets: [Item]
    var preset: String
    var visual: String
    var visuals: [Item]
    var media: [Media]
    var fx: [Param]
    var ledsHas, ledLive, ledChain: Bool
    var ledScenario: String
    var scenarios: [Item]
    var dmx: [Dmx]
    var lasersHas, armed: Bool
    var cues: [String]
    /// Mode de la sortie ("All", "Drawing", "Chemistry", "Media", "Visual") et genres présents.
    var mode: String
    var kinds: Set<String>
    /// Sorties de l'app, et celle de l'iPad.
    var outputs: [String]
    var output: Int
    let at = Date()

    init(_ j: JSON) {
        master = num(j, "master", 1); bpm = num(j, "bpm", 120); phase = num(j, "phase")
        visualMix = num(j, "visual_mix"); speed = num(j, "speed", 1)
        blackout = flag(j, "blackout"); white = flag(j, "white"); bypass = flag(j, "bypass"); grid = flag(j, "grid")
        recording = flag(j, "recording"); chain = flag(j, "chain"); auto = flag(j, "auto"); mix = flag(j, "mix")
        react = flag(j, "react"); paused = flag(j, "paused"); reverse = flag(j, "reverse")
        sel = Int(num(j, "sel")); content = Int(num(j, "content"))
        playing = (j["playing"] as? NSNumber)?.intValue
        layers = list(j, "layers").map { Layer(name: str($0, "name"), opacity: num($0, "opacity", 1), visible: flag($0, "visible"), sketch: flag($0, "sketch"), group: Int(num($0, "group"))) }
        groups = list(j, "groups").map { Group(id: Int(num($0, "id")), name: str($0, "name"), visible: flag($0, "visible"), fx: str($0, "fx")) }
        columns = list(j, "columns").map { Column(name: str($0, "name"), filled: flag($0, "filled"), active: flag($0, "active")) }
        presets = list(j, "presets").map { Item(id: str($0, "name"), key: str($0, "name"), name: str($0, "name"), family: str($0, "family")) }
        preset = j["preset"] as? String ?? ""
        visual = jsonKey(j["visual"])
        visuals = list(j, "visuals").map { Item(id: $0["id"] ?? NSNull(), key: jsonKey($0["id"]), name: str($0, "name"), family: str($0, "family")) }
        media = list(j, "media").map { Media(name: str($0, "name"), photo: flag($0, "photo"), folder: str($0, "folder")) }
        fx = list(j, "fx").map { Param(name: str($0, "name"), v: num($0, "v")) }
        let leds = j["leds"] as? JSON ?? [:]
        ledsHas = flag(leds, "has"); ledLive = flag(leds, "live"); ledChain = flag(leds, "chain")
        ledBrightness = num(leds, "brightness", 1); ledScenario = jsonKey(leds["scenario"])
        scenarios = list(leds, "scenarios").map { Item(id: $0["id"] ?? NSNull(), key: jsonKey($0["id"]), name: str($0, "name"), family: "") }
        dmx = list(j, "dmx").map { Dmx(id: Int(num($0, "id")), name: str($0, "name"), kind: jsonKey($0["kind"]), on: flag($0, "on")) }
        let lasers = j["lasers"] as? JSON ?? [:]
        lasersHas = flag(lasers, "has"); armed = flag(lasers, "armed")
        cues = (lasers["cues"] as? [Any] ?? []).map { $0 as? String ?? "" }
        mode = j["mode"] as? String ?? "All"
        kinds = Set((j["kinds"] as? [Any] ?? []).compactMap { $0 as? String })
        outputs = (j["outputs"] as? [Any] ?? []).map { $0 as? String ?? "" }
        output = Int(num(j, "output"))
    }

    /// Phase du temps (0..1) maintenant, d'après la dernière phase reçue et le tempo.
    func phaseNow(_ now: Date = Date()) -> Double {
        (phase + now.timeIntervalSince(at) * bpm / 60).truncatingRemainder(dividingBy: 1)
    }
}

/// Photo posée dans le dessin.
struct StickerInfo: Equatable {
    var id: Int
    var x, y, s, r, ar: Double
    var anim: String
    var outline: Bool
    var sym: Int
    var flip: Bool

    init(_ j: JSON) {
        id = Int(num(j, "id")); x = num(j, "x", 0.5); y = num(j, "y", 0.5); s = num(j, "s", 0.4); r = num(j, "r")
        ar = num(j, "ar", 1); anim = jsonKey(j["anim"]); outline = flag(j, "outline"); sym = Int(num(j, "sym", 1)); flip = flag(j, "flip")
    }
}

struct LinkError: LocalizedError {
    let text: String
    var errorDescription: String? { text }
}

/// Liaison avec OZ Mapping sur l'ordinateur : WebSocket (traits, commandes, aperçu de la sortie)
/// et requêtes https (photos, détourage, miniatures). Le certificat fabriqué par OZ Mapping
/// (auto-signé) est accepté pour les adresses du réseau local seulement.
final class Link: NSObject, ObservableObject, URLSessionDelegate, URLSessionWebSocketDelegate {
    @Published private(set) var connected = false
    @Published private(set) var status = "Connexion…"
    @Published private(set) var aspect: CGFloat = 16.0 / 9.0
    @Published private(set) var slots = [Bool](repeating: false, count: 8)
    @Published private(set) var live: LiveState?
    /// Salle 3D (scène des LEDs), quand l'onglet la demande.
    @Published private(set) var room: RoomState?
    /// Dernier message d'OZ Mapping (photo reçue, ajoutée, refusée).
    @Published private(set) var notice = ""

    /// Aperçu de la sortie (le plus récent), et combien on en a reçu.
    private(set) var preview: CGImage?
    private(set) var previews = 0

    var onConnect: (() -> Void)?
    var onStickers: (([StickerInfo]) -> Void)?
    var onFail: (() -> Void)?

    let base: URL
    private var session: URLSession!
    private var task: URLSessionWebSocketTask?
    private var gen = 0
    private var stopped = true
    private var everConnected = false
    private var failures = 0
    /// Dernière erreur de connexion (affichée tant qu'on n'est pas connecté).
    private var lastError = ""
    private var view: (w: Int, screen: Bool) = (1024, false)
    private var thumbs: [String: UIImage] = [:]
    private var noticeTimer: Timer?
    private let decodeQueue = DispatchQueue(label: "oz.preview", qos: .userInteractive)
    private var decoding = false
    private var pendingFrame: Data?

    init(url: URL) {
        base = url
        super.init()
        let config = URLSessionConfiguration.default
        config.waitsForConnectivity = false
        config.timeoutIntervalForRequest = 6
        session = URLSession(configuration: config, delegate: self, delegateQueue: .main)
    }

    // ------------------------------------------------------------------ connexion

    func start() {
        guard stopped else { return }
        stopped = false
        open()
    }

    func stop() {
        stopped = true
        gen += 1
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        connected = false
    }

    private func open() {
        guard !stopped else { return }
        gen += 1
        let g = gen
        var c = URLComponents(url: base, resolvingAgainstBaseURL: false)!
        c.scheme = base.scheme == "http" ? "ws" : "wss"
        c.path = "/ws"
        c.query = nil
        let t = session.webSocketTask(with: c.url!)
        t.maximumMessageSize = 32 << 20
        task = t
        t.resume()
        receive(t, g)
    }

    private func receive(_ t: URLSessionWebSocketTask, _ g: Int) {
        t.receive { [weak self] result in
            DispatchQueue.main.async {
                guard let self, g == self.gen else { return }
                switch result {
                case .failure(let e):
                    self.lastError = e.localizedDescription
                    print("OZ liaison :", e)
                    self.dropped(g)
                case .success(let m):
                    switch m {
                    case .string(let s): self.handle(s)
                    case .data(let d): self.frame(d)
                    @unknown default: break
                    }
                    self.receive(t, g)
                }
            }
        }
    }

    private func dropped(_ g: Int) {
        guard g == gen, !stopped else { return }
        gen += 1
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        if connected { connected = false } else { failures += 1 }
        status = (everConnected ? "Déconnecté, nouvelle tentative… " : "OZ Mapping ne répond pas (\(base.host ?? "")) : ") + lastError
        if !everConnected && failures >= 4 {
            onFail?()
            return
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in self?.open() }
    }

    func urlSession(_ session: URLSession, webSocketTask: URLSessionWebSocketTask, didOpenWithProtocol protocol: String?) {
        guard webSocketTask === task else { return }
        connected = true
        everConnected = true
        failures = 0
        status = "Connecté à OZ Mapping"
        sendView()
        onConnect?()
    }

    func urlSession(_ session: URLSession, webSocketTask: URLSessionWebSocketTask,
                    didCloseWith closeCode: URLSessionWebSocketTask.CloseCode, reason: Data?) {
        guard webSocketTask === task else { return }
        dropped(gen)
    }

    func urlSession(_ session: URLSession, didReceive challenge: URLAuthenticationChallenge,
                    completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        let space = challenge.protectionSpace
        if space.authenticationMethod == NSURLAuthenticationMethodServerTrust,
           let trust = space.serverTrust, Address.isLocal(space.host) {
            completionHandler(.useCredential, URLCredential(trust: trust))
        } else {
            completionHandler(.performDefaultHandling, nil)
        }
    }

    // ------------------------------------------------------------------ réception

    private func handle(_ text: String) {
        guard let m = (try? JSONSerialization.jsonObject(with: Data(text.utf8))) as? JSON else { return }
        switch m["t"] as? String {
        case "hello":
            if let a = (m["aspect"] as? NSNumber)?.doubleValue, a > 0.1 { aspect = CGFloat(a) }
        case "slots":
            slots = (m["full"] as? [Any] ?? []).map { ($0 as? NSNumber)?.boolValue ?? false }
        case "stickers":
            onStickers?(list(m, "list").map(StickerInfo.init))
        case "notice":
            show(notice: m["text"] as? String ?? "")
        case "live":
            live = LiveState(m)
        case "room":
            room = RoomState(m)
        default:
            break
        }
    }

    private func show(notice text: String) {
        notice = text
        status = text
        noticeTimer?.invalidate()
        noticeTimer = Timer.scheduledTimer(withTimeInterval: 6, repeats: false) { [weak self] _ in
            guard let self, self.connected else { return }
            self.status = "Connecté à OZ Mapping"
        }
    }

    /// Image JPEG de la sortie : décodée à part (la plus récente seulement si ça s'accumule).
    private func frame(_ data: Data) {
        if decoding {
            pendingFrame = data
            return
        }
        decoding = true
        decodeQueue.async { [weak self] in
            let img = Self.decode(data)
            DispatchQueue.main.async {
                guard let self else { return }
                if let img {
                    self.preview = img
                    self.previews += 1
                }
                self.decoding = false
                if let next = self.pendingFrame {
                    self.pendingFrame = nil
                    self.frame(next)
                }
            }
        }
    }

    private static func decode(_ data: Data) -> CGImage? {
        guard let src = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        return CGImageSourceCreateImageAtIndex(src, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
    }

    // ------------------------------------------------------------------ envoi

    func send(_ m: JSON) {
        guard connected, let t = task,
              let d = try? JSONSerialization.data(withJSONObject: m) else { return }
        t.send(.string(String(decoding: d, as: UTF8.self))) { _ in }
    }

    func act(_ a: Any, on: Bool = true) { send(["t": "act", "a": a, "on": on]) }
    func val(_ a: Any, _ v: Double) { send(["t": "val", "a": a, "v": min(1, max(0, v))]) }

    func chem(_ op: String, _ extra: JSON = [:]) {
        var m = extra
        m["t"] = "chem"
        m["op"] = op
        send(m)
    }

    /// Taille de l'aperçu voulu (pixels) et ce qu'il montre (le dessin seul ou toute la sortie).
    func setView(w: Int, screen: Bool) {
        guard w != view.w || screen != view.screen else { return }
        view = (w, screen)
        sendView()
    }

    private func sendView() { send(["t": "view", "w": view.w, "screen": view.screen]) }

    // ------------------------------------------------------------------ https

    private func url(_ path: String, _ query: String? = nil) -> URL {
        var c = URLComponents(url: base, resolvingAgainstBaseURL: false)!
        c.path = path
        c.percentEncodedQuery = query
        return c.url!
    }

    func post(_ path: String, query: String, body: Data) async throws -> Data {
        var r = URLRequest(url: url(path, query))
        r.httpMethod = "POST"
        r.httpBody = body
        r.timeoutInterval = 90
        r.setValue("application/octet-stream", forHTTPHeaderField: "Content-Type")
        let (data, resp) = try await session.data(for: r)
        guard (resp as? HTTPURLResponse)?.statusCode == 200 else {
            throw LinkError(text: String(decoding: data, as: UTF8.self))
        }
        return data
    }

    /// Miniature d'un média de l'app (gardée tant que le nom ne change pas).
    @MainActor
    func thumb(_ index: Int, name: String) async -> UIImage? {
        let key = "\(index)/\(name)"
        if let img = thumbs[key] { return img }
        let q = "n=" + (name.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? "")
        guard let (data, resp) = try? await session.data(from: url("/thumb/\(index)", q)),
              (resp as? HTTPURLResponse)?.statusCode == 200, let img = UIImage(data: data) else { return nil }
        thumbs[key] = img
        return img
    }
}
