import SwiftUI
import UIKit

/// Couleurs d'OZ Mapping (les mêmes que l'app de l'ordinateur et la page web).
enum Theme {
    static let bg = Color(hex: "#0b0d12")
    static let panel = Color(hex: "#161a22")
    static let panel2 = Color(hex: "#1f2430")
    static let panel3 = Color(hex: "#2a3040")
    static let card = Color(hex: "#11141b")
    static let line = Color(hex: "#252b38")
    static let text = Color(hex: "#e8eaf0")
    static let weak = Color(hex: "#8a91a3")
    static let accent = Color(hex: "#5ad19a")
    static let accentInk = Color(hex: "#062014")
    static let warn = Color(hex: "#ff6b6b")
    static let rec = Color(hex: "#ff3b4f")
}

extension Color {
    init(hex: String) { self.init(uiColor: UIColor(hex: hex)) }
}

extension UIColor {
    convenience init(hex: String) {
        var s = hex.trimmingCharacters(in: .whitespacesAndNewlines)
        if s.hasPrefix("#") { s.removeFirst() }
        let v = UInt32(s, radix: 16) ?? 0xffffff
        self.init(red: CGFloat((v >> 16) & 0xff) / 255, green: CGFloat((v >> 8) & 0xff) / 255,
                  blue: CGFloat(v & 0xff) / 255, alpha: 1)
    }

    var hex: String {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        getRed(&r, green: &g, blue: &b, alpha: &a)
        let c = { (v: CGFloat) in Int((min(1, max(0, v)) * 255).rounded()) }
        return String(format: "#%02x%02x%02x", c(r), c(g), c(b))
    }
}

/// Bouton-pastille : gris, ou vert quand il est choisi / actif.
struct Chip: ButtonStyle {
    var on = false
    var height: CGFloat = 40
    var danger = false
    /// Rouge plein (blackout, stop).
    var hot = false
    var font: Font = .system(size: 14)

    func makeBody(configuration: Configuration) -> some View {
        ChipBody(configuration: configuration, style: self)
    }

    private struct ChipBody: View {
        let configuration: Configuration
        let style: Chip
        @Environment(\.isEnabled) private var enabled

        var body: some View {
            let fill: Color = style.hot ? Theme.rec : style.on ? Theme.accent : Theme.panel2
            let ink: Color = style.hot ? .white : style.on ? Theme.accentInk : style.danger ? Theme.warn : Theme.text
            configuration.label
                .font(style.font.weight(style.on || style.hot ? .semibold : .regular))
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .minimumScaleFactor(0.7)
                .padding(.horizontal, 8)
                .frame(maxWidth: .infinity, minHeight: style.height)
                .foregroundStyle(ink)
                .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(fill))
                .contentShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                .scaleEffect(configuration.isPressed ? 0.95 : 1)
                .brightness(configuration.isPressed ? 0.08 : 0)
                .opacity(enabled ? 1 : 0.35)
                .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
                .animation(.easeOut(duration: 0.15), value: style.on)
        }
    }
}

/// Petit titre de rubrique.
struct SectionTitle: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text.uppercased())
            .font(.system(size: 11, weight: .semibold))
            .kerning(0.7)
            .foregroundStyle(Theme.weak)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 14)
            .padding(.bottom, 2)
    }
}

/// Explication en petit.
struct Hint: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.system(size: 12))
            .foregroundStyle(Theme.weak)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 4)
    }
}

func chipColumns(_ min: CGFloat = 86) -> [GridItem] { [GridItem(.adaptive(minimum: min), spacing: 6)] }

/// Un choix parmi des boutons (pinceau, symétrie…).
struct Choices<V: Hashable>: View {
    let options: [(V, String)]
    @Binding var value: V
    var min: CGFloat = 86
    var icons: [V: String] = [:]
    var onPick: ((V) -> Void)? = nil

    var body: some View {
        LazyVGrid(columns: chipColumns(min), spacing: 6) {
            ForEach(options.indices, id: \.self) { i in
                let (v, label) = options[i]
                Button {
                    value = v
                    onPick?(v)
                } label: {
                    if let icon = icons[v] {
                        Label(label, systemImage: icon)
                    } else {
                        Text(label)
                    }
                }
                .buttonStyle(Chip(on: value == v))
            }
        }
    }
}

/// Curseur avec son nom.
struct NamedSlider: View {
    let label: String
    @Binding var value: Double
    var range: ClosedRange<Double> = 0...1
    var onEditing: (Bool) -> Void = { _ in }

    var body: some View {
        HStack(spacing: 10) {
            Text(label)
                .font(.system(size: 13))
                .foregroundStyle(Theme.weak)
                .lineLimit(1)
                .frame(width: 112, alignment: .leading)
            Slider(value: $value, in: range, onEditingChanged: onEditing)
                .tint(Theme.accent)
        }
        .padding(.top, 4)
    }
}

/// Pastilles de couleur et nuancier.
struct ColorRow: View {
    let colors: [String]
    @Binding var hex: String

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 38, maximum: 46), spacing: 6)], alignment: .leading, spacing: 6) {
            ForEach(colors, id: \.self) { c in
                Button {
                    hex = c
                } label: {
                    Circle()
                        .fill(Color(hex: c))
                        .overlay(Circle().strokeBorder(.white.opacity(0.18), lineWidth: 2))
                        .padding(3)
                        .overlay(Circle().strokeBorder(hex.lowercased() == c ? Theme.text : .clear, lineWidth: 2.5))
                        .frame(width: 38, height: 38)
                }
                .buttonStyle(.plain)
            }
            ColorPicker("", selection: Binding(get: { Color(hex: hex) }, set: { hex = UIColor($0).hex }), supportsOpacity: false)
                .labelsHidden()
                .frame(width: 38, height: 38)
        }
    }
}
