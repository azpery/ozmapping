import SwiftUI
import UIKit

/// La toile : l'image de la sortie (ce qui est vraiment projeté), et par-dessus ce qu'on est en
/// train de faire, sans attendre le réseau. Au doigt ou à l'Apple Pencil (pression, et la paume
/// est ignorée dès que le stylet a servi).
struct CanvasRep: UIViewRepresentable {
    @ObservedObject var model: Model
    var aspect: CGFloat

    func makeUIView(context: Context) -> CanvasView { CanvasView(model: model) }

    func updateUIView(_ v: CanvasView, context: Context) {
        v.aspect = aspect
        v.overlayDirty = true
    }
}

final class CanvasView: UIView {
    let model: Model
    var link: Link { model.link }
    var aspect: CGFloat = 16.0 / 9.0 {
        didSet { if aspect != oldValue { setNeedsLayout() } }
    }
    var overlayDirty = true

    private let image = CALayer()
    private let overlay = Overlay()
    private(set) var box = CGRect(x: 0, y: 0, width: 1, height: 1)
    private var display: CADisplayLink?
    private var shownPreviews = -1
    private var shownStickers = -1

    /// Trait en cours (gardé après le lever, le temps que l'aperçu le montre).
    final class Stroke {
        let id: Int
        var pts: [[Double]]
        let color: UIColor
        let w: Double
        let kind: String
        let start: [Double]
        var cur: [Double]
        var end: CFTimeInterval?
        var seen = 0
        init(id: Int, p: [Double], color: UIColor, w: Double, kind: String) {
            self.id = id; pts = [p]; self.color = color; self.w = w; self.kind = kind; start = p; cur = p
        }
    }

    private struct Eraser { var x, y, r: Double; var touch: ObjectIdentifier; var dirty: Bool }
    private struct Grab { var id: Int; var from: StickerInfo; var pts: [CGPoint] }
    private struct ChemTouch { var p: [Double]; var t: CFTimeInterval; var poured: [Double]; var sent: CFTimeInterval }

    private static var nextId = 1
    private var strokes: [ObjectIdentifier: Stroke] = [:]
    private var outbox: [Int: [[Double]]] = [:]
    private var eraser: Eraser?
    private var fingers: [ObjectIdentifier: CGPoint] = [:]
    private var fingerOrder: [ObjectIdentifier] = []
    private var grab: Grab?
    private var stDirty = false
    private var chemTouches: [ObjectIdentifier: ChemTouch] = [:]
    private var penSeen = false

    init(model: Model) {
        self.model = model
        super.init(frame: .zero)
        backgroundColor = UIColor(Theme.bg)
        isMultipleTouchEnabled = true
        image.backgroundColor = UIColor.black.cgColor
        image.cornerRadius = 6
        image.masksToBounds = true
        image.borderColor = UIColor(hex: "#2a3040").cgColor
        image.borderWidth = 1
        image.contentsGravity = .resize
        image.magnificationFilter = .linear
        image.actions = ["contents": NSNull(), "bounds": NSNull(), "position": NSNull()]
        layer.addSublayer(image)
        overlay.canvas = self
        overlay.isOpaque = false
        overlay.backgroundColor = .clear
        overlay.isUserInteractionEnabled = false
        overlay.contentMode = .redraw
        addSubview(overlay)
        // Deux doigts touchés ensemble : annuler le dernier trait (quand on dessine au stylet).
        let undo = UITapGestureRecognizer(target: self, action: #selector(twoFingerTap))
        undo.numberOfTouchesRequired = 2
        undo.cancelsTouchesInView = false
        addGestureRecognizer(undo)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        display?.invalidate()
        display = nil
        if window != nil {
            let d = CADisplayLink(target: Ticker(self), selector: #selector(Ticker.tick))
            d.preferredFramesPerSecond = 60
            d.add(to: .main, forMode: .common)
            display = d
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let pad: CGFloat = 8
        let r = bounds.insetBy(dx: pad, dy: pad)
        guard r.width > 10, r.height > 10 else { return }
        var w = r.width, h = w / aspect
        if h > r.height { h = r.height; w = h * aspect }
        box = CGRect(x: r.midX - w / 2, y: r.midY - h / 2, width: w, height: h).integral
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        image.frame = box
        CATransaction.commit()
        overlay.frame = bounds
        overlay.setNeedsDisplay()
        model.setCanvasWidth(Int(box.width * (window?.screen.scale ?? 2)))
    }

    // ------------------------------------------------------------------ à chaque image

    fileprivate func tick() {
        flush()
        if link.previews != shownPreviews, let img = link.preview {
            shownPreviews = link.previews
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            image.contents = img
            CATransaction.commit()
        }
        if model.stickersRev != shownStickers {
            shownStickers = model.stickersRev
            overlayDirty = true
        }
        let now = CACurrentMediaTime()
        for (k, s) in strokes {
            if let end = s.end, link.previews - s.seen >= 3 || now - end > 1.5 {
                strokes[k] = nil
                overlayDirty = true
            }
        }
        if overlayDirty {
            overlayDirty = false
            overlay.setNeedsDisplay()
        }
    }

    /// Points pas encore envoyés, gomme, photo déplacée : partent une fois par image.
    private func flush() {
        for (id, pts) in outbox where !pts.isEmpty {
            link.send(["t": "move", "id": id, "pts": pts])
        }
        outbox.removeAll(keepingCapacity: true)
        if var e = eraser, e.dirty {
            link.send(["t": "erase", "x": e.x, "y": e.y, "r": e.r])
            e.dirty = false
            eraser = e
        }
        if stDirty, let s = model.selected {
            link.send(["t": "stmove", "id": s.id, "x": s.x, "y": s.y, "s": s.s, "r": s.r])
            stDirty = false
        }
    }

    // ------------------------------------------------------------------ toucher

    private func norm(_ t: UITouch, _ loc: CGPoint? = nil) -> [Double] {
        let p = loc ?? t.location(in: self)
        var pressure = 0.5
        if t.type == .pencil, t.maximumPossibleForce > 0 { pressure = Double(t.force / t.maximumPossibleForce) }
        return [Double((p.x - box.minX) / box.width), Double((p.y - box.minY) / box.height), min(1, max(0, pressure))]
    }

    private func pt(_ p: [Double]) -> CGPoint { CGPoint(x: p[0], y: p[1]) }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        for t in touches {
            if t.type == .pencil { penSeen = true }
            let p = norm(t), id = ObjectIdentifier(t)
            if model.tab == .chem {
                chemTouches[id] = ChemTouch(p: p, t: t.timestamp, poured: p, sent: 0)
                if model.chem.ing != "stir" { pour(p, 0, 0) }
                continue
            }
            // Onglet Photos : toucher une photo la saisit (un second doigt la tourne et
            // l'agrandit) ; toucher ailleurs dessine.
            if model.tab == .photos {
                let s = fingers.isEmpty ? sticker(at: p) : nil
                if !fingers.isEmpty || s != nil {
                    if let s { model.selSticker = s.id }
                    fingers[id] = pt(p)
                    fingerOrder.append(id)
                    startGrab()
                    continue
                }
            }
            if penSeen && t.type != .pencil { continue }
            if model.draw.tool == "erase" {
                eraser = Eraser(x: p[0], y: p[1], r: model.draw.eraserSize, touch: id, dirty: true)
                overlayDirty = true
                continue
            }
            let d = model.draw
            let s = Stroke(id: Self.nextId, p: p, color: UIColor(hex: d.brush == "rainbow" ? "#ffffff" : d.color),
                           w: d.strokeWidth, kind: d.tool)
            Self.nextId += 1
            strokes[id] = s
            if s.kind == "free" { sendDown(s, p) }
            overlayDirty = true
        }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        for t in touches {
            let id = ObjectIdentifier(t)
            if chemTouches[id] != nil { chemMove(t, id); continue }
            if fingers[id] != nil {
                fingers[id] = pt(norm(t))
                moveGrab()
                continue
            }
            if var e = eraser, e.touch == id {
                let p = norm(t)
                (e.x, e.y, e.dirty) = (p[0], p[1], true)
                eraser = e
                overlayDirty = true
                continue
            }
            guard let s = strokes[id], s.end == nil else { continue }
            if s.kind != "free" {
                s.cur = norm(t)
                overlayDirty = true
                continue
            }
            for c in event?.coalescedTouches(for: t) ?? [t] {
                let p = norm(c, c.location(in: self)), last = s.pts[s.pts.count - 1]
                if hypot((p[0] - last[0]) * box.width, (p[1] - last[1]) * box.height) < 1.5 { continue }
                s.pts.append(p)
                outbox[s.id, default: []].append(p.map { ($0 * 10000).rounded() / 10000 })
            }
            overlayDirty = true
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { finish(touches) }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { finish(touches) }

    private func finish(_ touches: Set<UITouch>) {
        for t in touches {
            let id = ObjectIdentifier(t)
            if chemTouches.removeValue(forKey: id) != nil { continue }
            if fingers.removeValue(forKey: id) != nil {
                fingerOrder.removeAll { $0 == id }
                if fingers.isEmpty {
                    flush()
                    grab = nil
                    model.grabId = nil
                } else {
                    startGrab()
                }
                continue
            }
            if let e = eraser, e.touch == id {
                flush()
                eraser = nil
                overlayDirty = true
                continue
            }
            guard let s = strokes[id], s.end == nil else { continue }
            if s.kind != "free" {
                // Forme : envoyée d'un coup au lâcher.
                let pts = shapePoints(s.kind, s.start, s.cur)
                s.pts = pts
                sendDown(s, pts[0])
                var i = 1
                while i < pts.count {
                    link.send(["t": "move", "id": s.id, "pts": Array(pts[i..<min(pts.count, i + 60)])])
                    i += 60
                }
            }
            flush()
            link.send(["t": "up", "id": s.id])
            s.end = CACurrentMediaTime()
            s.seen = link.previews
            overlayDirty = true
        }
    }

    @objc private func twoFingerTap() {
        guard penSeen, model.tab == .draw || model.tab == .media else { return }
        link.send(["t": "undo"])
    }

    private func sendDown(_ s: Stroke, _ p: [Double]) {
        let d = model.draw
        link.send(["t": "down", "id": s.id, "x": p[0], "y": p[1], "p": p.count > 2 ? p[2] : 0.5,
                   "color": d.color, "w": d.strokeWidth, "brush": d.brush, "anim": d.anim, "sym": d.sym,
                   "life": d.life, "out": d.out])
    }

    /// Forme (ligne, rectangle, cercle) : points du contour, en coordonnées 0..1.
    private func shapePoints(_ kind: String, _ a: [Double], _ b: [Double]) -> [[Double]] {
        let W = Double(box.width), H = Double(box.height)
        func lerp(_ p: [Double], _ q: [Double], _ k: Double) -> [Double] {
            [p[0] + (q[0] - p[0]) * k, p[1] + (q[1] - p[1]) * k, 0.5]
        }
        var pts: [[Double]] = []
        switch kind {
        case "line":
            for k in 0...40 { pts.append(lerp(a, b, Double(k) / 40)) }
        case "rect":
            let c = [[a[0], a[1]], [b[0], a[1]], [b[0], b[1]], [a[0], b[1]], [a[0], a[1]]]
            for s in 0..<4 { for k in 0..<20 { pts.append(lerp(c[s], c[s + 1], Double(k) / 20)) } }
            pts.append([a[0], a[1], 0.5])
        default:
            // Cercle : du centre (premier appui) jusqu'au doigt.
            let r = hypot((b[0] - a[0]) * W, (b[1] - a[1]) * H)
            for k in 0...96 {
                let t = Double(k) / 96 * .pi * 2
                pts.append([a[0] + cos(t) * r / W, a[1] + sin(t) * r / H, 0.5])
            }
        }
        return pts
    }

    // ------------------------------------------------------------------ photos du dessin

    private func sticker(at p: [Double]) -> StickerInfo? {
        let W = Double(box.width), H = Double(box.height)
        for s in model.stickers.reversed() {
            let dx = (p[0] - s.x) * W, dy = (p[1] - s.y) * H, a = -s.r * .pi / 180
            let lx = dx * cos(a) - dy * sin(a), ly = dx * sin(a) + dy * cos(a)
            if abs(lx) <= s.s * H * s.ar / 2 && abs(ly) <= s.s * H / 2 { return s }
        }
        return nil
    }

    private func startGrab() {
        guard let s = model.selected, !fingers.isEmpty else {
            grab = nil
            model.grabId = nil
            return
        }
        grab = Grab(id: s.id, from: s, pts: fingerOrder.compactMap { fingers[$0] })
        model.grabId = s.id
    }

    private func moveGrab() {
        guard let g = grab, let k = model.stickers.firstIndex(where: { $0.id == g.id }) else { return }
        let now = fingerOrder.compactMap { fingers[$0] }
        let W = box.width, H = box.height
        func centroid(_ p: [CGPoint]) -> CGPoint {
            CGPoint(x: p.map(\.x).reduce(0, +) / CGFloat(p.count), y: p.map(\.y).reduce(0, +) / CGFloat(p.count))
        }
        let c0 = centroid(g.pts), c1 = centroid(now)
        var s = model.stickers[k]
        s.x = g.from.x + Double(c1.x - c0.x)
        s.y = g.from.y + Double(c1.y - c0.y)
        if g.pts.count >= 2 && now.count >= 2 {
            let v0 = CGVector(dx: (g.pts[1].x - g.pts[0].x) * W, dy: (g.pts[1].y - g.pts[0].y) * H)
            let v1 = CGVector(dx: (now[1].x - now[0].x) * W, dy: (now[1].y - now[0].y) * H)
            s.s = min(3, max(0.03, g.from.s * Double(hypot(v1.dx, v1.dy) / max(1, hypot(v0.dx, v0.dy)))))
            s.r = g.from.r + Double(atan2(v1.dy, v1.dx) - atan2(v0.dy, v0.dx)) * 180 / .pi
        }
        model.stickers[k] = s
        stDirty = true
        overlayDirty = true
    }

    // ------------------------------------------------------------------ chimie

    private func pour(_ p: [Double], _ dx: Double, _ dy: Double, _ ing: String? = nil) {
        let c = model.chem
        if !model.chemUsed { model.chemUsed = true }
        let r4 = { (v: Double) in (v * 10000).rounded() / 10000 }
        let r3 = { (v: Double) in (v * 1000).rounded() / 1000 }
        link.chem("drop", ["ing": ing ?? c.ing, "x": r4(p[0]), "y": r4(p[1]), "size": r4(c.radius(p[2])),
                           "color": c.color, "dx": r3(dx), "dy": r3(dy)])
    }

    private func chemMove(_ t: UITouch, _ id: ObjectIdentifier) {
        guard var s = chemTouches[id] else { return }
        let p = norm(t), now = t.timestamp
        let dt = max(0.001, now - s.t)
        // Mouvement du doigt en hauteurs d'image par seconde (y vers le bas).
        let a = Double(box.width / box.height)
        let dx = (p[0] - s.p[0]) * a / dt, dy = (p[1] - s.p[1]) / dt
        s.p = p
        s.t = now
        defer { chemTouches[id] = s }
        guard now - s.sent >= 0.025 else { return }
        s.sent = now
        if model.chem.ing == "stir" {
            pour(p, dx * 0.5, dy * 0.5, "stir")
            return
        }
        // Traînée : une goutte chaque fois que le doigt a avancé d'un demi-rayon.
        let far = hypot((p[0] - s.poured[0]) * a, p[1] - s.poured[1])
        if far >= model.chem.radius(p[2]) * 0.5 {
            pour(p, dx * 0.25, dy * 0.25)
            s.poured = p
        } else {
            pour(p, dx * 0.5, dy * 0.5, "stir")
        }
    }

    // ------------------------------------------------------------------ dessin de la couche du dessus

    fileprivate func drawOverlay(_ ctx: CGContext) {
        let W = box.width, H = box.height
        ctx.saveGState()
        ctx.translateBy(x: box.minX, y: box.minY)
        ctx.clip(to: CGRect(x: 0, y: 0, width: W, height: H))
        // Axes de symétrie.
        let sym = model.draw.sym
        if (model.tab == .draw || model.tab == .media) && sym > 1 {
            ctx.setStrokeColor(UIColor.white.withAlphaComponent(0.14).cgColor)
            ctx.setLineWidth(1)
            ctx.setLineDash(phase: 0, lengths: [6, 8])
            let angles: [CGFloat] = sym == 2 ? [.pi / 2] : sym == 4 ? [0, .pi / 2] : (0..<sym).map { .pi * CGFloat($0) / CGFloat(sym) }
            for a in angles {
                let L = W + H
                ctx.move(to: CGPoint(x: W / 2 - cos(a) * L, y: H / 2 - sin(a) * L))
                ctx.addLine(to: CGPoint(x: W / 2 + cos(a) * L, y: H / 2 + sin(a) * L))
            }
            ctx.strokePath()
            ctx.setLineDash(phase: 0, lengths: [])
        }
        // Cadres des photos.
        if model.tab == .photos {
            for s in model.stickers {
                ctx.saveGState()
                ctx.translateBy(x: CGFloat(s.x) * W, y: CGFloat(s.y) * H)
                ctx.rotate(by: CGFloat(s.r) * .pi / 180)
                let h = CGFloat(s.s) * H, w = h * CGFloat(s.ar)
                let sel = s.id == model.selSticker
                ctx.setStrokeColor(sel ? UIColor(Theme.accent).cgColor : UIColor.white.withAlphaComponent(0.35).cgColor)
                ctx.setLineWidth(sel ? 3 : 1.5)
                ctx.setLineDash(phase: 0, lengths: [10, 8])
                ctx.stroke(CGRect(x: -w / 2, y: -h / 2, width: w, height: h))
                ctx.restoreGState()
            }
        }
        // Traits en cours.
        ctx.setLineCap(.round)
        ctx.setLineJoin(.round)
        ctx.setAlpha(0.8)
        for s in strokes.values {
            ctx.setStrokeColor(s.color.cgColor)
            ctx.setFillColor(s.color.cgColor)
            let pts = s.end == nil && s.kind != "free" ? shapePoints(s.kind, s.start, s.cur) : s.pts
            if pts.count == 1 {
                let a = pts[0], r = max(1, CGFloat(s.w) * H * (0.35 + 1.3 * CGFloat(a[2]))) / 2
                ctx.fillEllipse(in: CGRect(x: CGFloat(a[0]) * W - r, y: CGFloat(a[1]) * H - r, width: r * 2, height: r * 2))
                continue
            }
            for i in 1..<pts.count {
                let a = pts[i - 1], b = pts[i]
                ctx.setLineWidth(max(1, CGFloat(s.w) * H * (0.35 + 1.3 * CGFloat(b[2]))))
                ctx.move(to: CGPoint(x: CGFloat(a[0]) * W, y: CGFloat(a[1]) * H))
                ctx.addLine(to: CGPoint(x: CGFloat(b[0]) * W, y: CGFloat(b[1]) * H))
                ctx.strokePath()
            }
        }
        ctx.setAlpha(1)
        if let e = eraser {
            ctx.setStrokeColor(UIColor.white.withAlphaComponent(0.8).cgColor)
            ctx.setLineWidth(2)
            ctx.setLineDash(phase: 0, lengths: [4, 4])
            let r = CGFloat(e.r) * H
            ctx.strokeEllipse(in: CGRect(x: CGFloat(e.x) * W - r, y: CGFloat(e.y) * H - r, width: r * 2, height: r * 2))
        }
        ctx.restoreGState()
    }

    private final class Overlay: UIView {
        weak var canvas: CanvasView?
        override func draw(_ rect: CGRect) {
            guard let ctx = UIGraphicsGetCurrentContext() else { return }
            canvas?.drawOverlay(ctx)
        }
    }

    /// Cible de l'horloge d'affichage (sans retenir la toile).
    private final class Ticker: NSObject {
        weak var view: CanvasView?
        init(_ v: CanvasView) { view = v }
        @objc func tick(_ d: CADisplayLink) {
            guard let view else {
                d.invalidate()
                return
            }
            view.tick()
        }
    }
}
