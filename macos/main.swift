// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// Oh, Brother — native shell. A WKWebView window around the label-web
// UI, owning the server lifecycle: spawn the bundled Rust `label-web`
// binary (Contents/Resources/bin) on launch (unless one is already
// running, in which case attach and leave it alone on quit), wait for
// readiness, terminate what we spawned.

import Cocoa
import WebKit

let port = 8763

let splashHTML = """
<!doctype html><html><head><meta charset="utf-8"><style>
  body { background:#141517; color:#8b8e96; font: 15px "Avenir Next", sans-serif;
         display:flex; align-items:center; justify-content:center; height:100vh; margin:0; }
  .msg { text-align:center; }
  .msg b { color:#ffcf24; font-family:"DIN Alternate", sans-serif;
           letter-spacing:.1em; text-transform:uppercase; }
</style></head><body><div class="msg"><b>Oh, Brother</b><br><br>starting the label engine…</div></body></html>
"""

class AppDelegate: NSObject, NSApplicationDelegate {
    var window: NSWindow!
    var webView: WKWebView!
    var serverProcess: Process?
    var spawnedServer = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        buildMenu()
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 980, height: 780),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered, defer: false)
        window.title = "Oh, Brother"
        window.minSize = NSSize(width: 640, height: 480)
        window.setFrameAutosaveName("OhBrotherMain")
        webView = WKWebView(frame: window.contentView!.bounds)
        webView.autoresizingMask = [.width, .height]
        window.contentView!.addSubview(webView)
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        webView.loadHTMLString(splashHTML, baseURL: nil)
        probeServer { alive in
            if alive {
                self.loadApp()
            } else {
                self.spawnServer()
                self.waitForServer(deadline: Date().addingTimeInterval(20))
            }
        }
    }

    func probeServer(_ done: @escaping (Bool) -> Void) {
        var request = URLRequest(url: URL(string: "http://127.0.0.1:\(port)/api/meta")!)
        request.timeoutInterval = 1
        URLSession.shared.dataTask(with: request) { _, response, _ in
            let ok = (response as? HTTPURLResponse)?.statusCode == 200
            DispatchQueue.main.async { done(ok) }
        }.resume()
    }

    func spawnServer() {
        guard
            let resources = Bundle.main.resourcePath,
            FileManager.default.fileExists(atPath: resources + "/bin/label-web")
        else {
            showError("This build has no bundled label-web.\nRebuild the app with macos/build.sh from the repo.")
            return
        }
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: resources + "/bin/label-web")
        proc.arguments = ["--no-browser", "--port", "\(port)"]
        do {
            try proc.run()
            serverProcess = proc
            spawnedServer = true
        } catch {
            showError("Couldn't start the label server:\n\(error.localizedDescription)")
        }
    }

    func waitForServer(deadline: Date) {
        probeServer { alive in
            if alive {
                self.loadApp()
            } else if Date() < deadline {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                    self.waitForServer(deadline: deadline)
                }
            } else {
                self.showError("The label server didn't come up within 20 seconds.\nTry running Contents/Resources/bin/label-web from the app bundle to see why.")
            }
        }
    }

    func loadApp() {
        webView.load(URLRequest(url: URL(string: "http://127.0.0.1:\(port)/")!))
    }

    func showError(_ message: String) {
        let alert = NSAlert()
        alert.messageText = "Oh, Brother"
        alert.informativeText = message
        alert.runModal()
    }

    @objc func reloadPage() {
        if webView.url == nil {
            loadApp()
        } else {
            webView.reload()
        }
    }

    // Symlink the bundled CLI (Contents/Resources/bin/label) into
    // ~/.local/bin so `label` works from any shell — and `label --skill`
    // lets AI agents discover how to drive the printer.
    @objc func installCLITool() {
        let fm = FileManager.default
        guard let resources = Bundle.main.resourcePath,
              fm.fileExists(atPath: resources + "/bin/label") else {
            showError("This build has no bundled CLI.\nRebuild the app with macos/build.sh from the repo.")
            return
        }
        let source = resources + "/bin/label"
        let binDir = fm.homeDirectoryForCurrentUser.appendingPathComponent(".local/bin")
        let dest = binDir.appendingPathComponent("label")
        do {
            try fm.createDirectory(at: binDir, withIntermediateDirectories: true)
            if let type = try? fm.attributesOfItem(atPath: dest.path)[.type] as? FileAttributeType {
                if type == .typeSymbolicLink {
                    try fm.removeItem(at: dest)  // replace a previous install
                } else {
                    showError("\(dest.path) already exists and isn't a symlink — not touching it.")
                    return
                }
            }
            try fm.createSymbolicLink(at: dest, withDestinationURL: URL(fileURLWithPath: source))
            let alert = NSAlert()
            alert.messageText = "Command installed"
            alert.informativeText = """
                \(dest.path) → the app's bundled label tool.

                Make sure ~/.local/bin is on your PATH, then try `label --help`. \
                AI agents can run `label --skill` for the full usage guide.
                """
            alert.runModal()
        } catch {
            showError("Couldn't install the symlink:\n\(error.localizedDescription)")
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        if spawnedServer { serverProcess?.terminate() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func buildMenu() {
        let mainMenu = NSMenu()

        let appMenuItem = NSMenuItem()
        mainMenu.addItem(appMenuItem)
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "About Oh, Brother",
                        action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(NSMenuItem.separator())
        let installCLI = NSMenuItem(title: "Install 'label' Command in PATH…",
                                    action: #selector(installCLITool), keyEquivalent: "")
        installCLI.target = self
        appMenu.addItem(installCLI)
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(withTitle: "Hide Oh, Brother", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(NSMenuItem.separator())
        appMenu.addItem(withTitle: "Quit Oh, Brother", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appMenuItem.submenu = appMenu

        let editMenuItem = NSMenuItem()
        mainMenu.addItem(editMenuItem)
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        editMenu.addItem(withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "Z")
        editMenu.addItem(NSMenuItem.separator())
        editMenu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editMenu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editMenu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editMenuItem.submenu = editMenu

        let viewMenuItem = NSMenuItem()
        mainMenu.addItem(viewMenuItem)
        let viewMenu = NSMenu(title: "View")
        let reload = NSMenuItem(title: "Reload", action: #selector(reloadPage), keyEquivalent: "r")
        reload.target = self
        viewMenu.addItem(reload)
        viewMenuItem.submenu = viewMenu

        let windowMenuItem = NSMenuItem()
        mainMenu.addItem(windowMenuItem)
        let windowMenu = NSMenu(title: "Window")
        windowMenu.addItem(withTitle: "Minimize", action: #selector(NSWindow.miniaturize(_:)), keyEquivalent: "m")
        windowMenu.addItem(withTitle: "Zoom", action: #selector(NSWindow.zoom(_:)), keyEquivalent: "")
        windowMenu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        windowMenuItem.submenu = windowMenu
        NSApp.windowsMenu = windowMenu

        NSApp.mainMenu = mainMenu
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
