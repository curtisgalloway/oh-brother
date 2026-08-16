// SPDX-FileCopyrightText: 2026 Curtis Galloway
// SPDX-License-Identifier: Apache-2.0

// C-ABI Swift shim: IOBluetooth RFCOMM transport for the P-touch Cube.
//
// The behavior here ports the hardware-verified Python transport
// (protocol.py, retired — see git history) exactly:
//  - the Cube's SDP "Serial" service is RFCOMM channel 1;
//  - openRFCOMMChannelSync only works on the process main thread
//    (kIOReturnError elsewhere), so callers must stay on it;
//  - a stale half-open baseband link makes every open fail until it is
//    dropped — close the connection on a failed open and retry once;
//  - reopening within ~1 s of a close attaches to the dying session
//    and times out, so a minimum reopen gap is enforced;
//  - incoming bytes arrive via delegate callback; reads on the main
//    thread must pump the runloop for the callback to fire.

import Foundation
import IOBluetooth

private let kChannelId: BluetoothRFCOMMChannelID = 1
private let kReopenGap: TimeInterval = 1.5

private final class GapClock {
    static let shared = GapClock()
    private let lock = NSLock()
    private var lastClose = Date.distantPast

    func waitForGap() {
        lock.lock()
        let since = Date().timeIntervalSince(lastClose)
        lock.unlock()
        if since < kReopenGap {
            Thread.sleep(forTimeInterval: kReopenGap - since)
        }
    }

    func stampClose() {
        lock.lock()
        lastClose = Date()
        lock.unlock()
    }
}

private final class Conn: NSObject, IOBluetoothRFCOMMChannelDelegate {
    let device: IOBluetoothDevice
    var channel: IOBluetoothRFCOMMChannel?
    var mtu: Int = 127
    let cond = NSCondition()
    var buf = Data()

    init(device: IOBluetoothDevice) {
        self.device = device
    }

    func rfcommChannelData(
        _ rfcommChannel: IOBluetoothRFCOMMChannel!,
        data dataPointer: UnsafeMutableRawPointer!,
        length dataLength: Int
    ) {
        cond.lock()
        buf.append(Data(bytes: dataPointer, count: dataLength))
        cond.broadcast()
        cond.unlock()
    }

    func openOnce() -> IOReturn {
        var chan: IOBluetoothRFCOMMChannel?
        let ret = device.openRFCOMMChannelSync(
            &chan, withChannelID: kChannelId, delegate: self)
        if ret == kIOReturnSuccess, let chan = chan {
            channel = chan
            mtu = max(Int(chan.getMTU()), 1)
        }
        return ret
    }

    /// Wait for buffered bytes, pumping the runloop when on the main
    /// thread (delegate callbacks are delivered there) and waiting on
    /// the condition otherwise.
    func read(into out: UnsafeMutablePointer<UInt8>, cap: Int, timeout: Double) -> Int {
        let deadline = Date(timeIntervalSinceNow: timeout)
        while true {
            cond.lock()
            if !buf.isEmpty {
                let n = min(buf.count, cap)
                buf.copyBytes(to: out, count: n)
                buf.removeFirst(n)
                cond.unlock()
                return n
            }
            cond.unlock()
            if Date() >= deadline { return 0 }
            if Thread.isMainThread {
                RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
            } else {
                cond.lock()
                cond.wait(until: Date(timeIntervalSinceNow: 0.05))
                cond.unlock()
            }
        }
    }
}

private func fillError(_ msg: String, _ err: UnsafeMutablePointer<CChar>?, _ cap: Int) {
    guard let err = err, cap > 0 else { return }
    let bytes = Array(msg.utf8.prefix(cap - 1))
    for (i, b) in bytes.enumerated() { err[i] = CChar(bitPattern: b) }
    err[bytes.count] = 0
}

private func pairedCubes() -> [IOBluetoothDevice] {
    let paired = IOBluetoothDevice.pairedDevices() as? [IOBluetoothDevice] ?? []
    return paired
        .filter { ($0.name ?? "").hasPrefix("PT-P300BT") }
        .sorted { ($0.name ?? "") < ($1.name ?? "") }
}

/// Newline-joined names of paired PT-P300BT devices. Returns the byte
/// count written (which may be 0), truncating to the buffer.
@_cdecl("ptbt_list")
public func ptbt_list(_ out: UnsafeMutablePointer<CChar>, _ cap: Int) -> Int {
    let joined = pairedCubes().compactMap { $0.name }.joined(separator: "\n")
    fillError(joined, out, cap)  // same bounded C-string copy
    return min(joined.utf8.count, cap - 1)
}

/// Open the paired Cube called `name` (or the first one when nil).
/// Returns an opaque handle, or nil with `err` filled in.
@_cdecl("ptbt_open")
public func ptbt_open(
    _ name: UnsafePointer<CChar>?,
    _ err: UnsafeMutablePointer<CChar>?,
    _ errCap: Int
) -> UnsafeMutableRawPointer? {
    let wanted = name.map { String(cString: $0) }
    let cubes = pairedCubes()
    let device: IOBluetoothDevice?
    if let wanted = wanted {
        device = cubes.first { $0.name == wanted }
    } else {
        device = cubes.first
    }
    guard let device = device else {
        let what = wanted.map { "no paired Bluetooth printer called \($0)" }
            ?? "no paired PT-P300BT (pair the Cube in System Settings > Bluetooth)"
        fillError(what, err, errCap)
        return nil
    }

    GapClock.shared.waitForGap()
    let conn = Conn(device: device)
    var ret = conn.openOnce()
    if ret != kIOReturnSuccess {
        // Drop a possibly-stale baseband link and try once more.
        device.closeConnection()
        Thread.sleep(forTimeInterval: 1.0)
        ret = conn.openOnce()
    }
    if ret != kIOReturnSuccess {
        device.closeConnection()
        fillError(
            String(
                format: "could not reach %@ over Bluetooth (IOReturn 0x%x) — the "
                    + "Cube powers itself off when idle; press its power button",
                device.name ?? "PT-P300BT", UInt32(bitPattern: ret)),
            err, errCap)
        return nil
    }
    return Unmanaged.passRetained(conn).toOpaque()
}

@_cdecl("ptbt_write")
public func ptbt_write(
    _ handle: UnsafeMutableRawPointer,
    _ data: UnsafePointer<UInt8>,
    _ len: Int,
    _ err: UnsafeMutablePointer<CChar>?,
    _ errCap: Int
) -> Int32 {
    let conn = Unmanaged<Conn>.fromOpaque(handle).takeUnretainedValue()
    guard let channel = conn.channel else {
        fillError("channel is closed", err, errCap)
        return -1
    }
    var off = 0
    while off < len {
        let n = min(conn.mtu, len - off)
        // writeSync blocks on RFCOMM flow control while the printer
        // drains its buffer at mechanical speed — that is our pacing.
        let ret = channel.writeSync(
            UnsafeMutableRawPointer(mutating: data + off), length: UInt16(n))
        if ret != kIOReturnSuccess {
            fillError(
                String(format: "Bluetooth write failed (IOReturn 0x%x)",
                       UInt32(bitPattern: ret)),
                err, errCap)
            return -1
        }
        off += n
    }
    return 0
}

/// One bounded read attempt; 0 bytes when nothing arrived in time.
@_cdecl("ptbt_read")
public func ptbt_read(
    _ handle: UnsafeMutableRawPointer,
    _ out: UnsafeMutablePointer<UInt8>,
    _ cap: Int,
    _ timeoutSeconds: Double
) -> Int {
    let conn = Unmanaged<Conn>.fromOpaque(handle).takeUnretainedValue()
    return conn.read(into: out, cap: cap, timeout: timeoutSeconds)
}

@_cdecl("ptbt_drain")
public func ptbt_drain(_ handle: UnsafeMutableRawPointer) {
    let conn = Unmanaged<Conn>.fromOpaque(handle).takeUnretainedValue()
    conn.cond.lock()
    conn.buf.removeAll()
    conn.cond.unlock()
}

@_cdecl("ptbt_close")
public func ptbt_close(_ handle: UnsafeMutableRawPointer) {
    let conn = Unmanaged<Conn>.fromOpaque(handle).takeRetainedValue()
    conn.channel?.close()
    conn.device.closeConnection()
    GapClock.shared.stampClose()
}
