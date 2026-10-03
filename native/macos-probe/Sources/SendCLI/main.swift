import AppKit
import CoreGraphics
import Foundation
import OCRKit
import ProbeKit

private struct Request: Decodable {
    let id: UInt64
    let command: String
    let expected: SendObservation?
    let text: String?
    let actionId: String?
}

private struct Reply: Encodable {
    let schemaVersion = "foxbot.native-send-worker.v5"
    let id: UInt64
    var status: String
    var observation: SendObservation?
    var messages: [NativeReadMessage]?
    var writeAttempted = false
    var sendAttempted = false
    var verifiedOutgoing = false
}

private enum Failure: String, Error {
    case invalidRequest = "INVALID_REQUEST"
    case appNotFrontmost = "APP_NOT_FRONTMOST"
    case captureFailed = "CAPTURE_FAILED"
    case targetChanged = "TARGET_CHANGED"
    case draftNotEmpty = "DRAFT_NOT_EMPTY"
    case draftMismatch = "DRAFT_MISMATCH"
    case receiptRevisionMismatch = "RECEIPT_REVISION_MISMATCH"
    case sendButtonUnavailable = "SEND_BUTTON_UNAVAILABLE"
    case actionAlreadyAttempted = "ACTION_ALREADY_ATTEMPTED"
    case ownerLost = "OWNER_LOST"
    case inputFailed = "INPUT_FAILED"
}

private func frontmost() -> Bool {
    NSWorkspace.shared.frontmostApplication?.bundleIdentifier == TargetApp.wechat.bundleID
}

private func emit(_ reply: Reply) {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.outputFormatting = [.sortedKeys]
    guard var data = try? encoder.encode(reply) else { return }
    if data.count > 65_536 {
        var bounded = Reply(id: reply.id, status: "PAYLOAD_TOO_LARGE")
        bounded.writeAttempted = reply.writeAttempted
        bounded.sendAttempted = reply.sendAttempted
        guard let fallback = try? encoder.encode(bounded) else { return }
        data = fallback
    }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([10]))
}

private func click(_ point: CGPoint) throws {
    guard let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown,
                             mouseCursorPosition: point, mouseButton: .left),
          let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp,
                           mouseCursorPosition: point, mouseButton: .left) else { throw Failure.inputFailed }
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
}

private func input(_ text: String) throws {
    // Bounded Unicode packets; never synthesize Return, tabs, or IME composition.
    var packets: [[UniChar]] = [[]]
    for character in text {
        let units = Array(String(character).utf16)
        guard units.count <= 20 else { throw Failure.invalidRequest }
        if packets[packets.count - 1].count + units.count > 20 { packets.append([]) }
        packets[packets.count - 1].append(contentsOf: units)
    }
    for units in packets where !units.isEmpty {
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true),
              let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false)
        else { throw Failure.inputFailed }
        units.withUnsafeBufferPointer { buffer in
            if let base = buffer.baseAddress {
                down.keyboardSetUnicodeString(stringLength: units.count, unicodeString: base)
                up.keyboardSetUnicodeString(stringLength: units.count, unicodeString: base)
            }
        }
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }
}

private func commandKey(_ key: CGKeyCode) throws {
    guard let down = CGEvent(keyboardEventSource: nil, virtualKey: key, keyDown: true),
          let up = CGEvent(keyboardEventSource: nil, virtualKey: key, keyDown: false)
    else { throw Failure.inputFailed }
    down.flags = .maskCommand
    up.flags = .maskCommand
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
}

private func canonicalLineEndings(_ text: String) -> String {
    text.replacingOccurrences(of: "\r\n", with: "\n")
        .replacingOccurrences(of: "\r", with: "\n")
}

private func composerPoint(_ raw: WeChatComposerObservation) -> CGPoint {
    CGPoint(
        x: raw.window.frame.minX + raw.window.frame.width * WeChatDraftPolicy.focusPoint.x,
        y: raw.window.frame.minY + raw.window.frame.height * WeChatDraftPolicy.focusPoint.y
    )
}

private func copyFocusedDraft(_ raw: WeChatComposerObservation,
                              pasteboard: NSPasteboard) async throws -> String {
    guard frontmost() else { throw Failure.appNotFrontmost }
    try click(composerPoint(raw))
    try await Task.sleep(nanoseconds: 100_000_000)
    pasteboard.clearContents()
    let cleared = pasteboard.changeCount
    try commandKey(0) // A
    try await Task.sleep(nanoseconds: 50_000_000)
    try commandKey(8) // C
    try await Task.sleep(nanoseconds: 200_000_000)
    guard frontmost(), pasteboard.changeCount != cleared,
          let value = pasteboard.string(forType: .string)
    else { throw Failure.inputFailed }
    return canonicalLineEndings(value)
}

private func readBackExtendedDraft(_ raw: WeChatComposerObservation) async throws -> String {
    let pasteboard = NSPasteboard.general
    guard let snapshot = PasteboardSnapshot(pasteboard: pasteboard) else {
        throw Failure.inputFailed
    }
    var restored = false
    defer {
        if !restored { _ = snapshot.restore(to: pasteboard) }
    }
    let value = try await copyFocusedDraft(raw, pasteboard: pasteboard)
    guard snapshot.restore(to: pasteboard) else { throw Failure.inputFailed }
    restored = true
    return value
}

private func pasteExtendedText(_ text: String,
                               raw: WeChatComposerObservation) async throws {
    let pasteboard = NSPasteboard.general
    guard let snapshot = PasteboardSnapshot(pasteboard: pasteboard) else {
        throw Failure.inputFailed
    }
    var restored = false
    defer {
        if !restored { _ = snapshot.restore(to: pasteboard) }
    }
    pasteboard.clearContents()
    guard pasteboard.setString(text, forType: .string) else { throw Failure.inputFailed }
    guard frontmost() else { throw Failure.appNotFrontmost }
    try click(composerPoint(raw))
    try await Task.sleep(nanoseconds: 100_000_000)
    try commandKey(9) // V
    try await Task.sleep(nanoseconds: 500_000_000)
    guard snapshot.restore(to: pasteboard) else { throw Failure.inputFailed }
    restored = true
}

private func verifiedDraft(_ expected: String,
                           raw: WeChatComposerObservation) async throws -> String? {
    if WeChatSendPolicy.usesPasteboardInput(expected) {
        let copied = try await readBackExtendedDraft(raw)
        return copied == canonicalLineEndings(expected) ? expected : nil
    }
    return WeChatDraftPolicy.verifiedText(expected, snapshot: raw.draftSnapshot)
}

@main
struct SendMain {
    static func main() async {
        let args = Array(CommandLine.arguments.dropFirst())
        guard args == ["--worker"] || args == ["--worker", "--allow-single-send"] else { return }
        let allowWrite = args.count == 2
        let parent = getppid()
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let source = NativeWindowSource()
        var previousId: UInt64 = 0
        var filledAction: String?
        var filledText: String?
        var fillUsed = false
        var sendUsed = false

        func checkOwner() throws {
            guard parent > 1, getppid() == parent else { throw Failure.ownerLost }
            guard frontmost() else { throw Failure.appNotFrontmost }
        }
        func capture() async throws -> (WeChatComposerObservation, SendObservation) {
            try checkOwner()
            for attempt in 0..<3 {
                do {
                    let raw = try await WeChatComposerProbe.capture(source: source, includeSendControl: true)
                    let snapshot = WeChatSendPolicy.observation(raw, frontmost: frontmost())
                    guard snapshot.frontmost, snapshot.conversationResolved else { throw Failure.targetChanged }
                    return (raw, snapshot)
                } catch {
                    if attempt == 2 { throw Failure.captureFailed }
                    try await Task.sleep(nanoseconds: 200_000_000)
                }
            }
            throw Failure.captureFailed
        }

        while let line = readLine() {
            guard line.utf8.count <= 65_536,
                  let request = try? decoder.decode(Request.self, from: Data(line.utf8)),
                  request.id > previousId else { break }
            previousId = request.id
            var reply = Reply(id: request.id, status: "INVALID_REQUEST")
            do {
                if request.command == "warmup" {
                    reply.status = VisionOCR.warmup().succeeded ? "WARMED" : "WARMUP_FAILED"
                } else if ["inspect", "read"].contains(request.command) {
                    let (raw, snapshot) = try await capture()
                    if let filledText,
                       let canonical = try await verifiedDraft(filledText, raw: raw) {
                        reply.observation = snapshot.withDraftText(canonical)
                    } else {
                        reply.observation = snapshot
                    }
                    if request.command == "read" {
                        reply.messages = WeChatSendPolicy.readMessages(raw.chatSnapshot)
                    }
                    reply.status = "OBSERVED"
                } else if request.command == "recover_read" {
                    guard allowWrite, let text = request.text,
                          let action = request.actionId, !action.isEmpty, action.utf8.count <= 128,
                          WeChatSendPolicy.supportedText(text) else { throw Failure.invalidRequest }
                    let (raw, snapshot) = try await capture()
                    guard let canonical = try await verifiedDraft(text, raw: raw)
                    else { throw Failure.draftMismatch }
                    reply.observation = snapshot.withDraftText(canonical)
                    reply.messages = WeChatSendPolicy.readMessages(raw.chatSnapshot)
                    reply.status = "OBSERVED"
                } else if ["fill", "send", "recover_send", "reconcile"].contains(request.command) {
                    guard let expected = request.expected, let text = request.text,
                          let action = request.actionId, !action.isEmpty, action.utf8.count <= 128,
                          WeChatSendPolicy.supportedText(text) else { throw Failure.invalidRequest }
                    let (raw, current) = try await capture()
                    reply.observation = current
                    guard expected.sameSurface(as: current) else { throw Failure.targetChanged }
                    let supportedRevisions = [WeChatSendPolicy.legacyEvidenceRevision,
                                              WeChatSendPolicy.evidenceRevision]
                    guard expected.evidenceRevision.map(supportedRevisions.contains) == true,
                          current.evidenceRevision.map(supportedRevisions.contains) == true
                    else { throw Failure.receiptRevisionMismatch }
                    if request.command == "reconcile" {
                        reply.verifiedOutgoing = WeChatSendPolicy.verifiedOutgoing(before: expected, after: current, text: text)
                        reply.status = reply.verifiedOutgoing ? "VERIFIED_OUTGOING" : "UNKNOWN"
                    } else if request.command == "fill" {
                        guard allowWrite else { throw Failure.invalidRequest }
                        guard !fillUsed else { throw Failure.actionAlreadyAttempted }
                        guard current.hasNoNewMessages(since: expected) else { throw Failure.targetChanged }
                        guard current.draftState == .emptyHeuristic else { throw Failure.draftNotEmpty }
                        fillUsed = true
                        try checkOwner()
                        reply.writeAttempted = true
                        try click(composerPoint(raw))
                        try await Task.sleep(nanoseconds: 150_000_000)
                        try checkOwner()
                        if WeChatSendPolicy.usesPasteboardInput(text) {
                            try await pasteExtendedText(text, raw: raw)
                        } else {
                            try input(text)
                            try await Task.sleep(nanoseconds: 600_000_000)
                        }
                        let (afterRaw, observedAfter) = try await capture()
                        guard expected.sameSurface(as: observedAfter),
                              observedAfter.hasNoNewMessages(since: expected)
                        else { throw Failure.targetChanged }
                        guard let verified = try await verifiedDraft(text, raw: afterRaw)
                        else { throw Failure.draftMismatch }
                        let after = observedAfter.withDraftText(verified)
                        reply.observation = after
                        filledAction = action
                        filledText = text
                        reply.status = "FILLED"
                    } else {
                        guard allowWrite else { throw Failure.invalidRequest }
                        if request.command == "send" {
                            guard filledAction == action, filledText == text else { throw Failure.invalidRequest }
                        } else {
                            guard request.command == "recover_send", filledAction == nil, filledText == nil
                            else { throw Failure.invalidRequest }
                        }
                        guard !sendUsed else { throw Failure.actionAlreadyAttempted }
                        guard current.hasNoNewMessages(since: expected) else { throw Failure.targetChanged }
                        guard let verified = try await verifiedDraft(text, raw: raw)
                        else { throw Failure.draftMismatch }
                        let before = current.withDraftText(verified)
                        reply.observation = before
                        guard let button = before.sendButton else { throw Failure.sendButtonUnavailable }
                        try checkOwner()
                        sendUsed = true
                        reply.sendAttempted = true
                        try click(CGPoint(x: raw.window.frame.minX + raw.window.frame.width * button.x,
                                          y: raw.window.frame.minY + raw.window.frame.height * button.y))
                        reply.status = "UNKNOWN"
                        for _ in 0..<6 {
                            try await Task.sleep(nanoseconds: 500_000_000)
                            let (_, after) = try await capture()
                            reply.observation = after
                            if WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: text) {
                                reply.verifiedOutgoing = true
                                reply.status = "VERIFIED_OUTGOING"
                                break
                            }
                        }
                    }
                }
            } catch let error as Failure {
                reply.status = error.rawValue
            } catch {
                reply.status = "NATIVE_OPERATION_FAILED"
            }
            emit(reply)
        }
    }
}
