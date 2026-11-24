import SwiftUI
import UIKit

/// Détourage au stylet : la photo en plein écran, le détourage automatique d'OZ Mapping comme
/// brouillon ; on entoure, on retire, on peint ou on gomme le masque, puis Valider envoie la
/// photo détourée (PNG transparent) dans le dessin.
final class CutEditor: ObservableObject {
    enum Mode: String, CaseIterable { case lassoAdd, lassoSub, brushAdd, brushSub }

    let photo: CGImage
    let w: Int, h: Int
    let mask: CGContext
    @Published var mode: Mode = .lassoAdd
    @Published var size: Double = 28
    @Published var msg = ""
    @Published var canUndo = false
    var touched = false
    var changed: () -> Void = {}
    private var undo: [Data] = []

    init?(photo img: UIImage) {
        guard let cg = img.cgImage else { return nil }
        photo = cg
        w = cg.width
        h = cg.height
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue) else { return nil }
        mask = ctx
        // Coordonnées du haut à gauche, comme la photo à l'écran.
        mask.translateBy(x: 0, y: CGFloat(h))
        mask.scaleBy(x: 1, y: -1)
        mask.setFillColor(gray: 0, alpha: 1)
        mask.fill(CGRect(x: 0, y: 0, width: w, height: h))
    }

    var bytes: Int { mask.bytesPerRow * h }

    func push() {
        guard let p = mask.data else { return }
        undo.append(Data(bytes: p, count: bytes))
        if undo.count > 12 { undo.removeFirst() }
        canUndo = true
    }

    func popUndo() {
        guard let prev = undo.popLast(), let p = mask.data else { return }
        prev.withUnsafeBytes { p.copyMemory(from: $0.baseAddress!, byteCount: bytes) }
        canUndo = !undo.isEmpty
        changed()
    }

    func dropUndo() {
        _ = undo.popLast()
        canUndo = !undo.isEmpty
    }

    func clear() {
        push()
        mask.setFillColor(gray: 0, alpha: 1)
        mask.fill(CGRect(x: 0, y: 0, width: w, height: h))
        touched = true
        changed()
    }

    func invert() {
        guard let p = mask.data?.assumingMemoryBound(to: UInt8.self) else { return }
        push()
        for i in 0..<bytes { p[i] = 255 &- p[i] }
        touched = true
        changed()
    }

    func fill(_ pts: [CGPoint], add: Bool) {
        guard pts.count >= 3 else { return }
        mask.setFillColor(gray: add ? 1 : 0, alpha: 1)
        mask.beginPath()
        mask.addLines(between: pts)
        mask.closePath()
        mask.fillPath()
        touched = true
        changed()
    }

    func stamp(_ p: CGPoint, radius: CGFloat, add: Bool) {
        mask.setFillColor(gray: add ? 1 : 0, alpha: 1)
        mask.fillEllipse(in: CGRect(x: p.x - radius, y: p.y - radius, width: radius * 2, height: radius * 2))
        touched = true
    }

    /// Masque automatique (blanc = sujet) à la place du masque actuel.
    func setMask(_ img: CGImage) {
        mask.saveGState()
        mask.translateBy(x: 0, y: CGFloat(h))
        mask.scaleBy(x: 1, y: -1)
        mask.interpolationQuality = .high
        mask.draw(img, in: CGRect(x: 0, y: 0, width: w, height: h))
        mask.restoreGState()
        changed()
    }

    /// Le masque (blanc = gardé), en PNG gris.
    func maskPNG() -> Data? {
        guard let m = mask.makeImage() else { return nil }
        return UIImage(cgImage: m).pngData()
    }

    /// La photo entière (sans détourage), en JPEG.
    func photoJPEG() -> Data? {
        UIImage(cgImage: photo).jpegData(compressionQuality: 0.9)
    }

    /// La photo, transparente hors du masque, en PNG.
    func output() -> Data? {
        guard let m = mask.makeImage(),
              let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return nil }
        let r = CGRect(x: 0, y: 0, width: w, height: h)
        ctx.clip(to: r, mask: m)
        ctx.draw(photo, in: r)
        guard let out = ctx.makeImage() else { return nil }
        return UIImage(cgImage: out).pngData()
    }
}

/// Objet détouré à mapper : la photo entière, le masque, et s'il faut afficher la photo.
struct ObjectCut {
    var jpeg: Data
    var mask: Data
    var withPhoto: Bool
}

struct CutEditorView: View {
    @EnvironmentObject var link: Link
    @StateObject private var ed: CutEditor
    var done: (Data?) -> Void
    /// Détourage d'un objet à mapper (au lieu d'une photo pour le dessin).
    var object: ((ObjectCut?) -> Void)?

    init(photo: UIImage, done: @escaping (Data?) -> Void) {
        _ed = StateObject(wrappedValue: CutEditor(photo: photo) ?? CutEditor(photo: UIImage(systemName: "photo")!.normalized(max: 64))!)
        self.done = done
    }

    init(object photo: UIImage, done: @escaping (ObjectCut?) -> Void) {
        _ed = StateObject(wrappedValue: CutEditor(photo: photo) ?? CutEditor(photo: UIImage(systemName: "photo")!.normalized(max: 64))!)
        self.done = { _ in }
        self.object = done
    }

    private func finishObject(withPhoto: Bool) {
        guard let jpeg = ed.photoJPEG(), let mask = ed.maskPNG() else { object?(nil); return }
        object?(ObjectCut(jpeg: jpeg, mask: mask, withPhoto: withPhoto))
    }

    var body: some View {
        VStack(spacing: 0) {
            CutCanvasRep(ed: ed)
            VStack(spacing: 8) {
                // Outils : les quatre gestes du stylet, bien visibles, puis les réglages.
                HStack(spacing: 6) {
                    tool(.lassoAdd, "Entourer", "lasso")
                    tool(.lassoSub, "Retirer", "scissors")
                    tool(.brushAdd, "Pinceau", "paintbrush.pointed.fill")
                    tool(.brushSub, "Gomme", "eraser.fill")
                    Image(systemName: "circle.fill").font(.system(size: 8)).foregroundStyle(Theme.weak)
                    Slider(value: $ed.size, in: 4...120).frame(minWidth: 90, maxWidth: 160).tint(Theme.accent)
                    Image(systemName: "circle.fill").font(.system(size: 18)).foregroundStyle(Theme.weak)
                }
                HStack(spacing: 6) {
                    Button { auto(force: true) } label: { Label("Auto", systemImage: "wand.and.stars") }.buttonStyle(Chip()).frame(width: 96)
                    Button { ed.invert() } label: { Label("Inverser", systemImage: "circle.lefthalf.filled") }.buttonStyle(Chip()).frame(width: 116)
                    Button { ed.popUndo() } label: { Image(systemName: "arrow.uturn.backward") }
                        .buttonStyle(Chip()).frame(width: 50).disabled(!ed.canUndo)
                    Button { ed.clear() } label: { Label("Vider", systemImage: "trash") }.buttonStyle(Chip(danger: true)).frame(width: 96)
                    Text(ed.msg)
                        .font(.system(size: 12))
                        .foregroundStyle(Theme.accent)
                        .lineLimit(2)
                        .minimumScaleFactor(0.8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if object != nil {
                        Button("Annuler") { object?(nil) }.buttonStyle(Chip(danger: true)).frame(width: 90)
                        Button("Masque seul") { finishObject(withPhoto: false) }.buttonStyle(Chip()).frame(width: 110)
                        Button("Mapper la photo") { finishObject(withPhoto: true) }.buttonStyle(Chip(on: true)).frame(width: 140)
                    } else {
                        Button("Annuler") { done(nil) }.buttonStyle(Chip(danger: true)).frame(width: 90)
                        Button("Valider") { done(ed.output()) }.buttonStyle(Chip(on: true)).frame(width: 90)
                    }
                }
            }
            .padding(10)
            .background(Theme.panel)
        }
        .background(Color(hex: "#05060a").ignoresSafeArea())
        .preferredColorScheme(.dark)
        .onAppear {
            ed.msg = "Détourage automatique… vous pouvez déjà entourer le sujet au stylet"
            auto(force: false)
        }
    }

    /// Bouton d'un outil du stylet.
    private func tool(_ m: CutEditor.Mode, _ label: String, _ icon: String) -> some View {
        Button { ed.mode = m } label: { Label(label, systemImage: icon) }
            .buttonStyle(Chip(on: ed.mode == m, height: 46))
            .frame(minWidth: 100, maxWidth: 150)
    }

    /// Masque automatique d'OZ Mapping ; `force` : remplace aussi un masque déjà retouché.
    private func auto(force: Bool) {
        if force { ed.msg = "Détourage automatique…" }
        guard let jpeg = UIImage(cgImage: ed.photo).jpegData(compressionQuality: 0.9) else { return }
        Task { @MainActor in
            do {
                let png = try await link.post("/cutout", query: "", body: jpeg)
                guard let img = UIImage(data: png)?.cgImage else { throw LinkError(text: "masque illisible") }
                if ed.touched && !force {
                    ed.msg = ""
                    return
                }
                if force { ed.push() }
                ed.setMask(img)
                ed.msg = "Retouchez au stylet : Entourer / Retirer, Pinceau / Gomme. Deux doigts : zoom."
            } catch {
                ed.msg = "Pas de détourage automatique : entourez le sujet au stylet"
            }
        }
    }
}

private struct CutCanvasRep: UIViewRepresentable {
    @ObservedObject var ed: CutEditor
    func makeUIView(context: Context) -> CutCanvas { CutCanvas(ed: ed) }
    func updateUIView(_ v: CutCanvas, context: Context) { v.setNeedsDisplay() }
}

/// Le stylet trace ; les doigts déplacent et zooment (sans stylet : un doigt trace, deux zooment).
private final class CutCanvas: UIView {
    let ed: CutEditor
    private var view = (s: CGFloat(1), x: CGFloat(0), y: CGFloat(0))
    private var fitted = CGSize.zero
    private var path: (touch: ObjectIdentifier, pts: [CGPoint])?
    private var touches: [ObjectIdentifier: CGPoint] = [:]
    private var touchOrder: [ObjectIdentifier] = []
    private var pinch: (v: (s: CGFloat, x: CGFloat, y: CGFloat), t: [CGPoint])?
    private var cursor: CGPoint?
    private var pen = false

    init(ed: CutEditor) {
        self.ed = ed
        super.init(frame: .zero)
        backgroundColor = UIColor(hex: "#05060a")
        isMultipleTouchEnabled = true
        contentMode = .redraw
        ed.changed = { [weak self] in self?.setNeedsDisplay() }
        let hover = UIHoverGestureRecognizer(target: self, action: #selector(hovered(_:)))
        addGestureRecognizer(hover)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        if bounds.size != fitted { fit() }
    }

    private func fit() {
        fitted = bounds.size
        let s = min(bounds.width / CGFloat(ed.w), bounds.height / CGFloat(ed.h)) * 0.94
        view = (s, (bounds.width - CGFloat(ed.w) * s) / 2, (bounds.height - CGFloat(ed.h) * s) / 2)
        setNeedsDisplay()
    }

    private func toPhoto(_ p: CGPoint) -> CGPoint { CGPoint(x: (p.x - view.x) / view.s, y: (p.y - view.y) / view.s) }

    private var isBrush: Bool { ed.mode == .brushAdd || ed.mode == .brushSub }

    private func radius(_ t: UITouch) -> CGFloat {
        var k: CGFloat = 1
        if pen, t.type == .pencil, t.maximumPossibleForce > 0 { k = 0.4 + 1.2 * t.force / t.maximumPossibleForce }
        return max(0.5, CGFloat(ed.size) / view.s / 2 * k)
    }

    @objc private func hovered(_ g: UIHoverGestureRecognizer) {
        cursor = g.state == .ended || g.state == .cancelled ? nil : toPhoto(g.location(in: self))
        if isBrush { setNeedsDisplay() }
    }

    private func startPinch() {
        pinch = (view, touchOrder.compactMap { touches[$0] })
    }

    override func touchesBegan(_ set: Set<UITouch>, with event: UIEvent?) {
        for t in set {
            let id = ObjectIdentifier(t)
            if t.type == .pencil { pen = true }
            let finger = t.type != .pencil && (pen || !touches.isEmpty || path != nil)
            if finger {
                // Un second doigt sans stylet : c'était un zoom, pas un tracé.
                if path != nil && !pen {
                    path = nil
                    ed.popUndo()
                }
                touches[id] = t.location(in: self)
                touchOrder.append(id)
                startPinch()
                continue
            }
            if t.type != .pencil {
                touches[id] = t.location(in: self)
                touchOrder.append(id)
            }
            ed.push()
            let p = toPhoto(t.location(in: self))
            path = (id, [p])
            if isBrush {
                ed.stamp(p, radius: radius(t), add: ed.mode == .brushAdd)
            }
            setNeedsDisplay()
        }
    }

    override func touchesMoved(_ set: Set<UITouch>, with event: UIEvent?) {
        for t in set {
            let id = ObjectIdentifier(t)
            cursor = toPhoto(t.location(in: self))
            if touches[id] != nil && path?.touch != id {
                touches[id] = t.location(in: self)
                let now = touchOrder.compactMap { touches[$0] }
                guard let z = pinch, now.count == z.t.count, !now.isEmpty else {
                    startPinch()
                    continue
                }
                func mid(_ a: [CGPoint]) -> CGPoint {
                    CGPoint(x: a.map(\.x).reduce(0, +) / CGFloat(a.count), y: a.map(\.y).reduce(0, +) / CGFloat(a.count))
                }
                let m0 = mid(z.t), m1 = mid(now)
                var s = z.v.s
                if now.count >= 2 {
                    s = z.v.s * hypot(now[1].x - now[0].x, now[1].y - now[0].y) / max(1, hypot(z.t[1].x - z.t[0].x, z.t[1].y - z.t[0].y))
                }
                s = min(20, max(0.05, s))
                let ip = CGPoint(x: (m0.x - z.v.x) / z.v.s, y: (m0.y - z.v.y) / z.v.s)
                view = (s, m1.x - ip.x * s, m1.y - ip.y * s)
                setNeedsDisplay()
                continue
            }
            guard var p = path, p.touch == id else { continue }
            for c in event?.coalescedTouches(for: t) ?? [t] {
                let q = toPhoto(c.location(in: self)), last = p.pts[p.pts.count - 1]
                if isBrush {
                    // Tampons rapprochés le long du geste.
                    let step = max(0.5, CGFloat(ed.size) / view.s / 6), d = hypot(q.x - last.x, q.y - last.y)
                    let n = max(1, Int((d / step).rounded(.up)))
                    for k in 1...n {
                        let f = CGFloat(k) / CGFloat(n)
                        ed.stamp(CGPoint(x: last.x + (q.x - last.x) * f, y: last.y + (q.y - last.y) * f),
                                 radius: radius(c), add: ed.mode == .brushAdd)
                    }
                }
                p.pts.append(q)
            }
            path = p
            setNeedsDisplay()
        }
    }

    override func touchesEnded(_ set: Set<UITouch>, with event: UIEvent?) { up(set) }
    override func touchesCancelled(_ set: Set<UITouch>, with event: UIEvent?) { up(set) }

    private func up(_ set: Set<UITouch>) {
        for t in set {
            let id = ObjectIdentifier(t)
            if touches.removeValue(forKey: id) != nil {
                touchOrder.removeAll { $0 == id }
                pinch = nil
                if !touches.isEmpty { startPinch() }
            }
            guard let p = path, p.touch == id else { continue }
            path = nil
            if !isBrush {
                if p.pts.count < 3 {
                    ed.dropUndo()
                } else {
                    ed.fill(p.pts, add: ed.mode == .lassoAdd)
                }
            }
            ed.touched = true
            setNeedsDisplay()
        }
    }

    // Affichage : la photo en sombre, la partie gardée en clair, le tracé en cours.
    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext() else { return }
        let W = CGFloat(ed.w), H = CGFloat(ed.h)
        ctx.saveGState()
        ctx.translateBy(x: view.x, y: view.y + H * view.s)
        ctx.scaleBy(x: view.s, y: -view.s)
        let r = CGRect(x: 0, y: 0, width: W, height: H)
        ctx.interpolationQuality = view.s < 1 ? .medium : .low
        ctx.setAlpha(0.28)
        ctx.draw(ed.photo, in: r)
        ctx.setAlpha(1)
        if let m = ed.mask.makeImage() {
            ctx.saveGState()
            ctx.clip(to: r, mask: m)
            ctx.draw(ed.photo, in: r)
            ctx.restoreGState()
        }
        ctx.restoreGState()
        let scr = { (p: CGPoint) in CGPoint(x: self.view.x + p.x * self.view.s, y: self.view.y + p.y * self.view.s) }
        ctx.setStrokeColor(UIColor.white.withAlphaComponent(0.25).cgColor)
        ctx.setLineWidth(1)
        ctx.stroke(CGRect(x: view.x, y: view.y, width: W * view.s, height: H * view.s))
        if let p = path, !isBrush, p.pts.count > 1 {
            ctx.setStrokeColor(UIColor(hex: ed.mode == .lassoAdd ? "#5ad19a" : "#ff6b6b").cgColor)
            ctx.setLineWidth(2.5)
            ctx.addLines(between: p.pts.map(scr))
            ctx.strokePath()
        }
        if let c = cursor, isBrush {
            ctx.setStrokeColor(UIColor.white.withAlphaComponent(0.8).cgColor)
            ctx.setLineWidth(1.5)
            let q = scr(c), r = CGFloat(ed.size) / 2
            ctx.strokeEllipse(in: CGRect(x: q.x - r, y: q.y - r, width: r * 2, height: r * 2))
        }
    }
}
