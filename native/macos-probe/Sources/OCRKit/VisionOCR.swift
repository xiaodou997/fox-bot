import Foundation
import CoreGraphics
import Vision

/// Text exists only in the ephemeral OCR result. It is deliberately not Codable.
public struct OCRLine {
    public let text: String
    public let confidence: Float
    public let bounds: CGRect
    public init(text: String, confidence: Float, bounds: CGRect) {
        self.text = text; self.confidence = confidence; self.bounds = bounds
    }
}
public struct OCRStatistics: Encodable {
    public var lineCount: Int
    public var characterCount: Int
    public var lowConfidenceLines: Int
    public var partialReasons: [String]
    public var completeRecognition: Bool
    public let engine = "APPLE_VISION"
    public let requestRevision = 3
    public let languageCorrection = false
}
public struct OCRSnapshot {
    public let lines: [OCRLine]
    public let statistics: OCRStatistics
}
public struct OCRWarmupSummary: Encodable {
    public let elapsedMilliseconds: Int
    public let lineCount: Int
    public let succeeded: Bool
}
public enum VisionOCR {
    public static let maxLines = 512
    public static let maxCharacters = 32768

    /// Only geometric ordering. This does not identify chat bubbles, authors, or conversations.
    public static func bounded(_ candidates: [OCRLine], overflow: Bool = false, missingCandidates: Bool = false) -> OCRSnapshot {
        var lines: [OCRLine] = [], characters = 0, reasons = Set<String>()
        if overflow || candidates.count > maxLines { reasons.insert("LINE_LIMIT") }
        if missingCandidates { reasons.insert("INVALID_OBSERVATION") }
        for line in candidates.prefix(maxLines) {
            guard line.confidence.isFinite, (0...1).contains(line.confidence),
                  let box = try? topLeftBounds(line.bounds) else {
                reasons.insert("INVALID_OBSERVATION"); continue
            }
            guard !line.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { continue }
            let count = line.text.count
            guard count <= 4096, count <= maxCharacters - characters else {
                reasons.insert("TEXT_LIMIT"); break
            }
            characters += count
            lines.append(OCRLine(text: line.text, confidence: line.confidence, bounds: box))
        }
        lines.sort { a, b in a.bounds.minY == b.bounds.minY ? a.bounds.minX < b.bounds.minX : a.bounds.minY < b.bounds.minY }
        let stats = OCRStatistics(lineCount: lines.count, characterCount: characters,
                                  lowConfidenceLines: lines.filter { $0.confidence < 0.7 }.count,
                                  partialReasons: reasons.sorted(), completeRecognition: reasons.isEmpty)
        return OCRSnapshot(lines: lines, statistics: stats)
    }

    /// One local request per image. No upload, image file, lexical repair, or visual language model.
    /// The supervising process supplies the hard deadline; Vision's synchronous perform is not assumed cancellable.
    public static func recognize(_ image: CGImage, topLeftRegion: CGRect? = nil) throws -> OCRSnapshot {
        guard ImagePlan.accepts(width: image.width, height: image.height) else { throw OCRFailure.resourceLimit }
        return try autoreleasepool {
            var visionRegion: CGRect?
            if let region = topLeftRegion {
                let values = [region.minX, region.minY, region.width, region.height, region.maxX, region.maxY]
                guard values.allSatisfy({ $0.isFinite }), region.width > 0, region.height > 0,
                      region.minX >= 0, region.minY >= 0, region.maxX <= 1, region.maxY <= 1 else {
                    throw OCRFailure.invalidGeometry
                }
                visionRegion = CGRect(x: region.minX, y: 1 - region.maxY,
                                      width: region.width, height: region.height)
            }

            func configuredRequest() throws -> VNRecognizeTextRequest {
                let request = VNRecognizeTextRequest()
                request.revision = VNRecognizeTextRequestRevision3
                request.recognitionLevel = .accurate
                request.usesLanguageCorrection = false
                request.automaticallyDetectsLanguage = false
                request.minimumTextHeight = 0.006
                request.preferBackgroundProcessing = true
                let languages = ["zh-Hans", "en-US"]
                let supported = try request.supportedRecognitionLanguages()
                guard languages.allSatisfy({ supported.contains($0) }) else {
                    throw OCRFailure.unsupportedLanguages
                }
                request.recognitionLanguages = languages
                if let visionRegion { request.regionOfInterest = visionRegion }
                return request
            }

            var observations: [VNRecognizedTextObservation]?
            // Vision can transiently reject the first request immediately after model warmup.
            // Retry once with a fresh request/handler; this remains a local, idempotent read.
            for attempt in 0..<2 {
                let request = try configuredRequest()
                do {
                    try VNImageRequestHandler(cgImage: image, orientation: .up, options: [:])
                        .perform([request])
                    if let results = request.results {
                        observations = results
                        break
                    }
                } catch {
                    if attempt == 0 {
                        Thread.sleep(forTimeInterval: 0.05)
                        continue
                    }
                }
            }
            guard let observations else { throw OCRFailure.recognitionFailed }
            let candidates = observations.prefix(maxLines).compactMap { observation -> OCRLine? in
                guard let first = observation.topCandidates(1).first else { return nil }
                var box = observation.boundingBox
                if let roi = visionRegion {
                    box = CGRect(x: roi.minX + box.minX * roi.width,
                                 y: roi.minY + box.minY * roi.height,
                                 width: box.width * roi.width,
                                 height: box.height * roi.height)
                }
                return OCRLine(text: first.string, confidence: first.confidence, bounds: box)
            }
            return bounded(candidates, overflow: observations.count > maxLines,
                           missingCandidates: candidates.count != min(observations.count, maxLines))
        }
    }

    /// Pay Vision's model-load cost on a blank in-memory image. No chat pixels, file or network.
    public static func warmup() -> OCRWarmupSummary {
        let started = ProcessInfo.processInfo.systemUptime
        guard let context = CGContext(data: nil, width: 64, height: 64, bitsPerComponent: 8,
                                      bytesPerRow: 64 * 4, space: CGColorSpaceCreateDeviceRGB(),
                                      bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else {
            return OCRWarmupSummary(elapsedMilliseconds: 0, lineCount: 0, succeeded: false)
        }
        context.setFillColor(CGColor(gray: 1, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 64, height: 64))
        guard let image = context.makeImage() else {
            return OCRWarmupSummary(elapsedMilliseconds: 0, lineCount: 0, succeeded: false)
        }
        do {
            let snapshot = try recognize(image)
            let elapsed = max(0, Int(((ProcessInfo.processInfo.systemUptime - started) * 1000).rounded()))
            return OCRWarmupSummary(elapsedMilliseconds: elapsed,
                                    lineCount: snapshot.statistics.lineCount, succeeded: true)
        } catch {
            let elapsed = max(0, Int(((ProcessInfo.processInfo.systemUptime - started) * 1000).rounded()))
            return OCRWarmupSummary(elapsedMilliseconds: elapsed, lineCount: 0, succeeded: false)
        }
    }
}
