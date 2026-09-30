import CoreGraphics
import Foundation

public enum DraftReadState: String, Codable, Equatable {
    case emptyHeuristic = "EMPTY_HEURISTIC"
    case nonempty = "NONEMPTY"
    case unreadable = "UNREADABLE"
}

public enum WeChatDraftPolicy {
    /// Current WeChat 4.1.x composer body. The right-side send/control cluster is excluded.
    public static let draftRegion = CGRect(x: 0.26, y: 0.79, width: 0.62, height: 0.17)
    public static let focusPoint = CGPoint(x: 0.46, y: 0.82)
    private static let emptyPlaceholders: Set<String> = [
        "按住鼠标 语音输入文字",
        "按住说话",
        "输入文字",
        "语音输入文字",
    ]

    private static func isPlaceholder(_ text: String) -> Bool {
        emptyPlaceholders.contains(
            text.trimmingCharacters(in: .whitespacesAndNewlines)
        )
    }

    public static func readState(_ snapshot: OCRSnapshot) -> DraftReadState {
        guard snapshot.statistics.completeRecognition else { return .unreadable }
        let visible = snapshot.lines.filter {
            $0.confidence >= 0.30
                && draftRegion.contains(CGPoint(x: $0.bounds.midX, y: $0.bounds.midY))
                && !isPlaceholder($0.text)
                && !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
        return visible.isEmpty ? .emptyHeuristic : .nonempty
    }

    public static func normalizeVerificationText(_ text: String) -> String {
        var value = text
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        // Vision can recognize the insertion caret as a trailing pipe.
        if value.hasSuffix("|") {
            value.removeLast()
            value = value.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return value.replacingOccurrences(of: #"[ \t]+"#, with: " ", options: .regularExpression)
    }

    public static func verified(_ expected: String, snapshot: OCRSnapshot) -> Bool {
        guard snapshot.statistics.completeRecognition else { return false }
        let joined = snapshot.lines
            .filter {
                $0.confidence >= 0.30
                    && draftRegion.contains(CGPoint(x: $0.bounds.midX, y: $0.bounds.midY))
                    && !isPlaceholder($0.text)
            }
            .sorted {
                if abs($0.bounds.minY - $1.bounds.minY) < 0.006 {
                    return $0.bounds.minX < $1.bounds.minX
                }
                return $0.bounds.minY < $1.bounds.minY
            }
            .map(\.text)
            .joined(separator: "\n")
        return normalizeVerificationText(joined) == normalizeVerificationText(expected)
    }
}
