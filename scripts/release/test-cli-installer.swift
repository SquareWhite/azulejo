// Tests for apps/macos/Sources/Azulejo/CLIInstaller.swift, compiled together
// with it by test-cli-installer.sh. Temp directories only: never touches the
// user's real PATH directories.
import Foundation

var failures = 0
func check(_ ok: Bool, _ what: String, line: Int = #line) {
    if ok { print("ok   \(what)") } else { print("FAIL \(what) (line \(line))"); failures += 1 }
}

let fm = FileManager.default
let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("azj-cli-test-\(getpid())")
try? fm.removeItem(at: tmp)
let home = tmp.appendingPathComponent("home")
let app = tmp.appendingPathComponent("Azulejo.app/Contents/MacOS")
try! fm.createDirectory(at: home, withIntermediateDirectories: true)
try! fm.createDirectory(at: app, withIntermediateDirectories: true)
let source = app.appendingPathComponent("azj")
fm.createFile(atPath: source.path, contents: Data("#!/bin/sh\necho azj-test\n".utf8), attributes: [.posixPermissions: 0o755])
let installer = CLIInstaller()
let localBin = home.appendingPathComponent(".local/bin").path

// Missing source.
do {
    _ = try installer.plan(source: tmp.appendingPathComponent("nope"), loginPath: [], home: home, override: nil)
    check(false, "missing source throws")
} catch { check(error as? CLIInstaller.Failure != nil, "missing source throws") }

// ~/.local/bin preferred when on the login PATH.
let p1 = try! installer.plan(source: source, loginPath: ["/usr/bin", localBin], home: home, override: nil)
check(p1.link.path == localBin + "/azj", "~/.local/bin chosen when on PATH")
check(!p1.needsPathHint, "no PATH hint when on PATH")
check(p1.existing == .none, "nothing there yet")

// Override wins, and gets a PATH hint when not on PATH.
let target = tmp.appendingPathComponent("bin")
let p2 = try! installer.plan(source: source, loginPath: ["/usr/bin"], home: home, override: target.path)
check(p2.link.path == target.path + "/azj", "AZJ_CLI_INSTALL_DIR override used")
check(p2.needsPathHint, "PATH hint when target not on PATH")

// Install creates the directory and the link; the link runs.
try! installer.install(p2)
check((try? fm.destinationOfSymbolicLink(atPath: p2.link.path)) == source.path, "symlink points at bundled azj")
let run = Process(); run.executableURL = p2.link; let out = Pipe(); run.standardOutput = out
try! run.run(); run.waitUntilExit()
check(String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self) == "azj-test\n", "linked azj runs")

// Re-plan sees it as current; install is a no-op.
let p3 = try! installer.plan(source: source, loginPath: [], home: home, override: target.path)
check(p3.existing == .current, "existing link detected as current")
try! installer.install(p3)

// A link to an older app is replaced.
try! fm.removeItem(at: p2.link)
try! fm.createSymbolicLink(atPath: p2.link.path, withDestinationPath: "/Applications/Old.app/Contents/MacOS/azj")
let p4 = try! installer.plan(source: source, loginPath: [], home: home, override: target.path)
check(p4.existing == .otherLink("/Applications/Old.app/Contents/MacOS/azj"), "other link detected")
try! installer.install(p4)
check((try? fm.destinationOfSymbolicLink(atPath: p4.link.path)) == source.path, "other link replaced")

// A regular file is never replaced.
try! fm.removeItem(at: p2.link)
fm.createFile(atPath: p2.link.path, contents: Data("mine".utf8))
let p5 = try! installer.plan(source: source, loginPath: [], home: home, override: target.path)
check(p5.existing == .file, "regular file detected")
do { try installer.install(p5); check(false, "regular file refused") }
catch { check(error as? CLIInstaller.Failure == .wouldClobber(p5.link.path), "regular file refused") }
check(String(decoding: fm.contents(atPath: p5.link.path) ?? Data(), as: UTF8.self) == "mine", "regular file untouched")

// Login-shell PATH lookup returns something.
check(!CLIInstaller.loginShellPath().isEmpty, "login shell PATH read")

try? fm.removeItem(at: tmp)
print(failures == 0 ? "all passed" : "\(failures) failed")
exit(failures == 0 ? 0 : 1)
