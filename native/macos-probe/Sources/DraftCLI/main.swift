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
        let application = NSApplication.shared
        application.setActivationPolicy(.prohibited)
        try? await Task.sleep(nanoseconds: 250_000_000)
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
            guard wechatIsFrontmost() else { throw DraftError.prerequisites }
            let before = try await WeChatComposerProbe.capture(source: source)
            let beforeState = WeChatDraftPolicy.readState(before.draftSnapshot)
            guard before.privateSnapshot.conversationFingerprint == expectedConversation,
                  !before.privateSnapshot.partialReasons.contains("CONVERSATION_IDENTITY_UNRESOLVED")
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

            clickComposer(window: before.window)
            try await Task.sleep(nanoseconds: 200_000_000)
            guard wechatIsFrontmost() else {
                write(DraftReport(status: "TARGET_CHANGED_BEFORE_WRITE", writeAttempted: false,
                                  writeVerified: false, draftStateBefore: beforeState))
                return
            }
            injectUnicode(text)
            try await Task.sleep(nanoseconds: 700_000_000)

            let after = try await WeChatComposerProbe.capture(source: source)
            guard after.privateSnapshot.applicationSessionFingerprint
                    == before.privateSnapshot.applicationSessionFingerprint,
                  after.privateSnapshot.conversationFingerprint == expectedConversation
            else {
                write(DraftReport(status: "TARGET_CHANGED_AFTER_WRITE", writeAttempted: true,
                                  writeVerified: false, draftStateBefore: beforeState))
                return
            }
            let verified = WeChatDraftPolicy.verified(text, snapshot: after.draftSnapshot)
            write(DraftReport(status: verified ? "DRAFT_WRITE_VERIFIED" : "DRAFT_WRITE_UNVERIFIED",
                              writeAttempted: true, writeVerified: verified,
                              draftStateBefore: beforeState))
        } catch {
            write(DraftReport(status: "DRAFT_OPERATION_FAILED", writeAttempted: false,
                              writeVerified: false, draftStateBefore: .unreadable))
        }
    }
}
