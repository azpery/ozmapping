import AVFoundation
import CoreMotion
import UIKit

/// Capteurs de l'iPad pour l'onglet Chimie : gravité dans le plan de l'écran (selon son
/// orientation), secousses, et niveau du souffle dans le micro (le son n'est pas gardé).
final class Sensors {
    var onMotion: ((Double, Double, Double) -> Void)?
    var onBlow: ((Double) -> Void)?
    /// Son autour de l'iPad (0..1) : niveau, basses, aigus — pour faire vibrer le bassin.
    var onSound: ((Double, Double, Double) -> Void)?
    var onStatus: ((String, Bool) -> Void)?

    private let motion = CMMotionManager()
    private var shake = 0.0
    private var engine: AVAudioEngine?
    private var level = 0.0
    /// Filtres (basses sous ~150 Hz, aigus au-dessus de ~3 kHz) et plafonds qui s'adaptent
    /// au volume (le niveau d'une salle de concert comme celui d'un salon donne 0..1).
    private var low = Float(0), mid = Float(0)
    private var peaks = (level: 0.02, bass: 0.02, high: 0.01)

    // ------------------------------------------------------------------ mouvement

    func startMotion() {
        guard motion.isDeviceMotionAvailable else {
            onStatus?("motion", false)
            return
        }
        guard !motion.isDeviceMotionActive else { return }
        motion.deviceMotionUpdateInterval = 1.0 / 30.0
        motion.startDeviceMotionUpdates(to: .main) { [weak self] m, _ in
            guard let self, let m else { return }
            let (gx, gy) = Self.toScreen(m.gravity)
            // Secousse : l'accélération propre (sans la gravité), en g.
            let a = m.userAcceleration
            let jolt = (a.x * a.x + a.y * a.y + a.z * a.z).squareRoot()
            self.shake = max(self.shake * 0.85, min(1.0, max(0.0, (jolt - 0.08) * 1.6)))
            self.onMotion?(Self.round(gx), Self.round(gy), Self.round(self.shake))
        }
        onStatus?("motion", true)
    }

    func stopMotion() {
        motion.stopDeviceMotionUpdates()
    }

    /// Gravité (repère de l'appareil : x à droite, y vers le haut en portrait) → plan de
    /// l'écran tel qu'on le voit (y vers le bas), selon l'orientation de l'interface.
    private static func toScreen(_ g: CMAcceleration) -> (Double, Double) {
        let orientation = UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.interfaceOrientation }.first ?? .portrait
        switch orientation {
        case .portraitUpsideDown: return (-g.x, g.y)
        case .landscapeLeft: return (g.y, g.x)     // bouton principal à gauche
        case .landscapeRight: return (-g.y, -g.x)  // bouton principal à droite
        default: return (g.x, -g.y)
        }
    }

    private static func round(_ v: Double) -> Double { (v * 1000).rounded() / 1000 }

    // ------------------------------------------------------------------ souffle

    func startMic() {
        guard engine == nil else { return }
        AVAudioSession.sharedInstance().requestRecordPermission { [weak self] granted in
            DispatchQueue.main.async {
                guard let self else { return }
                guard granted else {
                    self.onStatus?("mic", false)
                    return
                }
                self.runMic()
            }
        }
    }

    private func runMic() {
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.record, mode: .measurement, options: [])
            try session.setActive(true)
            let engine = AVAudioEngine()
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
                guard let data = buffer.floatChannelData?[0] else { return }
                let n = Int(buffer.frameLength)
                var sum: Float = 0
                for i in 0..<n { sum += data[i] * data[i] }
                let rms = Double((sum / Float(max(n, 1))).squareRoot())
                // Un souffle : un bruit fort et continu ; la voix et la salle restent sous le seuil.
                let level = min(1.0, max(0.0, (rms - 0.03) * 9.0))
                // Bandes : passe-bas pour les basses, ce qui reste au-dessus de ~3 kHz pour les aigus.
                let rate = Float(format.sampleRate > 0 ? format.sampleRate : 44100)
                let aLow = 1 - exp(-2 * Float.pi * 150 / rate), aMid = 1 - exp(-2 * Float.pi * 3000 / rate)
                var lo = Float(0), hi = Float(0)
                if let self {
                    var l = self.low, m = self.mid
                    for i in 0..<n {
                        let x = data[i]
                        l += aLow * (x - l)
                        m += aMid * (x - m)
                        lo += l * l
                        hi += (x - m) * (x - m)
                    }
                    self.low = l
                    self.mid = m
                }
                let bass = Double((lo / Float(max(n, 1))).squareRoot()), high = Double((hi / Float(max(n, 1))).squareRoot())
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.level = max(level, self.level * 0.8)
                    if self.level > 0.03 { self.onBlow?(Self.round(self.level)) }
                    // Plafonds qui suivent le volume : montent tout de suite, redescendent en ~10 s.
                    let fall = 0.997
                    self.peaks = (max(rms, self.peaks.level * fall, 0.02), max(bass, self.peaks.bass * fall, 0.015),
                                  max(high, self.peaks.high * fall, 0.008))
                    let k = { (v: Double, p: Double) in Self.round(min(1, max(0, v / p * 1.15 - 0.15))) }
                    self.onSound?(k(rms, self.peaks.level), k(bass, self.peaks.bass), k(high, self.peaks.high))
                }
            }
            try engine.start()
            self.engine = engine
            onStatus?("mic", true)
        } catch {
            onStatus?("mic", false)
        }
    }

    func stopMic() {
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        engine = nil
        try? AVAudioSession.sharedInstance().setActive(false)
    }

    func stopAll() {
        stopMotion()
        stopMic()
    }
}
