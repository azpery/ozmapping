import SwiftUI

/// Bouton qui agit dès qu'on le touche (et au relâchement pour ceux qu'on tient) : le tempo
/// tapé tombe juste, le noir tenu dure tant que le doigt reste.
struct PressButton<Label: View>: View {
    var hold = false
    var on = false
    var hot = false
    var height: CGFloat = 54
    let press: () -> Void
    var release: () -> Void = {}
    @ViewBuilder let label: () -> Label
    @GestureState private var down = false
    @State private var held = false

    var body: some View {
        label()
            .font(.system(size: 15, weight: .semibold))
            .lineLimit(1)
            .minimumScaleFactor(0.7)
            .padding(.horizontal, 10)
            .frame(maxWidth: .infinity, minHeight: height)
            .foregroundStyle(hot || (hold && held) ? .white : on ? Theme.accentInk : Theme.text)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(hot || (hold && held) ? Theme.rec : on ? Theme.accent : Theme.panel2))
            .scaleEffect(down ? 0.95 : 1)
            .animation(.easeOut(duration: 0.1), value: down)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .updating($down) { _, state, _ in state = true }
                    .onChanged { _ in
                        if !held {
                            held = true
                            press()
                        }
                    }
                    .onEnded { _ in
                        held = false
                        if hold { release() }
                    }
            )
    }
}

/// Curseur relié à un réglage de l'app : la valeur part au fil du geste ; l'état reçu ne le
/// fait pas sauter pendant qu'on le touche.
struct LiveSlider: View {
    @EnvironmentObject var link: Link
    let label: String
    let action: Any
    let value: Double
    @State private var local = 0.0
    @State private var editing = false
    @State private var lastEdit = Date.distantPast

    var body: some View {
        NamedSlider(label: label, value: Binding(get: { local }, set: { v in
            local = v
            lastEdit = Date()
            link.val(action, v)
        }), onEditing: { e in
            editing = e
            lastEdit = Date()
        })
        .onAppear { local = value }
        .onChange(of: value) { v in
            if !editing && Date().timeIntervalSince(lastEdit) > 0.8 { local = v }
        }
    }
}

/// Carte d'une rubrique.
private struct Card<Content: View>: View {
    let title: String
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title.uppercased())
                .font(.system(size: 11, weight: .semibold))
                .kerning(0.7)
                .foregroundStyle(Theme.weak)
                .lineLimit(1)
                .padding(.top, 10)
            content()
        }
        .padding(.horizontal, 10)
        .padding(.bottom, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(Theme.card))
    }
}

/// Onglet Live : tout l'écran, des commandes du spectacle rangées en colonnes (chacune défile
/// seule). En haut, toujours sous le doigt : noir, tempo, enregistrement.
struct LivePanel: View {
    @EnvironmentObject var link: Link

    var body: some View {
        if let s = link.live {
            VStack(spacing: 8) {
                header(s)
                GeometryReader { g in
                    let n = max(1, min(3, Int((g.size.width - 20) / 300)))
                    HStack(alignment: .top, spacing: 10) {
                        ForEach(0..<n, id: \.self) { c in
                            ScrollView {
                                VStack(spacing: 10) {
                                    ForEach(groups(n)[c], id: \.self) { g in group(g, s) }
                                }
                                .padding(.bottom, 20)
                            }
                        }
                    }
                    .padding(.horizontal, 10)
                }
            }
            .background(Theme.bg)
        } else {
            VStack(spacing: 12) {
                ProgressView().tint(Theme.weak)
                Text("En attente d'OZ Mapping…").foregroundStyle(Theme.weak)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    /// Rubriques : la scène à gauche, les effets au milieu, le contenu (lecture, visuels,
    /// lumières) à droite ; sur un écran plus étroit, deux colonnes, puis une seule.
    private func groups(_ n: Int) -> [[Int]] {
        switch n {
        case 1: return [[0, 1, 2]]
        case 2: return [[0, 1], [2]]
        default: return [[0], [1], [2]]
        }
    }

    @ViewBuilder
    private func group(_ g: Int, _ s: LiveState) -> some View {
        switch g {
        case 0: VStack(spacing: 10) { show(s); scene(s); layers(s); board(s) }
        case 1: VStack(spacing: 10) { effects(s); presets(s) }
        default: VStack(spacing: 10) { playback(s); visuals(s); lights(s) }
        }
    }

    // ------------------------------------------------------------------ en haut

    private func header(_ s: LiveState) -> some View {
        HStack(spacing: 8) {
            PressButton(hot: s.blackout, press: { link.act("Blackout") }) { Text("BLACKOUT") }
                .frame(maxWidth: 170)
            PressButton(hold: true, press: { link.act("BlackoutHold", on: true) }, release: { link.act("BlackoutHold", on: false) }) {
                Text("Noir tenu")
            }
            .frame(maxWidth: 130)
            TapButton(state: s)
            PressButton(height: 54, press: { link.act("TempoHalf") }) { Text("÷2") }.frame(width: 60)
            PressButton(height: 54, press: { link.act("TempoDouble") }) { Text("×2") }.frame(width: 60)
            PressButton(hot: s.recording, press: { link.act("Record") }) {
                Label(s.recording ? "REC" : "REC", systemImage: "record.circle")
            }
            .frame(maxWidth: 110)
        }
        .padding(.horizontal, 10)
        .padding(.top, 4)
    }

    // ------------------------------------------------------------------ scène, calques, tableau

    private func named(_ s: LiveState, _ i: Int) -> String {
        guard s.layers.indices.contains(i) else { return "" }
        return (s.layers[i].sketch ? "✏︎ " : "") + s.layers[i].name
    }

    /// Ce que montre la sortie : tout, ou un seul genre de calque ; et la sortie de l'iPad.
    private func show(_ s: LiveState) -> some View {
        Card(title: "Sortie : ce qu'on voit") {
            LazyVGrid(columns: chipColumns(96), spacing: 6) {
                ForEach(LivePanel.modes, id: \.0) { m, label, icon in
                    Button { link.act(A.mode(m)) } label: { Label(label, systemImage: icon) }
                        .buttonStyle(Chip(on: s.mode == m, height: 48))
                        // Dessin et chimie : le calque est créé s'il n'existe pas.
                        .disabled(!(m == "All" || m == "Drawing" || m == "Chemistry" || s.kinds.contains(m)))
                }
            }
            if s.outputs.count > 1 {
                Text("L'IPAD PROJETTE SUR")
                    .font(.system(size: 10, weight: .semibold))
                    .kerning(0.6)
                    .foregroundStyle(Theme.weak)
                    .padding(.top, 6)
                LazyVGrid(columns: chipColumns(96), spacing: 6) {
                    ForEach(s.outputs.indices, id: \.self) { k in
                        Button(s.outputs[k].isEmpty ? "Sortie \(k + 1)" : s.outputs[k]) { link.act(A.output(k)) }
                            .buttonStyle(Chip(on: s.output == k))
                    }
                }
            }
        }
    }

    static let groupEffects: [(String, String)] = [
        ("Off", "Aucun"), ("Chase", "Chenillard"), ("Bounce", "Aller-retour"), ("Wave", "Vague"), ("Random", "Aléatoire"),
        ("Fill", "Remplissage"), ("Alternate", "Alterné"), ("ColorCycle", "Couleurs tournantes"), ("Meter", "VU-mètre (son)"),
    ]

    static let modes: [(String, String, String)] = [
        ("All", "Tout", "square.stack.3d.up.fill"), ("Drawing", "Dessin", "scribble"),
        ("Chemistry", "Chimie", "drop.fill"), ("Media", "Médias", "film"), ("Visual", "Visuel généré", "sparkles"),
    ]

    private func scene(_ s: LiveState) -> some View {
        Card(title: "Scène") {
            LazyVGrid(columns: chipColumns(80), spacing: 6) {
                Button("Blanc") { link.act("White") }.buttonStyle(Chip(on: s.white))
                Button("Sans effets") { link.act("Bypass") }.buttonStyle(Chip(on: s.bypass))
                Button("Mire") { link.act("Grid") }.buttonStyle(Chip(on: s.grid))
            }
            LiveSlider(label: "Luminosité", action: "Master", value: s.master)
        }
    }

    private func layers(_ s: LiveState) -> some View {
        Card(title: "Calques") {
            // Du premier plan vers le fond, comme dans l'app ; un groupe en tête de ses calques.
            ForEach(Array(s.layers.indices.reversed()), id: \.self) { i in
                let l = s.layers[i]
                if l.group != 0, i + 1 == s.layers.count || s.layers[i + 1].group != l.group,
                   let g = s.groups.first(where: { $0.id == l.group }) {
                    HStack(spacing: 6) {
                        Image(systemName: "folder.fill").foregroundStyle(Theme.accent)
                        Text(g.name).font(.system(size: 14, weight: .semibold)).lineLimit(1)
                        Spacer()
                        // Effet de groupe : les calques s'allument tour à tour.
                        Menu {
                            ForEach(LivePanel.groupEffects, id: \.0) { k, name in
                                Button { link.act(A.groupFx(g.id, k == "Off" ? (g.fx.isEmpty ? "Off" : g.fx) : k)) } label: {
                                    if g.fx == k || (k == "Off" && (g.fx.isEmpty || g.fx == "Off")) { Label(name, systemImage: "checkmark") } else { Text(name) }
                                }
                            }
                        } label: {
                            Label(LivePanel.groupEffects.first { $0.0 == g.fx }?.1 ?? "Effet", systemImage: "sparkles")
                                .font(.system(size: 13, weight: .semibold))
                                .padding(.horizontal, 10)
                                .frame(height: 40)
                                .background(RoundedRectangle(cornerRadius: 10).fill(g.fx.isEmpty || g.fx == "Off" ? Theme.panel2 : Theme.accent))
                                .foregroundStyle(g.fx.isEmpty || g.fx == "Off" ? Theme.text : Theme.accentInk)
                        }
                        Button { link.act(A.groupVis(g.id)) } label: {
                            Label(g.visible ? "Groupe visible" : "Groupe caché", systemImage: g.visible ? "eye" : "eye.slash")
                        }
                        .buttonStyle(Chip(on: g.visible))
                        .frame(width: 170)
                    }
                    .padding(.top, 4)
                }
                HStack(spacing: 6) {
                    if l.group != 0 {
                        RoundedRectangle(cornerRadius: 2).fill(Theme.accent.opacity(0.6)).frame(width: 3, height: 34)
                    }
                    Button(named(s, i)) { link.act(A.layer(i)) }
                        .buttonStyle(Chip(on: s.sel == i))
                        .frame(width: 120)
                    Button { link.act(A.vis(i)) } label: {
                        Image(systemName: l.visible ? "eye" : "eye.slash")
                    }
                    .buttonStyle(Chip())
                    .frame(width: 44)
                    .opacity(l.visible ? 1 : 0.5)
                    LiveSliderBare(action: A.opacity(i), value: l.opacity)
                }
            }
        }
    }

    private func board(_ s: LiveState) -> some View {
        Card(title: "Tableau") {
            LazyVGrid(columns: chipColumns(100), spacing: 6) {
                ForEach(s.columns.indices, id: \.self) { k in
                    Button(s.columns[k].name) { link.act(A.col(k)) }
                        .buttonStyle(Chip(on: s.columns[k].active, height: 46))
                        .disabled(!s.columns[k].filled)
                }
            }
            Button("Enchaîner les colonnes") { link.act("Chain") }.buttonStyle(Chip(on: s.chain))
        }
    }

    // ------------------------------------------------------------------ effets

    private func effects(_ s: LiveState) -> some View {
        Card(title: "Effets · \(named(s, s.sel))") {
            LazyVGrid(columns: chipColumns(96), spacing: 6) {
                Button("Au hasard") { link.act("RandomEffect") }.buttonStyle(Chip())
                Button("AUTO") { link.act("AutoFx") }.buttonStyle(Chip(on: s.auto))
                Button("Casser !") { link.act("Shatter") }.buttonStyle(Chip())
                Button("Sans effet") { link.act("ClearEffects") }.buttonStyle(Chip())
                Button("Suit le rythme") { link.act("React") }.buttonStyle(Chip(on: s.react))
            }
            DisclosureGroup("Réglages des effets") {
                ForEach(s.fx.indices, id: \.self) { k in
                    LiveSlider(label: s.fx[k].name, action: A.fx(k), value: s.fx[k].v)
                }
            }
            .font(.system(size: 13))
            .foregroundStyle(Theme.weak)
            .padding(.top, 4)
        }
    }

    private func presets(_ s: LiveState) -> some View {
        Card(title: "Presets d'effets · \(named(s, s.sel))") {
            Families(items: s.presets) { p in
                Button(p.name) { link.act(A.preset(p.name)) }.buttonStyle(Chip(on: p.name == s.preset, height: 44))
            }
        }
    }

    // ------------------------------------------------------------------ lecture, visuels, lumières

    private func playback(_ s: LiveState) -> some View {
        Card(title: "Lecture · \(named(s, s.content))") {
            LazyVGrid(columns: chipColumns(90), spacing: 6) {
                Button { link.act("PrevMedia") } label: { Label("Média", systemImage: "backward.end.fill") }.buttonStyle(Chip())
                Button { link.act("NextMedia") } label: { Label("Média", systemImage: "forward.end.fill") }.buttonStyle(Chip())
                Button { link.act("Pause") } label: { Label("Pause", systemImage: "pause.fill") }.buttonStyle(Chip(on: s.paused))
                Button("À l'envers") { link.act("Reverse") }.buttonStyle(Chip(on: s.reverse))
                Button("Mix auto") { link.act("MixAuto") }.buttonStyle(Chip(on: s.mix))
            }
            LiveSlider(label: "Vitesse", action: "Speed", value: (log(max(0.01, s.speed)) / log(4) + 1) / 2)
        }
    }

    private func visuals(_ s: LiveState) -> some View {
        Card(title: "Visuels générés · \(named(s, s.sel))") {
            Button("Aucun") { link.act(A.visual("Off")) }
                .buttonStyle(Chip(on: s.visual.isEmpty || s.visual == "Off"))
            Families(items: s.visuals) { v in
                Button(v.name) { link.act(A.visual(v.id)) }.buttonStyle(Chip(on: v.key == s.visual, height: 44))
            }
            LiveSlider(label: "Part du visuel", action: "VisualMix", value: s.visualMix)
        }
    }

    @ViewBuilder
    private func lights(_ s: LiveState) -> some View {
        if s.ledsHas {
            Card(title: "LEDs") {
                HStack(spacing: 6) {
                    Button("Envoi aux LEDs") { link.act("LedLive") }.buttonStyle(Chip(on: s.ledLive))
                    Button("Enchaîner") { link.act("LedChain") }.buttonStyle(Chip(on: s.ledChain))
                }
                LazyVGrid(columns: chipColumns(100), spacing: 6) {
                    ForEach(s.scenarios.indices, id: \.self) { k in
                        let sc = s.scenarios[k]
                        Button(sc.name) { link.act(A.scen(sc.id)) }.buttonStyle(Chip(on: sc.key == s.ledScenario))
                    }
                }
                LiveSlider(label: "Luminosité", action: "LedBrightness", value: s.ledBrightness)
            }
        }
        if !s.dmx.isEmpty {
            Card(title: "DMX") {
                LazyVGrid(columns: chipColumns(110), spacing: 6) {
                    ForEach(s.dmx.indices, id: \.self) { k in
                        let d = s.dmx[k]
                        if d.kind == "Smoke" || d.kind == "Bubble" {
                            PressButton(hold: true, press: { link.act(A.hold(d.id), on: true) },
                                        release: { link.act(A.hold(d.id), on: false) }) { Text("\(d.name) (tenir)") }
                        } else {
                            Button(d.name) { link.act(A.tog(d.id)) }.buttonStyle(Chip(on: d.on, height: 54))
                        }
                    }
                }
            }
        }
        if s.lasersHas {
            Card(title: "Lasers") {
                HStack(spacing: 6) {
                    Button("Armer") { link.act("LaserArm") }.buttonStyle(Chip(on: s.armed))
                    Button("STOP") { link.act("LaserStop") }.buttonStyle(Chip(hot: true))
                }
                LazyVGrid(columns: chipColumns(100), spacing: 6) {
                    ForEach(s.cues.indices, id: \.self) { i in
                        Button(s.cues[i].isEmpty ? "Réglage \(i + 1)" : s.cues[i]) { link.act(A.cue(i)) }.buttonStyle(Chip())
                    }
                }
            }
        }
    }
}

/// Curseur sans nom (opacité d'un calque).
struct LiveSliderBare: View {
    @EnvironmentObject var link: Link
    let action: Any
    let value: Double
    @State private var local = 0.0
    @State private var editing = false
    @State private var lastEdit = Date.distantPast

    var body: some View {
        Slider(value: Binding(get: { local }, set: { v in
            local = v
            lastEdit = Date()
            link.val(action, v)
        }), onEditingChanged: { e in
            editing = e
            lastEdit = Date()
        })
        .tint(Theme.accent)
        .onAppear { local = value }
        .onChange(of: value) { v in
            if !editing && Date().timeIntervalSince(lastEdit) > 0.8 { local = v }
        }
    }
}

/// Liste rangée par familles (déjà dans l'ordre) : un petit titre au début de chaque famille.
private struct Families<B: View>: View {
    let items: [LiveState.Item]
    @ViewBuilder let button: (LiveState.Item) -> B

    var body: some View {
        let groups = Self.split(items)
        ForEach(groups.indices, id: \.self) { g in
            Text(groups[g].0.uppercased())
                .font(.system(size: 10, weight: .semibold))
                .kerning(0.6)
                .foregroundStyle(Theme.weak)
                .padding(.top, 6)
            LazyVGrid(columns: chipColumns(104), spacing: 6) {
                ForEach(groups[g].1.indices, id: \.self) { k in button(groups[g].1[k]) }
            }
        }
    }

    private static func split(_ items: [LiveState.Item]) -> [(String, [LiveState.Item])] {
        var out: [(String, [LiveState.Item])] = []
        for it in items {
            if out.last?.0 == it.family { out[out.count - 1].1.append(it) } else { out.append((it.family, [it])) }
        }
        return out
    }
}

/// TAP : bat au tempo d'OZ Mapping, et compte dès qu'on le touche.
private struct TapButton: View {
    @EnvironmentObject var link: Link
    let state: LiveState

    var body: some View {
        TimelineView(.animation) { t in
            let ph = state.phaseNow(t.date)
            let k = exp(-ph * 60 / max(30, state.bpm) * 7)
            PressButton(press: { link.act("Tap") }) {
                Text("TAP · \(Int(state.bpm.rounded())) BPM")
            }
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Theme.accent.opacity(0.85 * k))
                    .allowsHitTesting(false)
            )
            .overlay(
                Text("TAP · \(Int(state.bpm.rounded())) BPM")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Theme.accentInk.opacity(k > 0.5 ? 1 : 0))
                    .allowsHitTesting(false)
            )
        }
        .frame(minWidth: 170)
    }
}
