import Foundation
import CoreGraphics
import ProbeKit

public struct CaptureMetadata {
    public var runningInstances: Int
    public var pid: Int32?
    public var launchTime: TimeInterval?
    public var permission: Bool
    public var osVersion: String
    public var applicationVersion: String?
    public init(runningInstances: Int, pid: Int32? = nil, launchTime: TimeInterval? = nil,
                permission: Bool, osVersion: String, applicationVersion: String? = nil) {
        self.runningInstances = runningInstances; self.pid = pid; self.launchTime = launchTime
        self.permission = permission; self.osVersion = osVersion; self.applicationVersion = applicationVersion
    }
}
public protocol WindowSource: AnyObject {
    func metadata(_ app: TargetApp) -> CaptureMetadata
    func windows(_ app: TargetApp, pid: Int32) async throws -> [CaptureWindow]
    func focusedFrame(_ app: TargetApp, pid: Int32) throws -> CGRect
    func capture(_ window: CaptureWindow, plan: ImagePlan) async throws -> CGImage
}
public extension WindowSource {
    func focusedFrame(_ app: TargetApp, pid: Int32) throws -> CGRect { throw OCRFailure.focusedWindowUnavailable }
}
public struct WindowMatchSummary: Encodable {
    public let candidateCount: Int
    public let originMatches: Int
    public let sizeMatches: Int
    public let frameMatches: Int
}
public struct CapturedImageSummary: Encodable {
    public let width: Int
    public let height: Int
    public let downscaled: Bool
}
public struct WindowOCRReport: Encodable {
    public let schemaVersion = "foxbot.macos-ocr.v1"
    public let readOnly = true
    public let rawTextIncluded = false
    public let imageSaved = false
    public let networkRequests = 0
    public let captureScope = "SINGLE_WINDOW"
    public let contentScope = "WINDOW_NOT_CHAT"
    public let accountIdentity = "UNVERIFIED"
    public let conversationIdentity = "UNVERIFIED"
    public let sendCapability = "NOT_IMPLEMENTED"
    public var snapshotId = UUID().uuidString
    public let app: String
    public let bundleId: String
    public let osVersion: String
    public let applicationVersion: String?
    public let captureRequested: Bool
    public let ocrRequested: Bool
    public let selectionMode: WindowSelectionMode
    public let screenCapturePreflight: Bool
    public let runningInstances: Int
    public var eligibleWindows: Int?
    public var windowMatching: WindowMatchSummary?
    public var status = "METADATA_ONLY"
    // UNKNOWN is retained once the capture API has been invoked but no image was returned.
    public var captureState = "NOT_ATTEMPTED"
    public var ocrAttempted = false
    public var windowStable: Bool?
    public var image: CapturedImageSummary?
    public var ocr: OCRStatistics?
}

public func captureGate(instances: Int, requested: Bool, permission: Bool) -> String {
    if instances == 0 { return "NOT_RUNNING" }
    if instances != 1 { return "AMBIGUOUS_INSTANCE" }
    if !requested { return "METADATA_ONLY" }
    if !permission { return "PERMISSION_REQUIRED" }
    return "READY"
}

public enum WindowOCRProbe {
    /// No retries and no whole-display fallback. Unknown identity never becomes a send target.
    public static func run(app: TargetApp, requested: Bool, ocrRequested: Bool = true, source: WindowSource,
                           selection: WindowSelectionMode = .unique,
                           recognize: (CGImage) throws -> OCRSnapshot = VisionOCR.recognize,
                           now: () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) async -> WindowOCRReport {
        let started = now()
        let metadata = source.metadata(app)
        var report = WindowOCRReport(app: app.rawValue, bundleId: app.bundleID,
            osVersion: metadata.osVersion, applicationVersion: metadata.applicationVersion,
            captureRequested: requested, ocrRequested: requested && ocrRequested,
            selectionMode: selection, screenCapturePreflight: metadata.permission,
            runningInstances: metadata.runningInstances)
        report.status = captureGate(instances: metadata.runningInstances, requested: requested, permission: metadata.permission)
        guard report.status == "READY" else { return report }
        guard let pid = metadata.pid else { report.status = "TARGET_CHANGED"; report.windowStable = false; return report }
        let selected: CaptureWindow
        do {
            let choice = try await candidates(app, pid, selection, source)
            let eligible = choice.windows
            report.windowMatching = choice.matching
            report.eligibleWindows = eligible.count
            guard !eligible.isEmpty else { report.status = "NO_ELIGIBLE_WINDOW"; return report }
            guard eligible.count == 1 else { report.status = "AMBIGUOUS_WINDOW"; return report }
            selected = eligible[0]
        } catch OCRFailure.accessibilityRequired { report.status = "ACCESSIBILITY_REQUIRED"; return report }
        catch OCRFailure.focusedWindowUnavailable { report.status = "NO_FOCUSED_WINDOW"; return report }
        catch OCRFailure.resourceLimit { report.status = "RESOURCE_LIMIT"; return report }
        catch { report.status = "ENUMERATION_FAILED"; return report }
        guard selected.hasValidGeometry else { report.status = "INVALID_GEOMETRY"; return report }
        let plan: ImagePlan
        do { plan = try ImagePlan.make(size: selected.contentSize, scale: selected.scale) }
        catch { report.status = "INVALID_GEOMETRY"; return report }
        guard now() - started < 10 else { report.status = "TIME_BUDGET_EXCEEDED"; return report }
        // Re-enumerate just before capture. A changed/multiple/hidden window is not silently substituted.
        guard await stable(app, metadata, selected, selection, source) else {
            report.status = "TARGET_CHANGED"; report.windowStable = false; return report
        }
        guard now() - started < 10 else { report.status = "TIME_BUDGET_EXCEEDED"; return report }
        report.captureState = "UNKNOWN"
        let image: CGImage
        do { image = try await source.capture(selected, plan: plan) }
        catch { report.status = "CAPTURE_FAILED"; return report }
        report.captureState = "IMAGE_OBTAINED"
        guard ImagePlan.accepts(width: image.width, height: image.height),
              image.width == plan.width, image.height == plan.height else {
            report.status = "CAPTURE_SIZE_MISMATCH"; return report
        }
        report.image = CapturedImageSummary(width: image.width, height: image.height, downscaled: plan.downscaled)
        guard await stable(app, metadata, selected, selection, source) else {
            report.status = "TARGET_CHANGED"; report.windowStable = false; return report
        }
        guard now() - started < 10 else { report.status = "TIME_BUDGET_EXCEEDED"; return report }
        if !report.ocrRequested {
            report.windowStable = true
            report.status = "CAPTURE_SUMMARY"
            return report
        }
        report.ocrAttempted = true
        let snapshot: OCRSnapshot
        do { snapshot = try recognize(image) }
        catch OCRFailure.unsupportedLanguages { report.status = "LANGUAGE_UNAVAILABLE"; return report }
        catch { report.status = "OCR_FAILED"; return report }
        guard now() - started < 10 else { report.status = "TIME_BUDGET_EXCEEDED"; return report }
        guard await stable(app, metadata, selected, selection, source) else {
            report.status = "TARGET_CHANGED"; report.windowStable = false; return report
        }
        guard now() - started < 10 else { report.status = "TIME_BUDGET_EXCEEDED"; return report }
        report.windowStable = true
        report.ocr = snapshot.statistics // NEVER serialize snapshot.lines.
        report.status = !snapshot.statistics.completeRecognition ? "OCR_PARTIAL_SUMMARY"
            : (snapshot.lines.isEmpty ? "OCR_EMPTY" : "OCR_SUMMARY")
        return report
    }

    private static func candidates(_ app: TargetApp, _ pid: Int32, _ selection: WindowSelectionMode,
                                   _ source: WindowSource) async throws -> (windows: [CaptureWindow], matching: WindowMatchSummary?) {
        let windows = try await source.windows(app, pid: pid)
        guard windows.count <= 4096 else { throw OCRFailure.resourceLimit }
        let eligible = windows.filter {
            $0.eligible(for: app, pid: pid, allowOffscreen: selection == .focused)
        }
        guard selection == .focused else { return (eligible, nil) }
        let frame = try source.focusedFrame(app, pid: pid)
        guard frame.width >= 16, frame.height >= 16, frame.origin.x.isFinite, frame.origin.y.isFinite,
              frame.width.isFinite, frame.height.isFinite else { throw OCRFailure.focusedWindowUnavailable }
        let matches = eligible.filter { sameWindowFrame($0.frame, frame) }
        let origins = eligible.filter { abs($0.frame.minX - frame.minX) <= 0.5 && abs($0.frame.minY - frame.minY) <= 0.5 }.count
        let sizes = eligible.filter { abs($0.frame.width - frame.width) <= 0.5 && abs($0.frame.height - frame.height) <= 0.5 }.count
        return (matches, WindowMatchSummary(candidateCount: eligible.count, originMatches: origins,
                                            sizeMatches: sizes, frameMatches: matches.count))
    }

    private static func stable(_ app: TargetApp, _ expected: CaptureMetadata,
                               _ window: CaptureWindow, _ selection: WindowSelectionMode, _ source: WindowSource) async -> Bool {
        let current = source.metadata(app)
        guard current.permission, current.runningInstances == 1, current.pid == expected.pid,
              current.launchTime == expected.launchTime, let pid = current.pid else { return false }
        guard let choice = try? await candidates(app, pid, selection, source) else { return false }
        return choice.windows.count == 1 && choice.windows[0] == window
    }
}
