import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A Craft app's whole window: the canvas; optionally native chrome (a Liquid Glass toolbar and the
/// items inspector for layers, tracks, pages); the system document picker, exporter and share sheet
/// (also straight from the app: `Platform::share`), the Photos picker (the toolbar's, or the app's
/// own menu item through `Platform::pick_photos`), and Save to Photos for image exports (the app's
/// Info.plist needs `NSPhotoLibraryAddUsageDescription`). Put it in a `WindowGroup`.
public struct CraftShell: View {
    /// The types File › Open accepts when the app doesn't say (the app's own documents first).
    let documentTypes: [UTType]
    let drawing: CraftDrawingInput
    let colorSpace: CraftColorSpace
    let pencil: CraftPencilActions
    let chrome: Bool

    @State private var bridge = CraftBridge.shared
    @State private var showItems = false
    @State private var photos: [PhotosPickerItem] = []

    /// - Parameters:
    ///   - documentTypes: what the app opens, its own documents first. Any image type among them
    ///     adds Import from Photos.
    ///   - drawing: whether fingers draw, or only Pencil.
    ///   - colorSpace: what the app's pixels are; `.displayP3` only if its colour management
    ///     converts to Display P3.
    ///   - pencil: what Apple Pencil's double-tap and squeeze do (the app's command ids).
    ///   - chrome: native toolbar and items inspector over the canvas. Off by default: an app that
    ///     draws its own menus and panels would show everything twice.
    public init(documentTypes: [UTType], drawing: CraftDrawingInput = .anyTouch, colorSpace: CraftColorSpace = .sRGB, pencil: CraftPencilActions = CraftPencilActions(), chrome: Bool = false) {
        self.chrome = chrome
        self.documentTypes = documentTypes
        self.drawing = drawing
        self.colorSpace = colorSpace
        self.pencil = pencil
    }

    private var opensImages: Bool { documentTypes.contains { $0.conforms(to: .image) } }

    public var body: some View {
        CraftCanvas(drawing: drawing, colorSpace: colorSpace, pencil: pencil)
            // Under the home indicator, but never under the on-screen keyboard: the canvas shrinks so
            // the app's focused field and its dialog stay in view.
            .ignoresSafeArea(.container, edges: .bottom)
            .background(Color.black)
            .overlay(alignment: .bottom) {
                // While typing, the keyboard is the toolbar.
                if chrome, !bridge.keyboardVisible { toolbar.padding(.bottom, 20).transition(.opacity) }
            }
            .animation(.easeOut(duration: 0.2), value: bridge.keyboardVisible)
            .inspector(isPresented: Binding(get: { chrome && showItems }, set: { showItems = $0 })) {
                CraftItemsPanel(snapshot: bridge.snapshot, perform: bridge.perform)
                    .inspectorColumnWidth(min: 240, ideal: 300, max: 400)
            }
            .fileImporter(
                // Closing only hides the picker: SwiftUI closes it before it delivers the chosen
                // files, so the answer comes from the completion, or from the cancellation.
                isPresented: $bridge.pickShown,
                allowedContentTypes: UTType.forExtensions(bridge.pick?.extensions ?? [], fallback: documentTypes),
                allowsMultipleSelection: bridge.pick?.multiple ?? false,
                onCompletion: { result in bridge.answerPick((try? result.get()) ?? []) },
                onCancellation: { bridge.answerPick([]) }
            )
            .confirmationDialog(
                bridge.export?.name ?? "Export",
                isPresented: Binding(get: { bridge.export != nil && bridge.exportRoute == nil }, set: { if !$0, bridge.exportRoute == nil { bridge.export = nil } }),
                titleVisibility: .visible
            ) {
                Button("Save to Files", systemImage: "folder") { bridge.exportRoute = .save }
                if bridge.export?.isPhoto == true {
                    Button("Save to Photos", systemImage: "photo.badge.plus") { bridge.exportRoute = .photos }
                }
                Button("Share…", systemImage: "square.and.arrow.up") { bridge.exportRoute = .share }
            } message: {
                if let reason = bridge.export?.reason { Text(reason) }
            }
            .fileExporter(
                isPresented: Binding(get: { bridge.exportRoute == .save }, set: { if !$0 { bridge.export = nil } }),
                document: bridge.export.map(ExportDocument.init),
                contentType: bridge.export.flatMap { UTType(filenameExtension: ($0.name as NSString).pathExtension) } ?? .data,
                defaultFilename: bridge.export?.name
            ) { result in
                if case let .failure(error) = result {
                    bridge.fileError = "Couldn't save: \(error.localizedDescription)"
                }
                bridge.export = nil
            }
            .background {
                CraftSharePresenter(export: bridge.exportRoute == .share ? bridge.export : nil) { error in
                    if let error { bridge.fileError = "Couldn't share: \(error.localizedDescription)" }
                    bridge.export = nil
                }
            }
            .onChange(of: bridge.exportRoute) { _, route in
                guard route == .photos, let export = bridge.export, export.isPhoto else { return }
                Task {
                    await bridge.saveToPhotos(export)
                    bridge.export = nil
                }
            }
            .overlay(alignment: .top) {
                if let notice = bridge.notice {
                    Label(notice, systemImage: "checkmark.circle.fill")
                        .padding(.horizontal, 16)
                        .padding(.vertical, 10)
                        .glassEffect(.regular, in: .capsule)
                        .padding(.top, 20)
                        .transition(.move(edge: .top).combined(with: .opacity))
                        .task {
                            try? await Task.sleep(for: .seconds(2))
                            bridge.notice = nil
                        }
                }
            }
            .animation(.easeOut(duration: 0.25), value: bridge.notice)
            .photosPicker(isPresented: $bridge.showsPhotos, selection: $photos, matching: .images, preferredItemEncoding: .current)
            .onChange(of: photos) { _, items in
                guard !items.isEmpty else { return }
                photos = []
                Task { await importPhotos(items) }
            }
            .onOpenURL { url in bridge.open([url]) }
            .alert("File", isPresented: Binding(get: { bridge.fileError != nil }, set: { if !$0 { bridge.fileError = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(bridge.fileError ?? "")
            }
            .statusBarHidden()
            // Craft apps are dark pro tools; the native chrome matches the canvas.
            .preferredColorScheme(.dark)
            // Development aid (see CraftDevelopment): CRAFT_DEMO_SHOW_ITEMS=1 opens the inspector.
            .onAppear { showItems = ProcessInfo.processInfo.environment["CRAFT_DEMO_SHOW_ITEMS"] == "1" }
    }

    private var toolbar: some View {
        let snapshot = bridge.snapshot
        return GlassEffectContainer(spacing: 12) {
            HStack(spacing: 12) {
                HStack(spacing: 4) {
                    tool("Open", "folder") { bridge.perform(.open) }
                    if opensImages {
                        tool("Import from Photos", "photo.on.rectangle") { bridge.showsPhotos = true }
                    }
                    tool("Save", "square.and.arrow.down") { bridge.perform(.save) }
                        .disabled(snapshot.document == nil)
                }
                .padding(6)
                .glassEffect(.regular.interactive(), in: .capsule)

                HStack(spacing: 4) {
                    tool("Undo", "arrow.uturn.backward") { bridge.perform(.undo) }
                        .disabled(!snapshot.canUndo)
                    tool("Redo", "arrow.uturn.forward") { bridge.perform(.redo) }
                        .disabled(!snapshot.canRedo)
                }
                .padding(6)
                .glassEffect(.regular.interactive(), in: .capsule)

                tool(snapshot.itemsTitle, "square.3.layers.3d") { withAnimation { showItems.toggle() } }
                    .padding(6)
                    .glassEffect(showItems ? .regular.tint(.accentColor).interactive() : .regular.interactive(), in: .circle)
            }
        }
    }

    /// Open each photo as the app opens a file, in the library's own format (HEIC stays HEIC).
    private func importPhotos(_ items: [PhotosPickerItem]) async {
        for item in items {
            do {
                guard let photo = try await item.loadTransferable(type: CraftPhoto.self) else { continue }
                bridge.open(name: photo.name, data: photo.data)
            } catch {
                bridge.fileError = "Couldn't import the photo: \(error.localizedDescription)"
            }
        }
    }

    private func tool(_ label: String, _ symbol: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.title3)
                .frame(width: 44, height: 44)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// The active document and its items, as the app's own panel lists them.
struct CraftItemsPanel: View {
    let snapshot: CraftSnapshot
    let perform: (CraftAction) -> Void

    var body: some View {
        List {
            if let document = snapshot.document {
                Section {
                    ForEach(snapshot.items) { item in row(item) }
                } header: {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack(spacing: 6) {
                            Text(document.name).font(.headline)
                            if document.dirty {
                                Circle().fill(.secondary).frame(width: 6, height: 6).accessibilityLabel("Edited")
                            }
                        }
                        Text(document.detail).font(.caption).foregroundStyle(.secondary)
                    }
                    .textCase(nil)
                }
            } else {
                ContentUnavailableView("No Document", systemImage: "doc", description: Text("Open or create a document to see its \(snapshot.itemsTitle.lowercased())."))
            }
        }
        .navigationTitle(snapshot.itemsTitle)
    }

    private func row(_ item: CraftSnapshot.Item) -> some View {
        HStack(spacing: 10) {
            if let visible = item.visible {
                Button {
                    perform(.setVisible(item.id, !visible))
                } label: {
                    Image(systemName: visible ? "eye" : "eye.slash")
                        .foregroundStyle(visible ? .primary : .tertiary)
                        .frame(width: 28)
                }
                .buttonStyle(.borderless)
                .accessibilityLabel(visible ? "Hide \(item.name)" : "Show \(item.name)")
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(item.name).lineLimit(1)
                Text(item.kind).font(.caption2).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .padding(.leading, CGFloat(item.depth) * 16)
        .contentShape(.rect)
        .onTapGesture { perform(.select(item.id)) }
        .listRowBackground(item.selected ? Color.accentColor.opacity(0.25) : nil)
        .accessibilityAddTraits(item.selected ? .isSelected : [])
    }
}

/// Bytes the app offers to save, as the system exporter wants them.
struct ExportDocument: FileDocument {
    static let readableContentTypes: [UTType] = [.data]
    let data: Data

    init(_ export: CraftExport) { data = export.data }
    init(configuration: ReadConfiguration) throws { data = configuration.file.regularFileContents ?? Data() }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: data)
    }
}

/// A photo from the library: its original file, named for when it arrived.
struct CraftPhoto: Transferable {
    let name: String
    let data: Data

    static var transferRepresentation: some TransferRepresentation {
        // The file representation is the original (HEIC, PNG, JPEG, …), not a re-encoding.
        FileRepresentation(importedContentType: .image) { received in
            let file = received.file
            let ext = file.pathExtension.isEmpty ? "jpeg" : file.pathExtension.lowercased()
            return CraftPhoto(name: "Photo \(Date.now.formatted(photoDate)).\(ext)", data: try Data(contentsOf: file))
        }
    }

    /// "2026-11-06 at 14.02", as Screenshots and the camera name files.
    private static let photoDate = Date.VerbatimFormatStyle(
        format: "\(year: .defaultDigits)-\(month: .twoDigits)-\(day: .twoDigits) at \(hour: .twoDigits(clock: .twentyFourHour, hourCycle: .zeroBased)).\(minute: .twoDigits)",
        timeZone: .current,
        calendar: Calendar(identifier: .gregorian)
    )
}

/// Presents the share sheet for an export, from the toolbar's corner of the window, once nothing
/// else (the Save-or-Share dialog) is on screen. The bytes go through a temporary file so the
/// receiving app sees the export's name.
struct CraftSharePresenter: UIViewControllerRepresentable {
    let export: CraftExport?
    let done: @MainActor (Error?) -> Void

    func makeUIViewController(context: Context) -> UIViewController {
        let controller = UIViewController()
        controller.view.isUserInteractionEnabled = false
        return controller
    }

    func updateUIViewController(_ controller: UIViewController, context: Context) {
        guard let export, context.coordinator.shown != export else { return }
        context.coordinator.shown = export
        let done = done
        // Writing a large file takes a moment; finish it even if the user switches away.
        let task = CraftBackgroundTask(name: "Share")
        task.begin()
        defer { task.end() }
        let folder = FileManager.default.temporaryDirectory.appending(path: "Share/\(UUID().uuidString)", directoryHint: .isDirectory)
        let file = folder.appending(path: export.name)
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            try export.data.write(to: file)
        } catch {
            DispatchQueue.main.async { done(error) }
            return
        }
        let sheet = UIActivityViewController(activityItems: [file], applicationActivities: nil)
        sheet.completionWithItemsHandler = { [weak coordinator = context.coordinator] _, _, _, error in
            try? FileManager.default.removeItem(at: folder)
            MainActor.assumeIsolated {
                coordinator?.shown = nil
                done(error)
            }
        }
        if let popover = sheet.popoverPresentationController {
            popover.sourceView = controller.view
            let bounds = controller.view.bounds
            popover.sourceRect = CGRect(x: bounds.midX, y: bounds.maxY - 80, width: 1, height: 1)
            popover.permittedArrowDirections = .down
        }
        Self.present(sheet, over: controller)
    }

    /// Present once the dialog that asked Save or Share has gone: it is still on screen when its
    /// button's action runs, and anything presented over it would leave with it.
    private static func present(_ sheet: UIViewController, over controller: UIViewController, attempts: Int = 20) {
        var top: UIViewController = controller.view.window?.rootViewController ?? controller
        while let presented = top.presentedViewController {
            guard !(presented is UIAlertController), !presented.isBeingDismissed else {
                guard attempts > 0 else { return }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                    MainActor.assumeIsolated { present(sheet, over: controller, attempts: attempts - 1) }
                }
                return
            }
            top = presented
        }
        top.present(sheet, animated: true)
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor
    final class Coordinator {
        var shown: CraftExport?
    }
}
