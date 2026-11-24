import PhotosUI
import SwiftUI

// ======================================================================= Dessin

struct DrawPanel: View {
    @EnvironmentObject var model: Model
    @EnvironmentObject var link: Link
    @State private var saving = false
    @State private var confirmClear = false
    @State private var replaceSlot: Int?

    var body: some View {
        SectionTitle("Outil")
        Choices(options: Opt.tool, value: $model.draw.tool, min: 100, icons: Opt.toolIcon)
        SectionTitle("Couleur")
        ColorRow(colors: Opt.colors, hex: $model.draw.color)
        SectionTitle("Pinceau")
        Choices(options: Opt.brush, value: $model.draw.brush)
        NamedSlider(label: "Épaisseur", value: $model.draw.width, range: 1...100)
        LifeMenu()
        SectionTitle("Animation")
        Choices(options: Opt.anim, value: $model.draw.anim)
        SectionTitle("Symétrie")
        Choices(options: Opt.sym, value: $model.draw.sym, min: 70)

        SectionTitle("Mémoires")
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 36), spacing: 6)], spacing: 6) {
            ForEach(0..<8, id: \.self) { i in
                let full = link.slots.indices.contains(i) && link.slots[i]
                Button {
                    if saving {
                        saving = false
                        if full { replaceSlot = i } else { link.send(["t": "save", "slot": i]) }
                    } else if full {
                        link.send(["t": "recall", "slot": i])
                    }
                } label: {
                    Text("\(i + 1)")
                        .overlay(alignment: .topTrailing) {
                            Circle().fill(full ? Theme.accent : .clear).frame(width: 6, height: 6).offset(x: 9, y: -9)
                        }
                }
                .buttonStyle(Chip(on: false))
            }
        }
        Button(saving ? "Toucher une mémoire pour y garder le dessin" : "Garder le dessin dans une mémoire…") {
            saving.toggle()
        }
        .buttonStyle(Chip(hot: saving))
        .padding(.top, 6)
        Hint("Toucher une mémoire la rappelle ; « Garder » puis une mémoire y range le dessin actuel.")

        SectionTitle("Corrections")
        HStack(spacing: 6) {
            Button { link.send(["t": "undo"]) } label: { Label("Annuler le dernier trait", systemImage: "arrow.uturn.backward") }
                .buttonStyle(Chip())
            Button { confirmClear = true } label: { Label("Tout effacer", systemImage: "trash") }
                .buttonStyle(Chip(danger: true))
        }
        Hint("Au stylet : touchez la toile à deux doigts pour annuler le dernier trait.")

        SectionTitle("Sur l'iPad, montrer")
        Choices(options: Opt.show, value: $model.draw.show, min: 140)
        Hint(model.chemUsed
             ? "Le bassin de la chimie est en cours : la toile montre toute la sortie, pour dessiner par-dessus."
             : "« Toute la sortie » montre aussi les vidéos, visuels et effets, mais demande plus de travail à l'ordinateur.")

        SectionTitle("Calque du dessin")
        Button("Dessin sans effet") { link.act("DrawingPlain") }.buttonStyle(Chip())
        Hint("Retire les effets et le visuel posés sur le calque du dessin. Dans Live, effets et visuels vont au calque choisi dans « Calques » : choisissez « Dessin » pour agir sur le dessin.")
            .confirmationDialog("Effacer tout le dessin (traits et photos) ?", isPresented: $confirmClear, titleVisibility: .visible) {
                Button("Tout effacer", role: .destructive) { link.send(["t": "clear"]) }
            }
            .confirmationDialog("Remplacer la mémoire \((replaceSlot ?? 0) + 1) par le dessin actuel ?",
                                isPresented: Binding(get: { replaceSlot != nil }, set: { if !$0 { replaceSlot = nil } }),
                                titleVisibility: .visible) {
                Button("Remplacer", role: .destructive) {
                    if let i = replaceSlot { link.send(["t": "save", "slot": i]) }
                    replaceSlot = nil
                }
            }
    }
}

// ======================================================================= Photos du dessin

/// Photo choisie dans la photothèque ou prise à l'appareil, remise droite et réduite.
func loadPhoto(_ item: PhotosPickerItem, max side: CGFloat) async -> UIImage? {
    guard let data = try? await item.loadTransferable(type: Data.self), let img = UIImage(data: data) else { return nil }
    return img.normalized(max: side)
}

extension UIImage {
    /// Orientation appliquée, au plus `max` pixels de côté.
    func normalized(max side: CGFloat) -> UIImage {
        let k = min(1, side / Swift.max(size.width, size.height))
        let target = CGSize(width: Swift.max(1, (size.width * k).rounded()), height: Swift.max(1, (size.height * k).rounded()))
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        return UIGraphicsImageRenderer(size: target, format: format).image { _ in draw(in: CGRect(origin: .zero, size: target)) }
    }
}

struct PhotosPanel: View {
    @EnvironmentObject var model: Model
    @EnvironmentObject var link: Link
    @State private var cutItem: PhotosPickerItem?
    @State private var wholeItem: PhotosPickerItem?
    @State private var camera: CameraPicker.Purpose?
    @State private var editing: UIImage?
    @State private var msg = ""

    var body: some View {
        SectionTitle("Ajouter une photo de l'iPad")
        HStack(spacing: 6) {
            PhotosPicker(selection: $cutItem, matching: .images) {
                Label("Détourer au stylet", systemImage: "scissors")
            }
            .buttonStyle(Chip(height: 54))
            PhotosPicker(selection: $wholeItem, matching: .images) {
                Label("Photo entière", systemImage: "photo")
            }
            .buttonStyle(Chip(height: 54))
        }
        if UIImagePickerController.isSourceTypeAvailable(.camera) {
            HStack(spacing: 6) {
                Button { camera = .cut } label: { Label("Prendre et détourer", systemImage: "camera") }
                    .buttonStyle(Chip())
                Button { camera = .whole } label: { Label("Prendre une photo", systemImage: "camera.fill") }
                    .buttonStyle(Chip())
            }
            .padding(.top, 6)
        }
        Hint("« Détourer » : la photo s'ouvre en grand avec le détourage automatique comme brouillon ; entourez, gommez, puis Valider.")
        if !msg.isEmpty || !link.notice.isEmpty {
            Text(msg.isEmpty ? link.notice : msg)
                .font(.system(size: 13))
                .foregroundStyle(Theme.accent)
                .padding(.top, 8)
        }
        Hint("Sur la toile : un doigt déplace une photo, deux doigts l'agrandissent et la tournent. Toucher ailleurs dessine.")

        ObjectMapping()

        StickerControls()
            .onChange(of: cutItem) { item in
                guard let item else { return }
                cutItem = nil
                Task {
                    msg = "Ouverture de la photo…"
                    if let img = await loadPhoto(item, max: 1600) { editing = img; msg = "" } else { msg = "Photo illisible" }
                }
            }
            .onChange(of: wholeItem) { item in
                guard let item else { return }
                wholeItem = nil
                Task {
                    msg = "Préparation de la photo…"
                    if let img = await loadPhoto(item, max: 2048) { await sendWhole(img) } else { msg = "Photo illisible" }
                }
            }
            .onChange(of: link.notice) { _ in msg = "" }
            .fullScreenCover(item: $camera) { purpose in
                CameraPicker { img in
                    camera = nil
                    guard let img else { return }
                    let photo = img.normalized(max: purpose == .cut ? 1600 : 2048)
                    if purpose == .cut { editing = photo } else { Task { await sendWhole(photo) } }
                }
                .ignoresSafeArea()
            }
            .fullScreenCover(isPresented: Binding(get: { editing != nil }, set: { if !$0 { editing = nil } })) {
                if let img = editing {
                    CutEditorView(photo: img) { png in
                        editing = nil
                        if let png { Task { await send(png, query: "to=drawing&cut=0&ext=png") } }
                    }
                    .environmentObject(link)
                }
            }
    }

    private func sendWhole(_ img: UIImage) async {
        guard let jpeg = img.jpegData(compressionQuality: 0.9) else { return }
        await send(jpeg, query: "to=drawing&cut=0")
    }

    private func send(_ data: Data, query: String) async {
        msg = "Envoi (\(data.count / 1024) Ko)…"
        model.pendingSelect = model.stickers.count
        do {
            _ = try await link.post("/upload", query: query, body: data)
            msg = "Elle arrive sur la toile"
        } catch {
            msg = "Échec de l'envoi : \(error.localizedDescription)"
        }
    }
}

/// Mapper un objet : on le photographie (la sortie en blanc l'éclaire et montre où tombe
/// l'image projetée), on le détoure au stylet, et l'ordinateur pose la photo et son masque sur
/// le calque choisi, calés sur l'objet.
struct ObjectMapping: View {
    @EnvironmentObject var link: Link
    @AppStorage("object-white") private var white = true
    @State private var camera = false
    @State private var item: PhotosPickerItem?
    @State private var editing: (UIImage, Bool)?
    @State private var turnedWhite = false
    @State private var msg = ""

    var body: some View {
        SectionTitle("Mapper un objet")
        Toggle(isOn: $white) {
            Text("Sortie en blanc pendant la photo").font(.system(size: 14))
        }
        .tint(Theme.accent)
        HStack(spacing: 6) {
            if UIImagePickerController.isSourceTypeAvailable(.camera) {
                Button {
                    if white && !(link.live?.white ?? false) {
                        link.act("White")
                        turnedWhite = true
                    }
                    camera = true
                } label: { Label("Photographier l'objet", systemImage: "camera.viewfinder") }
                    .buttonStyle(Chip(height: 54))
            }
            PhotosPicker(selection: $item, matching: .images) {
                Label("Choisir une photo", systemImage: "photo")
            }
            .buttonStyle(Chip(height: 54))
        }
        if let target = link.live.flatMap({ s in s.layers.indices.contains(s.sel) ? s.layers[s.sel].name : nil }) {
            Hint("Sur le calque choisi : « \(target) ».")
        }
        if !msg.isEmpty {
            Text(msg).font(.system(size: 13)).foregroundStyle(Theme.accent).padding(.top, 6)
        }
        Hint("Placez-vous au plus près du vidéoprojecteur et cadrez tout l'objet avec la zone éclairée : l'ordinateur y retrouve l'image projetée et cale le masque exactement sur l'objet. Détourez ensuite au stylet (Entourer, Retirer, Pinceau, Gomme).")
            .onChange(of: item) { new in
                guard let new else { return }
                item = nil
                Task {
                    if let img = await loadPhoto(new, max: 1600) { editing = (img, false) } else { msg = "Photo illisible" }
                }
            }
            .fullScreenCover(isPresented: $camera) {
                CameraPicker { img in
                    camera = false
                    if turnedWhite {
                        link.act("White")
                        turnedWhite = false
                    }
                    if let img { editing = (img.normalized(max: 1600), white) }
                }
                .ignoresSafeArea()
            }
            .fullScreenCover(isPresented: Binding(get: { editing != nil }, set: { if !$0 { editing = nil } })) {
                if let (img, calibrate) = editing {
                    CutEditorView(object: img) { cut in
                        editing = nil
                        if let cut { Task { await send(cut, calibrate: calibrate) } }
                    }
                    .environmentObject(link)
                }
            }
            .onChange(of: link.notice) { n in if n.contains("mapp") { msg = n } }
    }

    private func send(_ cut: ObjectCut, calibrate: Bool) async {
        var body = Data()
        var n = UInt32(cut.jpeg.count).bigEndian
        body.append(Data(bytes: &n, count: 4))
        body.append(cut.jpeg)
        body.append(cut.mask)
        msg = "Envoi (\(body.count / 1024) Ko)…"
        do {
            _ = try await link.post("/object", query: "photo=\(cut.withPhoto ? 1 : 0)&calibrate=\(calibrate ? 1 : 0)", body: body)
            msg = "Reçu par l'ordinateur : mapping…"
        } catch {
            msg = "Échec de l'envoi : \(error.localizedDescription)"
        }
    }
}

/// Réglages de la photo choisie sur la toile.
struct StickerControls: View {
    @EnvironmentObject var model: Model
    @EnvironmentObject var link: Link

    var body: some View {
        let s = model.selected
        let _ = model.stickersRev
        VStack(alignment: .leading, spacing: 0) {
            SectionTitle(s == nil ? "Photo choisie : touchez une photo sur la toile" : "Photo choisie")
            LazyVGrid(columns: chipColumns(60), spacing: 6) {
                Button { model.moveSelected { $0.s = max(0.03, $0.s / 1.15) } } label: { Image(systemName: "minus.magnifyingglass") }
                Button { model.moveSelected { $0.s = min(3, $0.s * 1.15) } } label: { Image(systemName: "plus.magnifyingglass") }
                Button { model.moveSelected { $0.r -= 15 } } label: { Image(systemName: "rotate.left") }
                Button { model.moveSelected { $0.r += 15 } } label: { Image(systemName: "rotate.right") }
                Button { set(["flip": !(s?.flip ?? false)]) } label: { Image(systemName: "arrow.left.and.right.righttriangle.left.righttriangle.right") }
            }
            .buttonStyle(Chip())
            SectionTitle("Animation")
            Choices(options: Opt.stickerAnim, value: Binding(get: { s?.anim ?? "" }, set: { set(["anim": $0]) }))
            SectionTitle("Contour lumineux")
            Choices(options: [(false, "Aucun"), (true, "De la couleur du dessin")],
                    value: Binding(get: { s?.outline ?? false }, set: { set(["outline": $0 ? model.draw.color : ""]) }), min: 130)
            SectionTitle("Symétrie")
            Choices(options: Opt.sym, value: Binding(get: { s?.sym ?? 0 }, set: { set(["sym": $0]) }), min: 70)
            HStack(spacing: 6) {
                Button { if let s { link.send(["t": "stdup", "id": s.id]) } } label: { Label("Dupliquer", systemImage: "plus.square.on.square") }
                    .buttonStyle(Chip())
                Button { if let s { link.send(["t": "stdel", "id": s.id]) } } label: { Label("Supprimer", systemImage: "trash") }
                    .buttonStyle(Chip(danger: true))
            }
            .padding(.top, 14)
        }
        .disabled(s == nil)
    }

    private func set(_ fields: JSON) {
        guard let s = model.selected else { return }
        var m = fields
        m["t"] = "stset"
        m["id"] = s.id
        link.send(m)
    }
}

/// Appareil photo de l'iPad.
struct CameraPicker: UIViewControllerRepresentable {
    enum Purpose: Identifiable { case cut, whole; var id: Self { self } }
    var done: (UIImage?) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(done: done) }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let c = UIImagePickerController()
        c.sourceType = .camera
        c.delegate = context.coordinator
        return c
    }

    func updateUIViewController(_ c: UIImagePickerController, context: Context) {}

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let done: (UIImage?) -> Void
        init(done: @escaping (UIImage?) -> Void) { self.done = done }
        func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            done(info[.originalImage] as? UIImage)
        }
        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { done(nil) }
    }
}

// ======================================================================= Chimie

/// Ingrédients, rangés par familles.
struct IngredientGrid: View {
    @EnvironmentObject var model: Model
    var onPick: () -> Void = {}

    var body: some View {
        ForEach(Opt.chemFamilies, id: \.0) { family, items in
            SectionTitle(family)
            Choices(options: items, value: $model.chem.ing, min: 104, icons: Opt.chemIcon, onPick: { _ in onPick() })
        }
    }
}

struct ChemPanel: View {
    @EnvironmentObject var model: Model
    @EnvironmentObject var link: Link
    @State private var confirmClear = false

    var body: some View {
        IngredientGrid()
        Button {
            model.chem.boltBeat.toggle()
            link.chem("boltbeat", ["on": model.chem.boltBeat])
        } label: {
            Label("Foudre sur le rythme", systemImage: "metronome")
        }
        .buttonStyle(Chip(on: model.chem.boltBeat))
        .padding(.top, 6)

        SectionTitle("Couleur")
        ColorRow(colors: Opt.chemColors, hex: $model.chem.color)
        NamedSlider(label: "Taille des gouttes", value: $model.chem.size, range: 1...100)
        NamedSlider(label: "Se renouvelle", value: $model.chem.renew, range: 0...100)
            .onChange(of: model.chem.renew) { r in link.chem("renew", ["rate": r / 100]) }
        SectionTitle("Le bassin vibre avec")
        Choices(options: [("app", "Le son de l'ordinateur"), ("ipad", "Le micro de l'iPad"), ("off", "Rien")],
                value: Binding(get: { model.chem.listens }, set: { model.setListen($0) }), min: 104,
                icons: ["app": "desktopcomputer", "ipad": "mic.fill", "off": "speaker.slash"])
        NamedSlider(label: "Sensibilité", value: Binding(get: { model.chem.sound ?? 50 }, set: { model.chem.sound = $0 }), range: 0...100)
            .onChange(of: model.chem.sound) { v in link.chem("sound", ["amount": (v ?? 50) / 100]) }
        Hint("Se renouvelle : à 0, rien ne s'efface ; plus haut, les vieilles couleurs, l'huile et l'essence s'estompent peu à peu. La surface frémit avec le son, comme un bassin posé sur un haut-parleur : celui de l'ordinateur (son Rythme), ou celui qu'entend l'iPad, pour suivre une autre musique que l'ordinateur.")

        SectionTitle("Fond du bassin")
        Choices(options: Opt.chemBase, value: $model.chem.base, min: 130, icons: Opt.chemBaseIcon,
                onPick: { b in link.chem("base", ["base": b]) })
        Hint("Huile de lin : l'encre, le colorant et l'alcool y éclatent en doigts. Bulle de savon : soufflez vers l'iPad pour l'agiter.")

        SectionTitle("Pencher, secouer, souffler")
        HStack(spacing: 6) {
            Button { model.setMotion(!model.chem.motion) } label: {
                Label(model.motionOn ? "Pencher : activé" : "Pencher l'iPad", systemImage: "gyroscope")
            }
            .buttonStyle(Chip(on: model.motionOn, height: 50))
            Button { model.setMic(!model.chem.mic) } label: {
                Label(model.micOn && model.chem.mic ? "Souffle : écoute" : "Souffler", systemImage: "wind")
            }
            .buttonStyle(Chip(on: model.micOn && model.chem.mic, height: 50))
        }
        HStack(spacing: 6) {
            Button { link.chem("shake", ["amount": 1]) } label: { Label("Secouer", systemImage: "waveform.path") }
                .buttonStyle(Chip())
            Button { model.chem.flip.toggle() } label: { Label("Inverser la pente", systemImage: "arrow.up.arrow.down") }
                .buttonStyle(Chip(on: model.chem.flip))
        }
        .padding(.top, 6)
        Hint(model.sensorMsg.isEmpty
             ? "Penchez l'iPad pour faire couler, secouez pour agiter. Si le liquide part du mauvais côté : « Inverser la pente ». Le son du micro n'est ni enregistré ni envoyé, seulement son niveau."
             : model.sensorMsg)

        SectionTitle("Vider")
        Button { confirmClear = true } label: { Label("Tout vider", systemImage: "trash") }
            .buttonStyle(Chip(danger: true))
        Hint("Toucher la toile verse l'ingrédient choisi ; glisser en verse une traînée. « Touiller » remue sans rien verser. Essayez : de l'encre puis du savon ; du lait, quelques couleurs de colorant, puis une goutte de savon ; de l'huile colorée puis penchez l'iPad ; de l'essence sur l'eau noire.")
            .confirmationDialog("Vider le bassin ?", isPresented: $confirmClear, titleVisibility: .visible) {
                Button("Tout vider", role: .destructive) {
                    link.chem("clear")
                    model.chemUsed = false
                }
            }
    }
}

// ======================================================================= Médias

struct MediaPanel: View {
    @EnvironmentObject var model: Model
    @EnvironmentObject var link: Link
    @State private var item: PhotosPickerItem?
    @State private var msg = ""

    var body: some View {
        let s = link.live
        SectionTitle("Médias de l'app")
        if let s, !s.media.isEmpty {
            // Rangés par dossier (ceux ajoutés un par un d'abord) ; un titre à chaque dossier.
            let sections = Self.sections(s.media)
            ForEach(sections.indices, id: \.self) { g in
                if !sections[g].0.isEmpty {
                    Label(sections[g].0, systemImage: "folder.fill")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Theme.weak)
                        .padding(.top, 10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 112), spacing: 8)], spacing: 8) {
                ForEach(sections[g].1, id: \.self) { i in
                    Button { link.act(A.media(i)) } label: {
                        VStack(spacing: 0) {
                            Thumb(index: i, name: s.media[i].name)
                            Text(s.media[i].name)
                                .font(.system(size: 11))
                                .lineLimit(1)
                                .padding(.horizontal, 6)
                                .padding(.vertical, 5)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .background(Theme.panel2)
                        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .strokeBorder(s.playing == i ? Theme.accent : .clear, lineWidth: 3))
                    }
                    .buttonStyle(.plain)
                }
            }
            }
        } else {
            Hint(s == nil ? "En attente d'OZ Mapping…" : "Aucun média dans l'app pour l'instant.")
        }
        Hint("Toucher un média l'envoie sur le calque sélectionné (le dessin reste par-dessus).")

        SectionTitle("Photo de l'iPad sur le calque")
        PhotosPicker(selection: $item, matching: .images) {
            Label("Envoyer une photo", systemImage: "square.and.arrow.up")
        }
        .buttonStyle(Chip(height: 54))
        .onChange(of: item) { new in
            guard let new else { return }
            item = nil
            Task { await upload(new) }
        }
        if let s, s.layers.indices.contains(s.sel) {
            Hint("Elle arrive sur le calque sélectionné : « \(s.layers[s.sel].name) ».")
        }
        if !msg.isEmpty {
            Text(msg).font(.system(size: 13)).foregroundStyle(Theme.accent).padding(.top, 8)
        }
    }

    /// Index des médias regroupés par dossier, dans l'ordre d'arrivée.
    static func sections(_ media: [LiveState.Media]) -> [(String, [Int])] {
        var out: [(String, [Int])] = []
        for (i, m) in media.enumerated() {
            if let k = out.firstIndex(where: { $0.0 == m.folder }) { out[k].1.append(i) } else { out.append((m.folder, [i])) }
        }
        return out
    }

    private func upload(_ item: PhotosPickerItem) async {
        msg = "Préparation de la photo…"
        guard let img = await loadPhoto(item, max: 2048), let jpeg = img.jpegData(compressionQuality: 0.9) else {
            msg = "Photo illisible"
            return
        }
        msg = "Envoi (\(jpeg.count / 1024) Ko)…"
        do {
            _ = try await link.post("/upload", query: "to=layer", body: jpeg)
            msg = "Envoyée sur le calque ✓"
        } catch {
            msg = "Échec de l'envoi : \(error.localizedDescription)"
        }
    }
}

/// Miniature d'un média (demandée à l'ordinateur).
struct Thumb: View {
    @EnvironmentObject var link: Link
    let index: Int
    let name: String
    @State private var img: UIImage?

    var body: some View {
        Color.black
            .aspectRatio(16 / 9, contentMode: .fit)
            .overlay {
                if let img {
                    Image(uiImage: img).resizable().scaledToFill()
                } else {
                    ProgressView().tint(Theme.weak)
                }
            }
            .clipped()
            .task(id: "\(index)/\(name)") { img = await link.thumb(index, name: name) }
    }
}
