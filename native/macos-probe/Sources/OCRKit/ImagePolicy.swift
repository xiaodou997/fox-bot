import Foundation
import CoreGraphics
import ProbeKit

public enum OCRFailure: Error {
    case invalidGeometry, resourceLimit, unsupportedLanguages, recognitionFailed
    case accessibilityRequired, focusedWindowUnavailable, enumerationFailed
}

public enum WindowSelectionMode: String, Encodable {
    case unique = "UNIQUE_WINDOW"
    case focused = "FOCUSED_WINDOW"
}

/// AX and ScreenCaptureKit both expose screen-point rectangles. No title or coordinate is exported.
public func sameWindowFrame(_ first: CGRect, _ second: CGRect) -> Bool {
    let a = [first.minX, first.minY, first.width, first.height]
    let b = [second.minX, second.minY, second.width, second.height]
    return zip(a, b).allSatisfy { $0.isFinite && $1.isFinite && abs($0 - $1) <= 0.5 }
}

public struct ImagePlan: Equatable {
    public let width: Int
    public let height: Int
    public let downscaled: Bool
    public static let maxDimension = 4096
    public static let maxPixels = 8_388_608

    public static func make(size: CGSize, scale: Double) throws -> ImagePlan {
        let w = Double(size.width), h = Double(size.height)
        guard w.isFinite, h.isFinite, scale.isFinite, w >= 16, h >= 16,
              w <= 32768, h <= 32768, scale >= 0.5, scale <= 4 else {
            throw OCRFailure.invalidGeometry
        }
        let pw = w * scale, ph = h * scale
        let factor = min(1, Double(maxDimension) / max(pw, ph), sqrt(Double(maxPixels) / (pw * ph)))
        let width = Int(floor(pw * factor)), height = Int(floor(ph * factor))
        guard accepts(width: width, height: height) else { throw OCRFailure.resourceLimit }
        return ImagePlan(width: width, height: height, downscaled: factor < 1)
    }
    public static func accepts(width: Int, height: Int) -> Bool {
        width >= 8 && height >= 8 && width <= maxDimension && height <= maxDimension
            && width * height <= maxPixels
    }
}

/// Private-to-process target evidence, never serialized as a public report or used as send authority.
public struct CaptureWindow: Equatable {
    public let id: UInt32
    /// Stable target root process identity. The compositor owner may be a verified child app.
    public let pid: Int32
    public let bundleID: String
    public let ownerPid: Int32
    public let ownerBundleID: String
    public let frame: CGRect
    public let contentSize: CGSize
    public let scale: Double
    public let onScreen: Bool
    public let active: Bool
    public let layer: Int
    public init(id: UInt32, pid: Int32, bundleID: String, frame: CGRect,
                contentSize: CGSize, scale: Double, onScreen: Bool = true, layer: Int = 0,
                active: Bool = false, ownerPid: Int32? = nil, ownerBundleID: String? = nil) {
        self.id = id; self.pid = pid; self.bundleID = bundleID
        self.ownerPid = ownerPid ?? pid; self.ownerBundleID = ownerBundleID ?? bundleID
        self.frame = frame; self.contentSize = contentSize; self.scale = scale
        self.onScreen = onScreen; self.active = active; self.layer = layer
    }
    public func eligible(for app: TargetApp, pid: Int32, allowOffscreen: Bool = false) -> Bool {
        self.pid == pid && bundleID == app.bundleID && (allowOffscreen || onScreen) && layer == 0
    }
    public var hasValidGeometry: Bool {
        frame.origin.x.isFinite && frame.origin.y.isFinite && frame.width.isFinite && frame.height.isFinite
            && frame.width >= 16 && frame.height >= 16 && frame.width <= 32768 && frame.height <= 32768
    }
}

/// Convert Vision's normalized bottom-left rectangle to normalized top-left coordinates.
/// These are image-relative, NOT desktop click coordinates.
public func topLeftBounds(_ rect: CGRect) throws -> CGRect {
    let values = [rect.origin.x, rect.origin.y, rect.width, rect.height]
    guard values.allSatisfy({ $0.isFinite }), rect.width > 0, rect.height > 0,
          rect.minX >= 0, rect.minY >= 0, rect.maxX <= 1.000001, rect.maxY <= 1.000001 else {
        throw OCRFailure.invalidGeometry
    }
    return CGRect(x: rect.minX, y: max(0, 1 - rect.maxY),
                  width: min(rect.width, 1 - rect.minX), height: min(rect.height, 1 - rect.minY))
}
