import AppKit
import Foundation
import OCRKit
import ProbeKit

@main
struct OCRMain {
    @MainActor
    static func main() async {
        let arguments = Array(CommandLine.arguments.dropFirst())
        if arguments == ["--worker"] {
            await WorkerMode.run()
            return
        }
        if arguments.isEmpty || arguments == ["--help"] {
            print("foxbot-macos-ocr --worker | --app <qq|wechat> [--capture-only|--capture-and-ocr] [--focused-window]\nNo screenshot by default. Explicit capture uses one unambiguous target window; only redacted statistics are printed.")
            return
        }
        guard (2...4).contains(arguments.count), arguments[0] == "--app",
              let app = TargetApp(rawValue: arguments[1]),
              (arguments.count == 2 || arguments[2] == "--capture-and-ocr" || arguments[2] == "--capture-only"),
              (arguments.count < 4 || arguments[3] == "--focused-window") else {
            FileHandle.standardError.write(Data("invalid OCR probe arguments\n".utf8)); exit(2)
        }
        if arguments.count >= 3 {
            // ScreenCaptureKit needs a WindowServer/AppKit connection in a CLI process.
            // Initialize only our process, without creating a window or activating any app.
            let application = NSApplication.shared
            application.setActivationPolicy(.prohibited)
        }
        let captureRequested = arguments.count >= 3
        let ocrRequested = captureRequested && arguments[2] == "--capture-and-ocr"
        let report = await WindowOCRProbe.run(app: app, requested: captureRequested,
            ocrRequested: ocrRequested, source: NativeWindowSource(),
            selection: arguments.count == 4 ? .focused : .unique)
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        do {
            let data = try encoder.encode(report)
            FileHandle.standardOutput.write(data)
            FileHandle.standardOutput.write(Data([10]))
        } catch {
            FileHandle.standardError.write(Data("OCR report serialization failed\n".utf8)); exit(2)
        }
    }
}
