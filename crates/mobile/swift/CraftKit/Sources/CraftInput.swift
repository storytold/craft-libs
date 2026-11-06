import UIKit

/// Keyboards: hardware keys as egui key names, and the text the on-screen keyboard types.
enum CraftKeys {
    /// egui's name for a hardware key, or nil for keys the app doesn't know (they're dropped).
    static func name(_ code: UIKeyboardHIDUsage) -> String? {
        let raw = code.rawValue
        switch raw {
        case 0x04...0x1D:  // a–z
            return String(UnicodeScalar(UInt8(0x41 + raw - 0x04)))
        case 0x1E...0x26:  // 1–9
            return String(raw - 0x1E + 1)
        case 0x27:
            return "0"
        case 0x3A...0x45:  // F1–F12
            return "F\(raw - 0x3A + 1)"
        default:
            return named[raw]
        }
    }

    private static let named: [Int: String] = [
        0x28: "Enter", 0x29: "Escape", 0x2A: "Backspace", 0x2B: "Tab", 0x2C: "Space",
        0x2D: "Minus", 0x2E: "Equals", 0x2F: "OpenBracket", 0x30: "CloseBracket", 0x31: "Backslash",
        0x33: "Semicolon", 0x34: "Quote", 0x35: "Backtick", 0x36: "Comma", 0x37: "Period", 0x38: "Slash",
        0x49: "Insert", 0x4A: "Home", 0x4B: "PageUp", 0x4C: "Delete", 0x4D: "End", 0x4E: "PageDown",
        0x4F: "ArrowRight", 0x50: "ArrowLeft", 0x51: "ArrowDown", 0x52: "ArrowUp",
    ]

    /// craft-mobile's modifier bits: 1 shift, 2 control, 4 option, 8 command.
    static func bits(_ flags: UIKeyModifierFlags) -> UInt32 {
        (flags.contains(.shift) ? 1 : 0) | (flags.contains(.control) ? 2 : 0) | (flags.contains(.alternate) ? 4 : 0) | (flags.contains(.command) ? 8 : 0)
    }

    /// Forward hardware key presses. Returns the presses the app didn't take (for `super`).
    /// Command-V pastes from the system clipboard instead of reaching the app as a key.
    @MainActor
    static func forward(_ presses: Set<UIPress>, pressed: Bool) -> Set<UIPress> {
        var rest: Set<UIPress> = []
        for press in presses {
            guard let key = press.key, let name = name(key.keyCode) else {
                rest.insert(press)
                continue
            }
            if name == "V", key.modifierFlags.contains(.command) {
                if pressed { CraftPaste.fromClipboard() }
                continue
            }
            send(name, bytes: name.utf8, pressed: pressed, modifiers: bits(key.modifierFlags))
            // Printable characters still reach the text input, which types them as text; Enter,
            // Backspace and shortcuts don't, or the app would get them twice.
            if typesText(key) { rest.insert(press) }
        }
        return rest
    }

    /// Whether UIKit turns this key into typed text (a printable character, no shortcut modifier).
    @MainActor
    private static func typesText(_ key: UIKey) -> Bool {
        guard !key.modifierFlags.contains(.command), !key.modifierFlags.contains(.control),
              let scalar = key.characters.unicodeScalars.first else { return false }
        return scalar.value >= 0x20 && scalar.value != 0x7F
    }

    private static func send(_ name: String, bytes: String.UTF8View, pressed: Bool, modifiers: UInt32) {
        let data = Array(bytes)
        data.withUnsafeBufferPointer { craft_key($0.baseAddress, $0.count, pressed, modifiers) }
    }

    static func text(_ text: String) {
        let data = Array(text.utf8)
        data.withUnsafeBufferPointer { craft_text($0.baseAddress, $0.count) }
    }

    /// A key typed on the on-screen keyboard that isn't text (backspace): press and release.
    static func tap(_ name: String) {
        send(name, bytes: name.utf8, pressed: true, modifiers: 0)
        send(name, bytes: name.utf8, pressed: false, modifiers: 0)
    }
}

/// Pasting what the user asked to paste (⌘V, or the app's own Edit › Paste through request 10).
/// Reading what another app copied may show iPadOS's "Allow Paste" prompt, which is why the
/// clipboard is read only now. The image the app copied itself is never read back: while the
/// clipboard still holds it the app pastes its own copy, which keeps what an image can't.
enum CraftPaste {
    /// `UIPasteboard.changeCount` right after the app put an image there (request 5).
    @MainActor static var ownImage: Int?

    @MainActor
    static func fromClipboard() {
        let board = UIPasteboard.general
        if let own = ownImage, own == board.changeCount {
            craft_paste_own()
            return
        }
        // A focused text field wants text even when an image came with it (a web page's copy).
        if CraftBridge.shared.keyboardVisible, board.hasStrings, let text = board.string {
            send(text)
        } else if board.hasImages, let image = board.image, let (rgba, width, height) = CraftPixels.rgba(image) {
            rgba.withUnsafeBufferPointer { craft_paste_image($0.baseAddress, $0.count, width, height) }
        } else {
            // Nothing the app takes (or the user didn't allow it): empty text, so it can say so.
            send(board.hasStrings ? board.string ?? "" : "")
        }
    }

    private static func send(_ text: String) {
        let data = Array(text.utf8)
        data.withUnsafeBufferPointer { craft_paste_text($0.baseAddress, $0.count) }
    }
}

/// RGBA8 (unpremultiplied) to and from UIKit images, for the clipboard.
enum CraftPixels {
    /// An image from the app's "W×H" size and its pixels, when they agree.
    static func image(size: String, rgba: Data) -> UIImage? {
        let parts = size.split(separator: "×").compactMap { Int($0) }
        guard parts.count == 2, parts[0] > 0, parts[1] > 0, parts[0] * parts[1] * 4 == rgba.count,
              let provider = CGDataProvider(data: rgba as CFData),
              let space = CGColorSpace(name: CGColorSpace.sRGB),
              let cg = CGImage(width: parts[0], height: parts[1], bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: parts[0] * 4,
                               space: space, bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
                               provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)
        else { return nil }
        return UIImage(cgImage: cg)
    }

    /// An image's pixels as RGBA8, unpremultiplied as the app expects. Capped at 16k × 16k.
    static func rgba(_ image: UIImage) -> ([UInt8], UInt32, UInt32)? {
        guard let cg = image.cgImage, cg.width > 0, cg.height > 0, cg.width <= 16384, cg.height <= 16384,
              let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        let (width, height) = (cg.width, cg.height)
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        // CoreGraphics only draws premultiplied: draw, then undo the premultiplication.
        let drawn = pixels.withUnsafeMutableBytes { raw -> Bool in
            guard let context = CGContext(data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                                          space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
            return true
        }
        guard drawn else { return nil }
        for i in stride(from: 0, to: pixels.count, by: 4) where pixels[i + 3] != 0 && pixels[i + 3] != 255 {
            let alpha = Int(pixels[i + 3])
            for c in 0..<3 { pixels[i + c] = UInt8(min(255, (Int(pixels[i + c]) * 255 + alpha / 2) / alpha)) }
        }
        return (pixels, UInt32(width), UInt32(height))
    }
}

/// The on-screen keyboard's target. It becomes first responder only while the app has a text field
/// focused (request kind 7): a view that is always a text input would raise the keyboard on every
/// iPad without a hardware keyboard. The canvas takes hardware keys the rest of the time.
final class CraftTextInput: UIView, UIKeyInput {
    override var canBecomeFirstResponder: Bool { true }
    var hasText: Bool { true }

    // The app's text fields edit their own text: plain typing, no suggestions to undo.
    var autocorrectionType: UITextAutocorrectionType = .no
    var autocapitalizationType: UITextAutocapitalizationType = .none
    var spellCheckingType: UITextSpellCheckingType = .no
    var smartQuotesType: UITextSmartQuotesType = .no
    var smartDashesType: UITextSmartDashesType = .no

    func insertText(_ text: String) {
        // Return confirms the field, as the hardware key does.
        if text == "\n" {
            CraftKeys.tap("Enter")
        } else {
            CraftKeys.text(text)
        }
    }

    func deleteBackward() {
        CraftKeys.tap("Backspace")
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        // Keys go to the app; characters then arrive as insertText, as typed text.
        let rest = CraftKeys.forward(presses, pressed: true)
        super.pressesBegan(rest, with: event)
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let rest = CraftKeys.forward(presses, pressed: false)
        super.pressesEnded(rest, with: event)
    }
}
