import AppKit
import Foundation
import OCRKit

enum WorkerMode {
    private static let maxInputBytes = 4096
    private static let maxCommands = 4096

    @MainActor
    static func run() async {
        let application = NSApplication.shared
        application.setActivationPolicy(.prohibited)
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        var warmed = false
        var commands = 0

        while commands < maxCommands, let line = readLine(strippingNewline: true) {
            commands += 1
            let reply: OCRWorkerReply
            if line.utf8.count > maxInputBytes {
                reply = OCRWorkerReply(id: "invalid", status: "INVALID_REQUEST")
            } else if let data = line.data(using: .utf8),
                      let command = try? decoder.decode(OCRWorkerCommand.self, from: data),
                      command.validID {
                switch command.command {
                case .warmup:
                    let result = VisionOCR.warmup()
                    warmed = result.succeeded
                    reply = OCRWorkerReply(id: command.id,
                                           status: result.succeeded ? "WARMED" : "WARMUP_FAILED",
                                           warmup: result)
                case .captureOCR:
                    guard warmed else {
                        reply = OCRWorkerReply(id: command.id, status: "NOT_WARMED")
                        break
                    }
                    guard let app = command.app else {
                        reply = OCRWorkerReply(id: command.id, status: "INVALID_REQUEST")
                        break
                    }
                    let report = await WindowOCRProbe.run(app: app, requested: true, ocrRequested: true,
                        source: NativeWindowSource(),
                        selection: command.focusedWindow == true ? .focused : .unique)
                    reply = OCRWorkerReply(id: command.id, status: "REPORT", report: report)
                case .captureOnly:
                    guard let app = command.app else {
                        reply = OCRWorkerReply(id: command.id, status: "INVALID_REQUEST")
                        break
                    }
                    let report = await WindowOCRProbe.run(app: app, requested: true, ocrRequested: false,
                        source: NativeWindowSource(),
                        selection: command.focusedWindow == true ? .focused : .unique)
                    reply = OCRWorkerReply(id: command.id, status: "REPORT", report: report)
                case .shutdown:
                    reply = OCRWorkerReply(id: command.id, status: "SHUTDOWN")
                    write(reply, encoder)
                    return
                }
            } else {
                reply = OCRWorkerReply(id: "invalid", status: "INVALID_REQUEST")
            }
            write(reply, encoder)
        }
    }

    private static func write(_ reply: OCRWorkerReply, _ encoder: JSONEncoder) {
        guard let data = try? encoder.encode(reply), data.count <= 16_384 else { return }
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    }
}
