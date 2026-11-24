import SwiftUI

/// Une connexion à un ordinateur : la liaison et les réglages vivent le temps de cette vue.
struct SessionView: View {
    @StateObject private var model: Model
    @Environment(\.scenePhase) private var phase
    var onChangeComputer: () -> Void

    init(url: URL, onChangeComputer: @escaping () -> Void) {
        _model = StateObject(wrappedValue: Model(url: url))
        self.onChangeComputer = onChangeComputer
    }

    var body: some View {
        MainView(onChangeComputer: onChangeComputer)
            .environmentObject(model)
            .environmentObject(model.link)
            .onAppear {
                model.link.onFail = onChangeComputer
                model.start()
            }
            .onDisappear { model.stop() }
            .onChange(of: phase) { p in
                // En arrière-plan : on se déconnecte (l'ordinateur arrête d'envoyer l'aperçu).
                if p == .active { model.start() } else if p == .background { model.stop() }
            }
    }
}

struct MainView: View {
    @EnvironmentObject var model: Model
    @EnvironmentObject var link: Link
    var onChangeComputer: () -> Void

    var body: some View {
        GeometryReader { g in
            let landscape = g.size.width >= g.size.height
            VStack(spacing: 0) {
                TopBar(onChangeComputer: onChangeComputer)
                if model.tab == .room {
                    RoomPanel()
                } else if model.tab == .live {
                    LivePanel()
                } else if landscape {
                    HStack(spacing: 0) {
                        stage
                        if !model.full {
                            SidePanel()
                                .frame(width: min(400, g.size.width * 0.36))
                                .transition(.move(edge: .trailing))
                        }
                    }
                } else {
                    VStack(spacing: 0) {
                        stage
                        if !model.full {
                            SidePanel()
                                .frame(height: g.size.height * 0.44)
                                .transition(.move(edge: .bottom))
                        }
                    }
                }
            }
            .animation(.easeInOut(duration: 0.22), value: model.full)
        }
        .background(Theme.bg.ignoresSafeArea())
        .foregroundStyle(Theme.text)
        .tint(Theme.accent)
    }

    private var stage: some View {
        HStack(spacing: 0) {
            if model.tab == .chem {
                ChemQuickBar()
            } else {
                QuickBar()
            }
            CanvasRep(model: model, aspect: link.aspect)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// Barre du haut : état de la connexion, onglets, changement d'ordinateur.
struct TopBar: View {
    @EnvironmentObject var model: Model
    @EnvironmentObject var link: Link
    var onChangeComputer: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            HStack(spacing: 7) {
                Circle()
                    .fill(link.connected ? Theme.accent : Theme.warn)
                    .frame(width: 9, height: 9)
                Text(link.status)
                    .font(.system(size: 13))
                    .foregroundStyle(Theme.weak)
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Picker("Onglet", selection: $model.tab) {
                ForEach(Tab.allCases) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented)
            .frame(maxWidth: 540)
            // Sortie où l'iPad projette (s'il y en a plusieurs).
            if let s = link.live, s.outputs.count > 1 {
                Menu {
                    ForEach(s.outputs.indices, id: \.self) { k in
                        Button { link.act(A.output(k)) } label: {
                            if s.output == k { Label(outName(s, k), systemImage: "checkmark") } else { Text(outName(s, k)) }
                        }
                    }
                } label: {
                    Label(outName(s, s.output), systemImage: "display")
                        .font(.system(size: 13, weight: .semibold))
                        .lineLimit(1)
                        .padding(.horizontal, 10)
                        .frame(height: 32)
                        .background(Capsule().fill(Theme.panel2))
                }
            }
            Button(action: onChangeComputer) {
                Image(systemName: "desktopcomputer")
                    .font(.system(size: 17))
                    .foregroundStyle(Theme.weak)
                    .frame(width: 40, height: 34)
            }
            .accessibilityLabel("Changer d'ordinateur")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(Theme.bg)
    }
}

private func outName(_ s: LiveState, _ k: Int) -> String {
    guard s.outputs.indices.contains(k) else { return "Sortie \(k + 1)" }
    return s.outputs[k].isEmpty ? "Sortie \(k + 1)" : s.outputs[k]
}

/// Panneau de réglages à côté de la toile.
struct SidePanel: View {
    @EnvironmentObject var model: Model

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                switch model.tab {
                case .draw: DrawPanel()
                case .photos: PhotosPanel()
                case .chem: ChemPanel()
                case .media: MediaPanel()
                case .live, .room: EmptyView()
                }
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 24)
        }
        // Chaque onglet repart du haut de son panneau.
        .id(model.tab)
        .scrollDismissesKeyboard(.immediately)
        .background(Theme.panel)
        .overlay(alignment: .leading) { Rectangle().fill(Theme.line).frame(width: 1) }
    }
}

// ======================================================================= barre rapide

/// Bouton de la barre rapide : montre le réglage et ouvre un petit menu.
private struct QuickButton<Label: View, Menu: View>: View {
    let open: Bool
    let tap: () -> Void
    @ViewBuilder let label: () -> Label
    @ViewBuilder let menu: () -> Menu
    var dismiss: () -> Void

    var body: some View {
        Button(action: tap) {
            label()
                .font(.system(size: 11))
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .minimumScaleFactor(0.8)
                .frame(width: 54, height: 48)
                .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Theme.panel2.opacity(0.95)))
                .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(open ? Theme.accent : .clear, lineWidth: 2))
        }
        .buttonStyle(.plain)
        .popover(isPresented: Binding(get: { open }, set: { if !$0 { dismiss() } }), arrowEdge: .leading) {
            ScrollView {
                VStack(alignment: .leading, spacing: 6) { menu() }
                    .padding(12)
            }
            .frame(width: 320)
            .frame(maxHeight: 420)
            .background(Theme.panel)
            .foregroundStyle(Theme.text)
            .preferredColorScheme(.dark)
        }
    }
}

/// Toujours sur le bord de la toile : couleur, pinceau, épaisseur, outil, disparition, animation,
/// symétrie, annuler.
struct QuickBar: View {
    @EnvironmentObject var model: Model
    @State private var open: String?

    private static let short: [String: String] = [
        "free": "Main", "line": "Ligne", "rect": "Rect.", "circle": "Cercle", "erase": "Gomme",
        "rainbow": "Arc-en-\nciel", "sparkle": "Étin-\ncelles", "spray": "Aéro-\ngraphe", "dots": "Poin-\ntillés",
        "pulse": "Au\ntempo", "blink": "Cli-\ngnote", "trace": "Se\ndessine",
    ]

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(spacing: 6) {
                Spacer(minLength: 0)
                quick("panel") {
                    Image(systemName: model.full ? "sidebar.right" : "arrow.up.left.and.arrow.down.right")
                        .font(.system(size: 17))
                } menu: { EmptyView() }
                quick("color") {
                    Circle().fill(Color(hex: model.draw.color)).frame(width: 26, height: 26)
                        .overlay(Circle().strokeBorder(.white.opacity(0.25), lineWidth: 2))
                } menu: {
                    SectionTitle("Couleur")
                    ColorRow(colors: Opt.colors, hex: $model.draw.color)
                }
                quick("brush") {
                    Text(Self.short[model.draw.brush] ?? Opt.label(Opt.brush, model.draw.brush))
                } menu: {
                    SectionTitle("Pinceau")
                    Choices(options: Opt.brush, value: $model.draw.brush, onPick: { _ in open = nil })
                }
                quick("width") {
                    let d = 4 + model.draw.width / 100 * 24
                    Circle().fill(Color(hex: model.draw.color)).frame(width: d, height: d)
                } menu: {
                    SectionTitle("Épaisseur")
                    Slider(value: $model.draw.width, in: 1...100).tint(Theme.accent)
                }
                quick("tool") {
                    VStack(spacing: 2) {
                        Image(systemName: Opt.toolIcon[model.draw.tool] ?? "scribble").font(.system(size: 15))
                        Text(Self.short[model.draw.tool] ?? "Outil")
                    }
                } menu: {
                    SectionTitle("Outil")
                    Choices(options: Opt.tool, value: $model.draw.tool, icons: Opt.toolIcon, onPick: { _ in open = nil })
                }
                quick("life") {
                    Text(model.draw.life == 0 ? "Reste" : "Efface\n\(model.draw.life) s")
                } menu: {
                    LifeMenu()
                }
                quick("anim") {
                    Text(Self.short[model.draw.anim] ?? Opt.label(Opt.anim, model.draw.anim))
                } menu: {
                    SectionTitle("Animation")
                    Choices(options: Opt.anim, value: $model.draw.anim, onPick: { _ in open = nil })
                }
                quick("sym") {
                    Text(model.draw.sym == 1 ? "Sym." : Opt.label(Opt.sym, model.draw.sym).replacingOccurrences(of: "Kaléido", with: "Kaléi."))
                } menu: {
                    SectionTitle("Symétrie")
                    Choices(options: Opt.sym, value: $model.draw.sym, onPick: { _ in open = nil })
                }
                quick("undo") {
                    VStack(spacing: 2) {
                        Image(systemName: "arrow.uturn.backward").font(.system(size: 15))
                        Text("Annuler")
                    }
                } menu: { EmptyView() }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 6)
        }
        .frame(width: 66)
        .padding(.leading, 6)
    }

    private func quick<L: View, M: View>(_ k: String, @ViewBuilder label: @escaping () -> L,
                                         @ViewBuilder menu: @escaping () -> M) -> some View {
        QuickButton(open: open == k, tap: {
            switch k {
            case "panel": model.full.toggle()
            case "undo": model.link.send(["t": "undo"])
            default: open = open == k ? nil : k
            }
        }, label: label, menu: menu, dismiss: { if open == k { open = nil } })
    }
}

/// Disparition : après combien de temps, et comment.
struct LifeMenu: View {
    @EnvironmentObject var model: Model

    var body: some View {
        SectionTitle("S'efface après")
        Choices(options: Opt.life, value: $model.draw.life, min: 58)
        SectionTitle("En disparaissant")
        Choices(options: Opt.out, value: $model.draw.out, onPick: { v in
            // Comète sans durée : une seconde de traînée.
            if v == "trail" && model.draw.life == 0 { model.draw.life = 1 }
        })
    }
}

/// Barre rapide de la chimie : ingrédient, couleur, taille, secouer, vider.
struct ChemQuickBar: View {
    @EnvironmentObject var model: Model
    @State private var open: String?

    var body: some View {
        ScrollView(showsIndicators: false) {
            VStack(spacing: 6) {
                Spacer(minLength: 0)
                quick("panel") {
                    Image(systemName: model.full ? "sidebar.right" : "arrow.up.left.and.arrow.down.right").font(.system(size: 17))
                } menu: { EmptyView() }
                quick("ing") {
                    VStack(spacing: 2) {
                        Image(systemName: Opt.chemIcon[model.chem.ing] ?? "drop.fill").font(.system(size: 16))
                        Text(ingName(model.chem.ing))
                    }
                } menu: {
                    IngredientGrid(onPick: { open = nil })
                }
                quick("color") {
                    Circle().fill(Color(hex: model.chem.color)).frame(width: 26, height: 26)
                        .overlay(Circle().strokeBorder(.white.opacity(0.25), lineWidth: 2))
                } menu: {
                    SectionTitle("Couleur")
                    ColorRow(colors: Opt.chemColors, hex: $model.chem.color)
                }
                quick("size") {
                    let d = 6 + model.chem.size / 100 * 26
                    Circle().fill(Color(hex: model.chem.color).opacity(0.85)).frame(width: d, height: d)
                } menu: {
                    SectionTitle("Taille des gouttes")
                    Slider(value: $model.chem.size, in: 1...100).tint(Theme.accent)
                }
                quick("shake") {
                    VStack(spacing: 2) {
                        Image(systemName: "waveform.path").font(.system(size: 15))
                        Text("Secouer")
                    }
                } menu: { EmptyView() }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 6)
        }
        .frame(width: 66)
        .padding(.leading, 6)
    }

    private func ingName(_ k: String) -> String {
        Opt.chemFamilies.flatMap(\.1).first { $0.0 == k }?.1 ?? ""
    }

    private func quick<L: View, M: View>(_ k: String, @ViewBuilder label: @escaping () -> L,
                                         @ViewBuilder menu: @escaping () -> M) -> some View {
        QuickButton(open: open == k, tap: {
            switch k {
            case "panel": model.full.toggle()
            case "shake": model.link.chem("shake", ["amount": 1])
            default: open = open == k ? nil : k
            }
        }, label: label, menu: menu, dismiss: { if open == k { open = nil } })
    }
}
