import QuartzCore
import SwiftUI
import UIKit
import os

let craftLog = Logger(subsystem: "dev.craft.kit", category: "canvas")

/// What may draw on the canvas.
public enum CraftDrawingInput: Sendable {
    /// Pencil and fingers. While a Pencil is down, fingers (a resting palm) are ignored.
    case anyTouch
    /// Only Pencil draws. A lone finger never reaches the app; two or more together still do,
    /// so pinch and two-finger pans keep working.
    case pencilOnly
}

/// How the canvas's pixels are to be read by the display.
public enum CraftColorSpace: Sendable {
    /// sRGB values, which is what egui and an app without colour management produce.
    case sRGB
    /// Display P3 values: only for an app whose colour management converts to Display P3. sRGB
    /// content shown this way looks oversaturated.
    case displayP3

    var cgColorSpace: CGColorSpace? {
        switch self {
        case .sRGB: CGColorSpace(name: CGColorSpace.sRGB)
        case .displayP3: CGColorSpace(name: CGColorSpace.displayP3)
        }
    }
}

/// What the Apple Pencil's double-tap and squeeze do. `nil` does nothing; either does nothing
/// when the user set that gesture to "Off" in Settings.
public struct CraftPencilActions: Sendable {
    public var doubleTap: CraftAction?
    public var squeeze: CraftAction?

    public init(doubleTap: CraftAction? = nil, squeeze: CraftAction? = nil) {
        self.doubleTap = doubleTap
        self.squeeze = squeeze
    }
}

/// The Rust-drawn app as a SwiftUI view. Use one per process.
public struct CraftCanvas: UIViewRepresentable {
    let drawing: CraftDrawingInput
    let colorSpace: CraftColorSpace
    let pencil: CraftPencilActions

    public init(drawing: CraftDrawingInput = .anyTouch, colorSpace: CraftColorSpace = .sRGB, pencil: CraftPencilActions = CraftPencilActions()) {
        self.drawing = drawing
        self.colorSpace = colorSpace
        self.pencil = pencil
    }

    public func makeUIView(context: Context) -> CraftCanvasView {
        let view = CraftCanvasView(frame: .zero)
        updateUIView(view, context: context)
        return view
    }

    public func updateUIView(_ view: CraftCanvasView, context: Context) {
        view.drawing = drawing
        view.colorSpace = colorSpace
        view.pencil = pencil
    }
}

/// A Metal-backed view that Rust draws into. It forwards its size, the display's refresh and
/// every touch (with coalesced Pencil samples and their force); Rust decides when to draw.
///
/// It stops calling into the GPU while its scene isn't active (iPadOS terminates apps that submit
/// GPU work in the background), rejects palms while a Pencil is down, turns two- and three-finger
/// taps into undo and redo, and lowers the frame rate when the device is hot or saving power.
public final class CraftCanvasView: UIView, UIGestureRecognizerDelegate, UIPencilInteractionDelegate {
    override public class var layerClass: AnyClass { CAMetalLayer.self }

    var drawing = CraftDrawingInput.anyTouch
    var colorSpace = CraftColorSpace.sRGB {
        didSet { applyColorSpace() }
    }

    var pencil = CraftPencilActions()

    /// The canvas on screen, for the development aids.
    private(set) static weak var current: CraftCanvasView?

    private var link: CADisplayLink?
    private var started = false
    /// Whether the scene is in the foreground and active: only then may Rust touch the GPU.
    private var active = false
    private var observers: [NSObjectProtocol] = []
    /// Development aid: a thermal state to act on instead of the device's (CRAFT_DEMO_THERMAL).
    var thermalOverride: ProcessInfo.ThermalState?
    /// Development aid: Low Power Mode to act on instead of the device's (CRAFT_DEMO_LOW_POWER).
    var lowPowerOverride: Bool?
    /// Runs the app (it doesn't draw) while it's busy off screen, so it can finish its work.
    private var busyFrames: Timer?

    /// Fingers being forwarded and where they were last seen, so a Pencil can take them back.
    private var fingers: [UInt64: CGPoint] = [:]
    private var pencils: Set<UInt64> = []
    /// Palms: fingers that touched down with a Pencil, or before one. Never forwarded.
    private var palms: Set<UInt64> = []
    /// Pencil-only mode: a lone finger, held back until a second joins it (a pinch or pan).
    private var held: [UInt64: CGPoint] = [:]
    /// The on-screen keyboard's target, first responder only while the app has a text field focused.
    private let textInput = CraftTextInput(frame: .zero)
    /// Where the trackpad scroll gesture last reported, to send deltas.
    private var lastScroll = CGPoint.zero

    override init(frame: CGRect) {
        super.init(frame: frame)
        isMultipleTouchEnabled = true
        backgroundColor = .black
        applyColorSpace()
        addTap(touches: 2, action: #selector(undoTap))
        addTap(touches: 3, action: #selector(redoTap))
        let interaction = UIPencilInteraction(delegate: self)
        addInteraction(interaction)
        addPointerGestures()
        addSubview(textInput)
    }

    // MARK: Keyboards

    override public var canBecomeFirstResponder: Bool { true }

    override public func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        super.pressesBegan(CraftKeys.forward(presses, pressed: true), with: event)
    }

    override public func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        super.pressesEnded(CraftKeys.forward(presses, pressed: false), with: event)
    }

    /// The app's text field gained (show) or lost focus: raise or drop the on-screen keyboard.
    /// Without one, the canvas stays first responder so hardware shortcuts keep working.
    private func showKeyboard(_ show: Bool) {
        if show {
            textInput.becomeFirstResponder()
        } else if textInput.isFirstResponder {
            textInput.resignFirstResponder()
            becomeFirstResponder()
        }
    }

    // MARK: Trackpads and mice

    private func addPointerGestures() {
        // Hover: a trackpad or mouse pointer (and a hovering Pencil) moving without a button down.
        addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(hover(_:))))
        // Scroll: two fingers on a trackpad, or a mouse wheel. Scroll events only, never touches.
        let scroll = UIPanGestureRecognizer(target: self, action: #selector(scroll(_:)))
        scroll.allowedScrollTypesMask = .all
        scroll.allowedTouchTypes = []
        addGestureRecognizer(scroll)
        // Secondary click: right button, two-finger trackpad click, control-click.
        let secondary = UITapGestureRecognizer(target: self, action: #selector(secondaryClick(_:)))
        secondary.buttonMaskRequired = .secondary
        secondary.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.indirectPointer.rawValue)]
        addGestureRecognizer(secondary)
    }

    @objc private func hover(_ recognizer: UIHoverGestureRecognizer) {
        let at = recognizer.location(in: self)
        switch recognizer.state {
        case .began, .changed: craft_hover(Float(at.x), Float(at.y), true)
        default: craft_hover(Float(at.x), Float(at.y), false)
        }
    }

    @objc private func scroll(_ recognizer: UIPanGestureRecognizer) {
        let translation = recognizer.translation(in: self)
        if recognizer.state == .began { lastScroll = .zero }
        let delta = CGPoint(x: translation.x - lastScroll.x, y: translation.y - lastScroll.y)
        lastScroll = translation
        let at = recognizer.location(in: self)
        if delta != .zero {
            craft_scroll(Float(at.x), Float(at.y), Float(delta.x), Float(delta.y))
        }
    }

    @objc private func secondaryClick(_ recognizer: UITapGestureRecognizer) {
        let at = recognizer.location(in: self)
        craft_secondary_click(Float(at.x), Float(at.y))
    }

    required init?(coder: NSCoder) { nil }

    private func addTap(touches: Int, action: Selector) {
        let tap = UITapGestureRecognizer(target: self, action: action)
        tap.numberOfTouchesRequired = touches
        tap.allowedTouchTypes = [NSNumber(value: UITouch.TouchType.direct.rawValue)]
        // Recognised taps cancel their touches here, so they never also click inside the app;
        // touches still begin here at once (delaysTouchesBegan stays off), so drawing doesn't lag.
        tap.cancelsTouchesInView = true
        tap.delegate = self
        addGestureRecognizer(tap)
    }

    private func applyColorSpace() {
        guard let metal = layer as? CAMetalLayer else { return }
        metal.colorspace = colorSpace.cgColorSpace
        // Values above 1.0 would be read as HDR headroom; the app draws SDR.
        metal.wantsExtendedDynamicRangeContent = false
    }

    // MARK: Life cycle

    override public func didMoveToWindow() {
        super.didMoveToWindow()
        if let window {
            let link = CADisplayLink(target: self, selector: #selector(step))
            link.add(to: .main, forMode: .common)
            self.link = link
            Self.current = self
            observe(window.windowScene)
            setActive(window.windowScene?.activationState == .foregroundActive)
            // Hardware keys reach the app through the canvas while no text field is focused.
            becomeFirstResponder()
            applyFrameRate()
        } else {
            // The link retains this view: break the cycle, and drop the surface before the layer.
            link?.invalidate()
            link = nil
            busyFrames?.invalidate()
            busyFrames = nil
            for observer in observers { NotificationCenter.default.removeObserver(observer) }
            observers = []
            if started {
                craft_stop()
                started = false
            }
        }
    }

    private func observe(_ scene: UIWindowScene?) {
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        let center = NotificationCenter.default
        let on = { [weak self] (name: Notification.Name, object: AnyObject?, body: @escaping @MainActor (CraftCanvasView) -> Void) -> NSObjectProtocol in
            // Thermal and power notifications arrive on any thread: hop to the main queue.
            center.addObserver(forName: name, object: object, queue: .main) { _ in
                MainActor.assumeIsolated {
                    if let self { body(self) }
                }
            }
        }
        observers = [
            on(UIScene.didActivateNotification, scene) { $0.setActive(true) },
            on(UIScene.willDeactivateNotification, scene) { $0.setActive(false) },
            on(ProcessInfo.thermalStateDidChangeNotification, nil) { $0.applyFrameRate() },
            on(UIApplication.didReceiveMemoryWarningNotification, nil) { view in
                if view.started { craft_memory_warning() }
            },
            on(Notification.Name.NSProcessInfoPowerStateDidChange, nil) { $0.applyFrameRate() },
        ]
    }

    /// Pause the display link (so `craft_frame` isn't called) while the scene isn't active, and
    /// draw at once when it is again.
    func setActive(_ isActive: Bool) {
        guard isActive != active else { return }
        active = isActive
        link?.isPaused = !isActive
        craftLog.notice("canvas \(isActive ? "active: frames resume" : "inactive: frames paused", privacy: .public)")
        if !isActive, started {
            // The OS may end the app from here on without a word: the app starts writing recovery
            // copies of unsaved work, on workers, and returns.
            let task = CraftBackgroundTask(name: "Suspend")
            task.begin()
            craft_suspend()
            // A control used just before leaving (Save, then Home) still runs, without drawing.
            // Work the user waits for (the recovery copies, that save) turns the app busy here,
            // which holds background time until it's done: before the suspend's own time ends.
            _ = craft_frame()
            task.end()
        }
        runWhileBusy()
        if isActive {
            if started { craft_resume() }
            // A size that changed while inactive (app switcher snapshots rotate the window) lands
            // now, and the first frame doesn't wait for the next refresh.
            setNeedsLayout()
            layoutIfNeeded()
            if started { _ = craft_frame() }
        }
    }

    /// While the app is busy and the scene isn't active, call `craft_frame` a few times a second:
    /// craft-mobile runs the app then without drawing, so its work can finish and its result
    /// (an export, a write in place) reach the shell. The bridge holds the background time.
    func runWhileBusy() {
        let wanted = started && !active && CraftBridge.shared.busy
        if wanted, busyFrames == nil {
            craftLog.notice("busy off screen: running without drawing")
            busyFrames = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { _ in
                // Scheduled on the main run loop.
                MainActor.assumeIsolated { _ = craft_frame() }
            }
        } else if !wanted, let timer = busyFrames {
            timer.invalidate()
            busyFrames = nil
        }
    }

    /// 120 Hz normally; 60 when the device is getting hot or saving power; 30 when it's critical.
    /// The app hears the power state too, to lower its own costs (preview quality).
    func applyFrameRate() {
        let info = ProcessInfo.processInfo
        let thermal = thermalOverride ?? info.thermalState
        let lowPower = lowPowerOverride ?? info.isLowPowerModeEnabled
        let range = switch thermal {
        case .critical: CAFrameRateRange(minimum: 15, maximum: 30, preferred: 30)
        case .serious: CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60)
        case _ where lowPower: CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60)
        default: CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        }
        link?.preferredFrameRateRange = range
        if started { craft_power(UInt32(clamping: thermal.rawValue), lowPower) }
        craftLog.notice("frame rate up to \(Int(range.maximum)) Hz (thermal \(thermal.rawValue), low power \(lowPower))")
    }

    override public func layoutSubviews() {
        super.layoutSubviews()
        guard window != nil, let metal = layer as? CAMetalLayer else { return }
        let scale = traitCollection.displayScale
        contentScaleFactor = scale
        let width = UInt32(max(0, (bounds.width * scale).rounded()))
        let height = UInt32(max(0, (bounds.height * scale).rounded()))
        guard width > 0, height > 0 else { return }
        metal.drawableSize = CGSize(width: Int(width), height: Int(height))
        // Reconfiguring the surface waits for the scene to be active again (setActive lays out).
        guard active else { return }
        if started {
            craft_resize(width, height, Float(scale))
        } else {
            CraftBridge.shared.connect()
            CraftBridge.shared.onKeyboard = { [weak self] show in self?.showKeyboard(show) }
            CraftBridge.shared.onBusy = { [weak self] _ in self?.runWhileBusy() }
            Self.setDataDirectory()
            started = craft_start(Unmanaged.passUnretained(metal).toOpaque(), width, height, Float(scale))
            if started {
                applyFrameRate()
                CraftDevelopment.replay()
            }
        }
    }

    /// The app's own data (crash recovery, settings): Application Support, created if needed.
    private static func setDataDirectory() {
        guard let dir = try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true) else {
            craftLog.error("no Application Support folder: the app keeps no recovery data")
            return
        }
        let path = Array(dir.path.utf8)
        path.withUnsafeBufferPointer { craft_set_data_dir($0.baseAddress, $0.count) }
    }

    @objc private func step() {
        _ = craft_frame()
    }

    // MARK: Touches

    override public func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 0) }
    override public func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 1) }
    override public func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 2) }
    override public func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, event, phase: 3) }

    private func send(_ touches: Set<UITouch>, _ event: UIEvent?, phase: UInt32) {
        // Pencils first: one landing takes over from the palm that touched down before it.
        for touch in touches.sorted(by: { $0.type == .pencil && $1.type != .pencil }) {
            let id = UInt64(UInt(bitPattern: Unmanaged.passUnretained(touch).toOpaque()))
            let point = touch.preciseLocation(in: self)
            guard admit(id, touch.type, at: point, phase: phase) else { continue }
            // A Pencil reports up to 240 samples a second; the display shows 120. Keep them all.
            let samples = phase == 1 ? (event?.coalescedTouches(for: touch) ?? [touch]) : [touch]
            for sample in samples {
                let point = sample.preciseLocation(in: self)
                let force = sample.maximumPossibleForce > 0 ? sample.force / sample.maximumPossibleForce : 0
                if touch.type == .pencil {
                    // Pressure and tilt for the app's brushes; it reaches egui as a touch too.
                    craft_stylus(id, phase, Float(point.x), Float(point.y), Float(force),
                                 Float(sample.altitudeAngle), Float(sample.azimuthAngle(in: self)), false)
                } else {
                    craft_touch(id, phase, Float(point.x), Float(point.y), Float(force))
                }
            }
        }
    }

    /// Palm rejection and Pencil-only drawing: whether this touch reaches the app.
    private func admit(_ id: UInt64, _ type: UITouch.TouchType, at point: CGPoint, phase: UInt32) -> Bool {
        let ending = phase >= 2
        if palms.contains(id) {
            if ending { palms.remove(id) }
            return false
        }
        if held[id] != nil {
            held[id] = ending ? nil : point
            return false
        }
        switch type {
        case .pencil:
            if phase == 0 {
                pencils.insert(id)
                // Whatever was down before the Pencil is the hand resting: take it back. A
                // cancelled touch never clicks.
                for (finger, at) in fingers {
                    craft_touch(finger, 3, Float(at.x), Float(at.y), 0)
                }
                palms.formUnion(fingers.keys)
                palms.formUnion(held.keys)
                fingers = [:]
                held = [:]
            } else if ending {
                pencils.remove(id)
            }
            return true
        case .direct:
            if phase == 0 {
                if !pencils.isEmpty {
                    palms.insert(id)
                    return false
                }
                if drawing == .pencilOnly, fingers.isEmpty {
                    guard let (first, at) = held.first else {
                        held[id] = point
                        return false
                    }
                    // The second finger: forward both, as the gesture they are.
                    held[first] = nil
                    fingers[first] = at
                    craft_touch(first, 0, Float(at.x), Float(at.y), 0)
                }
            }
            fingers[id] = ending ? nil : point
            return true
        default:
            // Trackpad and mouse pointers are never palms.
            return true
        }
    }

    // MARK: Gestures

    public func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        // A palm tapping twice while drawing is not an undo.
        pencils.isEmpty
    }

    @objc func undoTap() {
        craftLog.notice("two-finger tap: undo")
        CraftBridge.shared.perform(.undo)
    }

    @objc func redoTap() {
        craftLog.notice("three-finger tap: redo")
        CraftBridge.shared.perform(.redo)
    }

    public func pencilInteraction(_ interaction: UIPencilInteraction, didReceiveTap tap: UIPencilInteraction.Tap) {
        pencilDoubleTap()
    }

    public func pencilInteraction(_ interaction: UIPencilInteraction, didReceiveSqueeze squeeze: UIPencilInteraction.Squeeze) {
        if squeeze.phase == .ended { pencilSqueeze() }
    }

    func pencilDoubleTap() {
        guard UIPencilInteraction.preferredTapAction != .ignore, let action = pencil.doubleTap else { return }
        craftLog.notice("Pencil double-tap: \(String(describing: action), privacy: .public)")
        CraftBridge.shared.perform(action)
    }

    func pencilSqueeze() {
        guard UIPencilInteraction.preferredSqueezeAction != .ignore, let action = pencil.squeeze else { return }
        craftLog.notice("Pencil squeeze: \(String(describing: action), privacy: .public)")
        CraftBridge.shared.perform(action)
    }
}

/// Development aids for the simulator, which can't be tapped or given files from a script.
/// - `CRAFT_DEMO_TAPS="x,y;x,y"` (points): taps there, a second apart, after start.
/// - `CRAFT_DEMO_ACTIONS="undo;open"`: performs those actions (by name) after the taps;
///   `toggle-first-item` shows or hides the top item as the inspector's eye button does;
///   `type:<text>` types text as the on-screen keyboard does; `memory-warning` posts UIKit's;
///   `two-finger-tap`, `three-finger-tap`, `pencil-double-tap`, `pencil-squeeze` run what those
///   gestures run; `photos` shows the Photos picker; `choose-save` / `choose-share` answer the
///   Save-or-Share dialog; `choose-photos` its Save to Photos; `background` / `foreground` act as
///   if the scene left (and came back to) the foreground; `wait:<seconds>` waits longer before
///   the next step; `tap:x,y` taps there (points) in sequence with the other steps;
///   `pasteboard-image:/path.png` and `pasteboard-text:<text>` put that on the clipboard as
///   another app would; `paste` pastes as ⌘V on a hardware keyboard does.
/// - `CRAFT_DEMO_OPEN="/path/a.png;/path/b.psd"`: opens those files as if the system handed them
///   over in place (Files' Open in place): Save writes them back. `CRAFT_DEMO_OPEN_COPY` opens
///   them as copies (as a share or the Photos library hands them over): Save goes to the exporter.
/// - `command:<id>` in `CRAFT_DEMO_ACTIONS` runs one of the app's commands as its menu item does
///   (`command:file.saveAs`).
/// - `CRAFT_DEMO_STEP=<seconds>`: the time between steps (1.5 s).
/// - `CRAFT_DEMO_EXPORT=save|share|photos`: answers every export's Save-or-Share choice.
/// - `CRAFT_DEMO_THERMAL=fair|serious|critical`: acts as if the device were that hot;
///   `CRAFT_DEMO_LOW_POWER=1` as if Low Power Mode were on.
enum CraftDevelopment {
    @MainActor
    static func replay() {
        let env = ProcessInfo.processInfo.environment
        if let canvas = CraftCanvasView.current, env["CRAFT_DEMO_THERMAL"] != nil || env["CRAFT_DEMO_LOW_POWER"] != nil {
            let states: [String: ProcessInfo.ThermalState] = ["nominal": .nominal, "fair": .fair, "serious": .serious, "critical": .critical]
            canvas.thermalOverride = env["CRAFT_DEMO_THERMAL"].flatMap { states[$0] }
            canvas.lowPowerOverride = env["CRAFT_DEMO_LOW_POWER"].map { $0 == "1" }
            canvas.applyFrameRate()
        }
        CraftBridge.shared.demoExportRoute = env["CRAFT_DEMO_EXPORT"].flatMap(CraftExportRoute.init(rawValue:))
        var delay = 2.0
        let spacing = env["CRAFT_DEMO_STEP"].flatMap(Double.init) ?? 1.5
        let after = { (step: @escaping @MainActor () -> Void) in
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { MainActor.assumeIsolated(step) }
            delay += spacing
        }
        for path in (env["CRAFT_DEMO_OPEN"] ?? "").split(separator: ";") {
            after { CraftBridge.shared.open([URL(fileURLWithPath: String(path))]) }
        }
        for path in (env["CRAFT_DEMO_OPEN_COPY"] ?? "").split(separator: ";") {
            after { CraftBridge.shared.open([URL(fileURLWithPath: String(path))], inPlace: false) }
        }
        for (index, pair) in (env["CRAFT_DEMO_TAPS"] ?? "").split(separator: ";").enumerated() {
            let xy = pair.split(separator: ",").compactMap { Float($0) }
            guard xy.count == 2 else { continue }
            let id = UInt64(1000 + index)
            after {
                craft_touch(id, 0, xy[0], xy[1], 0)
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { craft_touch(id, 2, xy[0], xy[1], 0) }
            }
        }
        let named: [String: CraftAction] = ["open": .open, "save": .save, "undo": .undo, "redo": .redo]
        let gestures: [String: @MainActor (CraftCanvasView) -> Void] = [
            "two-finger-tap": { $0.undoTap() },
            "three-finger-tap": { $0.redoTap() },
            "pencil-double-tap": { $0.pencilDoubleTap() },
            "pencil-squeeze": { $0.pencilSqueeze() },
        ]
        for name in (env["CRAFT_DEMO_ACTIONS"] ?? "").split(separator: ";").map(String.init) {
            if name.hasPrefix("wait:") {
                // Extra time before the next step.
                delay += Double(name.dropFirst(5)) ?? 0
            } else if name.hasPrefix("command:") {
                // As the app's menu item runs it.
                let id = String(name.dropFirst(8))
                after { CraftBridge.shared.perform(.command(id)) }
            } else if name.hasPrefix("tap:") {
                // As a finger tap, in order with the other steps.
                let xy = name.dropFirst(4).split(separator: ",").compactMap { Float($0) }
                guard xy.count == 2 else { continue }
                let (x, y) = (xy[0], xy[1])
                after {
                    craft_touch(2000, 0, x, y, 0)
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { craft_touch(2000, 2, x, y, 0) }
                }
            } else if name.hasPrefix("pasteboard-image:") {
                // As another app copying an image.
                let path = String(name.dropFirst(17))
                after { UIPasteboard.general.image = UIImage(contentsOfFile: path) }
            } else if name.hasPrefix("pasteboard-text:") {
                let text = String(name.dropFirst(16))
                after { UIPasteboard.general.string = text }
            } else if name.hasPrefix("type:") {
                // As the on-screen keyboard types it.
                let text = String(name.dropFirst(5))
                after { CraftKeys.text(text) }
            } else if name == "memory-warning" {
                // As UIKit reports memory pressure.
                after { NotificationCenter.default.post(name: UIApplication.didReceiveMemoryWarningNotification, object: UIApplication.shared) }
            } else if name == "toggle-first-item" {
                // As the inspector's eye button does, on the top item.
                after {
                    guard let item = CraftBridge.shared.snapshot.items.first, let visible = item.visible else { return }
                    CraftBridge.shared.perform(.setVisible(item.id, !visible))
                }
            } else if let route = ["choose-share": CraftExportRoute.share, "choose-save": .save, "choose-photos": .photos][name] {
                // As the Save-or-Share dialog's buttons do.
                after { CraftBridge.shared.exportRoute = route }
            } else if name == "background" || name == "foreground" {
                // As the scene leaving the foreground (the canvas pauses, the app suspends) and returning.
                after { CraftCanvasView.current?.setActive(name == "foreground") }
            } else if name == "paste" {
                after { CraftPaste.fromClipboard() }
            } else if name == "photos" {
                after { CraftBridge.shared.showsPhotos = true }
            } else if let gesture = gestures[name] {
                after { CraftCanvasView.current.map(gesture) }
            } else if let action = named[name] {
                after { CraftBridge.shared.perform(action) }
            }
        }
    }
}
