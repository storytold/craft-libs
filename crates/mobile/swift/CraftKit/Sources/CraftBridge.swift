import Foundation
import Observation
import Photos
import UIKit
import UniformTypeIdentifiers

/// What the app's native panels show (craft-mobile's `Snapshot`).
public struct CraftSnapshot: Decodable, Equatable, Sendable {
    public struct Document: Decodable, Equatable, Sendable {
        public var name: String
        public var detail: String
        public var dirty: Bool
    }

    public struct Item: Decodable, Equatable, Identifiable, Sendable {
        public var id: UInt64
        public var name: String
        public var kind: String
        public var depth: UInt32
        public var visible: Bool?
        public var selected: Bool
    }

    public var document: Document?
    public var canUndo = false
    public var canRedo = false
    public var itemsTitle = "Items"
    public var items: [Item] = []

    enum CodingKeys: String, CodingKey {
        case document, items
        case canUndo = "can_undo"
        case canRedo = "can_redo"
        case itemsTitle = "items_title"
    }

    public init() {}
}

/// A request to pick files to open.
public struct CraftPickRequest: Equatable {
    public var multiple: Bool
    /// Allowed extensions, without dots; empty for any the app declares.
    public var extensions: [String]
}

/// Bytes the app offers to save.
public struct CraftExport: Equatable {
    public var name: String
    public var data: Data
    /// Why the exporter is shown instead of saving in place, when a write back failed.
    public var reason: String?
}

/// A file read for the app: its name, contents, and its URL when the app may write it back.
struct CraftOpenedFile: Sendable {
    let name: String
    let data: Data
    /// Set when the file is edited in place: saves write back here.
    let location: URL?
}

/// Where the user sends an export.
enum CraftExportRoute: String {
    /// The system exporter: a place in Files.
    case save
    /// The share sheet: another app, AirDrop, Photos, or Files too.
    case share
    /// Straight into the Photos library (images only).
    case photos
}

extension CraftExport {
    /// Whether the export is an image the Photos library takes: offered Save to Photos. A layered
    /// document (PSD, the app's own) is an image type too, but not a photo.
    var isPhoto: Bool {
        Self.photoExtensions.contains((name as NSString).pathExtension.lowercased())
    }

    private static let photoExtensions: Set = ["png", "jpg", "jpeg", "heic", "heif", "gif", "webp", "tif", "tiff", "avif"]
}

/// The app's side of the shell: what it asks for, what its panels show, and how native controls
/// reach it. One per process, like the app itself.
@MainActor
@Observable
public final class CraftBridge {
    public static let shared = CraftBridge()

    public private(set) var snapshot = CraftSnapshot()
    /// The picker the app asked for; the shell shows it while this is set.
    public var pick: CraftPickRequest?
    /// Whether the document picker is on screen. Separate from `pick`: SwiftUI reports the picker
    /// closed *before* it hands over the chosen files, so closing must not answer the request.
    public var pickShown = false
    /// The export the app asked for; the shell asks Save or Share while this is set.
    public var export: CraftExport? {
        didSet {
            // The development aid's choice, where the dialog would offer it.
            exportRoute = demoExportRoute == .photos && export?.isPhoto != true ? nil : export.flatMap { _ in demoExportRoute }
        }
    }

    /// Where the current export goes, once the user chose.
    var exportRoute: CraftExportRoute?
    /// Development aid (CRAFT_DEMO_EXPORT): the route every export takes without asking.
    var demoExportRoute: CraftExportRoute?
    /// Whether the shell shows the Photos picker.
    var showsPhotos = false
    /// The last thing that went wrong with a file, for the shell to show.
    public var fileError: String?
    /// A short confirmation for the shell to show ("Saved to Photos").
    public var notice: String?

    /// Whether the app is busy with work the user is waiting for (an export encoding on a
    /// worker): background time is held for exactly that span, and the canvas keeps the app
    /// running (without drawing) while it's off screen.
    public private(set) var busy = false
    /// Called when ``busy`` changes.
    var onBusy: ((Bool) -> Void)?
    private let busyTask = CraftBackgroundTask(name: "Busy")
    /// Background time for a write in place, which runs after the app handed over the bytes.
    private let writing = CraftBackgroundTask(name: "Write in place")

    /// Files the app may write back to, by the location string the app holds (the URL's
    /// `absoluteString`). Security-scoped URLs from the document picker or Files: access is
    /// started for each read or write and stopped right after, so nothing leaks however long
    /// the documents stay open.
    private var locations: [String: URL] = [:]
    /// Writes in place, one at a time and in order, off the main thread.
    private let writes = DispatchQueue(label: "dev.craft.write-in-place", qos: .userInitiated)

    private init() {}

    /// Show (`true`) or hide the on-screen keyboard: the app's text field gained or lost focus.
    var onKeyboard: ((Bool) -> Void)?
    /// Whether the app has a text field focused (and so the keyboard up, when there's no hardware one).
    public private(set) var keyboardVisible = false

    /// Route the app's requests here. Call before the canvas starts.
    func connect() {
        craft_set_handler(Unmanaged.passUnretained(self).toOpaque(), craftHandler)
    }

    // MARK: Native controls

    public func perform(_ action: CraftAction) {
        guard let json = try? JSONSerialization.data(withJSONObject: action.json) else { return }
        json.withUnsafeBytes { raw in
            _ = craft_action(raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
        }
    }

    // MARK: Files

    /// Open files the system handed over (Files, Share, drag and drop). A file opened in place
    /// (`LSSupportsOpeningDocumentsInPlace`) is saved back where it is; a copy the system made
    /// for the app (its Inbox or temporary folder) is not.
    public func open(_ urls: [URL]) {
        open(urls, inPlace: true)
    }

    /// ``open(_:)``, or with `inPlace` false as copies: saving those goes to the exporter.
    func open(_ urls: [URL], inPlace: Bool) {
        Task {
            for file in await read(urls, inPlace: inPlace) {
                deliver(file, to: craft_open_file)
            }
        }
    }

    /// Open a file that came from somewhere other than a URL (the Photos library).
    public func open(name: String, data: Data) {
        deliver(CraftOpenedFile(name: name, data: data, location: nil), to: craft_open_file)
    }

    /// Answer the open picker with these files; none means cancelled. Picked files are edited in
    /// place.
    public func answerPick(_ urls: [URL]) {
        pick = nil
        pickShown = false
        Task {
            for file in await read(urls, inPlace: true) {
                deliver(file, to: craft_pick_add)
            }
            craft_pick_done()
        }
    }

    /// Read `urls` off the main thread (a large file, or one iCloud must download first, would
    /// hang it), reporting the ones that can't be read.
    private func read(_ urls: [URL], inPlace: Bool) async -> [CraftOpenedFile] {
        var files: [CraftOpenedFile] = []
        for url in urls {
            let keeps = inPlace && Self.isInPlace(url)
            let read = await Task.detached(priority: .userInitiated) { Result { try Self.read(url, keepingLocation: keeps) } }.value
            switch read {
            case let .success(file): files.append(file)
            case let .failure(error): fileError = "Couldn't read \(url.lastPathComponent): \(error.localizedDescription)"
            }
        }
        return files
    }

    /// Whether `url` is the user's file rather than a copy the system put in the app's own
    /// container for it (Documents/Inbox, or the temporary folder).
    private nonisolated static func isInPlace(_ url: URL) -> Bool {
        let path = url.standardizedFileURL.resolvingSymlinksInPath().path
        let inbox = URL.documentsDirectory.appending(path: "Inbox", directoryHint: .isDirectory).resolvingSymlinksInPath().path
        let temporary = FileManager.default.temporaryDirectory.resolvingSymlinksInPath().path
        return url.isFileURL && !path.hasPrefix(inbox + "/") && !path.hasPrefix(temporary + "/")
    }

    /// Read a file as its provider wants it read: inside its security scope, coordinated (so an
    /// iCloud file is downloaded first and no one writes it meanwhile).
    private nonisolated static func read(_ url: URL, keepingLocation: Bool) throws -> CraftOpenedFile {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        var coordination: NSError?
        var contents: Result<Data, Error> = .failure(CocoaError(.fileReadUnknown))
        NSFileCoordinator().coordinate(readingItemAt: url, options: [], error: &coordination) { target in
            contents = Result { try Data(contentsOf: target) }
        }
        if let coordination { throw coordination }
        let writable = keepingLocation && ((try? url.resourceValues(forKeys: [.isWritableKey]).isWritable) ?? true)
        return CraftOpenedFile(name: url.lastPathComponent, data: try contents.get(), location: writable ? url : nil)
    }

    private func deliver(_ file: CraftOpenedFile, to function: (UnsafePointer<UInt8>?, Int, UnsafePointer<UInt8>?, Int, UnsafePointer<UInt8>?, Int) -> Void) {
        if let url = file.location { locations[url.absoluteString] = url }
        let name = Data(file.name.utf8)
        let location = Data((file.location?.absoluteString ?? "").utf8)
        name.withUnsafeBytes { n in
            file.data.withUnsafeBytes { d in
                location.withUnsafeBytes { l in
                    function(
                        n.bindMemory(to: UInt8.self).baseAddress, n.count,
                        d.bindMemory(to: UInt8.self).baseAddress, d.count,
                        l.bindMemory(to: UInt8.self).baseAddress, l.count
                    )
                }
            }
        }
    }

    // MARK: Writing in place

    /// Write `data` back over the file at `location` (named `name`), then tell the app. If it
    /// can't be written, the user gets the exporter for the same bytes, so nothing is lost.
    private func writeInPlace(location: String, name: String, data: Data) {
        guard let url = locations[location] else {
            writeDone(location: location, name: name, data: data, error: "the app can no longer reach it")
            return
        }
        writing.begin()
        writes.async { [weak self] in
            let error = Self.write(data, to: url)
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.writeDone(location: location, name: name, data: data, error: error) }
            }
        }
    }

    private func writeDone(location: String, name: String, data: Data, error: String?) {
        writing.end()
        if let error {
            craftLog.error("couldn't write \(name, privacy: .public) in place: \(error, privacy: .public)")
            export = CraftExport(name: name, data: data, reason: "Couldn't save “\(name)” where it came from (\(error)). Choose where to save it.")
        }
        let location = Data(location.utf8)
        let error = Data((error ?? "").utf8)
        location.withUnsafeBytes { l in
            error.withUnsafeBytes { e in
                craft_write_done(l.bindMemory(to: UInt8.self).baseAddress, l.count, e.bindMemory(to: UInt8.self).baseAddress, e.count)
            }
        }
    }

    /// Replace the file's contents, coordinated with its provider (iCloud, File Provider
    /// extensions) and inside its security scope. Returns why it failed, or `nil`.
    ///
    /// An atomic write (a temporary file renamed over the original) first: a crash half-way
    /// leaves the old file whole. A provider that granted this one file, not its folder, has
    /// nowhere for that temporary file, so then the file is overwritten directly.
    private nonisolated static func write(_ data: Data, to url: URL) -> String? {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        var coordination: NSError?
        var failure: Error?
        NSFileCoordinator().coordinate(writingItemAt: url, options: .forReplacing, error: &coordination) { target in
            do {
                try data.write(to: target, options: .atomic)
            } catch {
                do {
                    try data.write(to: target)
                } catch {
                    failure = error
                }
            }
        }
        return (coordination ?? failure)?.localizedDescription
    }

    // MARK: Photos

    /// Add an exported image to the Photos library, asking only to add (the app never sees the
    /// library), under its own file name.
    func saveToPhotos(_ export: CraftExport) async {
        let task = CraftBackgroundTask(name: "Save to Photos")
        task.begin()
        defer { task.end() }
        let status = await PHPhotoLibrary.requestAuthorization(for: .addOnly)
        guard status == .authorized || status == .limited else {
            let app = Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "This app"
            fileError = "\(app) may not add photos. Allow it in Settings › Privacy & Security › Photos."
            return
        }
        let (name, data) = (export.name, export.data)
        do {
            try await PHPhotoLibrary.shared().performChanges {
                let options = PHAssetResourceCreationOptions()
                options.originalFilename = name
                PHAssetCreationRequest.forAsset().addResource(with: .photo, data: data, options: options)
            }
            craftLog.notice("saved \(name, privacy: .public) to Photos")
            notice = "Saved to Photos"
        } catch {
            fileError = "Couldn't save \(name) to Photos: \(error.localizedDescription)"
        }
    }

    // MARK: Requests from the app

    fileprivate func receive(kind: UInt32, text: String, data: Data) {
        switch kind {
        case 0, 1:
            let extensions = text.split(separator: ",").map(String.init)
            pick = CraftPickRequest(multiple: kind == 1, extensions: extensions)
            pickShown = true
        case 2:
            export = CraftExport(name: text, data: data)
        case 3:
            if let decoded = try? JSONDecoder().decode(CraftSnapshot.self, from: Data(text.utf8)), decoded != snapshot {
                snapshot = decoded
            }
        case 4:
            UIPasteboard.general.string = text
            CraftPaste.ownImage = nil
        case 5:
            if let image = CraftPixels.image(size: text, rgba: data) {
                UIPasteboard.general.image = image
                CraftPaste.ownImage = UIPasteboard.general.changeCount
            }
        case 6:
            if let url = URL(string: text), ["http", "https", "mailto"].contains(url.scheme?.lowercased()) {
                UIApplication.shared.open(url)
            }
        case 7:
            keyboardVisible = text == "1"
            onKeyboard?(keyboardVisible)
        case 8:
            // The location, a line break, the file name (which has none).
            guard let split = text.lastIndex(of: "\n") else { return }
            writeInPlace(location: String(text[..<split]), name: String(text[text.index(after: split)...]), data: data)
        case 9:
            busy = text == "1"
            if busy { busyTask.begin() } else { busyTask.end() }
            onBusy?(busy)
        case 10:
            CraftPaste.fromClipboard()
        case 11:
            // Straight to the share sheet: the user chose Share, not where to save.
            export = CraftExport(name: text, data: data)
            exportRoute = .share
        case 12:
            showsPhotos = true
        default:
            break
        }
    }
}

/// Background time from UIKit, held at most once. Expiry ends it.
@MainActor
final class CraftBackgroundTask {
    let name: String
    private var id = UIBackgroundTaskIdentifier.invalid

    init(name: String) { self.name = name }

    func begin() {
        guard id == .invalid else { return }
        id = UIApplication.shared.beginBackgroundTask(withName: name) { [weak self] in
            // UIKit calls this on the main thread.
            MainActor.assumeIsolated { self?.end() }
        }
        craftLog.notice("background task \(self.name, privacy: .public) began")
    }

    func end() {
        guard id != .invalid else { return }
        UIApplication.shared.endBackgroundTask(id)
        id = .invalid
        craftLog.notice("background task \(self.name, privacy: .public) ended")
    }
}

/// What a native control asks of the app (craft-mobile's `Action`).
public enum CraftAction: Sendable {
    case open, save, undo, redo
    case select(UInt64)
    case setVisible(UInt64, Bool)
    /// One of the app's own commands by id; without parameters it runs as its menu item does.
    case command(String, [String: any Sendable]? = nil)

    var json: [String: Any] {
        switch self {
        case .open: ["action": "open"]
        case .save: ["action": "save"]
        case .undo: ["action": "undo"]
        case .redo: ["action": "redo"]
        case let .select(id): ["action": "select", "id": id]
        case let .setVisible(id, visible): ["action": "set_visible", "id": id, "visible": visible]
        case let .command(id, params): ["action": "command", "id": id, "params": params ?? NSNull()]
        }
    }
}

/// Requests arrive on the main thread, during `craft_frame`, which the canvas calls from its
/// display link. The buffers are only valid during the call, so they are copied here.
private let craftHandler: CraftHandler = { context, kind, text, textLength, data, dataLength in
    guard let context else { return }
    let bridge = Unmanaged<CraftBridge>.fromOpaque(context).takeUnretainedValue()
    let text = text.map { String(decoding: UnsafeBufferPointer(start: $0, count: textLength), as: UTF8.self) } ?? ""
    let data = data.map { Data(bytes: $0, count: dataLength) } ?? Data()
    MainActor.assumeIsolated {
        bridge.receive(kind: kind, text: text, data: data)
    }
}

extension UTType {
    /// The types for a picker's extensions, falling back to `fallback` when none are known.
    static func forExtensions(_ extensions: [String], fallback: [UTType]) -> [UTType] {
        let types = extensions.compactMap { UTType(filenameExtension: $0) }
        return types.isEmpty ? fallback : types
    }
}
