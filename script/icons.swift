// Makes the icons of the packages from crates/migpad/resources/icon.svg:
//
// - macOS: resources/macos/MigPad.icns — the icon as drawn, with the shadow of macOS icons;
// - Windows: resources/windows/migpad.ico — the body alone, with a small margin;
// - Linux: resources/linux/icons/hicolor/… — the same as for Windows, PNG and SVG.
//
// The files it makes are kept in the repository. After changing the icon, run on macOS:
//
//     swift script/icons.swift
import AppKit

let resources = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent().deletingLastPathComponent()
    .appendingPathComponent("crates/migpad/resources")
let source = resources.appendingPathComponent("icon.svg")
guard let icon = NSImage(contentsOf: source) else { fatalError("cannot read \(source.path)") }

/// What icons of Windows and Linux show of the canvas, 1024 × 1024: the body, 824 × 824 at 100, 100,
/// with a margin of 32.
let flatBox = NSRect(x: 68, y: 68, width: 888, height: 888)

enum Style { case macOS, flat }

func render(_ size: Int, _ style: Style) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4,
        hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current!.imageInterpolation = .high
    let s = CGFloat(size)
    switch style {
    case .macOS:
        // The shadow is drawn here: AppKit draws no filters of SVG.
        let k = s / 1024
        let shadow = NSShadow()
        shadow.shadowOffset = NSSize(width: 0, height: -10 * k)
        shadow.shadowBlurRadius = 20 * k
        shadow.shadowColor = NSColor.black.withAlphaComponent(0.32)
        shadow.set()
        icon.draw(in: NSRect(x: 0, y: 0, width: s, height: s))
    case .flat:
        let k = s / flatBox.width
        icon.draw(in: NSRect(x: -flatBox.minX * k, y: -flatBox.minY * k, width: 1024 * k, height: 1024 * k))
    }
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

func png(_ rep: NSBitmapImageRep) -> Data {
    rep.representation(using: .png, properties: [:])!
}

func write(_ data: Data, to path: String) {
    let url = resources.appendingPathComponent(path)
    try! FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try! data.write(to: url)
}

extension Data {
    mutating func u16(_ value: Int) { append(contentsOf: [UInt8(value & 0xff), UInt8(value >> 8 & 0xff)]) }
    mutating func u32(_ value: Int) { u16(value & 0xffff); u16(value >> 16 & 0xffff) }
}

/// A 32-bit bitmap of an icon of Windows: the header, the pixels from the bottom row up in BGRA with
/// alpha not premultiplied, then the mask of the transparent pixels.
func dib(_ rep: NSBitmapImageRep) -> Data {
    let width = rep.pixelsWide, height = rep.pixelsHigh
    let maskRow = (width + 31) / 32 * 4
    var data = Data()
    data.u32(40); data.u32(width); data.u32(2 * height); data.u16(1); data.u16(32)
    data.u32(0); data.u32(width * height * 4 + maskRow * height); data.u32(0); data.u32(0); data.u32(0); data.u32(0)
    var mask = [UInt8](repeating: 0, count: maskRow * height)
    var pixel = [Int](repeating: 0, count: 4)
    for row in 0..<height {
        // The rows of the bitmap go up; those of the image (y) go down.
        let y = height - 1 - row
        for x in 0..<width {
            rep.getPixel(&pixel, atX: x, y: y)
            let alpha = pixel[3]
            let straight = { (c: Int) -> UInt8 in alpha == 0 ? 0 : UInt8(Swift.min(255, (c * 255 + alpha / 2) / alpha)) }
            data.append(contentsOf: [straight(pixel[2]), straight(pixel[1]), straight(pixel[0]), UInt8(alpha)])
            if alpha == 0 { mask[row * maskRow + x / 8] |= 0x80 >> (x % 8) }
        }
    }
    data.append(contentsOf: mask)
    return data
}

/// An icon of Windows: the sizes below 256 as bitmaps, which every part of Windows reads, 256 as PNG.
func ico(_ sizes: [Int]) -> Data {
    let images = sizes.map { size in (size, size >= 256 ? png(render(size, .flat)) : dib(render(size, .flat))) }
    var data = Data()
    data.u16(0); data.u16(1); data.u16(images.count)
    var offset = 6 + 16 * images.count
    for (size, image) in images {
        // 0 is 256.
        data.append(contentsOf: [UInt8(size % 256), UInt8(size % 256), 0, 0])
        data.u16(1); data.u16(32); data.u32(image.count); data.u32(offset)
        offset += image.count
    }
    for (_, image) in images { data.append(image) }
    return data
}

// macOS: an icon set, made into the .icns by iconutil.
let iconset = FileManager.default.temporaryDirectory.appendingPathComponent("MigPad.iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let name = "icon_\(points)x\(points)\(scale == 2 ? "@2x" : "").png"
        try! png(render(points * scale, .macOS)).write(to: iconset.appendingPathComponent(name))
    }
}
try! FileManager.default.createDirectory(at: resources.appendingPathComponent("macos"), withIntermediateDirectories: true)
let iconutil = Process()
iconutil.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
iconutil.arguments = ["-c", "icns", iconset.path, "-o", resources.appendingPathComponent("macos/MigPad.icns").path]
try! iconutil.run()
iconutil.waitUntilExit()
precondition(iconutil.terminationStatus == 0, "iconutil failed")
try? FileManager.default.removeItem(at: iconset)

// Windows.
write(ico([16, 20, 24, 32, 40, 48, 64, 256]), to: "windows/migpad.ico")

// Linux: the sizes of the hicolor theme and the SVG with the box of the flat icons.
for size in [16, 24, 32, 48, 64, 128, 256, 512] {
    write(png(render(size, .flat)), to: "linux/icons/hicolor/\(size)x\(size)/apps/com.migpad.MigPad.png")
}
let canvas = #"width="1024" height="1024" viewBox="0 0 1024 1024""#
let svg = try! String(contentsOf: source, encoding: .utf8)
precondition(svg.contains(canvas), "icon.svg has another canvas than 1024 × 1024")
let box = "width=\"\(Int(flatBox.width))\" height=\"\(Int(flatBox.height))\" viewBox=\"\(Int(flatBox.minX)) \(Int(flatBox.minY)) \(Int(flatBox.width)) \(Int(flatBox.height))\""
write(svg.replacingOccurrences(of: canvas, with: box).data(using: .utf8)!,
      to: "linux/icons/hicolor/scalable/apps/com.migpad.MigPad.svg")
