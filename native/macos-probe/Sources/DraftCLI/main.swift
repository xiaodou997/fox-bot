import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import OCRKit
import ProbeKit

private struct DraftReport: Encodable {
    let schemaVersion = "foxbot.macos-draft.v1"
    let status: String
    let app = "wechat"
    let readOnly = false
    let sendAttempted = false
    let writeAttempted: Bool
    let writeVerified: Bool
    let draftStateBefore: DraftReadState
    let rawTextIncluded = false
    let imageSaved = false
    let networkRequests = 0
}

private enum DraftError: Error {
    case invalidArguments
    case prerequisites
    case target
    case capture
}

private func write(_ report: DraftReport) {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    encoder.keyEncodingStrategy = .convertToSnakeCase
    if let data = try? encoder.encode(report) {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    }
}

private func isHex64(_ value: String) -> Bool {
    value.count == 64
        && value.allSatisfy { $0.isHexDigit && !$0.isUppercase }
        && !value.allSatisfy { $0 == "0" }
}

private func wechatIsFrontmost() -> Bool {
    NSWorkspace.shared.frontmostApplication?.bundleIdentifier == TargetApp.wechat.bundleID
}

private func selectedWindow(_ source: NativeWindowSource) async throws -> (CaptureWindow, ImagePlan) {
    let metadata = source.metadata(.wechat)
    guard metadata.runningInstances == 1, let pid = metadata.pid,
          metadata.permission, AXIsProcessTrusted(), wechatIsFrontmost()
    else { throw DraftError.prerequisites }
    let frame = try source.focusedFrame(.wechat, pid: pid)
    let windows = try await source.windows(.wechat, pid: pid)
        .filter { $0.eligible(for: .wechat, pid: pid, allowOffscreen: true) }
        .filter {
            abs($0.frame.minX - frame.minX) <= 0.5
                && abs($0.frame.minY - frame.minY) <= 0.5
                && abs($0.frame.width - frame.width) <= 0.5
                && abs($0.frame.height - frame.height) <= 0.5
        }
    let active = windows.filter { $0.onScreen && $0.active }
    let visible = windows.filter(\.onScreen)
    let window: CaptureWindow?
    if active.count == 1 { window = active[0] }
    else if active.isEmpty && visible.count == 1 { window = visible[0] }
    else { window = nil }
    guard let window, window.hasValidGeometry else { throw DraftError.target }
    return (window, try ImagePlan.make(size: window.contentSize, scale: window.scale))
}

private func capture(_ source: NativeWindowSource) async throws
    -> (CaptureWindow, CGImage, PrivateMessageSnapshot, OCRSnapshot)
{
    let metadata = source.metadata(.wechat)
    guard let launchTime = metadata.launchTime,
          let appSession = WeChatMessageParser.applicationSessionFingerprint(
            bundleID: TargetApp.wechat.bundleID, launchTime: launchTime)
    else { throw DraftError.target }
    let (window, plan) = try await selectedWindow(source)
    let image = try await source.capture(window, plan: plan)
    let chat = try VisionOCR.recognize(image, topLeftRegion: WeChatMessageParser.readRegion)
    guard let privateSnapshot = WeChatMessageParser.privateSnapshot(
        chat, applicationSessionFingerprint: appSession)
    else { throw DraftError.target }
    let draft = try VisionOCR.recognize(image, topLeftRegion: WeChatDraftPolicy.draftRegion)
    return (window, image, privateSnapshot, draft)
}

private func clickComposer(window: CaptureWindow) {
    let point = CGPoint(
        x: window.frame.minX + window.frame.width * WeChatDraftPolicy.focusPoint.x,
        y: window.frame.minY + window.frame.height * WeChatDraftPolicy.focusPoint.y
    )
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved,
            mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown,
            mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp,
            mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
}

private func injectUnicode(_ text: String) {
    let units = Array(text.utf16)
    units.withUnsafeBufferPointer { buffer in
        guard let base = buffer.baseAddress else { return }
        let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true)
        down?.keyboardSetUnicodeString(stringLength: units.count, unicodeString: base)
        down?.post(tap: .cghidEventTap)
        let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false)
        up?.keyboardSetUnicodeString(stringLength: units.count, unicodeString: base)
        up?.post(tap: .cghidEventTap)
    }
}

@main
struct DraftMain {
    static func main() async {
        _ = NSApplication.shared
        let args = Array(CommandLine.arguments.dropFirst())
        guard args.count == 3,
              args[0] == "--expected-conversation",
              isHex64(args[1]),
              args[2] == "--allow-heuristic-empty-test"
        else {
            write(DraftReport(status: "INVALID_ARGUMENTS", writeAttempted: false,
                              writeVerified: false, draftStateBefore: .unreadable))
            return
        }
        let expectedConversation = args[1]
        let input = FileHandle.standardInput.readDataToEndOfFile()
        guard input.count <= 4096, let text = String(data: input, encoding: .utf8),
              !text.isEmpty, !text.contains("\0"), text.utf8.count <= 4096
        else {
            write(DraftReport(status: "INVALID_DRAFT", writeAttempted: false,
                              writeVerified: false, draftStateBefore: .unreadable))
            return
        }

        do {
            let source = NativeWindowSource()
            let (beforeWindow, _, beforeIdentity, beforeDraft) = try await capture(source)
            let beforeState = WeChatDraftPolicy.readState(beforeDraft)
            guard beforeIdentity.conversationFingerprint == expectedConversation,
                  !beforeIdentity.partialReasons.contains("CONVERSATION_IDENTITY_UNRESOLVED")
            else {
                write(DraftReport(status: "IDENTITY_MISMATCH", writeAttempted: false,
                                  writeVerified: false, draftStateBefore: beforeState))
                return
            }
            guard beforeState == .emptyHeuristic else {
                write(DraftReport(status: "DRAFT_NOT_EMPTY_OR_UNREADABLE", writeAttempted: false,
                                  writeVerified: false, draftStateBefore: beforeState))
                return
            }

            clickComposer(window: beforeWindow)
            try await Task.sleep(nanoseconds: 200_000_000)
            guard wechatIsFrontmost() else {
                write(DraftReport(status: "TARGET_CHANGED_BEFORE_WRITE", writeAttempted: false,
                                  writeVerified: false, draftStateBefore: beforeState))
                return
            }
            injectUnicode(text)
            try await Task.sleep(nanoseconds: 700_000_000)

            let (_, _, afterIdentity, afterDraft) = try await capture(source)
            guard afterIdentity.applicationSessionFingerprint
                    == beforeIdentity.applicationSessionFingerprint,
                  afterIdentity.conversationFingerprint == expectedConversation
            else {
                write(DraftReport(status: "TARGET_CHANGED_AFTER_WRITE", writeAttempted: true,
                                  writeVerified: false, draftStateBefore: beforeState))
                return
            }
            let verified = WeChatDraftPolicy.verified(text, snapshot: afterDraft)
            write(DraftReport(status: verified ? "DRAFT_WRITE_VERIFIED" : "DRAFT_WRITE_UNVERIFIED",
                              writeAttempted: true, writeVerified: verified,
                              draftStateBefore: beforeState))
        } catch {
            write(DraftReport(status: "DRAFT_OPERATION_FAILED", writeAttempted: false,
                              writeVerified: false, draftStateBefore: .unreadable))
        }
    }
}
