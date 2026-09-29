import XCTest
import Foundation
import CoreGraphics
import ProbeKit
@testable import OCRKit

final class FakeWindowSource: WindowSource {
    var instances = 1, pid: Int32 = 42, permission = true
    var windowsCalls = 0, captures = 0, metadataCalls = 0
    var failEnumeration = false, failCapture = false, wrongImageSize = false
    var focusAvailable = true, focusPermission = true
    var focusFrameValue = CGRect(x: 100, y: 100, width: 600, height: 300)
    var permissionLostAt = Int.max, changeAt = Int.max
    var items = [CaptureWindow(id: 7, pid: 42, bundleID: TargetApp.wechat.bundleID,
                frame: CGRect(x: 100, y: 100, width: 600, height: 300), contentSize: CGSize(width: 600, height: 300), scale: 1)]
    func metadata(_ app: TargetApp) -> CaptureMetadata {
        metadataCalls += 1
        return CaptureMetadata(runningInstances: instances, pid: instances == 1 ? pid : nil,
            launchTime: 1, permission: permission && metadataCalls < permissionLostAt, osVersion: "27.0.0")
    }
    func windows(_ app: TargetApp, pid: Int32) async throws -> [CaptureWindow] {
        windowsCalls += 1
        if failEnumeration { throw OCRFailure.enumerationFailed }
        return windowsCalls >= changeAt ? [] : items
    }
    func focusedFrame(_ app: TargetApp, pid: Int32) throws -> CGRect {
        guard focusPermission else { throw OCRFailure.accessibilityRequired }
        guard focusAvailable else { throw OCRFailure.focusedWindowUnavailable }
        return focusFrameValue
    }
    func capture(_ window: CaptureWindow, plan: ImagePlan) async throws -> CGImage {
        captures += 1
        if failCapture { throw OCRFailure.resourceLimit }
        return imageFixture(text: false, width: wrongImageSize ? 10 : plan.width, height: plan.height)
    }
}
private func fakeRecognition(_ image: CGImage) -> OCRSnapshot {
    VisionOCR.bounded([OCRLine(text: "SYNTHETIC_PRIVATE_MESSAGE", confidence: 0.9,
                              bounds: CGRect(x: 0.1, y: 0.5, width: 0.5, height: 0.1))])
}

final class WindowProbeTests: XCTestCase {
    func testDefaultNeverEnumeratesCapturesOrRecognizes() async {
        let source = FakeWindowSource()
        let report = await WindowOCRProbe.run(app: .wechat, requested: false, source: source,
                                             recognize: { _ in XCTFail("OCR without permission"); return fakeRecognition(imageFixture(text: false)) })
        XCTAssertEqual(report.status, "METADATA_ONLY"); XCTAssertEqual(source.windowsCalls, 0)
        XCTAssertEqual(source.captures, 0); XCTAssertFalse(report.ocrAttempted)
    }
    func testMissingAndAmbiguousInstancesDoNotSelectTargets() async {
        for count in [0, 2] {
            let source = FakeWindowSource(); source.instances = count
            let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
            XCTAssertEqual(report.status, count == 0 ? "NOT_RUNNING" : "AMBIGUOUS_INSTANCE")
            XCTAssertEqual(source.windowsCalls, 0)
        }
    }
    func testDeniedPermissionNeverEnumeratesOrPrompts() async {
        let source = FakeWindowSource(); source.permission = false
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "PERMISSION_REQUIRED"); XCTAssertEqual(source.windowsCalls, 0)
    }
    func testMultipleEligibleWindowsDoNotPickFirst() async {
        let source = FakeWindowSource(); source.items.append(source.items[0])
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "AMBIGUOUS_WINDOW"); XCTAssertEqual(source.captures, 0)
    }
    func testWrongAppPidHiddenAndPopupWindowsAreNotCaptured() async {
        let source = FakeWindowSource()
        source.items = [CaptureWindow(id: 1, pid: 42, bundleID: TargetApp.qq.bundleID, frame: .zero, contentSize: .zero, scale: 1),
                        CaptureWindow(id: 2, pid: 99, bundleID: TargetApp.wechat.bundleID, frame: .zero, contentSize: .zero, scale: 1),
                        CaptureWindow(id: 3, pid: 42, bundleID: TargetApp.wechat.bundleID, frame: .zero, contentSize: .zero, scale: 1, onScreen: false),
                        CaptureWindow(id: 4, pid: 42, bundleID: TargetApp.wechat.bundleID, frame: .zero, contentSize: .zero, scale: 1, layer: 3)]
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "NO_ELIGIBLE_WINDOW"); XCTAssertEqual(source.captures, 0)
    }
    func testExplicitFocusedModeSelectsOnlyUniquelyMatchingGeometry() async {
        let source = FakeWindowSource()
        source.items.append(CaptureWindow(id: 8, pid: 42, bundleID: TargetApp.wechat.bundleID,
            frame: CGRect(x: 0, y: 0, width: 200, height: 300), contentSize: CGSize(width: 200, height: 300), scale: 1))
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source,
            selection: .focused, recognize: fakeRecognition)
        XCTAssertEqual(report.status, "OCR_SUMMARY"); XCTAssertEqual(source.captures, 1)
        XCTAssertEqual(report.selectionMode, .focused)
        XCTAssertEqual(report.windowMatching?.candidateCount, 2)
        XCTAssertEqual(report.windowMatching?.frameMatches, 1)
    }
    func testFocusedModeFailsWithoutAXPermissionOrStandardFocusedWindow() async {
        for permission in [false, true] {
            let source = FakeWindowSource(); source.focusPermission = permission; source.focusAvailable = false
            let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source, selection: .focused)
            XCTAssertEqual(report.status, permission ? "NO_FOCUSED_WINDOW" : "ACCESSIBILITY_REQUIRED")
            XCTAssertEqual(source.captures, 0)
        }
    }
    func testTwoWindowsAtSameFrameRemainAmbiguousEvenWithFocus() async {
        let source = FakeWindowSource(); source.items.append(source.items[0])
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source, selection: .focused)
        XCTAssertEqual(report.status, "AMBIGUOUS_WINDOW"); XCTAssertEqual(source.captures, 0)
    }
    func testActiveOnScreenDuplicateWinsOverOffscreenCompositorMirror() async {
        let source = FakeWindowSource()
        let frame = source.focusFrameValue
        source.items = [
            CaptureWindow(id: 7, pid: 42, bundleID: TargetApp.wechat.bundleID,
                frame: frame, contentSize: frame.size, scale: 1, onScreen: true, active: true),
            CaptureWindow(id: 8, pid: 42, bundleID: TargetApp.wechat.bundleID,
                frame: frame, contentSize: frame.size, scale: 1, onScreen: false, active: false,
                ownerPid: 77, ownerBundleID: "com.tencent.flue.WeChatAppEx")
        ]
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, ocrRequested: false,
            source: source, selection: .focused)
        XCTAssertEqual(report.status, "CAPTURE_SUMMARY")
        XCTAssertEqual(report.eligibleWindows, 1)
        XCTAssertEqual(report.windowMatching?.originMatches, 2)
        XCTAssertEqual(report.windowMatching?.sizeMatches, 2)
        XCTAssertEqual(report.windowMatching?.frameMatches, 1)
        XCTAssertEqual(source.captures, 1)
    }
    func testFocusedGeometryMismatchDoesNotFallBackToOnlyWindow() async {
        let source = FakeWindowSource(); source.focusFrameValue = CGRect(x: 400, y: 400, width: 600, height: 300)
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source, selection: .focused)
        XCTAssertEqual(report.status, "NO_ELIGIBLE_WINDOW"); XCTAssertEqual(source.captures, 0)
    }
    func testFocusedModeCanBindVerifiedOffscreenCompositorWindow() async {
        let source = FakeWindowSource()
        source.items = [CaptureWindow(id: 7, pid: 42, bundleID: TargetApp.wechat.bundleID,
            frame: source.focusFrameValue, contentSize: source.focusFrameValue.size, scale: 1,
            onScreen: false, ownerPid: 77, ownerBundleID: "com.tencent.flue.WeChatAppEx")]
        let focused = await WindowOCRProbe.run(app: .wechat, requested: true, source: source,
            selection: .focused, recognize: fakeRecognition)
        XCTAssertEqual(focused.status, "OCR_SUMMARY")
        XCTAssertEqual(source.captures, 1)
    }
    func testUniqueModeStillRejectsOffscreenOnlyWindow() async {
        let source = FakeWindowSource()
        source.items = [CaptureWindow(id: 7, pid: 42, bundleID: TargetApp.wechat.bundleID,
            frame: source.focusFrameValue, contentSize: source.focusFrameValue.size, scale: 1,
            onScreen: false, ownerPid: 77, ownerBundleID: "com.tencent.flue.WeChatAppEx")]
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "NO_ELIGIBLE_WINDOW")
        XCTAssertEqual(source.captures, 0)
    }
    func testWindowChangeBeforeCaptureDoesNotFallBackToDisplay() async {
        let source = FakeWindowSource(); source.changeAt = 2
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "TARGET_CHANGED"); XCTAssertEqual(source.captures, 0)
    }
    func testPermissionRevokedBeforeCapturePreventsCapture() async {
        let source = FakeWindowSource(); source.permissionLostAt = 2
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "TARGET_CHANGED"); XCTAssertEqual(source.captures, 0)
    }
    func testWindowChangeAfterCaptureDiscardsImageWithoutOCR() async {
        let source = FakeWindowSource(); source.changeAt = 3
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "TARGET_CHANGED"); XCTAssertEqual(report.captureState, "IMAGE_OBTAINED")
        XCTAssertFalse(report.ocrAttempted); XCTAssertNil(report.ocr)
    }
    func testWindowChangeAfterOCRDiscardsRecognizedContent() async {
        let source = FakeWindowSource(); source.changeAt = 4
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source, recognize: fakeRecognition)
        XCTAssertEqual(report.status, "TARGET_CHANGED"); XCTAssertTrue(report.ocrAttempted); XCTAssertNil(report.ocr)
    }
    func testCaptureFailureRetainsUnknownNotFalseNoCaptureClaim() async {
        let source = FakeWindowSource(); source.failCapture = true
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "CAPTURE_FAILED"); XCTAssertEqual(report.captureState, "UNKNOWN")
        XCTAssertFalse(report.ocrAttempted); XCTAssertEqual(source.captures, 1)
    }
    func testCaptureSizeMismatchNeverReachesOCR() async {
        let source = FakeWindowSource(); source.wrongImageSize = true
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "CAPTURE_SIZE_MISMATCH"); XCTAssertFalse(report.ocrAttempted)
    }
    func testEnumerationFailureNeverCaptures() async {
        let source = FakeWindowSource(); source.failEnumeration = true
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source)
        XCTAssertEqual(report.status, "ENUMERATION_FAILED"); XCTAssertEqual(source.captures, 0)
    }
    func testSoftDeadlineBeforeCaptureAndAfterRecognitionDiscardsResults() async {
        let source = FakeWindowSource(); var times = 0
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source,
            recognize: fakeRecognition, now: { times += 1; return times == 1 ? 0 : 11 })
        XCTAssertEqual(report.status, "TIME_BUDGET_EXCEEDED"); XCTAssertEqual(source.captures, 0)
        let later = FakeWindowSource(); times = 0
        let after = await WindowOCRProbe.run(app: .wechat, requested: true, source: later,
            recognize: fakeRecognition, now: { times += 1; return times < 5 ? 0 : 11 })
        XCTAssertEqual(after.status, "TIME_BUDGET_EXCEEDED"); XCTAssertNil(after.ocr)
    }
    func testSingleWindowSummaryDoesNotVerifyConversationOrLeakText() async throws {
        let source = FakeWindowSource()
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, source: source, recognize: fakeRecognition)
        XCTAssertEqual(report.status, "OCR_SUMMARY"); XCTAssertEqual(source.captures, 1)
        XCTAssertEqual(report.ocr?.lineCount, 1); XCTAssertEqual(report.windowStable, true)
        XCTAssertEqual(report.conversationIdentity, "UNVERIFIED")
        let json = String(decoding: try JSONEncoder().encode(report), as: UTF8.self)
        XCTAssertFalse(json.contains("SYNTHETIC_PRIVATE")); XCTAssertFalse(json.contains("windowID"))
    }
    func testCaptureOnlyReturnsVerifiedImageWithoutInvokingOCR() async {
        let source = FakeWindowSource()
        let report = await WindowOCRProbe.run(app: .wechat, requested: true, ocrRequested: false,
            source: source, recognize: { _ in
                XCTFail("capture-only must not invoke OCR")
                return fakeRecognition(imageFixture(text: false))
            })
        XCTAssertEqual(report.status, "CAPTURE_SUMMARY")
        XCTAssertEqual(report.captureState, "IMAGE_OBTAINED")
        XCTAssertEqual(report.windowStable, true)
        XCTAssertFalse(report.ocrAttempted)
        XCTAssertNil(report.ocr)
    }
    func testPartialRecognitionIsNotCompleteAndEmptyIsNotChatState() async {
        let source = FakeWindowSource()
        let partial = await WindowOCRProbe.run(app: .wechat, requested: true, source: source,
            recognize: { _ in VisionOCR.bounded([], overflow: true) })
        XCTAssertEqual(partial.status, "OCR_PARTIAL_SUMMARY")
        let empty = await WindowOCRProbe.run(app: .wechat, requested: true, source: FakeWindowSource(),
            recognize: { _ in VisionOCR.bounded([]) })
        XCTAssertEqual(empty.status, "OCR_EMPTY"); XCTAssertEqual(empty.conversationIdentity, "UNVERIFIED")
    }
}
