// RiseGlass — the AppKit half of tier 1 in docs/PLATFORM_GLASS.md.
//
// THE C ABI CONTRACT
//
//   Every entry point except rise_glass_probe_backing and
//   rise_glass_vibrancy_material_name touches AppKit view state and MUST be
//   called on the main thread. Calling one off it is undefined behaviour, not a
//   lint, so each of those functions checks Thread.isMainThread and refuses with
//   riseGlassNotMainThread rather than proceeding. The two exceptions are pure
//   lookups and are safe from any thread.
//
//   The handle returned by rise_glass_attach is a +1 reference. Exactly one
//   rise_glass_detach must be paired with it, and no other entry point may be
//   called after that.
//
//   A layout pass is rise_glass_begin, then one rise_glass_region per region,
//   then rise_glass_commit. Regions not named between a begin and its commit are
//   destroyed by that commit; a region named again keeps the view it already
//   has and is moved.

import AppKit

private let riseGlassOk: Int32 = 0
private let riseGlassNotMainThread: Int32 = -1
private let riseGlassNoHandle: Int32 = -2
private let riseGlassNoHostView: Int32 = -3
private let riseGlassNoViewClass: Int32 = -4
private let riseGlassUnknownMaterial: Int32 = -5

private let riseGlassBackingVibrancy: Int32 = 1
private let riseGlassBackingLiquidGlass: Int32 = 2

private let riseGlassMaterialChrome: Int32 = 0
private let riseGlassMaterialPanel: Int32 = 1
private let riseGlassMaterialOverlay: Int32 = 2

// Leaked once and never freed, so the pointer handed across the ABI stays valid
// for the life of the process. Three strings is the whole cost.
private let chromeMaterialName = strdup("NSVisualEffectMaterial.headerView")
private let panelMaterialName = strdup("NSVisualEffectMaterial.sidebar")
private let overlayMaterialName = strdup("NSVisualEffectMaterial.popover")

/// Flipped so that a region rectangle arrives in the same top-left origin the
/// layout pass computed it in, with no conversion on either side of the ABI.
private final class RiseGlassContainer: NSView {
    override var isFlipped: Bool { true }

    /// The container sits *below* the view GPUI renders into, so a click only
    /// ever reaches it through a rectangle that view declined. GPUI would never
    /// see such a click, and a material is not a control, so the whole subtree
    /// stays out of hit testing.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

private struct RiseGlassEntry {
    let view: NSView
    let backing: Int32
    let material: Int32
}

private final class RiseGlassHost {
    let container: RiseGlassContainer
    let hostView: NSView
    private var entries: [UInt64: RiseGlassEntry] = [:]
    private var touched: Set<UInt64> = []

    init(container: RiseGlassContainer, hostView: NSView) {
        self.container = container
        self.hostView = hostView
    }

    func begin() {
        touched.removeAll(keepingCapacity: true)
        reseat()
    }

    func region(
        id: UInt64,
        backing: Int32,
        material: Int32,
        rect: NSRect,
        radius: CGFloat
    ) -> Int32 {
        let existing = entries[id]
        let reusable = existing.map { $0.backing == backing && $0.material == material } ?? false

        let view: NSView
        if reusable, let entry = existing {
            view = entry.view
        } else {
            existing?.view.removeFromSuperview()
            guard let made = makeView(backing: backing, material: material) else {
                entries.removeValue(forKey: id)
                return backing == riseGlassBackingLiquidGlass
                    ? riseGlassNoViewClass
                    : riseGlassUnknownMaterial
            }
            container.addSubview(made)
            entries[id] = RiseGlassEntry(view: made, backing: backing, material: material)
            view = made
        }

        view.frame = rect
        apply(radius: radius, backing: backing, to: view)
        touched.insert(id)
        return riseGlassOk
    }

    func commit() -> Int32 {
        for id in entries.keys where !touched.contains(id) {
            entries[id]?.view.removeFromSuperview()
            entries.removeValue(forKey: id)
        }
        touched.removeAll(keepingCapacity: true)
        return riseGlassOk
    }

    func clear() -> Int32 {
        for entry in entries.values {
            entry.view.removeFromSuperview()
        }
        entries.removeAll()
        touched.removeAll()
        return riseGlassOk
    }

    func detach() {
        _ = clear()
        container.removeFromSuperview()
    }

    /// GPUI adds its Metal view once and never reorders the content view's
    /// children, but a blurred window background inserts an NSVisualEffectView
    /// at the very back at whatever moment the app sets it. Re-asserting the
    /// position each pass is what keeps the container between the two: at the
    /// very back it would be hidden behind that window material, and above the
    /// Metal view it would cover the whole app.
    private func reseat() {
        guard let parent = hostView.superview else { return }

        let seated =
            container.superview === parent
            && (parent.subviews.firstIndex(of: container) ?? Int.max)
                < (parent.subviews.firstIndex(of: hostView) ?? Int.min)

        if !seated {
            container.removeFromSuperview()
            parent.addSubview(container, positioned: .below, relativeTo: hostView)
        }

        container.frame = parent.bounds
    }

    private func makeView(backing: Int32, material: Int32) -> NSView? {
        switch backing {
        case riseGlassBackingLiquidGlass:
            return makeGlassView()
        case riseGlassBackingVibrancy:
            return makeVibrancyView(material)
        default:
            return nil
        }
    }

    /// Resolved by name rather than referenced, so this file still compiles
    /// against an SDK that has never heard of the class and the app it produces
    /// still falls back to vibrancy at runtime instead of failing to launch.
    private func makeGlassView() -> NSView? {
        guard #available(macOS 26.0, *) else { return nil }
        guard let glassClass = NSClassFromString("NSGlassEffectView") as? NSView.Type else {
            return nil
        }
        return glassClass.init(frame: .zero)
    }

    private func makeVibrancyView(_ material: Int32) -> NSView? {
        guard let vibrancy = vibrancyMaterial(material) else { return nil }

        let view = NSVisualEffectView(frame: .zero)
        view.material = vibrancy
        view.blendingMode = .behindWindow
        view.state = .followsWindowActiveState
        return view
    }

    /// A native backing cannot be clipped by GPUI drawing a rounded mask over
    /// it — the view is below the Metal layer, so the mask would land on top of
    /// it — which is why the radius travels with the region and is applied here.
    private func apply(radius: CGFloat, backing: Int32, to view: NSView) {
        if backing == riseGlassBackingLiquidGlass,
            view.responds(to: NSSelectorFromString("setCornerRadius:"))
        {
            view.setValue(NSNumber(value: Double(radius)), forKey: "cornerRadius")
            return
        }

        view.wantsLayer = true
        view.layer?.cornerRadius = radius
        view.layer?.masksToBounds = radius > 0
    }
}

private func vibrancyMaterial(_ code: Int32) -> NSVisualEffectView.Material? {
    switch code {
    case riseGlassMaterialChrome: return .headerView
    case riseGlassMaterialPanel: return .sidebar
    case riseGlassMaterialOverlay: return .popover
    default: return nil
    }
}

/// GPUI names its Metal-hosting view `GPUIView` and its blurred-background view
/// `BlurredView`, both registered at runtime. Matching the first by name and
/// falling back to "the topmost child that is not a visual effect view" keeps
/// this working if that name ever changes, without silently picking the window
/// material as the anchor.
private func hostView(in content: NSView) -> NSView? {
    for view in content.subviews.reversed() where !(view is RiseGlassContainer) {
        if String(cString: object_getClassName(view)) == "GPUIView" {
            return view
        }
    }

    for view in content.subviews.reversed()
    where !(view is RiseGlassContainer) && !(view is NSVisualEffectView) {
        return view
    }

    return nil
}

private func locateHostView() -> NSView? {
    let app = NSApplication.shared
    var windows: [NSWindow] = []
    if let key = app.keyWindow { windows.append(key) }
    if let main = app.mainWindow { windows.append(main) }
    windows.append(contentsOf: app.windows)

    for window in windows {
        if let content = window.contentView, let view = hostView(in: content) {
            return view
        }
    }
    return nil
}

/// The best backing the AppKit side can actually host, independent of what the
/// OS version alone would suggest. Pure, and safe from any thread.
@_cdecl("rise_glass_probe_backing")
public func rise_glass_probe_backing() -> Int32 {
    if #available(macOS 26.0, *), NSClassFromString("NSGlassEffectView") != nil {
        return riseGlassBackingLiquidGlass
    }
    return riseGlassBackingVibrancy
}

/// The `NSVisualEffectMaterial` case a material is configured with, so the Rust
/// side can assert this file and `Material::vibrancy_material` never drifted
/// apart. Pure, and safe from any thread.
@_cdecl("rise_glass_vibrancy_material_name")
public func rise_glass_vibrancy_material_name(_ material: Int32) -> UnsafePointer<CChar>? {
    let name: UnsafeMutablePointer<CChar>?
    switch material {
    case riseGlassMaterialChrome: name = chromeMaterialName
    case riseGlassMaterialPanel: name = panelMaterialName
    case riseGlassMaterialOverlay: name = overlayMaterialName
    default: return nil
    }
    return name.map { UnsafePointer($0) }
}

/// Main thread only. Returns nil when there is no window to host regions in,
/// which is an ordinary state before the first window opens.
@_cdecl("rise_glass_attach")
public func rise_glass_attach() -> UnsafeMutableRawPointer? {
    guard Thread.isMainThread else { return nil }
    guard let host = locateHostView(), let parent = host.superview else { return nil }

    let container = RiseGlassContainer(frame: parent.bounds)
    container.autoresizingMask = [.width, .height]
    parent.addSubview(container, positioned: .below, relativeTo: host)

    return Unmanaged.passRetained(RiseGlassHost(container: container, hostView: host)).toOpaque()
}

/// Main thread only.
@_cdecl("rise_glass_begin")
public func rise_glass_begin(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard Thread.isMainThread else { return riseGlassNotMainThread }
    guard let host = handle else { return riseGlassNoHandle }
    Unmanaged<RiseGlassHost>.fromOpaque(host).takeUnretainedValue().begin()
    return riseGlassOk
}

/// Main thread only.
@_cdecl("rise_glass_region")
public func rise_glass_region(
    _ handle: UnsafeMutableRawPointer?,
    _ id: UInt64,
    _ backing: Int32,
    _ material: Int32,
    _ x: Double,
    _ y: Double,
    _ width: Double,
    _ height: Double,
    _ radius: Double
) -> Int32 {
    guard Thread.isMainThread else { return riseGlassNotMainThread }
    guard let host = handle else { return riseGlassNoHandle }

    return Unmanaged<RiseGlassHost>.fromOpaque(host).takeUnretainedValue().region(
        id: id,
        backing: backing,
        material: material,
        rect: NSRect(x: x, y: y, width: width, height: height),
        radius: CGFloat(radius)
    )
}

/// Main thread only.
@_cdecl("rise_glass_commit")
public func rise_glass_commit(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard Thread.isMainThread else { return riseGlassNotMainThread }
    guard let host = handle else { return riseGlassNoHandle }
    return Unmanaged<RiseGlassHost>.fromOpaque(host).takeUnretainedValue().commit()
}

/// Main thread only.
@_cdecl("rise_glass_clear")
public func rise_glass_clear(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard Thread.isMainThread else { return riseGlassNotMainThread }
    guard let host = handle else { return riseGlassNoHandle }
    return Unmanaged<RiseGlassHost>.fromOpaque(host).takeUnretainedValue().clear()
}

/// Main thread only. Consumes the handle; nothing may use it afterwards.
@_cdecl("rise_glass_detach")
public func rise_glass_detach(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let host = handle else { return riseGlassNoHandle }
    guard Thread.isMainThread else { return riseGlassNotMainThread }

    let box = Unmanaged<RiseGlassHost>.fromOpaque(host)
    box.takeUnretainedValue().detach()
    box.release()
    return riseGlassOk
}
