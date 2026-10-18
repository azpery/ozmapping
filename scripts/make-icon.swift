// Génère l'icône de l'app (PNG 1024×1024) : fond sombre arrondi + pictogramme Phosphor.
// Usage : swift scripts/make-icon.swift assets/Phosphor.ttf sortie.png

import AppKit
import CoreText

let args = CommandLine.arguments
guard args.count == 3 else {
    FileHandle.standardError.write("usage: make-icon.swift <Phosphor.ttf> <sortie.png>\n".data(using: .utf8)!)
    exit(1)
}
let fontURL = URL(fileURLWithPath: args[1])
let outURL = URL(fileURLWithPath: args[2])

var error: Unmanaged<CFError>?
CTFontManagerRegisterFontsForURL(fontURL as CFURL, .process, &error)

let size: CGFloat = 1024
let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()

// Fond : carré arrondi (gabarit d'icône macOS : marge de 100 px) avec dégradé.
let tile = NSRect(x: 100, y: 100, width: 824, height: 824)
let path = NSBezierPath(roundedRect: tile, xRadius: 185, yRadius: 185)
NSGradient(
    starting: NSColor(srgbRed: 0.13, green: 0.15, blue: 0.19, alpha: 1),
    ending: NSColor(srgbRed: 0.05, green: 0.06, blue: 0.075, alpha: 1)
)!.draw(in: path, angle: -90)
NSColor(srgbRed: 0.31, green: 1.0, blue: 0.63, alpha: 0.35).setStroke()
path.lineWidth = 6
path.stroke()

// Pictogramme : écran de projection (Phosphor U+E658), vert accent.
let font = NSFont(name: "Phosphor", size: 540) ?? NSFont.systemFont(ofSize: 540)
let glyph = NSAttributedString(
    string: "\u{E658}",
    attributes: [
        .font: font,
        .foregroundColor: NSColor(srgbRed: 0.31, green: 1.0, blue: 0.63, alpha: 1),
    ]
)
let bounds = glyph.boundingRect(with: NSSize(width: size, height: size), options: [.usesLineFragmentOrigin])
glyph.draw(at: NSPoint(x: (size - bounds.width) / 2 - bounds.minX, y: (size - bounds.height) / 2 - bounds.minY))

image.unlockFocus()

let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
try! rep.representation(using: .png, properties: [:])!.write(to: outURL)
