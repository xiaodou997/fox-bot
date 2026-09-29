import Foundation
import ProbeKit

public enum OCRWorkerAction: String, Codable {
    case warmup
    case captureOCR = "capture_ocr"
    case captureOnly = "capture_only"
    case captureSnapshot = "capture_snapshot"
    case shutdown
}

public struct OCRWorkerCommand: Codable {
    public let id: String
    public let command: OCRWorkerAction
    public let app: TargetApp?
    public let focusedWindow: Bool?

    public init(id: String, command: OCRWorkerAction,
                app: TargetApp? = nil, focusedWindow: Bool? = nil) {
        self.id = id; self.command = command; self.app = app; self.focusedWindow = focusedWindow
    }

    public var validID: Bool {
        !id.isEmpty && id.utf8.count <= 64
            && id.unicodeScalars.allSatisfy {
                CharacterSet.alphanumerics.contains($0) || $0 == "_" || $0 == "-"
            }
    }
}

public struct OCRWorkerReply: Encodable {
    public let id: String
    public let status: String
    public let warmup: OCRWarmupSummary?
    public let report: WindowOCRReport?
    public let privateSnapshot: PrivateMessageSnapshot?

    public init(id: String, status: String,
                warmup: OCRWarmupSummary? = nil, report: WindowOCRReport? = nil,
                privateSnapshot: PrivateMessageSnapshot? = nil) {
        self.id = id; self.status = status; self.warmup = warmup; self.report = report
        self.privateSnapshot = privateSnapshot
    }
}
