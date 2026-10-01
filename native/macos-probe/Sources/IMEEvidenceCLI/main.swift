import AppKit
import Foundation
import OCRKit
import ProbeKit

private struct EvidenceReport: Encodable {
    let schemaVersion = "foxbot.macos-ime-evidence.v1"
    let status: String
    let authoritativeState: AuthoritativeCompositionState
    let compositionVerified: Bool
    let composing: Bool
    let positiveBlockers: [String]
    let appFocusedUIAvailable: Bool
    let systemFocusedUIAvailable: Bool
    let selectedTextRangeAvailable: Bool
    let inputSourceAvailable: Bool
    let inputMethodProcessCount: Int
    let inputMethodWindowCount: Int
    let inputMethodOnScreenWindowCount: Int
    let recentUserInput: Bool
    let targetFrontmost: Bool
    let readOnly = true
    let rawTextIncluded = false
    let imageSaved = false
    let writeOperations = 0
    let sendOperations = 0
    let networkRequests = 0
    let signalClasses: [String: IMEEvidenceClass] = [
        "composition_state": .authoritative,
        "ax_focus": .contextOnly,
        "selected_text_range": .contextOnly,
        "input_source": .contextOnly,
        "input_method_window_visibility": .positiveBlocker,
        "recent_user_input": .positiveBlocker,
    ]
}

private func emit(_ report: EvidenceReport) {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.outputFormatting = [.sortedKeys]
    if let data = try? encoder.encode(report) {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    }
}

@main
struct IMEEvidenceMain {
    static func main() {
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        let facts = NativeIMEEvidenceProbe.collect(target: .wechat)
        let decision = IMEEvidencePolicy.evaluate(facts)
        emit(EvidenceReport(
            status: decision.compositionVerified ? "AUTHORITATIVE_STATE" : "NO_AUTHORITATIVE_STATE",
            authoritativeState: facts.authoritativeState,
            compositionVerified: decision.compositionVerified,
            composing: decision.composing,
            positiveBlockers: decision.positiveBlockers,
            appFocusedUIAvailable: facts.appFocusedUIAvailable,
            systemFocusedUIAvailable: facts.systemFocusedUIAvailable,
            selectedTextRangeAvailable: facts.selectedTextRangeAvailable,
            inputSourceAvailable: facts.inputSourceAvailable,
            inputMethodProcessCount: facts.inputMethodProcessCount,
            inputMethodWindowCount: facts.inputMethodWindowCount,
            inputMethodOnScreenWindowCount: facts.inputMethodOnScreenWindowCount,
            recentUserInput: facts.recentUserInput,
            targetFrontmost: facts.targetFrontmost
        ))
    }
}
