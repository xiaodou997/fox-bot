import CoreGraphics
import CryptoKit
import Foundation

public enum MessageDirection: String, Codable {
    case me = "ME"
    case them = "THEM"
    case unknown = "UNKNOWN"
}

/// Raw text stays process-local and this type is deliberately not Codable.
public struct ParsedChatMessage {
    public var text: String
    public var direction: MessageDirection
    public var sender: String?
    public var confidence: Float
    public var bounds: CGRect
    public var lines: [String]
}

public struct MessageSnapshot {
    public var messages: [ParsedChatMessage]
    public var region: CGRect
    public var partialReasons: [String]
    public var strategy: String

    public var summary: MessageSnapshotSummary {
        MessageSnapshotSummary(
            strategy: strategy,
            messageCount: messages.count,
            meCount: messages.filter { $0.direction == .me }.count,
            themCount: messages.filter { $0.direction == .them }.count,
            unknownCount: messages.filter { $0.direction == .unknown }.count,
            senderLabeledCount: messages.filter { $0.sender != nil }.count,
            usedLineCount: messages.reduce(0) { $0 + $1.lines.count },
            complete: partialReasons.isEmpty,
            partialReasons: partialReasons
        )
    }
}

/// Safe diagnostic projection: no recognized text, sender names, boxes or desktop coordinates.
public struct MessageSnapshotSummary: Encodable {
    public let strategy: String
    public let messageCount: Int
    public let meCount: Int
    public let themCount: Int
    public let unknownCount: Int
    public let senderLabeledCount: Int
    public let usedLineCount: Int
    public let complete: Bool
    public let partialReasons: [String]
}

public enum WeChatMessageParser {
    // Reference project used x>=0.32 and bottom-origin body 0.16...0.90 on WeChat 4.x.
    // FoxBot stores Vision boxes in top-origin coordinates, yielding y 0.10...0.84.
    public static let strategy = "WECHAT_HEURISTIC_V0"
    // Read slightly left of the body region so wide/group layouts whose title begins around
    // x≈0.28 are still available for conversation identity. Message parsing remains gated by
    // chatRegion x>=0.32, so sidebar text cannot become chat messages.
    public static let readRegion = CGRect(x: 0.27, y: 0.00, width: 0.73, height: 0.84)
    public static let chatRegion = CGRect(x: 0.32, y: 0.10, width: 0.68, height: 0.74)
    private static let minimumConfidence: Float = 0.30

    public static func direction(x: CGFloat, width: CGFloat) -> MessageDirection {
        let right = x + width
        if x >= 0.66 || (x >= 0.50 && right >= 0.80) { return .me }
        if x <= 0.50 && right < 0.80 { return .them }
        return .unknown
    }

    private static func isNoise(_ text: String) -> Bool {
        let value = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.isEmpty { return true }
        if value.range(of: #"^\d{1,2}:\d{2}(:\d{2})?$"#, options: .regularExpression) != nil {
            return true
        }
        let separators = [
            #"^\d{4}年\d{1,2}月\d{1,2}日\s*\d{1,2}:\d{2}$"#,
            #"^星期[一二三四五六日天]\s*\d{1,2}:\d{2}$"#,
        ]
        if separators.contains(where: {
            value.range(of: $0, options: .regularExpression) != nil
        }) {
            return true
        }
        let callEvents = [
            #"^通话时长\s*\d{1,2}:\d{2}\s*[0-9～~]?$"#,
            #"^对方已拒绝[9～~风一]?$"#,
            #"^已取消[9の～~]?$"#,
            #"^[•へ]?\s*已在其它设备拒绝$"#,
        ]
        if callEvents.contains(where: {
            value.range(of: $0, options: .regularExpression) != nil
        }) {
            return true
        }
        let exact: Set<String> = ["搜索", "发送", "拖入文件", "按住说话", "输入文字", "语音输入文字"]
        return exact.contains(value)
    }

    private static func union(_ first: CGRect, _ second: CGRect) -> CGRect {
        first.union(second)
    }

    private static func multilineGapLimit(for line: OCRLine) -> CGFloat {
        min(0.04, max(0.012, line.bounds.height * 1.35))
    }

    private static func digest(_ value: String, domain: String) -> String {
        let data = Data((domain + "\0" + value).utf8)
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    private static func normalizedTitle(_ snapshot: OCRSnapshot) -> String? {
        let candidates = snapshot.lines.filter {
            $0.confidence >= minimumConfidence
                && $0.bounds.midY >= 0.015 && $0.bounds.midY < 0.10
                && $0.bounds.minX >= 0.27 && $0.bounds.minX < 0.94
                && !isNoise($0.text)
        }.sorted {
            if abs($0.bounds.minY - $1.bounds.minY) < 0.005 { return $0.bounds.minX < $1.bounds.minX }
            return $0.bounds.minY < $1.bounds.minY
        }
        guard let first = candidates.first else { return nil }
        let row = candidates.filter { abs($0.bounds.minY - first.bounds.minY) < 0.03 }
            .sorted { $0.bounds.minX < $1.bounds.minX }
        var kept: [OCRLine] = []
        for line in row {
            if let last = kept.last,
               line.bounds.minX - last.bounds.maxX > 1.5 * max(line.bounds.height, last.bounds.height) {
                break
            }
            kept.append(line)
        }
        var title = kept.map(\.text).joined(separator: " ")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        title = title.replacingOccurrences(of: #"\s*[（(]\s*\d+\s*[）)]\s*$"#,
                                           with: "", options: .regularExpression)
        title = title.replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, title.utf8.count <= 512 else { return nil }
        return title
    }

    public static func conversationFingerprint(_ snapshot: OCRSnapshot) -> String? {
        normalizedTitle(snapshot).map { digest($0, domain: "foxbot.wechat.conversation.v1") }
    }

    public static func applicationSessionFingerprint(
        bundleID: String,
        launchTime: TimeInterval
    ) -> String? {
        guard !bundleID.isEmpty, bundleID.utf8.count <= 256,
              launchTime.isFinite, launchTime > 0 else { return nil }
        return digest(
            bundleID + "\0" + String(format: "%.6f", launchTime),
            domain: "foxbot.application-session.v1"
        )
    }

    private static func senderFingerprint(_ sender: String?) -> String? {
        guard let sender else { return nil }
        let normalized = sender.replacingOccurrences(of: #"\s+"#, with: " ",
            options: .regularExpression).trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.isEmpty ? nil : digest(normalized, domain: "foxbot.wechat.sender.v1")
    }

    private static func mergeFragments(_ lines: [OCRLine]) -> [OCRLine] {
        let sorted = lines.sorted {
            if abs($0.bounds.minY - $1.bounds.minY) < 0.000_001 { return $0.bounds.minX < $1.bounds.minX }
            return $0.bounds.minY < $1.bounds.minY
        }
        var merged: [OCRLine] = []
        for line in sorted {
            guard let last = merged.last else { merged.append(line); continue }
            let sameRow = abs(last.bounds.midY - line.bounds.midY) <= 0.012
            let gap = line.bounds.minX - last.bounds.maxX
            let compatibleSide = direction(x: last.bounds.minX, width: last.bounds.width)
                == direction(x: line.bounds.minX, width: line.bounds.width)
            if sameRow && compatibleSide && gap >= -0.005 && gap <= 0.04 {
                merged.removeLast()
                merged.append(OCRLine(text: last.text + " " + line.text,
                                      confidence: min(last.confidence, line.confidence),
                                      bounds: union(last.bounds, line.bounds)))
            } else {
                merged.append(line)
            }
        }
        return merged
    }

    public static func parse(_ snapshot: OCRSnapshot, maxMessages: Int = 12) -> MessageSnapshot {
        let boundedMax = min(max(maxMessages, 1), 64)
        let filtered = snapshot.lines.filter { line in
            line.confidence >= minimumConfidence
                && chatRegion.contains(CGPoint(x: line.bounds.midX, y: line.bounds.midY))
                && !isNoise(line.text)
        }
        let lines = mergeFragments(filtered)
        var headerIndexes = Set<Int>()
        if lines.count > 1 {
            for index in 0..<(lines.count - 1) {
                let line = lines[index], next = lines[index + 1]
                let firstSide = direction(x: line.bounds.minX, width: line.bounds.width)
                let nextSide = direction(x: next.bounds.minX, width: next.bounds.width)
                let gap = next.bounds.minY - line.bounds.maxY
                let senderGapLimit = min(0.03, max(0.012, next.bounds.height * 1.5))
                if firstSide == .them && nextSide == .them
                    && line.text.count <= 32
                    && line.bounds.height <= next.bounds.height * 0.88
                    && abs(line.bounds.minX - next.bounds.minX) < 0.03
                    && gap >= -0.003 && gap <= senderGapLimit {
                    headerIndexes.insert(index)
                }
            }
        }

        var messages: [ParsedChatMessage] = []
        var pendingSender: String?
        for (index, line) in lines.enumerated() {
            if headerIndexes.contains(index) {
                pendingSender = line.text.trimmingCharacters(in: .whitespacesAndNewlines)
                    .trimmingCharacters(in: CharacterSet(charactersIn: ":："))
                continue
            }
            let side = direction(x: line.bounds.minX, width: line.bounds.width)
            if pendingSender == nil, let last = messages.last {
                let gap = line.bounds.minY - last.bounds.maxY
                let aligned = abs(line.bounds.minX - last.bounds.minX) < 0.025
                let compatible = side == last.direction || side == .unknown || last.direction == .unknown
                if aligned && compatible && gap >= -0.005 && gap < multilineGapLimit(for: line) {
                    var updated = last
                    messages.removeLast()
                    updated.lines.append(line.text)
                    updated.text = updated.lines.joined(separator: "\n")
                    updated.confidence = min(updated.confidence, line.confidence)
                    updated.bounds = union(updated.bounds, line.bounds)
                    updated.direction = direction(x: updated.bounds.minX, width: updated.bounds.width)
                    messages.append(updated)
                    continue
                }
            }
            messages.append(ParsedChatMessage(
                text: line.text,
                direction: side,
                sender: pendingSender,
                confidence: line.confidence,
                bounds: line.bounds,
                lines: [line.text]
            ))
            pendingSender = nil
        }

        if messages.count > boundedMax { messages = Array(messages.suffix(boundedMax)) }
        var reasons = ["HEURISTIC_REGION"]
        if !snapshot.statistics.completeRecognition { reasons.append("OCR_PARTIAL") }
        if messages.contains(where: { $0.direction == .unknown }) { reasons.append("UNKNOWN_DIRECTION") }
        return MessageSnapshot(messages: messages, region: chatRegion,
                               partialReasons: reasons, strategy: strategy)
    }

    public static func privateSnapshot(
        _ snapshot: OCRSnapshot,
        applicationSessionFingerprint: String
    ) -> PrivateMessageSnapshot? {
        guard let conversationFingerprint = conversationFingerprint(snapshot),
              applicationSessionFingerprint.count == 64,
              applicationSessionFingerprint.allSatisfy({ $0.isHexDigit && !$0.isUppercase })
        else { return nil }
        let parsed = parse(snapshot)
        return PrivateMessageSnapshot(
            schemaVersion: "foxbot.private-message-snapshot.v1",
            strategy: strategy,
            applicationSessionFingerprint: applicationSessionFingerprint,
            conversationFingerprint: conversationFingerprint,
            partialReasons: parsed.partialReasons,
            messages: parsed.messages.map {
                PrivateBridgeMessage(
                    text: $0.text,
                    direction: $0.direction,
                    senderFingerprint: senderFingerprint($0.sender),
                    complete: snapshot.statistics.completeRecognition && $0.direction != .unknown
                )
            }
        )
    }
}

/// Private local IPC only. Unlike diagnostic summaries this intentionally carries message text.
/// It must never be printed by public probe wrappers or persisted in receipts.
public struct PrivateBridgeMessage: Codable {
    public let text: String
    public let direction: MessageDirection
    public let senderFingerprint: String?
    public let complete: Bool
}

public struct PrivateMessageSnapshot: Codable {
    public let schemaVersion: String
    public let strategy: String
    public let applicationSessionFingerprint: String
    public let conversationFingerprint: String
    public let partialReasons: [String]
    public let messages: [PrivateBridgeMessage]
}
