import SceneKit
import SwiftUI
import UIKit

// ======================================================================= données

/// Un objet de la salle : bandeau de LEDs, décor, appareil DMX, laser. Les LEDs et les faces
/// arrivent déjà placées dans la salle (coordonnées en mètres, y vers le haut).
struct RoomItem {
    struct Face { var q: [SIMD3<Float>]; var color: UIColor; var output: Bool }
    var id, cat, kind, name: String
    var pos: SIMD3<Float>
    var yaw: Float
    var pitch: Float
    var leds: [SIMD3<Float>]
    var faces: [Face]
    /// Signature : l'objet n'est refait que si elle change.
    var sig: String

    /// LEDs et décor : géométrie déjà tournée (le nœud n'a pas de rotation propre).
    var baked: Bool { cat == "fixture" || cat == "surface" }
}

private func vec(_ a: Any?) -> SIMD3<Float>? {
    guard let a = a as? [Any], a.count == 3 else { return nil }
    let v = a.compactMap { ($0 as? NSNumber)?.floatValue }
    return v.count == 3 ? SIMD3(v[0], v[1], v[2]) : nil
}

struct RoomState {
    var size: SIMD2<Float>
    var items: [RoomItem]
    var addLeds: [(String, String)]
    var addDecor: [(String, String)]

    init(_ j: JSON) {
        let r = (j["room"] as? [Any] ?? []).compactMap { ($0 as? NSNumber)?.floatValue }
        size = r.count == 2 ? SIMD2(r[0], r[1]) : SIMD2(8, 10)
        items = (j["items"] as? [JSON] ?? []).compactMap { it in
            guard let id = it["id"] as? String, let pos = vec(it["pos"]) else { return nil }
            let sig = (try? JSONSerialization.data(withJSONObject: it, options: [.sortedKeys])).map { String(decoding: $0, as: UTF8.self) } ?? ""
            let faces: [RoomItem.Face] = (it["faces"] as? [JSON] ?? []).compactMap { f in
                let q = (f["q"] as? [Any] ?? []).compactMap(vec)
                guard q.count == 4 else { return nil }
                return RoomItem.Face(q: q, color: UIColor(hex: f["c"] as? String ?? "#888888"), output: (f["o"] as? NSNumber)?.boolValue ?? false)
            }
            return RoomItem(id: id, cat: it["cat"] as? String ?? "", kind: it["kind"] as? String ?? "", name: it["name"] as? String ?? "",
                            pos: pos, yaw: (it["yaw"] as? NSNumber)?.floatValue ?? 0, pitch: (it["pitch"] as? NSNumber)?.floatValue ?? 0,
                            leds: (it["leds"] as? [Any] ?? []).compactMap(vec), faces: faces, sig: sig)
        }
        let add = j["add"] as? JSON ?? [:]
        let pairs = { (k: String) in (add[k] as? [[Any]] ?? []).compactMap { p -> (String, String)? in
            guard p.count == 2, let a = p[0] as? String, let b = p[1] as? String else { return nil }
            return (a, b)
        } }
        addLeds = pairs("leds")
        addDecor = pairs("decor")
    }
}

// ======================================================================= scène SceneKit

/// La salle en 3D : on tourne autour au doigt (un doigt), on zoome (pincer), on se déplace
/// (deux doigts) ; toucher un objet le choisit, le faire glisser le déplace sur le sol.
final class RoomScene: NSObject, ObservableObject, UIGestureRecognizerDelegate, SCNSceneRendererDelegate {
    @Published var selected: String?
    /// Position et cap affichés de l'objet choisi (suivent les gestes avant la réponse de l'ordinateur).
    @Published var livePos: SIMD3<Float>?
    @Published var liveYaw: Float?

    weak var link: Link?
    let scene = SCNScene()
    let view = SCNView()
    private let world = SCNNode()
    private let floorNode = SCNNode()
    private let camera = SCNNode()
    private var nodes: [String: SCNNode] = [:]
    private var items: [String: RoomItem] = [:]
    private var size = SIMD2<Float>(0, 0)
    private var dragging: String?
    private var grabOffset = SIMD3<Float>(0, 0, 0)
    private var lastSend = Date.distantPast
    /// Un objet vient d'être demandé : le choisir dès qu'il arrive.
    var expectNew = false

    // Drone, comme sur l'ordinateur : l'œil se déplace, on tourne la tête.
    /// Manette (x : à droite, y : en avant, -1..1) et montée (-1..1), lues à chaque image.
    var stick = SIMD2<Float>(0, 0)
    var climb: Float = 0
    private var eye = SIMD3<Float>(0, 2, -3)
    private var yaw: Float = 0
    private var pitch: Float = -0.3
    private var lastFrame: TimeInterval?
    private var pinchStart: CGFloat = 1

    override init() {
        super.init()
        view.scene = scene
        view.backgroundColor = UIColor(hex: "#07080c")
        view.antialiasingMode = .multisampling4X
        view.autoenablesDefaultLighting = false
        // Comme sur l'ordinateur : en regardant vers le fond de la salle (+z), +x est à droite
        // (SceneKit le met à gauche) : la salle est retournée de gauche à droite.
        world.scale = SCNVector3(-1, 1, 1)
        scene.rootNode.addChildNode(world)
        world.addChildNode(floorNode)
        // Lumières : ambiance et une lumière d'en haut.
        let amb = SCNNode()
        amb.light = SCNLight()
        amb.light?.type = .ambient
        amb.light?.color = UIColor(white: 0.45, alpha: 1)
        scene.rootNode.addChildNode(amb)
        let sun = SCNNode()
        sun.light = SCNLight()
        sun.light?.type = .directional
        sun.light?.color = UIColor(white: 0.75, alpha: 1)
        sun.eulerAngles = SCNVector3(-Float.pi / 3, Float.pi / 5, 0)
        scene.rootNode.addChildNode(sun)
        camera.camera = SCNCamera()
        camera.camera?.zNear = 0.05
        camera.camera?.zFar = 400
        camera.camera?.fieldOfView = 55
        scene.rootNode.addChildNode(camera)
        view.pointOfView = camera
        view.delegate = self
        view.isPlaying = true
        view.preferredFramesPerSecond = 60

        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped(_:)))
        view.addGestureRecognizer(tap)
        let pan = UIPanGestureRecognizer(target: self, action: #selector(panned(_:)))
        pan.maximumNumberOfTouches = 1
        pan.delegate = self
        view.addGestureRecognizer(pan)
        // Un doigt ailleurs : regarder autour de soi (si ce n'est pas l'objet choisi qu'on tire).
        let look = UIPanGestureRecognizer(target: self, action: #selector(looked(_:)))
        look.maximumNumberOfTouches = 1
        look.require(toFail: pan)
        view.addGestureRecognizer(look)
        // Deux doigts : se décaler et monter ; pincer : avancer dans la direction du regard.
        let slide = UIPanGestureRecognizer(target: self, action: #selector(slid(_:)))
        slide.minimumNumberOfTouches = 2
        slide.delegate = self
        view.addGestureRecognizer(slide)
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinched(_:)))
        pinch.delegate = self
        view.addGestureRecognizer(pinch)
    }

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        // Pincer et glisser à deux doigts vont ensemble.
        (g is UIPinchGestureRecognizer && other is UIPanGestureRecognizer && other.numberOfTouches >= 2)
            || (other is UIPinchGestureRecognizer && g is UIPanGestureRecognizer && g.numberOfTouches >= 2)
    }

    // ------------------------------------------------------------------ drone

    /// Regard (vers -z de la caméra) et directions à plat pour avancer et se décaler.
    private var forward: SIMD3<Float> { SIMD3(-sin(yaw) * cos(pitch), sin(pitch), -cos(yaw) * cos(pitch)) }
    private var flatForward: SIMD3<Float> { SIMD3(-sin(yaw), 0, -cos(yaw)) }
    private var flatRight: SIMD3<Float> { SIMD3(cos(yaw), 0, -sin(yaw)) }

    private func applyCamera() {
        eye.y = max(eye.y, 0.15)
        camera.simdPosition = eye
        camera.eulerAngles = SCNVector3(pitch, yaw, 0)
    }

    /// Placer l'œil en `from`, regardant `to`.
    private func look(from: SIMD3<Float>, to: SIMD3<Float>) {
        eye = from
        let d = simd_normalize(to - from)
        pitch = asin(max(-1, min(1, d.y)))
        yaw = atan2(-d.x, -d.z)
        applyCamera()
    }

    /// À chaque image : la manette et la montée font voler le drone (plus vite dans une grande salle).
    func renderer(_ r: SCNSceneRenderer, updateAtTime time: TimeInterval) {
        let dt = Float(min(0.05, time - (lastFrame ?? time)))
        lastFrame = time
        let (s, c) = (stick, climb)
        guard s != .zero || c != 0 else { return }
        let speed = max(2, max(size.x, size.y) * 0.25)
        DispatchQueue.main.async {
            self.eye += (self.flatForward * s.y + self.flatRight * s.x) * speed * dt
            self.eye.y += c * speed * 0.6 * dt
            self.applyCamera()
        }
    }

    @objc private func looked(_ g: UIPanGestureRecognizer) {
        let t = g.translation(in: view)
        g.setTranslation(.zero, in: view)
        // Glisser vers la droite : la vue suit le doigt (on tourne la tête vers la gauche).
        yaw += Float(t.x) * 0.005
        pitch = max(-1.5, min(1.5, pitch + Float(t.y) * 0.005))
        applyCamera()
    }

    @objc private func slid(_ g: UIPanGestureRecognizer) {
        let t = g.translation(in: view)
        g.setTranslation(.zero, in: view)
        let k = max(1, max(size.x, size.y)) * 0.0025
        eye += flatRight * Float(-t.x) * k
        eye.y += Float(t.y) * k
        applyCamera()
    }

    @objc private func pinched(_ g: UIPinchGestureRecognizer) {
        if g.state == .began { pinchStart = 1 }
        let step = Float(g.scale - pinchStart)
        pinchStart = g.scale
        eye += forward * step * max(2, max(size.x, size.y) * 0.4)
        applyCamera()
    }

    // ------------------------------------------------------------------ mise à jour

    func update(_ room: RoomState) {
        if room.size != size {
            size = room.size
            buildFloor()
            home()
        }
        var seen = Set<String>()
        var fresh: [String] = []
        for it in room.items {
            seen.insert(it.id)
            if items[it.id] == nil { fresh.append(it.id) }
            // Objet qu'on tient sous le doigt : on ne le refait pas sous lui.
            if it.id == dragging { items[it.id] = it; continue }
            if items[it.id]?.sig == it.sig, nodes[it.id] != nil { continue }
            nodes[it.id]?.removeFromParentNode()
            let n = makeNode(it)
            world.addChildNode(n)
            nodes[it.id] = n
            items[it.id] = it
        }
        for id in items.keys where !seen.contains(id) {
            nodes[id]?.removeFromParentNode()
            nodes[id] = nil
            items[id] = nil
        }
        if expectNew, let id = fresh.last {
            expectNew = false
            select(id)
        }
        if let s = selected {
            if items[s] == nil { select(nil) } else { highlight(s, true) }
            if dragging == nil, let it = items[s] { livePos = it.pos; liveYaw = it.yaw }
        }
    }

    var current: RoomItem? { selected.flatMap { items[$0] } }

    /// Vue d'ensemble : depuis l'avant de la salle, en hauteur.
    func home() {
        let d = max(size.y, 2), w = max(size.x, 2)
        look(from: SIMD3(0, max(3.5, w * 0.45), -max(3, d * 0.55)), to: SIMD3(0, 1, d * 0.5))
    }

    private func buildFloor() {
        floorNode.childNodes.forEach { $0.removeFromParentNode() }
        let w = CGFloat(max(size.x, 1)), d = CGFloat(max(size.y, 1))
        let plane = SCNPlane(width: w, height: d)
        let m = SCNMaterial()
        m.diffuse.contents = UIColor(hex: "#14171f")
        m.isDoubleSided = true
        plane.materials = [m]
        let p = SCNNode(geometry: plane)
        p.eulerAngles.x = -.pi / 2
        p.position = SCNVector3(0, -0.002, Float(d) / 2)
        floorNode.addChildNode(p)
        // Quadrillage d'un mètre.
        var v: [SCNVector3] = []
        var x = -Float(w) / 2
        while x <= Float(w) / 2 + 0.001 { v += [SCNVector3(x, 0, 0), SCNVector3(x, 0, Float(d))]; x += 1 }
        var z: Float = 0
        while z <= Float(d) + 0.001 { v += [SCNVector3(-Float(w) / 2, 0, z), SCNVector3(Float(w) / 2, 0, z)]; z += 1 }
        let g = SCNGeometry(sources: [SCNGeometrySource(vertices: v)],
                            elements: [SCNGeometryElement(indices: (0..<UInt32(v.count)).map { $0 }, primitiveType: .line)])
        let lm = SCNMaterial()
        lm.lightingModel = .constant
        lm.diffuse.contents = UIColor(white: 1, alpha: 0.08)
        g.materials = [lm]
        floorNode.addChildNode(SCNNode(geometry: g))
    }

    // ------------------------------------------------------------------ objets

    private func makeNode(_ it: RoomItem) -> SCNNode {
        let node = SCNNode()
        node.name = it.id
        node.position = SCNVector3(it.pos)
        var lo = SIMD3<Float>(repeating: .greatestFiniteMagnitude), hi = -lo
        func grow(_ p: SIMD3<Float>) { lo = simd_min(lo, p); hi = simd_max(hi, p) }

        if !it.leds.isEmpty {
            let pts = it.leds.map { $0 - it.pos }
            pts.forEach(grow)
            let el = SCNGeometryElement(indices: (0..<UInt32(pts.count)).map { $0 }, primitiveType: .point)
            el.pointSize = 0.045
            el.minimumPointScreenSpaceRadius = 2
            el.maximumPointScreenSpaceRadius = 9
            let g = SCNGeometry(sources: [SCNGeometrySource(vertices: pts.map { SCNVector3($0) })], elements: [el])
            let m = SCNMaterial()
            m.lightingModel = .constant
            m.diffuse.contents = UIColor(hex: "#7dffc4")
            g.materials = [m]
            node.addChildNode(SCNNode(geometry: g))
            // Le fil qui relie les LEDs.
            if pts.count > 1 {
                let idx = (0..<UInt32(pts.count - 1)).flatMap { [$0, $0 + 1] }
                let lg = SCNGeometry(sources: [SCNGeometrySource(vertices: pts.map { SCNVector3($0) })],
                                     elements: [SCNGeometryElement(indices: idx, primitiveType: .line)])
                let lm = SCNMaterial()
                lm.lightingModel = .constant
                lm.diffuse.contents = UIColor(hex: "#5ad19a").withAlphaComponent(0.35)
                lg.materials = [lm]
                node.addChildNode(SCNNode(geometry: lg))
            }
        }
        if !it.faces.isEmpty {
            var verts: [SCNVector3] = [], normals: [SCNVector3] = [], colors: [Float] = [], idx: [UInt32] = []
            for f in it.faces {
                let base = UInt32(verts.count)
                var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
                // Ce qui montre l'image de la sortie : bleuté et clair.
                (f.output ? UIColor(hex: "#8fb3ff") : f.color).getRed(&r, green: &g, blue: &b, alpha: &a)
                // Normale de la face (les triangles d'un quad dégénéré prennent le troisième coin).
                let e1 = f.q[1] - f.q[0], e2 = (simd_length(f.q[3] - f.q[0]) > 1e-6 ? f.q[3] : f.q[2]) - f.q[0]
                var n = simd_cross(e2, e1)
                n = simd_length(n) > 1e-9 ? simd_normalize(n) : SIMD3(0, 1, 0)
                for p in f.q {
                    let q = p - it.pos
                    grow(q)
                    verts.append(SCNVector3(q))
                    normals.append(SCNVector3(n))
                    colors += [Float(r), Float(g), Float(b), 1]
                }
                idx += [base, base + 1, base + 2, base, base + 2, base + 3]
            }
            let cdata = colors.withUnsafeBufferPointer { Data(buffer: $0) }
            let csrc = SCNGeometrySource(data: cdata, semantic: .color, vectorCount: verts.count, usesFloatComponents: true,
                                         componentsPerVector: 4, bytesPerComponent: 4, dataOffset: 0, dataStride: 16)
            let g = SCNGeometry(sources: [SCNGeometrySource(vertices: verts), SCNGeometrySource(normals: normals), csrc],
                                elements: [SCNGeometryElement(indices: idx, primitiveType: .triangles)])
            let m = SCNMaterial()
            m.lightingModel = .lambert
            m.diffuse.contents = UIColor.white
            m.isDoubleSided = true
            g.materials = [m]
            node.addChildNode(SCNNode(geometry: g))
        }
        if !it.baked {
            // Appareil DMX ou laser : un boîtier et son faisceau, vers l'avant (+z) du boîtier.
            let body = SCNNode(geometry: SCNBox(width: 0.3, height: 0.2, length: 0.3, chamferRadius: 0.03))
            body.geometry?.firstMaterial?.diffuse.contents = UIColor(hex: "#30343e")
            body.position.y = 0.1
            let beam = SCNCone(topRadius: 0, bottomRadius: 0.35, height: 1.6)
            let bm = SCNMaterial()
            bm.lightingModel = .constant
            bm.diffuse.contents = UIColor(hex: it.cat == "laser" ? "#30d158" : "#ffd60a").withAlphaComponent(0.18)
            bm.writesToDepthBuffer = false
            bm.isDoubleSided = true
            beam.materials = [bm]
            let bn = SCNNode(geometry: beam)
            bn.eulerAngles.x = .pi / 2
            bn.position = SCNVector3(0, 0.1, 0.95)
            node.addChildNode(body)
            node.addChildNode(bn)
            node.eulerAngles = SCNVector3(-it.pitch * .pi / 180, it.yaw * .pi / 180, 0)
            grow(SIMD3(-0.2, 0, -0.2))
            grow(SIMD3(0.2, 0.3, 0.2))
        }
        // Boîte de sélection : invisible (sert à toucher l'objet), dessinée en fil une fois choisi.
        if lo.x <= hi.x {
            let ext = simd_max(hi - lo, SIMD3(repeating: 0.2)) + 0.08
            let box = SCNBox(width: CGFloat(ext.x), height: CGFloat(ext.y), length: CGFloat(ext.z), chamferRadius: 0)
            let m = SCNMaterial()
            m.lightingModel = .constant
            m.diffuse.contents = UIColor(Theme.accent)
            m.fillMode = .lines
            m.colorBufferWriteMask = []
            m.writesToDepthBuffer = false
            box.materials = [m]
            let pick = SCNNode(geometry: box)
            pick.name = "pick"
            pick.position = SCNVector3((lo + hi) / 2)
            node.addChildNode(pick)
        }
        return node
    }

    private func highlight(_ id: String, _ on: Bool) {
        nodes[id]?.childNode(withName: "pick", recursively: false)?.geometry?.firstMaterial?.colorBufferWriteMask = on ? .all : []
    }

    func select(_ id: String?) {
        if let s = selected { highlight(s, false) }
        selected = id
        if let id {
            highlight(id, true)
            livePos = items[id]?.pos
            liveYaw = items[id]?.yaw
        } else {
            livePos = nil
            liveYaw = nil
        }
    }

    // ------------------------------------------------------------------ gestes

    private func pickedId(at p: CGPoint) -> String? {
        let hits = view.hitTest(p, options: [.searchMode: SCNHitTestSearchMode.all.rawValue, .ignoreHiddenNodes: false])
        for h in hits where h.node.name == "pick" {
            if let id = h.node.parent?.name { return id }
        }
        return nil
    }

    @objc private func tapped(_ g: UITapGestureRecognizer) {
        select(pickedId(at: g.location(in: view)))
    }

    /// Point du plan horizontal de hauteur `y` sous le doigt.
    private func ground(_ p: CGPoint, y: Float) -> SIMD3<Float>? {
        let a = SIMD3<Float>(view.unprojectPoint(SCNVector3(Float(p.x), Float(p.y), 0)))
        let b = SIMD3<Float>(view.unprojectPoint(SCNVector3(Float(p.x), Float(p.y), 1)))
        let d = b - a
        guard abs(d.y) > 1e-5 else { return nil }
        let t = (y - a.y) / d.y
        guard t > 0 else { return nil }
        // Dans les coordonnées de la salle (retournée de gauche à droite).
        return SIMD3<Float>(world.convertPosition(SCNVector3(a + d * t), from: nil))
    }

    /// Point du sol au milieu de la vue (là où arrive un objet ajouté).
    func centerGround() -> SIMD3<Float>? {
        guard var p = ground(CGPoint(x: view.bounds.midX, y: view.bounds.midY), y: 0) else { return nil }
        p.x = min(max(p.x, -size.x / 2), size.x / 2)
        p.z = min(max(p.z, 0), size.y)
        return p
    }

    func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
        guard g.view === view, g is UIPanGestureRecognizer, (g as? UIPanGestureRecognizer)?.minimumNumberOfTouches == 1 else { return true }
        guard let s = selected, let id = pickedId(at: g.location(in: view)) else { return false }
        return id == s
    }

    @objc private func panned(_ g: UIPanGestureRecognizer) {
        guard let id = selected, let node = nodes[id], let it = items[id] else { return }
        let y = node.position.y
        let p = g.location(in: view)
        switch g.state {
        case .began:
            dragging = id
            if let hit = ground(p, y: y) { grabOffset = SIMD3<Float>(node.position) - hit }
        case .changed:
            guard let hit = ground(p, y: y) else { return }
            var pos = hit + grabOffset
            pos.y = y
            node.position = SCNVector3(pos)
            livePos = pos
            if Date().timeIntervalSince(lastSend) > 0.07 {
                lastSend = Date()
                link?.send(["t": "room", "op": "place", "id": id, "pos": [pos.x, pos.y, pos.z]])
            }
        default:
            let pos = SIMD3<Float>(node.position)
            link?.send(["t": "room", "op": "place", "id": id, "pos": [pos.x, pos.y, pos.z]])
            dragging = nil
            items[id]?.sig = ""  // refait à la prochaine scène reçue
            _ = it
        }
    }

    /// Hauteur ou cap réglés au curseur : aperçu tout de suite, envoi au fil du geste.
    func adjust(height: Float? = nil, yaw: Float? = nil, final: Bool = false) {
        guard let id = selected, let node = nodes[id], let it = items[id] else { return }
        var pos = SIMD3<Float>(node.position)
        if let height { pos.y = height; node.position.y = height }
        let newYaw = yaw ?? (liveYaw ?? it.yaw)
        if yaw != nil {
            // Géométrie déjà tournée : on tourne le nœud de la différence ; sinon du cap entier.
            node.eulerAngles.y = (it.baked ? newYaw - it.yaw : newYaw) * .pi / 180
        }
        livePos = pos
        liveYaw = newYaw
        guard final || Date().timeIntervalSince(lastSend) > 0.07 else { return }
        lastSend = Date()
        link?.send(["t": "room", "op": "place", "id": id, "pos": [pos.x, pos.y, pos.z], "yaw": newYaw])
    }
}

private extension SCNVector3 {
    init(_ v: SIMD3<Float>) { self.init(v.x, v.y, v.z) }
}

private extension SIMD3 where Scalar == Float {
    init(_ v: SCNVector3) { self.init(v.x, v.y, v.z) }
}

private struct SceneRep: UIViewRepresentable {
    let scene: RoomScene
    func makeUIView(context: Context) -> SCNView { scene.view }
    func updateUIView(_ v: SCNView, context: Context) {}
}

// ======================================================================= drone

/// Manette (avancer, reculer, se décaler) en bas à gauche, montée et descente en bas à droite.
private struct DroneControls: View {
    let scene: RoomScene
    @State private var knob = CGSize.zero
    private let radius: CGFloat = 58

    var body: some View {
        VStack {
            Spacer()
            HStack(alignment: .bottom) {
                ZStack {
                    Circle().fill(.white.opacity(0.07)).overlay(Circle().strokeBorder(.white.opacity(0.18), lineWidth: 1.5))
                    Image(systemName: "arrow.up.and.down.and.arrow.left.and.right")
                        .font(.system(size: 15)).foregroundStyle(.white.opacity(0.3))
                    Circle().fill(Theme.accent.opacity(0.85)).frame(width: 46, height: 46).offset(knob)
                }
                .frame(width: radius * 2, height: radius * 2)
                .contentShape(Circle())
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { v in
                        var t = v.translation
                        let l = hypot(t.width, t.height)
                        if l > radius { t = CGSize(width: t.width / l * radius, height: t.height / l * radius) }
                        knob = t
                        // Doux près du centre, à fond au bord.
                        let k = { (x: CGFloat) in Float(x / radius) * min(1, Float(l / radius)) }
                        scene.stick = SIMD2(k(t.width), -k(t.height))
                    }
                    .onEnded { _ in
                        knob = .zero
                        scene.stick = .zero
                    })
                Spacer()
                VStack(spacing: 10) {
                    hold("arrow.up", "Monter", 1)
                    hold("arrow.down", "Descendre", -1)
                }
            }
            .padding(18)
        }
        .allowsHitTesting(true)
    }

    private func hold(_ icon: String, _ label: String, _ dir: Float) -> some View {
        PressButton(hold: true, height: 52, press: { scene.climb = dir }, release: { scene.climb = 0 }) {
            Image(systemName: icon).font(.system(size: 20, weight: .semibold))
        }
        .frame(width: 64)
        .accessibilityLabel(label)
    }
}

// ======================================================================= onglet

/// Onglet Salle 3D : la scène des LEDs et du décor, à parcourir et à arranger.
struct RoomPanel: View {
    @EnvironmentObject var link: Link
    @StateObject private var scene = RoomScene()
    @State private var confirmDelete = false

    var body: some View {
        HStack(spacing: 0) {
            ZStack(alignment: .topLeading) {
                SceneRep(scene: scene)
                if link.room == nil {
                    VStack(spacing: 10) {
                        ProgressView().tint(Theme.weak)
                        Text("Chargement de la salle…").foregroundStyle(Theme.weak)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
                Button { scene.home() } label: { Label("Recentrer", systemImage: "scope") }
                    .buttonStyle(Chip())
                    .frame(width: 130)
                    .padding(10)
                DroneControls(scene: scene)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 0) { side }
                    .padding(.horizontal, 12)
                    .padding(.bottom, 24)
            }
            .frame(width: 330)
            .background(Theme.panel)
        }
        .onAppear {
            scene.link = link
            link.send(["t": "room", "op": "watch"])
            if let r = link.room { scene.update(r) }
        }
        // Tant que l'onglet est ouvert : « je regarde la salle » (l'ordinateur l'envoie alors).
        .task {
            while !Task.isCancelled {
                link.send(["t": "room", "op": "watch"])
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
        .onChange(of: link.room?.items.map(\.sig) ?? []) { _ in
            if let r = link.room { scene.update(r) }
        }
    }

    @ViewBuilder
    private var side: some View {
        if let it = scene.current {
            SectionTitle("\(it.kind) choisi")
            Text(it.name).font(.system(size: 17, weight: .semibold))
            let pos = scene.livePos ?? it.pos
            Text(String(format: "x %.2f m · z %.2f m", pos.x, pos.z)).font(.system(size: 12)).foregroundStyle(Theme.weak)
            NamedSlider(label: String(format: "Hauteur %.2f m", pos.y),
                        value: Binding(get: { Double(pos.y) }, set: { scene.adjust(height: Float($0)) }),
                        range: -0.5...8, onEditing: { e in if !e { scene.adjust(final: true) } })
            let yaw = scene.liveYaw ?? it.yaw
            NamedSlider(label: String(format: "Cap %.0f°", yaw),
                        value: Binding(get: { Double(yaw) }, set: { scene.adjust(yaw: Float($0)) }),
                        range: 0...360, onEditing: { e in if !e { scene.adjust(final: true) } })
            HStack(spacing: 6) {
                Button { scene.adjust(yaw: turn(yaw, -15), final: true) } label: { Image(systemName: "rotate.left") }
                Button { scene.adjust(yaw: turn(yaw, 15), final: true) } label: { Image(systemName: "rotate.right") }
                Button { scene.adjust(height: 0, final: true) } label: { Label("Au sol", systemImage: "arrow.down.to.line") }
            }
            .buttonStyle(Chip())
            .padding(.top, 6)
            HStack(spacing: 6) {
                if it.baked {
                    Button {
                        scene.expectNew = true
                        link.send(["t": "room", "op": "dup", "id": it.id])
                    } label: { Label("Dupliquer", systemImage: "plus.square.on.square") }
                        .buttonStyle(Chip())
                }
                Button { confirmDelete = true } label: { Label("Supprimer", systemImage: "trash") }
                    .buttonStyle(Chip(danger: true))
            }
            .padding(.top, 6)
            .confirmationDialog("Supprimer « \(it.name) » ?", isPresented: $confirmDelete, titleVisibility: .visible) {
                Button("Supprimer", role: .destructive) {
                    link.send(["t": "room", "op": "del", "id": it.id])
                    scene.select(nil)
                }
            }
            Hint("Faites glisser l'objet choisi pour le déplacer sur le sol. Touchez le vide pour le lâcher.")
        } else {
            SectionTitle("Salle 3D")
            Hint("Comme un petit drone : la manette en bas à gauche avance, recule et se décale ; les flèches à droite montent et descendent. Un doigt sur la vue : regarder autour de soi. Pincer : avancer dans la direction du regard. Deux doigts : se décaler. Touchez un objet pour le choisir, puis faites-le glisser pour le déplacer.")
        }
        if let r = link.room {
            SectionTitle("Ajouter des LEDs")
            LazyVGrid(columns: chipColumns(96), spacing: 6) {
                ForEach(r.addLeds, id: \.0) { what, name in
                    Button(name) { add(what) }.buttonStyle(Chip())
                }
            }
            SectionTitle("Ajouter au décor")
            LazyVGrid(columns: chipColumns(96), spacing: 6) {
                ForEach(r.addDecor, id: \.0) { what, name in
                    Button(name) { add(what) }.buttonStyle(Chip())
                }
            }
            Hint("L'objet arrive au milieu de la vue. Les appareils DMX et les lasers (qui ont une adresse) s'ajoutent sur l'ordinateur ; on peut les placer ici.")
        }
    }

    private func turn(_ yaw: Float, _ by: Float) -> Float {
        ((yaw + by).truncatingRemainder(dividingBy: 360) + 360).truncatingRemainder(dividingBy: 360)
    }

    private func add(_ what: String) {
        scene.expectNew = true
        var m: JSON = ["t": "room", "op": "add", "what": what]
        if let p = scene.centerGround() { m["pos"] = [p.x, p.y, p.z] }
        link.send(m)
    }
}
