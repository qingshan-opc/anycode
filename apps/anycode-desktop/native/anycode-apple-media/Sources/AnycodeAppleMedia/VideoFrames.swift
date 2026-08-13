import AVFoundation
import AppKit
import Foundation

enum VideoFramesError: LocalizedError {
    case fileMissing(String)
    case noVideoTrack
    case exportFailed(String)

    var errorDescription: String? {
        switch self {
        case .fileMissing(let path):
            return "Video file not found: \(path)"
        case .noVideoTrack:
            return "No video track in file"
        case .exportFailed(let why):
            return "Audio export failed: \(why)"
        }
    }
}

struct VideoFramesResult {
    let frames: [String]
    let duration: Double
    let audioPath: String?
}

/// Uniformly sample frames (≈1 frame / 2s, capped) as JPEG (q70, short side
/// ≤ maxDimension), and export the audio track to m4a when present.
func extractVideoFrames(
    videoPath: String,
    outputDir: String,
    maxFrames: Int,
    maxDimension: Int
) throws -> VideoFramesResult {
    guard FileManager.default.fileExists(atPath: videoPath) else {
        throw VideoFramesError.fileMissing(videoPath)
    }
    let url = URL(fileURLWithPath: videoPath)
    let asset = AVURLAsset(url: url)
    let durationSeconds = CMTimeGetSeconds(asset.duration)
    guard durationSeconds.isFinite, durationSeconds > 0 else {
        throw VideoFramesError.noVideoTrack
    }
    guard !asset.tracks(withMediaType: .video).isEmpty else {
        throw VideoFramesError.noVideoTrack
    }

    try FileManager.default.createDirectory(atPath: outputDir, withIntermediateDirectories: true)

    // ~1 frame per 2s, capped at maxFrames.
    let count = max(1, min(maxFrames, Int(ceil(durationSeconds / 2.0))))
    let generator = AVAssetImageGenerator(asset: asset)
    generator.appliesPreferredTrackTransform = true
    generator.requestedTimeToleranceBefore = CMTime(seconds: 0.5, preferredTimescale: 600)
    generator.requestedTimeToleranceAfter = CMTime(seconds: 0.5, preferredTimescale: 600)

    var frames: [String] = []
    for i in 0..<count {
        let t = durationSeconds * (Double(i) + 0.5) / Double(count)
        let time = CMTime(seconds: t, preferredTimescale: 600)
        guard let cgImage = try? generator.copyCGImage(at: time, actualTime: nil) else {
            continue
        }
        let path = (outputDir as NSString).appendingPathComponent(
            String(format: "frame-%03d.jpg", frames.count)
        )
        if let data = jpegData(from: cgImage, maxDimension: maxDimension) {
            try data.write(to: URL(fileURLWithPath: path), options: .atomic)
            frames.append(path)
        }
    }

    // Audio track → m4a for the STT stage.
    var audioPath: String? = nil
    if !asset.tracks(withMediaType: .audio).isEmpty,
       let session = AVAssetExportSession(asset: asset, presetName: AVAssetExportPresetAppleM4A) {
        let outURL = URL(fileURLWithPath: outputDir).appendingPathComponent("audio.m4a")
        try? FileManager.default.removeItem(at: outURL)
        session.outputURL = outURL
        session.outputFileType = .m4a
        var done = false
        session.exportAsynchronously { done = true }
        try pumpRunLoopUntil({ done }, timeout: 180)
        if session.status == .completed {
            audioPath = outURL.path
        }
        // A failed audio export must not sink frame delivery — frames alone
        // still serve vision models; the transcript stage just skips.
    }

    return VideoFramesResult(frames: frames, duration: durationSeconds, audioPath: audioPath)
}

/// JPEG (q70) with the short side clamped to `maxDimension` pixels.
private func jpegData(from cgImage: CGImage, maxDimension: Int) -> Data? {
    let w = cgImage.width
    let h = cgImage.height
    let shortSide = min(w, h)
    guard shortSide > 0 else { return nil }
    let scale = min(1.0, Double(maxDimension) / Double(shortSide))
    let targetW = max(1, Int((Double(w) * scale).rounded()))
    let targetH = max(1, Int((Double(h) * scale).rounded()))

    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: targetW,
        pixelsHigh: targetH,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ) else { return nil }

    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high
    NSRect(x: 0, y: 0, width: targetW, height: targetH).fill()
    let nsImage = NSImage(cgImage: cgImage, size: NSSize(width: w, height: h))
    nsImage.draw(in: NSRect(x: 0, y: 0, width: targetW, height: targetH))
    NSGraphicsContext.restoreGraphicsState()

    return rep.representation(using: .jpeg, properties: [.compressionFactor: 0.7])
}
