import CoreGraphics
import CryptoKit
import Foundation

public struct SendPoint: Codable, Equatable {
    public let x: Double
    public let y: Double
}

public struct SendMessageSignature: Codable, Equatable {
    public let digest: String
    public let direction: String
    public let complete: Bool
}

/// Private worker IPC. Only the draft text is plaintext; chat history is hashed.
public struct SendObservation: Codable, Equatable {
    public let applicationSession: String
    public let conversation: String
    public let windowRef: String
    public let layoutRef: String
    public let frontmost: Bool
    public let conversationResolved: Bool
    public let draftState: DraftReadState
    public let draftText: String?
    public let messages: [SendMessageSignature]
    public let sendButton: SendPoint?

    public func sameSurface(as other: SendObservation) -> Bool {
        applicationSession == other.applicationSession && conversation == other.conversation
            && windowRef == other.windowRef && layoutRef == other.layoutRef
            && conversationResolved && other.conversationResolved
    }
}

public enum WeChatSendPolicy {
    public static let controlRegion = CGRect(x: 0.86, y: 0.80, width: 0.14, height: 0.20)

    public static func digest(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8))
            .map { String(format: "%02x", $0) }.joined()
    }

    /// G3c-1 deliberately supports short, single-line plain text only. Reject before input.
    public static func supportedText(_ text: String) -> Bool {
        !text.isEmpty && text.utf16.count <= 80
            && text == text.trimmingCharacters(in: .whitespacesAndNewlines)
            && !text.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }
            && !text.hasSuffix("|")
    }

    public static func sendButton(_ snapshot: OCRSnapshot?) -> SendPoint? {
        guard let snapshot, snapshot.statistics.completeRecognition else { return nil }
        let candidates = snapshot.lines.filter {
            $0.confidence >= 0.8
                && ["发送", "Send"].contains($0.text.trimmingCharacters(in: .whitespacesAndNewlines))
                && controlRegion.contains(CGPoint(x: $0.bounds.midX, y: $0.bounds.midY))
        }
        guard candidates.count == 1 else { return nil }
        return SendPoint(x: Double(candidates[0].bounds.midX), y: Double(candidates[0].bounds.midY))
    }

    public static func observation(_ raw: WeChatComposerObservation, frontmost: Bool) -> SendObservation {
        // Exclude the composer/tool strip from receipt evidence. The legacy G2 ROI overlaps it.
        let chatOnly = OCRSnapshot(
            lines: raw.chatSnapshot.lines.filter { $0.bounds.maxY < 0.765 },
            statistics: raw.chatSnapshot.statistics
        )
        let parsed = WeChatMessageParser.parse(chatOnly, maxMessages: 64)
        let frame = raw.window.frame
        let layout = [frame.minX, frame.minY, frame.width, frame.height, raw.window.scale]
            .map { String(format: "%.4f", Double($0)) }.joined(separator: ":")
        return SendObservation(
            applicationSession: raw.privateSnapshot.applicationSessionFingerprint,
            conversation: raw.privateSnapshot.conversationFingerprint,
            windowRef: String(raw.window.id),
            layoutRef: digest(layout),
            frontmost: frontmost,
            conversationResolved: !raw.privateSnapshot.partialReasons.contains("CONVERSATION_IDENTITY_UNRESOLVED"),
            draftState: WeChatDraftPolicy.readState(raw.draftSnapshot),
            draftText: WeChatDraftPolicy.observedText(raw.draftSnapshot),
            messages: parsed.messages.map {
                SendMessageSignature(digest: digest($0.text), direction: $0.direction.rawValue,
                                     complete: chatOnly.statistics.completeRecognition && $0.direction != .unknown)
            },
            sendButton: sendButton(raw.controlSnapshot)
        )
    }

    /// A new matching ME message after an unambiguous suffix/prefix overlap, not a delivery ACK.
    public static func verifiedOutgoing(before: SendObservation, after: SendObservation, text: String) -> Bool {
        guard before.sameSurface(as: after), after.frontmost,
              after.draftState == .emptyHeuristic,
              before.messages.allSatisfy(\.complete), after.messages.allSatisfy(\.complete)
        else { return false }
        let expected = digest(text)
        if before.messages.isEmpty {
            return after.messages.count == 1 && after.messages[0].direction == "ME"
                && after.messages[0].digest == expected
        }
        let upper = min(before.messages.count, after.messages.count)
        let lower = min(2, before.messages.count)
        guard upper >= lower else { return false }
        let overlaps = (lower...upper).filter { count in
            Array(before.messages.suffix(count)) == Array(after.messages.prefix(count))
        }
        guard overlaps.count == 1, let overlap = overlaps.first else { return false }
        let added = after.messages.dropFirst(overlap)
        return added.filter { $0.direction == "ME" && $0.digest == expected }.count == 1
    }
}
