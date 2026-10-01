import ApplicationServices
import CoreGraphics
import Foundation
import ProbeKit

public struct WeChatComposerObservation {
    public let window: CaptureWindow
    public let privateSnapshot: PrivateMessageSnapshot
    public let draftSnapshot: OCRSnapshot
}

public enum WeChatComposerProbe {
    public static func capture(
        source: NativeWindowSource = NativeWindowSource()
    ) async throws -> WeChatComposerObservation {
        let metadata = source.metadata(.wechat)
        guard metadata.runningInstances == 1,
              let pid = metadata.pid,
              let launchTime = metadata.launchTime,
              metadata.permission,
              AXIsProcessTrusted(),
              let appSession = WeChatMessageParser.applicationSessionFingerprint(
                bundleID: TargetApp.wechat.bundleID,
                launchTime: launchTime
              )
        else {
            throw OCRFailure.focusedWindowUnavailable
        }

        let focused = try source.focusedFrame(.wechat, pid: pid)
        let matches = try await source.windows(.wechat, pid: pid)
            .filter { $0.eligible(for: .wechat, pid: pid, allowOffscreen: true) }
            .filter { sameWindowFrame($0.frame, focused) }
        let active = matches.filter { $0.onScreen && $0.active }
        let visible = matches.filter(\.onScreen)
        let window: CaptureWindow?
        if active.count == 1 {
            window = active[0]
        } else if active.isEmpty && visible.count == 1 {
            window = visible[0]
        } else {
            window = nil
        }
        guard let window, window.hasValidGeometry else {
            throw OCRFailure.focusedWindowUnavailable
        }

        let plan = try ImagePlan.make(size: window.contentSize, scale: window.scale)
        let image = try await source.capture(window, plan: plan)
        let chat = try VisionOCR.recognize(
            image,
            topLeftRegion: WeChatMessageParser.readRegion
        )
        guard let privateSnapshot = WeChatMessageParser.privateSnapshot(
            chat,
            applicationSessionFingerprint: appSession
        ) else {
            throw OCRFailure.recognitionFailed
        }
        let draft = try VisionOCR.recognize(
            image,
            topLeftRegion: WeChatDraftPolicy.draftRegion
        )
        return WeChatComposerObservation(
            window: window,
            privateSnapshot: privateSnapshot,
            draftSnapshot: draft
        )
    }
}
